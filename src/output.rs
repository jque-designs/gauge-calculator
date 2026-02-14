use anyhow::Result;
use clap::ValueEnum;
use comfy_table::{presets::UTF8_FULL, Cell, ContentArrangement, Table};
use serde::Serialize;

use crate::{
    analysis::GaugeConcentration,
    calculator::{AcquisitionStrategy, StrategyComparison},
    types::{GaugeVoteState, VaultPoolState, VotexEpoch, VotexPhase},
};

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum OutputFormat {
    Table,
    Json,
    Csv,
}

pub fn to_json<T: Serialize>(value: &T) -> Result<String> {
    Ok(serde_json::to_string_pretty(value)?)
}

pub fn render_status_table(pool: &VaultPoolState, gauge: &GaugeVoteState) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec!["Metric", "Value"]);
    table.add_row(vec!["Pool Total", &format!("{:.0} SOL", pool.total_sol)]);
    table.add_row(vec![
        "Gauge Reserve",
        &format!(
            "{:.0} SOL ({:.0}%)",
            pool.gauge_reserve_sol,
            pool.gauge_reserve_pct * 100.0
        ),
    ]);
    table.add_row(vec![
        "Total veV Voting",
        &format!("{:.0} veV", gauge.total_vev_voting),
    ]);
    table.add_row(vec!["Validators", &pool.validator_count.to_string()]);
    table.add_row(vec!["Pool APY", &format!("{:.2}%", pool.apy * 100.0)]);
    table.add_row(vec!["Epoch Phase", phase_label(gauge.epoch.phase)]);
    table.add_row(vec![
        "Vote Deadline",
        &gauge
            .epoch
            .vote_deadline
            .format("%Y-%m-%d %H:%M UTC")
            .to_string(),
    ]);

    let mut validator_table = Table::new();
    validator_table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(vec![
            "#",
            "Validator",
            "veV Weight",
            "Share",
            "Projected SOL",
        ]);
    for (idx, v) in gauge.validators.iter().enumerate() {
        validator_table.add_row(vec![
            Cell::new(idx + 1),
            Cell::new(&v.name),
            Cell::new(format!("{:.0}", v.vev_weight)),
            Cell::new(format!("{:.2}%", v.vote_share * 100.0)),
            Cell::new(format!("{:.0}", v.projected_sol)),
        ]);
    }

    format!("{table}\n\nTop Gauge Validators\n{validator_table}")
}

pub fn render_strategy_table(comparison: &StrategyComparison) -> String {
    let mut table = Table::new();
    table.load_preset(UTF8_FULL).set_header(vec![
        "Strategy",
        "Cost (USDC)",
        "$/SOL",
        "Annual ROI",
        "Payback",
    ]);
    for strategy in &comparison.strategies {
        let (name, cost_display) = match &strategy.strategy {
            AcquisitionStrategy::BuyOnVotex { usdc_per_epoch, .. } => (
                "Buy on Votex".to_string(),
                format!("{:.2}/epoch", usdc_per_epoch),
            ),
            AcquisitionStrategy::LockVTokens {
                lock_years,
                acquisition_cost_usdc,
                ..
            } => (
                format!("Lock $V ({lock_years:.1}y)"),
                format!("{acquisition_cost_usdc:.2} once"),
            ),
            AcquisitionStrategy::Hybrid {
                lock_years,
                votex_usdc_topup,
                ..
            } => (
                format!("Hybrid ({lock_years:.1}y + Votex)"),
                format!("{votex_usdc_topup:.2}/epoch + lock"),
            ),
        };

        table.add_row(vec![
            Cell::new(name),
            Cell::new(cost_display),
            Cell::new(format!("{:.2}", strategy.cost_per_sol_usdc)),
            Cell::new(format!("{:.2}%", strategy.roi_annualized * 100.0)),
            Cell::new(format!("{} epochs", strategy.payback_epochs)),
        ]);
    }

    let recommended = comparison
        .strategies
        .get(comparison.recommended)
        .map(|s| match &s.strategy {
            AcquisitionStrategy::BuyOnVotex { .. } => "Buy on Votex".to_string(),
            AcquisitionStrategy::LockVTokens { lock_years, .. } => {
                format!("Lock $V ({lock_years:.1}y)")
            }
            AcquisitionStrategy::Hybrid { lock_years, .. } => {
                format!("Hybrid ({lock_years:.1}y + Votex)")
            }
        })
        .unwrap_or_else(|| "N/A".to_string());

    format!(
        "{table}\n\nRecommended: {recommended}\nReason: {}",
        comparison.recommendation_reason
    )
}

