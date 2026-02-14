use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::{presets::UTF8_FULL, Cell, Table};
use gauge_calculator::{
    analysis::{classify_competitors, ConcentrationAnalyzer},
    calculator::{compare_strategies, AcquisitionStrategy, StrategyRequest},
    config::{default_config_path, AppConfig},
    live::{LiveDataClient, LiveDataSnapshot},
    output::{
        concentration_csv, render_concentration_table, render_epoch_text, render_status_table,
        render_strategy_table, strategy_csv, to_json, OutputFormat,
    },
    snapshot::{resolve_path as resolve_snapshot_path, SnapshotStore, StoredSnapshot},
    types::{GaugeEligibility, SampleContext},
};
use serde::Serialize;
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "gauge-calc",
    version,
    about = "Vault Delegation Program Optimizer"
)]
struct Cli {
    #[arg(
        short = 'c',
        long = "config",
        default_value_os_t = default_config_path(),
        global = true
    )]
    config: PathBuf,

    #[arg(short = 'r', long = "rpc", global = true)]
    rpc: Option<String>,

    #[arg(
        short = 'o',
        long = "output",
        value_enum,
        default_value_t = OutputFormat::Table,
        global = true
    )]
    output: OutputFormat,

    #[arg(short = 'v', long = "verbose", global = true)]
    verbose: bool,

    #[arg(long = "live", global = true, default_value_t = false)]
    live: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum StrategyFilter {
    Lock,
    Buy,
    Hybrid,
    All,
}

#[derive(Debug, Subcommand)]
enum SnapshotCommand {
    Save,
    Load {
        #[arg(long)]
        epoch: Option<u32>,
    },
    List,
}

