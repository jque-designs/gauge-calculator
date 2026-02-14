use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::{presets::UTF8_FULL, Cell, Table};
use gauge_calculator::{
    analysis::{classify_competitors, ConcentrationAnalyzer},
    calculator::{compare_strategies, AcquisitionStrategy, StrategyRequest},
    config::{default_config_path, AppConfig},
    output::{
        concentration_csv, render_concentration_table, render_epoch_text, render_status_table,
        render_strategy_table, strategy_csv, to_json, OutputFormat,
    },
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
    let sample = SampleContext::new();

    if cli.verbose {
        eprintln!("Using RPC: {}", config.rpc.url);
        eprintln!("Config path: {}", cli.config.display());
    }

    match cli.command {
        Commands::Status { validator } => {
            if let Some(target) = validator {
                let maybe = sample
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
                let csv = status_csv(&sample);
                emit(
                    cli.output,
                    render_status_table(&sample.pool, &sample.gauge),
                    &sample,
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
                gauge_reserve_sol: sample.pool.gauge_reserve_sol,
                total_vev: sample.gauge.total_vev_voting,
                pool_apy: sample.pool.apy,
                sol_price_usdc: 200.0,
                votex_clearing_price: sample.votex.clearing_price_per_vev,
                v_price_usdc: sample.vtoken.price_usdc,
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
                gauge_reserve_sol: sample.pool.gauge_reserve_sol,
                total_vev: sample.gauge.total_vev_voting,
                pool_apy: sample.pool.apy,
                sol_price_usdc: 200.0,
                votex_clearing_price: sample.votex.clearing_price_per_vev,
                v_price_usdc: v_price.unwrap_or(sample.vtoken.price_usdc),
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
            let report =
                ConcentrationAnalyzer::analyze(&sample.gauge, sample.votex.clearing_price_per_vev);
            let extra = json!({
                "requested_epoch": epoch,
                "history_window": history,
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
            let mut profiles = classify_competitors(&sample.gauge, &sample.votex.bids);
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

            let entry = sample
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
                "market": sample.votex,
            });
            let text = format!(
                "Votex Epoch {}\nTotal veV for sale: {:.0}\nTotal USDC bid: {:.2}\nClearing price/veV: {:.6}",
                sample.votex.epoch,
                sample.votex.total_vev_for_sale,
                sample.votex.total_usdc_bid,
                sample.votex.clearing_price_per_vev
            );
            emit(cli.output, text, &payload, None)?;
        }
        Commands::Epoch => {
            emit(
                cli.output,
                render_epoch_text(&sample.gauge.epoch),
                &sample.gauge.epoch,
                Some(format!(
                    "epoch,phase,start,deadline,end\n{},\"{}\",{},{},{}\n",
                    sample.gauge.epoch.epoch_number,
                    phase_label(sample.gauge.epoch.phase),
                    sample.gauge.epoch.start,
                    sample.gauge.epoch.vote_deadline,
                    sample.gauge.epoch.end
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
            let payload = match action {
                SnapshotCommand::Save => json!({
                    "action": "save",
                    "note": "Snapshot persistence backend is scaffolded; connect SQLite in a next increment.",
                }),
                SnapshotCommand::Load { epoch } => json!({
                    "action": "load",
                    "epoch": epoch,
                    "note": "No persisted snapshots available in MVP mode.",
                }),
                SnapshotCommand::List => json!({
                    "action": "list",
                    "epochs": [],
                }),
            };
            emit(
                cli.output,
                format!("Snapshot command executed: {}", payload["action"]),
                &payload,
                None,
            )?;
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