pub fn render_concentration_table(report: &GaugeConcentration) -> String {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_header(vec!["Metric", "Value"]);
    table.add_row(vec!["Epoch", &report.epoch.to_string()]);
    table.add_row(vec!["HHI", &format!("{:.2}", report.hhi)]);
    table.add_row(vec!["Gini", &format!("{:.4}", report.gini)]);
    table.add_row(vec![
        "Top-1 Share",
        &format!("{:.2}%", report.top_1_share * 100.0),
    ]);
    table.add_row(vec![
        "Top-5 Share",
        &format!("{:.2}%", report.top_5_share * 100.0),
    ]);
    table.add_row(vec![
        "Top-10 Share",
        &format!("{:.2}%", report.top_10_share * 100.0),
    ]);
    table.add_row(vec![
        "Effective Competitors",
        &format!("{:.2}", report.effective_competitors),
    ]);

    let mut displacement = Table::new();
    displacement.load_preset(UTF8_FULL).set_header(vec![
        "Validator",
        "veV to Match",
        "veV to Overtake",
        "Est. Cost (USDC)",
    ]);
    for target in report.displacement_cost.iter().take(10) {
        displacement.add_row(vec![
            Cell::new(&target.name),
            Cell::new(format!("{:.0}", target.vev_to_match)),
            Cell::new(format!("{:.0}", target.vev_to_overtake)),
            Cell::new(format!("{:.2}", target.estimated_cost_usdc)),
        ]);
    }

    format!("{table}\n\nDisplacement Targets\n{displacement}")
}

pub fn render_epoch_text(epoch: &VotexEpoch) -> String {
    format!(
        "Vault Epoch: {}\nPhase:       {}\nStarted:     {}\nDeadline:    {}\nEnds:        {}",
        epoch.epoch_number,
        phase_label(epoch.phase),
        epoch.start.format("%Y-%m-%d %H:%M UTC"),
        epoch.vote_deadline.format("%Y-%m-%d %H:%M UTC"),
        epoch.end.format("%Y-%m-%d %H:%M UTC")
    )
}

pub fn strategy_csv(comparison: &StrategyComparison) -> String {
    let mut out = String::from("strategy,cost_usdc,cost_per_sol,annual_roi,payback_epochs\n");
    for strategy in &comparison.strategies {
        let name = match &strategy.strategy {
            AcquisitionStrategy::BuyOnVotex { .. } => "buy_on_votex",
            AcquisitionStrategy::LockVTokens { .. } => "lock_v_tokens",
            AcquisitionStrategy::Hybrid { .. } => "hybrid",
        };
        out.push_str(&format!(
            "{name},{:.6},{:.6},{:.6},{}\n",
            strategy.total_cost_usdc,
            strategy.cost_per_sol_usdc,
            strategy.roi_annualized,
            strategy.payback_epochs
        ));
    }
    out
}

pub fn concentration_csv(report: &GaugeConcentration) -> String {
    format!(
        "epoch,hhi,gini,top_1_share,top_5_share,top_10_share,effective_competitors\n{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6}\n",
        report.epoch,
        report.hhi,
        report.gini,
        report.top_1_share,
        report.top_5_share,
        report.top_10_share,
        report.effective_competitors
    )
}

fn phase_label(phase: VotexPhase) -> &'static str {
    match phase {
        VotexPhase::VoteBuying => "Vote Buying",
        VotexPhase::Voting => "Voting",
        VotexPhase::Distributing => "Distributing",
        VotexPhase::Committed => "Committed",
    }
}