#[derive(Debug, Subcommand)]
enum Commands {
    Status {
        #[arg(long)]
        validator: Option<String>,
    },
    Calculate {
        #[arg(long)]
        target_sol: f64,
        #[arg(long)]
        validator: Option<String>,
        #[arg(long, value_enum, default_value_t = StrategyFilter::All)]
        strategy: StrategyFilter,
    },
    Compare {
        #[arg(long)]
        target_sol: f64,
        #[arg(long)]
        lock_years: Option<f64>,
        #[arg(long)]
        v_price: Option<f64>,
    },
    Concentration {
        #[arg(long)]
        epoch: Option<u32>,
        #[arg(long)]
        history: Option<u32>,
    },
    Whales {
        #[arg(long)]
        min_vev: Option<f64>,
        #[arg(long, default_value_t = false)]
        sells_on_votex: bool,
    },
    Competitors {
        #[arg(long)]
        validator: Option<String>,
        #[arg(long, default_value_t = 10)]
        top: usize,
    },
    Eligibility {
        #[arg(long)]
        validator: Option<String>,
    },
    Votex {
        #[arg(long)]
        epoch: Option<u32>,
        #[arg(long)]
        history: Option<u32>,
    },
    Epoch,
    Watch {
        #[arg(long)]
        validator: Option<String>,
        #[arg(long)]
        alert_displacement: Option<f64>,
        #[arg(long)]
        alert_deadline: Option<u32>,
    },
    Snapshot {
        #[command(subcommand)]
        action: SnapshotCommand,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut config = AppConfig::load_or_default(&cli.config)?;
    config.merge_cli_overrides(cli.rpc.clone());
    let mut context = SampleContext::new();
    let live_data = maybe_fetch_live_data(&cli, &config, &mut context)?;
    let sol_price_usdc = live_data
        .as_ref()
        .and_then(|snapshot| snapshot.sol_price_usdc)
        .unwrap_or(200.0);

    if cli.verbose {
        eprintln!("Using RPC: {}", config.rpc.url);
        eprintln!("Config path: {}", cli.config.display());
        eprintln!("Live mode: {}", cli.live);
        if let Some(live) = &live_data {
            eprintln!("Live warnings: {}", live.warnings.len());
        }
    }

    match cli.command {
        Commands::Status { validator } => {
            if let Some(target) = validator {
                let maybe = context
                    .gauge
                    .validators
                    .iter()
                    .find(|v| v.vote_account == target || v.name.eq_ignore_ascii_case(&target));
                if let Some(v) = maybe {
                    emit(
                        cli.output,
                        format!(
                            "Validator: {}\nVote account: {}\nveV: {:.0}\nShare: {:.2}%\nProjected SOL: {:.0}\nEligible: {}",
                            v.name,
                            v.vote_account,
                            v.vev_weight,
                            v.vote_share * 100.0,
                            v.projected_sol,
                            v.eligible
                        ),
                        v,
                        Some(format!(
                            "validator,vote_account,vev_weight,vote_share,projected_sol,eligible\n{},{},{:.6},{:.6},{:.6},{}\n",
                            v.name, v.vote_account, v.vev_weight, v.vote_share, v.projected_sol, v.eligible
                        )),
                    )?;
                } else {
                    return Err(anyhow!("validator not found: {target}"));
                }
            } else {
                let csv = status_csv(&context);
                let mut status_text = render_status_table(&context.pool, &context.gauge);
                if let Some(live) = &live_data {
                    status_text.push_str("\n\n");
                    status_text.push_str(&live_data_table(live));
                }
                emit(
                    cli.output,
                    status_text,
                    &json!({"context": context, "live": live_data}),
                    Some(csv),
                )?;
            }
        }
        Commands::Calculate {
            target_sol,
            validator: _,
            strategy,
        } => {
            let request = StrategyRequest {
                target_sol,
                gauge_reserve_sol: context.pool.gauge_reserve_sol,
                total_vev: context.gauge.total_vev_voting,
                pool_apy: context.pool.apy,
                sol_price_usdc,
                votex_clearing_price: context.votex.clearing_price_per_vev,
                v_price_usdc: context.vtoken.price_usdc,
                lock_years: 3.0,
            };
            let mut comparison = compare_strategies(&request)?;
            filter_strategies(&mut comparison, strategy);
            emit(
                cli.output,
                render_strategy_table(&comparison),
                &comparison,
                Some(strategy_csv(&comparison)),
            )?;
        }
        Commands::Compare {
            target_sol,
            lock_years,
            v_price,
        } => {
            let request = StrategyRequest {
                target_sol,
                gauge_reserve_sol: context.pool.gauge_reserve_sol,
                total_vev: context.gauge.total_vev_voting,
                pool_apy: context.pool.apy,
                sol_price_usdc,
                votex_clearing_price: context.votex.clearing_price_per_vev,
                v_price_usdc: v_price.unwrap_or(context.vtoken.price_usdc),
                lock_years: lock_years.unwrap_or(3.0),
            };
            let comparison = compare_strategies(&request)?;
            emit(
                cli.output,
                render_strategy_table(&comparison),
                &comparison,
                Some(strategy_csv(&comparison)),
            )?;
        }
        Commands::Concentration { epoch, history } => {
            let report = ConcentrationAnalyzer::analyze(
                &context.gauge,
                context.votex.clearing_price_per_vev,
            );
            let extra = json!({
                "requested_epoch": epoch,
                "history_window": history,
                "live_mode": cli.live,
                "report": report,
            });
            emit(
                cli.output,
                render_concentration_table(&report),
                &extra,
                Some(concentration_csv(&report)),
            )?;
        }
        Commands::Whales {
            min_vev,
            sells_on_votex,
        } => {
            let whales = json!({
                "note": "Whale mapping requires on-chain locker/gauge account indexing; this MVP exposes command wiring and output support.",
                "filters": {
                    "min_vev": min_vev,
                    "sells_on_votex": sells_on_votex
                }
            });
            emit(
                cli.output,
                whales["note"].as_str().unwrap_or(""),
                &whales,
                None,
            )?;
        }
        Commands::Competitors { validator, top } => {
            let mut profiles = classify_competitors(&context.gauge, &context.votex.bids);
            if let Some(target) = validator {
                profiles.retain(|p| p.validator == target || p.name.eq_ignore_ascii_case(&target));
            }
            profiles.truncate(top);

            let text = competitor_table(&profiles);
            let csv = competitor_csv(&profiles);
            emit(cli.output, text, &profiles, Some(csv))?;
        }
        Commands::Eligibility { validator } => {
            let target = validator
                .or_else(|| config.validator.vote_account.clone())
                .ok_or_else(|| {
                    anyhow!("validator is required (or set [validator].vote_account in config)")
                })?;

            let entry = context
                .gauge
                .validators
                .iter()
                .find(|v| v.vote_account == target || v.name.eq_ignore_ascii_case(&target))
                .ok_or_else(|| anyhow!("validator not found: {target}"))?;
            let eligibility = GaugeEligibility {
                vote_account: entry.vote_account.clone(),
                eligible: entry.eligible,
                issues: entry.eligibility_issues.clone(),
                evaluated_over_epochs: 10,
            };
            let text = format!(
                "Validator: {}\nEligible: {}\nIssues: {}",
                entry.name,
                eligibility.eligible,
                if eligibility.issues.is_empty() {
                    "none".to_string()
                } else {
                    format!("{:?}", eligibility.issues)
                }
            );
            emit(cli.output, text, &eligibility, None)?;
        }
        Commands::Votex { epoch, history } => {
            let payload = json!({
                "requested_epoch": epoch,
                "history_window": history,
                "market": context.votex,
                "live_probe": live_data.as_ref().and_then(|d| d.votex_probe.clone()),
            });
            let text = format!(
                "Votex Epoch {}\nTotal veV for sale: {:.0}\nTotal USDC bid: {:.2}\nClearing price/veV: {:.6}",
                context.votex.epoch,
                context.votex.total_vev_for_sale,
                context.votex.total_usdc_bid,
                context.votex.clearing_price_per_vev
            );
            emit(cli.output, text, &payload, None)?;
        }
        Commands::Epoch => {
            let mut epoch_text = render_epoch_text(&context.gauge.epoch);
            if let Some(live) = &live_data {
                if let Some(epoch) = &live.rpc_epoch {
                    epoch_text.push_str(&format!(
                        "\n\nRPC Slot Progress: {}/{} (epoch {})",
                        epoch.slot_index, epoch.slots_in_epoch, epoch.epoch
                    ));
                }
            }
            emit(
                cli.output,
                epoch_text,
                &context.gauge.epoch,
                Some(format!(
                    "epoch,phase,start,deadline,end\n{},\"{}\",{},{},{}\n",
                    context.gauge.epoch.epoch_number,
                    phase_label(context.gauge.epoch.phase),
                    context.gauge.epoch.start,
                    context.gauge.epoch.vote_deadline,
                    context.gauge.epoch.end
                )),
            )?;
        }
        Commands::Watch {
            validator,
            alert_displacement,
            alert_deadline,
        } => {
            let payload = json!({
                "status": "configured",
                "validator": validator,
                "alert_displacement": alert_displacement,
                "alert_deadline": alert_deadline,
                "note": "Long-running watch loop intentionally omitted in cloud execution mode.",
            });
            emit(
                cli.output,
                "Watch configuration validated. Use periodic cron invocations for automated alerts.",
                &payload,
                None,
            )?;
        }
        Commands::Snapshot { action } => {
            let db_path = resolve_snapshot_path(&config.snapshot.db_path);
            let store = SnapshotStore::open(&config.snapshot.db_path)?;
            match action {
                SnapshotCommand::Save => {
                    let epoch = live_data
                        .as_ref()
                        .and_then(|l| l.rpc_epoch.as_ref())
                        .map(|e| e.epoch)
                        .unwrap_or(context.gauge.epoch.epoch_number as u64)
                        .min(u32::MAX as u64) as u32;
                    let payload = json!({
                        "context": context,
                        "live": live_data,
                    });
                    let saved = store.save_payload(epoch, "state", &payload)?;
                    let text = format!(
                        "Saved snapshot id={} epoch={} at {}\nDB: {}",
                        saved.id,
                        saved.epoch,
                        saved.fetched_at.format("%Y-%m-%d %H:%M:%S UTC"),
                        db_path.display()
                    );
                    emit(cli.output, text, &saved, None)?;
                }
                SnapshotCommand::Load { epoch } => {
                    let loaded = store.load("state", epoch)?;
                    let text = if let Some(snapshot) = &loaded {
                        format!(
                            "Loaded snapshot id={} epoch={} kind={}\nFetched: {}",
                            snapshot.id,
                            snapshot.epoch,
                            snapshot.snapshot_kind,
                            snapshot.fetched_at.format("%Y-%m-%d %H:%M:%S UTC")
                        )
                    } else {
                        "No matching snapshot found".to_string()
                    };
                    let payload = json!({
                        "db_path": db_path,
                        "requested_epoch": epoch,
                        "snapshot": loaded,
                    });
                    emit(cli.output, text, &payload, None)?;
                }
                SnapshotCommand::List => {
                    let snapshots = store.list(Some("state"), 50)?;
                    let text = snapshot_table(&snapshots, &db_path);
                    let csv = Some(snapshot_list_csv(&snapshots));
                    emit(cli.output, text, &snapshots, csv)?;
                }
            }
        }
    }

    Ok(())
}

fn phase_label(phase: gauge_calculator::types::VotexPhase) -> &'static str {
    match phase {
        gauge_calculator::types::VotexPhase::VoteBuying => "Vote Buying",
        gauge_calculator::types::VotexPhase::Voting => "Voting",
        gauge_calculator::types::VotexPhase::Distributing => "Distributing",
        gauge_calculator::types::VotexPhase::Committed => "Committed",
    }
}

