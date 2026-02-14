use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

pub type Pubkey = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultPoolState {
    pub pool_address: Pubkey,
    pub total_sol: f64,
    pub vsol_supply: f64,
    pub vsol_price: f64,
    pub validator_count: u32,
    pub apy: f64,
    pub gauge_reserve_pct: f64,
    pub gauge_reserve_sol: f64,
    pub elite_pool_pct: f64,
    pub direct_stake_pct: f64,
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaugeVoteState {
    pub epoch: VotexEpoch,
    pub total_vev_voting: f64,
    pub validators: Vec<ValidatorGaugeEntry>,
    pub committed: bool,
    pub commit_deadline: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorGaugeEntry {
    pub vote_account: Pubkey,
    pub name: String,
    pub vev_weight: f64,
    pub vote_share: f64,
    pub projected_sol: f64,
    pub eligible: bool,
    pub eligibility_issues: Vec<EligibilityIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VTokenState {
    pub total_supply: u64,
    pub circulating_supply: u64,
    pub price_usdc: f64,
    pub total_locked: u64,
    pub total_vev: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VeLockPosition {
    pub owner: Pubkey,
    pub v_locked: u64,
    pub lock_start: DateTime<Utc>,
    pub lock_end: DateTime<Utc>,
    pub lock_duration_years: f64,
    pub multiplier: f64,
    pub vev_at_lock: f64,
    pub vev_current: f64,
    pub vote_target: Option<Pubkey>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LockMultiplierTable;

impl LockMultiplierTable {
    pub fn multiplier(years: f64) -> f64 {
        (years.clamp(0.0, 5.0) * 2.0).max(1.0)
    }

    pub fn effective_vev(v_amount: u64, years: f64) -> f64 {
        v_amount as f64 * Self::multiplier(years)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexEpoch {
    pub epoch_number: u32,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub vote_deadline: DateTime<Utc>,
    pub phase: VotexPhase,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum VotexPhase {
    VoteBuying,
    Voting,
    Distributing,
    Committed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexBid {
    pub validator: Pubkey,
    pub validator_name: String,
    pub usdc_deposited: f64,
    pub vev_acquired: f64,
    pub effective_cost_per_vev: f64,
    pub epoch: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexMarketSnapshot {
    pub epoch: u32,
    pub total_vev_for_sale: f64,
    pub total_usdc_bid: f64,
    pub clearing_price_per_vev: f64,
    pub bids: Vec<VotexBid>,
    pub platform_fee_pct: f64,
    pub seller_payout_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaugeEligibility {
    pub vote_account: Pubkey,
    pub eligible: bool,
    pub issues: Vec<EligibilityIssue>,
    pub evaluated_over_epochs: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EligibilityIssue {
    InSuperminority,
    CommissionTooHigh { current: f64, max: f64 },
    MevCommissionTooHigh { current: f64, max: f64 },
    VoteCreditsLow { current: f64, threshold: f64 },
    NotOnAllowlist,
    NoJitoMevClient,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleContext {
    pub pool: VaultPoolState,
    pub gauge: GaugeVoteState,
    pub vtoken: VTokenState,
    pub votex: VotexMarketSnapshot,
}

impl SampleContext {
    pub fn new() -> Self {
        let now = Utc::now();
        let start = now - Duration::days(2);
        let end = start + Duration::days(7);
        let vote_deadline = end - Duration::hours(24);
        let epoch = VotexEpoch {
            epoch_number: 47,
            start,
            end,
            vote_deadline,
            phase: VotexPhase::VoteBuying,
        };

        let pool = VaultPoolState {
            pool_address: "Fu9BYC6tWBo1KMKaP3CFoKfRhqv9akmy3DuYwnCyWiyC".to_string(),
            total_sol: 1_242_657.0,
            vsol_supply: 1_093_888.0,
            vsol_price: 1.136,
            validator_count: 128,
            apy: 0.0584,
            gauge_reserve_pct: 0.10,
            gauge_reserve_sol: 124_265.7,
            elite_pool_pct: 0.50,
            direct_stake_pct: 0.40,
            fetched_at: now,
        };

        let validators_seed = [
            (
                "Vote111111111111111111111111111111111111111",
                "Helius",
                1_204_500.0,
            ),
            (
                "Vote222222222222222222222222222222222222222",
                "Triton",
                892_300.0,
            ),
            (
                "Vote333333333333333333333333333333333333333",
                "Coinbase Cloud",
                743_100.0,
            ),
            (
                "Vote444444444444444444444444444444444444444",
                "Everstake",
                621_800.0,
            ),
            (
                "Vote555555555555555555555555555555555555555",
                "Laine",
                534_200.0,
            ),
            (
                "Vote666666666666666666666666666666666666666",
                "Figment",
                412_700.0,
            ),
            (
                "Vote777777777777777777777777777777777777777",
                "Chorus One",
                389_400.0,
            ),
            (
                "Vote888888888888888888888888888888888888888",
                "Staking Facilities",
                301_500.0,
            ),
            (
                "Vote999999999999999999999999999999999999999",
                "P2P Validator",
                278_900.0,
            ),
            (
                "VoteAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
                "Shinobi Systems",
                244_600.0,
            ),
        ];

        let total_vev_voting: f64 = validators_seed.iter().map(|(_, _, vev)| vev).sum();
        let validators = validators_seed
            .iter()
            .enumerate()
            .map(|(idx, (vote_account, name, vev_weight))| {
                let vote_share = *vev_weight / total_vev_voting;
                let projected_sol = pool.gauge_reserve_sol * vote_share;
                let eligible = idx != 9;
                let eligibility_issues = if eligible {
                    Vec::new()
                } else {
                    vec![EligibilityIssue::VoteCreditsLow {
                        current: 0.936,
                        threshold: 0.95,
                    }]
                };

                ValidatorGaugeEntry {
                    vote_account: (*vote_account).to_string(),
                    name: (*name).to_string(),
                    vev_weight: *vev_weight,
                    vote_share,
                    projected_sol,
                    eligible,
                    eligibility_issues,
                }
            })
            .collect::<Vec<_>>();

        let gauge = GaugeVoteState {
            epoch: epoch.clone(),
            total_vev_voting,
            validators: validators.clone(),
            committed: false,
            commit_deadline: vote_deadline,
        };

        let total_usdc_bid = 91_432.0;
        let total_vev_for_sale = 1_602_000.0;
        let clearing_price_per_vev = total_usdc_bid / total_vev_for_sale;
        let bids = validators
            .iter()
            .take(5)
            .enumerate()
            .map(|(idx, v)| {
                let usdc_deposited = 4_500.0 + (idx as f64 * 1_200.0);
                let vev_acquired = usdc_deposited / clearing_price_per_vev;
                VotexBid {
                    validator: v.vote_account.clone(),
                    validator_name: v.name.clone(),
                    usdc_deposited,
                    vev_acquired,
                    effective_cost_per_vev: usdc_deposited / vev_acquired,
                    epoch: epoch.epoch_number,
                }
            })
            .collect::<Vec<_>>();

        let votex = VotexMarketSnapshot {
            epoch: epoch.epoch_number,
            total_vev_for_sale,
            total_usdc_bid,
            clearing_price_per_vev,
            bids,
            platform_fee_pct: 0.15,
            seller_payout_pct: 0.85,
        };

        let vtoken = VTokenState {
            total_supply: 100_000_000,
            circulating_supply: 1_000_000,
            price_usdc: 0.50,
            total_locked: 47_500_000,
            total_vev: 8_432_100.0,
        };

        Self {
            pool,
            gauge,
            vtoken,
            votex,
        }
    }
}
