use serde::{Deserialize, Serialize};

use crate::{
    calculator::CostCalculator,
    types::{GaugeVoteState, Pubkey, VotexBid},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaugeConcentration {
    pub epoch: u32,
    pub hhi: f64,
    pub gini: f64,
    pub top_1_share: f64,
    pub top_5_share: f64,
    pub top_10_share: f64,
    pub effective_competitors: f64,
    pub displacement_cost: Vec<DisplacementTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplacementTarget {
    pub validator: Pubkey,
    pub name: String,
    pub current_vev: f64,
    pub current_sol: f64,
    pub vev_to_match: f64,
    pub vev_to_overtake: f64,
    pub estimated_cost_usdc: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompetitorProfile {
    pub validator: Pubkey,
    pub name: String,
    pub style: CompetitorStyle,
    pub vev_weight: f64,
    pub votex_spend_usdc: f64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompetitorStyle {
    LockHeavy,
    VotexBuyer,
    WhalePartnership,
    Mixed,
}

pub struct ConcentrationAnalyzer;

impl ConcentrationAnalyzer {
    pub fn hhi(shares: &[f64]) -> f64 {
        shares.iter().map(|s| (s * 100.0).powi(2)).sum()
    }

    pub fn gini(shares: &[f64]) -> f64 {
        let n = shares.len() as f64;
        if n == 0.0 {
            return 0.0;
        }
        let mean = shares.iter().sum::<f64>() / n;
        if mean == 0.0 {
            return 0.0;
        }
        let mut sorted = shares.to_vec();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let numerator: f64 = sorted
            .iter()
            .enumerate()
            .map(|(i, &x)| (2.0 * (i + 1) as f64 - n - 1.0) * x)
            .sum();
        numerator / (n * n * mean)
    }

    pub fn analyze(gauge_state: &GaugeVoteState, votex_clearing_price: f64) -> GaugeConcentration {
        let mut shares = gauge_state
            .validators
            .iter()
            .map(|v| v.vote_share)
            .collect::<Vec<_>>();
        shares.sort_by(|a, b| b.total_cmp(a));
        let hhi = Self::hhi(&shares);
        let gini = Self::gini(&shares);
        let top_1_share = shares.first().copied().unwrap_or(0.0);
        let top_5_share = shares.iter().take(5).sum();
        let top_10_share = shares.iter().take(10).sum();
        let effective_competitors = if hhi > 0.0 { 10_000.0 / hhi } else { 0.0 };

        let displacement_cost = gauge_state
            .validators
            .iter()
            .map(|target| {
                let vev_to_match = target.vev_weight;
                let vev_to_overtake = target.vev_weight * 1.01;
                let cost = CostCalculator::votex_cost(vev_to_overtake, votex_clearing_price);
                DisplacementTarget {
                    validator: target.vote_account.clone(),
                    name: target.name.clone(),
                    current_vev: target.vev_weight,
                    current_sol: target.projected_sol,
                    vev_to_match,
                    vev_to_overtake,
                    estimated_cost_usdc: cost,
                }
            })
            .collect::<Vec<_>>();

        GaugeConcentration {
            epoch: gauge_state.epoch.epoch_number,
            hhi,
            gini,
            top_1_share,
            top_5_share,
            top_10_share,
            effective_competitors,
            displacement_cost,
        }
    }
}

pub fn classify_competitors(
    gauge_state: &GaugeVoteState,
    bids: &[VotexBid],
) -> Vec<CompetitorProfile> {
    gauge_state
        .validators
        .iter()
        .map(|validator| {
            let votex_spend_usdc = bids
                .iter()
                .filter(|b| b.validator == validator.vote_account)
                .map(|b| b.usdc_deposited)
                .sum::<f64>();

            let style = if votex_spend_usdc > 10_000.0 {
                CompetitorStyle::VotexBuyer
            } else if validator.vev_weight > gauge_state.total_vev_voting * 0.12 {
                CompetitorStyle::WhalePartnership
            } else if votex_spend_usdc == 0.0 && validator.vev_weight > 200_000.0 {
                CompetitorStyle::LockHeavy
            } else {
                CompetitorStyle::Mixed
            };

            CompetitorProfile {
                validator: validator.vote_account.clone(),
                name: validator.name.clone(),
                style,
                vev_weight: validator.vev_weight,
                votex_spend_usdc,
            }
        })
        .collect::<Vec<_>>()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SampleContext;

    #[test]
    fn concentration_metrics_are_bounded() {
        let sample = SampleContext::new();
        let report =
            ConcentrationAnalyzer::analyze(&sample.gauge, sample.votex.clearing_price_per_vev);
        assert!(report.hhi > 0.0);
        assert!((0.0..=1.0).contains(&report.gini));
    }

    #[test]
    fn competitor_classification_returns_all_validators() {
        let sample = SampleContext::new();
        let profiles = classify_competitors(&sample.gauge, &sample.votex.bids);
        assert_eq!(profiles.len(), sample.gauge.validators.len());
    }
}