fn filter_strategies(
    comparison: &mut gauge_calculator::calculator::StrategyComparison,
    strategy: StrategyFilter,
) {
    if matches!(strategy, StrategyFilter::All) {
        return;
    }

    comparison.strategies.retain(|s| match strategy {
        StrategyFilter::All => true,
        StrategyFilter::Lock => matches!(s.strategy, AcquisitionStrategy::LockVTokens { .. }),
        StrategyFilter::Buy => matches!(s.strategy, AcquisitionStrategy::BuyOnVotex { .. }),
        StrategyFilter::Hybrid => matches!(s.strategy, AcquisitionStrategy::Hybrid { .. }),
    });
    comparison.recommended = 0;
    comparison.recommendation_reason = "Filtered to requested strategy type".to_string();
}

fn emit<T: Serialize>(
    format: OutputFormat,
    table_text: impl AsRef<str>,
    json_value: &T,
    csv: Option<String>,
) -> Result<()> {
    match format {
        OutputFormat::Table => println!("{}", table_text.as_ref()),
        OutputFormat::Json => println!("{}", to_json(json_value)?),
        OutputFormat::Csv => {
            if let Some(csv) = csv {
                println!("{csv}");
            } else {
                println!("{}", to_json(json_value)?);
            }
        }
    }
    Ok(())
}

