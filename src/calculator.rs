use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

use crate::types::LockMultiplierTable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LockCostEstimate {
    pub v_tokens: u64,
    pub lock_years: f64,
    pub multiplier: f64,
    pub cost_usdc: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoiEstimate {
    pub annual_reward: f64,
    pub annual_cost: f64,
    pub roi: f64,
    pub payback_epochs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPerSolEstimate {
    pub target_sol: f64,
    pub current_gauge_reserve: f64,
    pub current_total_vev: f64,
    pub vev_needed: f64,
    pub strategy: AcquisitionStrategy,
    pub total_cost_usdc: f64,
    pub cost_per_sol_usdc: f64,
    pub annual_staking_reward: f64,
    pub roi_annualized: f64,
    pub payback_epochs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AcquisitionStrategy {
    BuyOnVotex {
        usdc_per_epoch: f64,
        recurring: bool,
    },
    LockVTokens {
        v_tokens_needed: u64,
        lock_years: f64,
        acquisition_cost_usdc: f64,
        multiplier: f64,
    },
    Hybrid {
        v_to_lock: u64,
        lock_years: f64,
        votex_usdc_topup: f64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyComparison {
    pub strategies: Vec<CostPerSolEstimate>,
    pub recommended: usize,
    pub recommendation_reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyRequest {
    pub target_sol: f64,
    pub gauge_reserve_sol: f64,
    pub total_vev: f64,
    pub pool_apy: f64,
    pub sol_price_usdc: f64,
    pub votex_clearing_price: f64,
    pub v_price_usdc: f64,
    pub lock_years: f64,
}

pub struct CostCalculator;

impl CostCalculator {
    pub fn vev_needed(gauge_reserve_sol: f64, total_vev: f64, target_sol: f64) -> Result<f64> {
        if gauge_reserve_sol <= 0.0 {
            return Err(anyhow!("gauge reserve must be positive"));
        }
        if target_sol <= 0.0 {
            return Err(anyhow!("target SOL must be positive"));
        }
        if target_sol >= gauge_reserve_sol {
            return Err(anyhow!("target exceeds gauge reserve"));
        }
        if total_vev <= 0.0 {
            // With no existing votes, any positive vote can win most/all reserve.
            return Ok(1.0);
        }

        Ok((target_sol * total_vev) / (gauge_reserve_sol - target_sol))
    }

    pub fn votex_cost(vev_needed: f64, clearing_price: f64) -> f64 {
        vev_needed * clearing_price / 0.85
    }

    pub fn lock_cost(vev_needed: f64, v_price_usdc: f64, lock_years: f64) -> LockCostEstimate {
        let multiplier = LockMultiplierTable::multiplier(lock_years);
        let v_tokens = (vev_needed / multiplier).ceil() as u64;
        let cost = v_tokens as f64 * v_price_usdc;
        LockCostEstimate {
            v_tokens,
            lock_years,
            multiplier,
            cost_usdc: cost,
        }
    }

    pub fn roi(
        target_sol: f64,
        pool_apy: f64,
        sol_price_usdc: f64,
        acquisition_cost: f64,
        strategy_is_recurring: bool,
    ) -> RoiEstimate {
        let annual_reward = target_sol * pool_apy * sol_price_usdc;
        let annual_cost = if strategy_is_recurring {
            acquisition_cost * 52.0
        } else {
            acquisition_cost
        };
        let roi = if annual_cost > 0.0 {
            (annual_reward - annual_cost) / annual_cost
        } else {
            0.0
        };
        let payback_epochs = if annual_reward > 0.0 {
            (acquisition_cost / (annual_reward / 52.0)).ceil() as u32
        } else {
            u32::MAX
        };
        RoiEstimate {
            annual_reward,
            annual_cost,
            roi,
            payback_epochs,
        }
    }
}

pub fn compare_strategies(request: &StrategyRequest) -> Result<StrategyComparison> {
    let vev_needed = CostCalculator::vev_needed(
        request.gauge_reserve_sol,
        request.total_vev,
        request.target_sol,
    )?;
    let annual_staking_reward = request.target_sol * request.pool_apy * request.sol_price_usdc;

    let buy_per_epoch = CostCalculator::votex_cost(vev_needed, request.votex_clearing_price);
    let buy_roi = CostCalculator::roi(
        request.target_sol,
        request.pool_apy,
        request.sol_price_usdc,
        buy_per_epoch,
        true,
    );
    let buy = CostPerSolEstimate {
        target_sol: request.target_sol,
        current_gauge_reserve: request.gauge_reserve_sol,
        current_total_vev: request.total_vev,
        vev_needed,
        strategy: AcquisitionStrategy::BuyOnVotex {
            usdc_per_epoch: buy_per_epoch,
            recurring: true,
        },
        total_cost_usdc: buy_per_epoch,
        cost_per_sol_usdc: buy_per_epoch / request.target_sol,
        annual_staking_reward,
        roi_annualized: buy_roi.roi,
        payback_epochs: buy_roi.payback_epochs,
    };

    let lock = CostCalculator::lock_cost(vev_needed, request.v_price_usdc, request.lock_years);
    let lock_roi = CostCalculator::roi(
        request.target_sol,
        request.pool_apy,
        request.sol_price_usdc,
        lock.cost_usdc,
        false,
    );
    let lock_estimate = CostPerSolEstimate {
        target_sol: request.target_sol,
        current_gauge_reserve: request.gauge_reserve_sol,
        current_total_vev: request.total_vev,
        vev_needed,
        strategy: AcquisitionStrategy::LockVTokens {
            v_tokens_needed: lock.v_tokens,
            lock_years: lock.lock_years,
            acquisition_cost_usdc: lock.cost_usdc,
            multiplier: lock.multiplier,
        },
        total_cost_usdc: lock.cost_usdc,
        cost_per_sol_usdc: lock.cost_usdc / request.target_sol,
        annual_staking_reward,
        roi_annualized: lock_roi.roi,
        payback_epochs: lock_roi.payback_epochs,
    };

    // Hybrid assumes 40% of veV from locking and 60% from Votex every epoch.
    let lock_share = 0.40;
    let vev_lock = vev_needed * lock_share;
    let vev_buy = vev_needed * (1.0 - lock_share);
    let hybrid_lock = CostCalculator::lock_cost(vev_lock, request.v_price_usdc, request.lock_years);
    let hybrid_votex = CostCalculator::votex_cost(vev_buy, request.votex_clearing_price);
    let hybrid_first_year_cost = hybrid_lock.cost_usdc + (hybrid_votex * 52.0);
    let hybrid_roi = if hybrid_first_year_cost > 0.0 {
        (annual_staking_reward - hybrid_first_year_cost) / hybrid_first_year_cost
    } else {
        0.0
    };
    let hybrid_payback = if annual_staking_reward > 0.0 {
        (hybrid_first_year_cost / (annual_staking_reward / 52.0)).ceil() as u32
    } else {
        u32::MAX
    };
    let hybrid = CostPerSolEstimate {
        target_sol: request.target_sol,
        current_gauge_reserve: request.gauge_reserve_sol,
        current_total_vev: request.total_vev,
        vev_needed,
        strategy: AcquisitionStrategy::Hybrid {
            v_to_lock: hybrid_lock.v_tokens,
            lock_years: request.lock_years,
            votex_usdc_topup: hybrid_votex,
        },
        total_cost_usdc: hybrid_first_year_cost,
        cost_per_sol_usdc: hybrid_first_year_cost / request.target_sol,
        annual_staking_reward,
        roi_annualized: hybrid_roi,
        payback_epochs: hybrid_payback,
    };

    let strategies = vec![buy, lock_estimate, hybrid];
    let (recommended, _) = strategies
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.roi_annualized.total_cmp(&b.roi_annualized))
        .ok_or_else(|| anyhow!("no strategy candidates"))?;

    Ok(StrategyComparison {
        strategies,
        recommended,
        recommendation_reason:
            "Highest annualized ROI based on current market and lock assumptions".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vev_needed_is_positive() {
        let vev = CostCalculator::vev_needed(124_266.0, 8_432_100.0, 5_000.0).unwrap();
        assert!(vev > 0.0);
    }

    #[test]
    fn lock_cost_drops_with_longer_lock() {
        let vev_needed = 100_000.0;
        let one_year = CostCalculator::lock_cost(vev_needed, 0.5, 1.0);
        let five_year = CostCalculator::lock_cost(vev_needed, 0.5, 5.0);
        assert!(five_year.cost_usdc < one_year.cost_usdc);
    }

    #[test]
    fn compare_returns_recommendation() {
        let req = StrategyRequest {
            target_sol: 5_000.0,
            gauge_reserve_sol: 124_266.0,
            total_vev: 8_432_100.0,
            pool_apy: 0.0584,
            sol_price_usdc: 200.0,
            votex_clearing_price: 0.057,
            v_price_usdc: 0.5,
            lock_years: 3.0,
        };
        let comparison = compare_strategies(&req).unwrap();
        assert!(!comparison.strategies.is_empty());
        assert!(comparison.recommended < comparison.strategies.len());
    }
}