fn status_csv(sample: &SampleContext) -> String {
    let mut out =
        String::from("validator,vote_account,vev_weight,vote_share,projected_sol,eligible\n");
    for v in &sample.gauge.validators {
        out.push_str(&format!(
            "{},{},{:.6},{:.6},{:.6},{}\n",
            v.name, v.vote_account, v.vev_weight, v.vote_share, v.projected_sol, v.eligible
        ));
    }
    out
}

fn competitor_table(profiles: &[gauge_calculator::analysis::CompetitorProfile]) -> String {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).set_header(vec![
        "Validator",
        "Style",
        "veV Weight",
        "Votex Spend (USDC)",
    ]);
    for p in profiles {
        table.add_row(vec![
            Cell::new(&p.name),
            Cell::new(format!("{:?}", p.style)),
            Cell::new(format!("{:.0}", p.vev_weight)),
            Cell::new(format!("{:.2}", p.votex_spend_usdc)),
        ]);
    }
    table.to_string()
}

fn competitor_csv(profiles: &[gauge_calculator::analysis::CompetitorProfile]) -> String {
    let mut out = String::from("validator,name,style,vev_weight,votex_spend_usdc\n");
    for p in profiles {
        out.push_str(&format!(
            "{},{},{:?},{:.6},{:.6}\n",
            p.validator, p.name, p.style, p.vev_weight, p.votex_spend_usdc
        ));
    }
    out
}

fn maybe_fetch_live_data(
    cli: &Cli,
    config: &AppConfig,
    context: &mut SampleContext,
) -> Result<Option<LiveDataSnapshot>> {
    if !cli.live {
        return Ok(None);
    }

    let client = LiveDataClient::new()?;
    let snapshot = client.fetch_all(
        &config.rpc.url,
        &config.price.jupiter_api,
        &config.votex.scrape_url,
    );
    apply_live_overlay(context, &snapshot);
    Ok(Some(snapshot))
}

fn apply_live_overlay(context: &mut SampleContext, live: &LiveDataSnapshot) {
    if let Some(vote_accounts) = &live.vote_accounts {
        let total = vote_accounts.current_count + vote_accounts.delinquent_count;
        if total > 0 {
            context.pool.validator_count = total as u32;
        }
    }
    if let Some(epoch) = &live.rpc_epoch {
        context.gauge.epoch.epoch_number = epoch.epoch.min(u32::MAX as u64) as u32;
    }
}

fn live_data_table(live: &LiveDataSnapshot) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_header(vec!["Live Signal", "Value"]);
    table.add_row(vec![
        Cell::new("Fetched At"),
        Cell::new(live.fetched_at.format("%Y-%m-%d %H:%M:%S UTC").to_string()),
    ]);
    if let Some(price) = live.sol_price_usdc {
        table.add_row(vec![
            Cell::new("SOL Price (USDC)"),
            Cell::new(format!("{price:.4}")),
        ]);
    }
    if let Some(epoch) = &live.rpc_epoch {
        table.add_row(vec![
            Cell::new("RPC Epoch"),
            Cell::new(format!("{}", epoch.epoch)),
        ]);
        table.add_row(vec![
            Cell::new("RPC Slot Progress"),
            Cell::new(format!("{}/{}", epoch.slot_index, epoch.slots_in_epoch)),
        ]);
    }
    if let Some(votes) = &live.vote_accounts {
        table.add_row(vec![
            Cell::new("Vote Accounts"),
            Cell::new(format!(
                "current={} delinquent={}",
                votes.current_count, votes.delinquent_count
            )),
        ]);
        table.add_row(vec![
            Cell::new("Avg Commission"),
            Cell::new(format!("{:.2}%", votes.avg_commission_current)),
        ]);
    }
    if let Some(votex) = &live.votex_probe {
        table.add_row(vec![
            Cell::new("Votex Reachable"),
            Cell::new(format!(
                "status={} bytes={} vault_keyword={}",
                votex.status_code, votex.content_length, votex.contains_vault_keyword
            )),
        ]);
    }
    if !live.warnings.is_empty() {
        table.add_row(vec![
            Cell::new("Warnings"),
            Cell::new(format!("{}", live.warnings.len())),
        ]);
    }
    table.to_string()
}

fn snapshot_table(snapshots: &[StoredSnapshot], db_path: &std::path::Path) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_header(vec!["ID", "Epoch", "Kind", "Fetched At"]);
    for snapshot in snapshots {
        table.add_row(vec![
            Cell::new(snapshot.id),
            Cell::new(snapshot.epoch),
            Cell::new(&snapshot.snapshot_kind),
            Cell::new(
                snapshot
                    .fetched_at
                    .format("%Y-%m-%d %H:%M:%S UTC")
                    .to_string(),
            ),
        ]);
    }
    format!("DB: {}\n{table}", db_path.display())
}

fn snapshot_list_csv(snapshots: &[StoredSnapshot]) -> String {
    let mut out = String::from("id,epoch,snapshot_kind,fetched_at\n");
    for snapshot in snapshots {
        out.push_str(&format!(
            "{},{},{},{}\n",
            snapshot.id,
            snapshot.epoch,
            snapshot.snapshot_kind,
            snapshot.fetched_at.to_rfc3339()
        ));
    }
    out
}
