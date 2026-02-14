# GAUGE-CALCULATOR: Vault Delegation Program Optimizer

> **Status:** Spec v1.0 — Codex-ready
> **Crate:** `gauge-calculator`
> **Binary:** `gauge-calc`
> **License:** MIT/Apache-2.0

## What This Is

A strategy engine for The Vault's gauge-weighted stake delegation system. The Vault allocates ~1.24M SOL across 128 validators, with **10% (~124K SOL) directed by gauge votes** from veV (vote-escrowed $V token) holders. Validators compete for this gauge bucket through direct $V token locking or purchasing voting power via Votex's USDC vote market.

This tool answers: **"What's the cheapest path to capture X SOL of gauge-directed stake, and is the ROI positive?"**

No existing tool models the gauge economics end-to-end. Votex shows current bids. The Vault docs explain mechanics. Neither helps a validator compute optimal capital allocation across the buy-vs-lock-vs-partner decision space.

---

## Domain Model

### How The Vault Gauge System Works

```
                     The Vault Stake Pool (~1.24M SOL)
                              │
              ┌───────────────┼───────────────┐
              │               │               │
        Elite Perf (50%)  Direct Stake (40%)  Gauges (10%)
        ~620K SOL          ~496K SOL          ~124K SOL
        Top 50 by          Top 100 by         Proportional to
        vote credits       self-stake         veV gauge votes
```

**Gauge allocation formula:**

```
validator_gauge_stake = gauge_reserve * (validator_votes / total_votes)
```

**veV mechanics:**
- Lock $V tokens → receive veV (voting power)
- Lock multiplier: 1x (min lock) → 10x (5-year lock)
- veV decays linearly toward unlock date (ve-tokenomics, similar to Curve's veCRV)

**Votex vote market:**
- 7-day epochs synchronized with The Vault
- Validators deposit USDC → buys veV delegation for one epoch
- 15% platform fee, 85% to vote sellers
- Deadline: votes must be cast ≥24h before epoch start
- Monday commit → stake movement begins next Solana epoch
- Full allocation arrives within ~5 epochs (Solana warm-up rules)

**Validator gauge eligibility (rolling 10-epoch window):**
- Not in superminority
- Commission ≤ 5%
- MEV commission ≤ 10%
- Vote credits ≥ 95% of epoch average

---

## Novel Differentiators

1. **Cost-Per-SOL Calculator** — Computes the marginal USDC cost to capture one additional SOL of gauge-directed stake, accounting for the relative voting dynamics (your votes dilute everyone else's share).
2. **Lock vs Buy Optimizer** — Models the NPV tradeoff: locking $V tokens ties up capital for years but provides persistent voting power; buying on Votex is pay-per-epoch but avoids token exposure. Finds the crossover point.
3. **Whale Voter Mapping** — Identifies the largest veV holders, their current vote allocations, and whether they sell on Votex. Reveals partnership and bribery targets.
4. **Gauge Concentration Index** — Measures how concentrated gauge votes are (HHI, Gini). High concentration = cheaper to displace a mid-tier validator. Low concentration = expensive land grab.
5. **Epoch Deadline Sniper** — Tracks the 7-day vote cycle and alerts when the 24h submission deadline approaches. Late votes = missed epoch = wasted capital.
6. **Competitor Strategy Classifier** — Infers whether competitors are locking $V, buying on Votex, or running whale partnerships based on vote weight patterns across epochs.

---

## Architecture

```
gauge-calculator/
├── Cargo.toml
├── src/
│   ├── main.rs                     # CLI entrypoint
│   ├── lib.rs                      # Public API surface
│   ├── config.rs                   # TOML config + CLI arg merge
│   ├── vault/
│   │   ├── mod.rs
│   │   ├── pool.rs                 # Vault stake pool state (total SOL, reserve split)
│   │   ├── gauges.rs               # Gauge account parsing, vote weights
│   │   ├── eligibility.rs          # Validator gauge eligibility checker
│   │   └── epochs.rs               # Vault 7-day epoch cycle tracking
│   ├── token/
│   │   ├── mod.rs
│   │   ├── v_token.rs              # $V token supply, price, holders
│   │   ├── locker.rs               # veV lock state parsing (duration, multiplier, decay)
│   │   └── holders.rs              # Top veV holder enumeration
│   ├── votex/
│   │   ├── mod.rs
│   │   ├── market.rs               # Current epoch bids, historical prices
│   │   ├── rounds.rs               # Round timing, deadlines, commit status
│   │   └── scraper.rs              # Votex UI data extraction (no public API)
│   ├── calculator/
│   │   ├── mod.rs
│   │   ├── cost_per_sol.rs         # Marginal cost to capture N SOL via gauge
│   │   ├── lock_vs_buy.rs          # NPV comparison: lock $V vs buy on Votex
│   │   ├── roi.rs                  # Staking reward ROI vs vote acquisition cost
│   │   └── strategy.rs             # Optimal capital allocation recommender
│   ├── analysis/
│   │   ├── mod.rs
│   │   ├── concentration.rs        # HHI, Gini, top-N share of gauge votes
│   │   ├── competitors.rs          # Per-validator gauge strategy inference
│   │   ├── whales.rs               # Large veV holder mapping + behavior
│   │   └── trends.rs               # Vote weight movement across epochs
│   ├── snapshot/
│   │   ├── mod.rs
│   │   ├── store.rs                # SQLite epoch-keyed snapshots
│   │   └── migrations.rs           # Schema versioning
│   ├── alert/
│   │   ├── mod.rs
│   │   ├── engine.rs               # Alert evaluation loop
│   │   ├── rules.rs                # Deadline, displacement, concentration alerts
│   │   └── sink.rs                 # Webhook, Discord, stdout
│   └── output/
│       ├── mod.rs
│       ├── table.rs                # Terminal table rendering
│       ├── json.rs                 # Structured JSON output
│       └── csv.rs                  # CSV export
```

### Key Dependencies

```toml
[dependencies]
solana-client = "2.1"
solana-sdk = "2.1"
solana-account-decoder = "2.1"
spl-stake-pool = "4"
borsh = "1.5"
tokio = { version = "1", features = ["full"] }
reqwest = { version = "0.12", features = ["json", "cookies"] }
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
clap = { version = "4", features = ["derive"] }
comfy-table = "7"
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = "0.3"
scraper = "0.20"           # HTML parsing for Votex scraping
```

---

## Data Types

### Vault Gauge State

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultPoolState {
    pub pool_address: Pubkey,          // Fu9BYC6tWBo1KMKaP3CFoKfRhqv9akmy3DuYwnCyWiyC
    pub total_sol: f64,                // ~1,242,657 SOL
    pub vsol_supply: f64,              // vSOL token supply
    pub vsol_price: f64,               // SOL per vSOL (~1.136)
    pub validator_count: u32,          // ~128
    pub apy: f64,                      // ~5.84%
    pub gauge_reserve_pct: f64,        // 10%
    pub gauge_reserve_sol: f64,        // total_sol * 0.10
    pub elite_pool_pct: f64,           // 50%
    pub direct_stake_pct: f64,         // 40%
    pub fetched_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaugeVoteState {
    pub epoch: VaultEpoch,
    pub total_vev_voting: f64,         // total veV weight cast
    pub validators: Vec<ValidatorGaugeEntry>,
    pub committed: bool,
    pub commit_deadline: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorGaugeEntry {
    pub vote_account: Pubkey,
    pub name: String,
    pub vev_weight: f64,               // veV votes received
    pub vote_share: f64,               // vev_weight / total_vev_voting
    pub projected_sol: f64,            // gauge_reserve_sol * vote_share
    pub eligible: bool,
    pub eligibility_issues: Vec<EligibilityIssue>,
}
```

### veV Token Economics

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VTokenState {
    pub total_supply: u64,             // 100,000,000 $V
    pub circulating_supply: u64,       // initially ~1% at TGE
    pub price_usdc: f64,               // current market price
    pub total_locked: u64,             // $V locked in veV contracts
    pub total_vev: f64,                // aggregate veV (lock-weighted)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VeLockPosition {
    pub owner: Pubkey,
    pub v_locked: u64,
    pub lock_start: DateTime<Utc>,
    pub lock_end: DateTime<Utc>,
    pub lock_duration_years: f64,
    pub multiplier: f64,               // 1x–10x based on duration
    pub vev_at_lock: f64,              // v_locked * multiplier
    pub vev_current: f64,              // decayed linearly toward unlock
    pub vote_target: Option<Pubkey>,   // validator being voted for
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct LockMultiplierTable;

impl LockMultiplierTable {
    pub fn multiplier(years: f64) -> f64 {
        // Linear interpolation: 1 year = 2x, 5 years = 10x
        (years.clamp(0.0, 5.0) * 2.0).max(1.0)
    }

    pub fn effective_vev(v_amount: u64, years: f64) -> f64 {
        v_amount as f64 * Self::multiplier(years)
    }
}
```

### Votex Market State

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexEpoch {
    pub epoch_number: u32,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub vote_deadline: DateTime<Utc>,  // end - 24h
    pub phase: VotexPhase,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub enum VotexPhase {
    VoteBuying,                        // bids open
    Voting,                            // votes being cast
    Distributing,                      // USDC payouts to sellers
    Committed,                         // on-chain, stake moving
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexBid {
    pub validator: Pubkey,
    pub validator_name: String,
    pub usdc_deposited: f64,
    pub vev_acquired: f64,             // proportional to bid vs total bids
    pub effective_cost_per_vev: f64,   // usdc_deposited / vev_acquired
    pub epoch: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexMarketSnapshot {
    pub epoch: u32,
    pub total_vev_for_sale: f64,       // veV delegated to Votex
    pub total_usdc_bid: f64,           // sum of all bids
    pub clearing_price_per_vev: f64,   // total_usdc / total_vev (after 15% fee)
    pub bids: Vec<VotexBid>,
    pub platform_fee_pct: f64,         // 15%
    pub seller_payout_pct: f64,        // 85%
}
```

### Strategy Engine

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostPerSolEstimate {
    pub target_sol: f64,
    pub current_gauge_reserve: f64,
    pub current_total_vev: f64,
    pub vev_needed: f64,               // to capture target_sol
    pub strategy: AcquisitionStrategy,
    pub total_cost_usdc: f64,
    pub cost_per_sol_usdc: f64,        // total_cost / target_sol
    pub annual_staking_reward: f64,    // target_sol * pool_apy
    pub roi_annualized: f64,           // (reward - cost) / cost
    pub payback_epochs: u32,           // epochs until cost recovered
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AcquisitionStrategy {
    BuyOnVotex {
        usdc_per_epoch: f64,
        recurring: bool,
    },
    LockVTokens {
        v_tokens_needed: u64,
        lock_years: f64,
        acquisition_cost_usdc: f64,    // market price * tokens
        multiplier: f64,
    },
    Hybrid {
        v_to_lock: u64,
        lock_years: f64,
        votex_usdc_topup: f64,         // per-epoch supplement
    },
    WhalePartnership {
        target_whale: Pubkey,
        vev_available: f64,
        estimated_bribe_usdc: f64,     // competitive with Votex clearing price
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StrategyComparison {
    pub strategies: Vec<CostPerSolEstimate>,
    pub recommended: usize,            // index of best strategy
    pub recommendation_reason: String,
}
```

### Concentration Analysis

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaugeConcentration {
    pub epoch: u32,
    pub hhi: f64,                      // Herfindahl-Hirschman Index (0–10000)
    pub gini: f64,                     // Gini coefficient (0–1)
    pub top_1_share: f64,              // largest validator's vote share
    pub top_5_share: f64,
    pub top_10_share: f64,
    pub effective_competitors: f64,    // 1/HHI normalized
    pub displacement_cost: Vec<DisplacementTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisplacementTarget {
    pub validator: Pubkey,
    pub name: String,
    pub current_vev: f64,
    pub current_sol: f64,
    pub vev_to_match: f64,            // match their position
    pub vev_to_overtake: f64,         // exceed by 1%
    pub estimated_cost_usdc: f64,     // at current Votex clearing price
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhaleVoter {
    pub address: Pubkey,
    pub total_vev: f64,
    pub pct_of_total_vev: f64,
    pub current_vote_target: Option<Pubkey>,
    pub sells_on_votex: bool,
    pub lock_expiry: DateTime<Utc>,
    pub epochs_active: u32,
    pub vote_history: Vec<(u32, Pubkey)>,  // (epoch, validator)
}
```

### Eligibility

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GaugeEligibility {
    pub vote_account: Pubkey,
    pub eligible: bool,
    pub issues: Vec<EligibilityIssue>,
    pub evaluated_over_epochs: u32,    // rolling 10-epoch window
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EligibilityIssue {
    InSuperminority,
    CommissionTooHigh { current: f64, max: 5.0 },
    MevCommissionTooHigh { current: f64, max: 10.0 },
    VoteCreditsLow { current: f64, threshold: f64 },  // 95% of avg
    NotOnAllowlist,
    NoJitoMevClient,
}
```

---

## CLI Design

```
gauge-calc [OPTIONS] <COMMAND>

Options:
  -c, --config <PATH>     Config file [default: ~/.config/gauge-calc/config.toml]
  -r, --rpc <URL>         Solana RPC endpoint
  -o, --output <FORMAT>   Output format: table, json, csv [default: table]
  -v, --verbose           Enable debug logging

Commands:
  status         Current gauge state overview
  calculate      Cost-per-SOL calculations
  compare        Strategy comparison (lock vs buy vs hybrid)
  concentration  Gauge vote concentration analysis
  whales         Map large veV holders and their voting patterns
  competitors    Analyze competitor validator gauge strategies
  eligibility    Check gauge eligibility for a validator
  votex          Votex market state and historical clearing prices
  epoch          Current epoch timing and deadlines
  watch          Continuous monitoring with alerts
  snapshot       Save/load gauge state snapshots
```

### Command Details

```
gauge-calc status [--validator <PUBKEY>]
```
Shows pool state, gauge reserve size, current vote distribution, and epoch timing. If `--validator` given, highlights that validator's position.

```
gauge-calc calculate --target-sol <SOL> [--validator <PUBKEY>] [--strategy <lock|buy|hybrid|all>]
```
Computes how much veV is needed to capture `<SOL>` from the gauge reserve, the cost via each strategy, and the annualized ROI vs staking rewards.

```
gauge-calc compare --target-sol <SOL> [--lock-years <1-5>] [--v-price <USDC>]
```
Side-by-side comparison of all acquisition strategies. Shows NPV, payback period, capital efficiency, and risk factors.

```
gauge-calc concentration [--epoch <N>] [--history <EPOCHS>]
```
Gauge vote concentration metrics. With `--history`, shows trend over N epochs to detect consolidation or fragmentation.

```
gauge-calc whales [--min-vev <AMOUNT>] [--sells-on-votex]
```
Enumerates large veV holders. `--sells-on-votex` filters to those who delegate votes for sale.

```
gauge-calc competitors --validator <PUBKEY> [--top <N>]
```
Analyzes the top N validators by gauge weight. Infers whether each is locking $V, buying on Votex, or partnering with whales. Shows displacement cost.

```
gauge-calc eligibility --validator <PUBKEY>
```
Checks all gauge eligibility criteria against the 10-epoch rolling window.

```
gauge-calc votex [--epoch <N>] [--history <EPOCHS>]
```
Votex market state: current bids, clearing prices, total veV for sale, fee breakdown. Historical mode shows price trends.

```
gauge-calc epoch
```
Current Vault epoch number, phase, time remaining, vote deadline, and next commit date.

```
gauge-calc watch --validator <PUBKEY> [--alert-displacement <SOL>] [--alert-deadline <HOURS>]
```
Continuous monitoring. Alerts when: a competitor's votes threaten to displace you by more than `<SOL>`, the epoch deadline is within `<HOURS>`, or the gauge reserve size changes significantly.

---

## Data Sources

### On-Chain (Solana RPC)

| Data | Method | Address/Program |
|------|--------|-----------------|
| Vault stake pool state | `getAccountInfo` | `Fu9BYC6tWBo1KMKaP3CFoKfRhqv9akmy3DuYwnCyWiyC` (spl-stake-pool) |
| Pool validator list | spl-stake-pool deserialization | Derived from pool state |
| $V token supply/holders | `getTokenSupply`, `getTokenLargestAccounts` | $V mint address (TBD — resolve at runtime via known metadata) |
| veV locker accounts | `getProgramAccounts` with filters | Vault governance program (Tribeca-based, address TBD) |
| Gauge vote accounts | `getProgramAccounts` with filters | Gauge program (Tribeca gauge, address TBD) |
| Validator vote accounts | `getVoteAccounts` | Standard Solana RPC |
| Epoch info | `getEpochInfo` | Standard Solana RPC |

### Off-Chain / Web

| Data | Source | Method |
|------|--------|--------|
| Votex bid data | `https://votex.so/daos/vault/gauges` | HTML scraping (no public API) |
| Votex round timing | `https://votex.so/daos/vault/gauges` | HTML scraping |
| $V token price | Jupiter aggregator API / Birdeye API | REST |
| Vault pool stats | `https://solanacompass.com/stake-pools/Fu9BYC6...` | HTML scraping or StakeView API |
| Gauge voting UI | `https://tribeca.so/gov/vault/gauges` | Reference only |

### Program Address Resolution

The Vault's governance infrastructure is Tribeca-based. Gauge and locker program addresses are not publicly documented and must be discovered:

```rust
pub struct ProgramDiscovery;

impl ProgramDiscovery {
    /// Resolve Vault program addresses by following on-chain references
    /// starting from the known stake pool address.
    pub async fn discover(rpc: &RpcClient) -> Result<VaultPrograms> {
        let pool = Pubkey::from_str("Fu9BYC6tWBo1KMKaP3CFoKfRhqv9akmy3DuYwnCyWiyC")?;
        // 1. Parse spl-stake-pool state for manager/staker authorities
        // 2. Trace governance program from Tribeca's known factory
        // 3. Enumerate gauge program accounts by authority
        // 4. Resolve $V mint from governance token config
        todo!("Runtime discovery — addresses may also be hardcoded once confirmed")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultPrograms {
    pub stake_pool: Pubkey,
    pub governance: Pubkey,            // Tribeca governance
    pub gauge: Pubkey,                 // Tribeca gauge program
    pub locker: Pubkey,                // veV locker program
    pub v_mint: Pubkey,                // $V token mint
    pub vsol_mint: Pubkey,             // vSOL mint
}
```

---

## Core Calculations

### Cost-Per-SOL via Gauge

```rust
impl CostCalculator {
    /// How much veV is needed to capture `target_sol` from the gauge reserve?
    ///
    /// gauge_reserve * (my_vev / (total_vev + my_vev)) = target_sol
    /// Solving for my_vev:
    /// my_vev = (target_sol * total_vev) / (gauge_reserve - target_sol)
    pub fn vev_needed(
        gauge_reserve_sol: f64,
        total_vev: f64,
        target_sol: f64,
    ) -> Result<f64> {
        if target_sol >= gauge_reserve_sol {
            return Err(anyhow!("target exceeds gauge reserve"));
        }
        Ok((target_sol * total_vev) / (gauge_reserve_sol - target_sol))
    }

    /// Cost to acquire veV via Votex at current clearing price (per epoch)
    pub fn votex_cost(vev_needed: f64, clearing_price: f64) -> f64 {
        vev_needed * clearing_price / 0.85  // gross up for 15% platform fee
    }

    /// Cost to acquire veV by buying and locking $V tokens
    pub fn lock_cost(
        vev_needed: f64,
        v_price_usdc: f64,
        lock_years: f64,
    ) -> LockCostEstimate {
        let multiplier = LockMultiplierTable::multiplier(lock_years);
        let v_tokens = (vev_needed / multiplier).ceil() as u64;
        let cost = v_tokens as f64 * v_price_usdc;
        LockCostEstimate { v_tokens, lock_years, multiplier, cost_usdc: cost }
    }

    /// Annualized ROI: staking rewards earned vs cost of vote acquisition
    pub fn roi(
        target_sol: f64,
        pool_apy: f64,
        acquisition_cost: f64,
        strategy_is_recurring: bool,
    ) -> RoiEstimate {
        let annual_reward = target_sol * pool_apy;
        let annual_cost = if strategy_is_recurring {
            acquisition_cost * 52.0  // 52 weekly epochs per year
        } else {
            acquisition_cost  // one-time lock cost, amortized
        };
        let roi = (annual_reward - annual_cost) / annual_cost;
        let payback_epochs = if annual_reward > 0.0 {
            (acquisition_cost / (annual_reward / 52.0)).ceil() as u32
        } else {
            u32::MAX
        };
        RoiEstimate { annual_reward, annual_cost, roi, payback_epochs }
    }
}
```

### Displacement Cost

```rust
impl DisplacementAnalyzer {
    /// Cost to overtake a specific competitor in gauge ranking
    pub fn displacement_cost(
        target: &ValidatorGaugeEntry,
        gauge_state: &GaugeVoteState,
        votex_clearing_price: f64,
    ) -> DisplacementTarget {
        let vev_to_match = target.vev_weight;
        let vev_to_overtake = target.vev_weight * 1.01;
        let cost = CostCalculator::votex_cost(vev_to_overtake, votex_clearing_price);
        DisplacementTarget {
            validator: target.vote_account,
            name: target.name.clone(),
            current_vev: target.vev_weight,
            current_sol: target.projected_sol,
            vev_to_match,
            vev_to_overtake,
            estimated_cost_usdc: cost,
        }
    }
}
```

### Concentration Metrics

```rust
impl ConcentrationAnalyzer {
    pub fn hhi(shares: &[f64]) -> f64 {
        shares.iter().map(|s| (s * 100.0).powi(2)).sum()
    }

    pub fn gini(shares: &[f64]) -> f64 {
        let n = shares.len() as f64;
        let mean = shares.iter().sum::<f64>() / n;
        if mean == 0.0 { return 0.0; }
        let mut sorted = shares.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let numerator: f64 = sorted.iter().enumerate()
            .map(|(i, &x)| (2.0 * (i + 1) as f64 - n - 1.0) * x)
            .sum();
        numerator / (n * n * mean)
    }
}
```

---

## Configuration

```toml
# ~/.config/gauge-calc/config.toml

[rpc]
url = "https://api.mainnet-beta.solana.com"
requests_per_second = 5

[vault]
pool_address = "Fu9BYC6tWBo1KMKaP3CFoKfRhqv9akmy3DuYwnCyWiyC"
gauge_reserve_pct = 10

[validator]
vote_account = "YourVoteAccountPubkeyHere"

[votex]
scrape_url = "https://votex.so/daos/vault/gauges"
scrape_interval_minutes = 30

[price]
jupiter_api = "https://price.jup.ag/v6/price"
birdeye_api = "https://public-api.birdeye.so/defi/price"

[alerts]
displacement_threshold_sol = 1000
deadline_warning_hours = 6
webhook_url = ""
discord_webhook = ""

[snapshot]
db_path = "~/.local/share/gauge-calc/snapshots.db"
retain_epochs = 52
```

---

## SQLite Schema

```sql
CREATE TABLE pool_snapshots (
    id INTEGER PRIMARY KEY,
    epoch INTEGER NOT NULL,
    total_sol REAL NOT NULL,
    gauge_reserve_sol REAL NOT NULL,
    validator_count INTEGER NOT NULL,
    apy REAL NOT NULL,
    fetched_at TEXT NOT NULL
);

CREATE TABLE gauge_votes (
    id INTEGER PRIMARY KEY,
    epoch INTEGER NOT NULL,
    vote_account TEXT NOT NULL,
    validator_name TEXT NOT NULL,
    vev_weight REAL NOT NULL,
    vote_share REAL NOT NULL,
    projected_sol REAL NOT NULL,
    eligible INTEGER NOT NULL,
    UNIQUE(epoch, vote_account)
);

CREATE TABLE votex_rounds (
    id INTEGER PRIMARY KEY,
    epoch INTEGER NOT NULL UNIQUE,
    total_vev_for_sale REAL NOT NULL,
    total_usdc_bid REAL NOT NULL,
    clearing_price REAL NOT NULL,
    bid_count INTEGER NOT NULL,
    fetched_at TEXT NOT NULL
);

CREATE TABLE votex_bids (
    id INTEGER PRIMARY KEY,
    epoch INTEGER NOT NULL,
    vote_account TEXT NOT NULL,
    validator_name TEXT NOT NULL,
    usdc_deposited REAL NOT NULL,
    vev_acquired REAL NOT NULL,
    UNIQUE(epoch, vote_account)
);

CREATE TABLE vev_holders (
    id INTEGER PRIMARY KEY,
    epoch INTEGER NOT NULL,
    owner TEXT NOT NULL,
    total_vev REAL NOT NULL,
    v_locked INTEGER NOT NULL,
    lock_expiry TEXT NOT NULL,
    vote_target TEXT,
    sells_on_votex INTEGER NOT NULL,
    UNIQUE(epoch, owner)
);

CREATE TABLE concentration_history (
    id INTEGER PRIMARY KEY,
    epoch INTEGER NOT NULL UNIQUE,
    hhi REAL NOT NULL,
    gini REAL NOT NULL,
    top_1_share REAL NOT NULL,
    top_5_share REAL NOT NULL,
    top_10_share REAL NOT NULL,
    effective_competitors REAL NOT NULL
);

CREATE INDEX idx_gauge_votes_epoch ON gauge_votes(epoch);
CREATE INDEX idx_votex_bids_epoch ON votex_bids(epoch);
CREATE INDEX idx_vev_holders_epoch ON vev_holders(epoch);
```

---

## Implementation Notes

### Votex Scraping Strategy

Votex has no public API. Data must be scraped from the web UI or derived from on-chain state:

1. **Primary**: Parse on-chain gauge/locker program accounts directly. This gives veV balances, lock durations, and vote allocations without scraping.
2. **Fallback**: Scrape `votex.so/daos/vault/gauges` for bid amounts and clearing prices that aren't derivable from on-chain state alone.
3. **Rate limit**: Respect robots.txt and scrape at ≤2 requests/minute.

### Tribeca Program Account Layout

The Vault uses Tribeca's governance framework. Key account types to deserialize:

- **Locker**: Contains veV parameters (lock duration, base amount, escrow configuration)
- **Escrow**: Per-user lock position (amount, start, end, veV balance)
- **Gauge**: Per-validator vote weight aggregation
- **GaugeVoter**: Per-user vote allocation across gauges
- **GaugeVote**: Individual vote record linking escrow to gauge

These use Borsh serialization. Account discriminators follow Anchor's 8-byte SHA256 prefix convention.

### Edge Cases

- **Zero total votes**: If no one votes, gauge reserve distributes equally among eligible validators (or not at all). Handle the division-by-zero in `vev_needed`.
- **Validator loses eligibility mid-epoch**: Votes for ineligible validators are wasted. The tool should warn if a target validator has deteriorating metrics.
- **veV decay**: A position locked 2 years ago for 3 years has different current veV than at lock time. Always compute veV at the query timestamp, not at lock time.
- **Votex bid competition**: The clearing price is endogenous — your bid changes it. For large positions, model the price impact of your own bid.

---

## Example Output

```
$ gauge-calc status

┌────────────────────────────────────────────────────────┐
│ Vault Gauge Status — Epoch 47                          │
├──────────────────────┬─────────────────────────────────┤
│ Pool Total           │ 1,242,657 SOL                   │
│ Gauge Reserve (10%)  │ 124,266 SOL                     │
│ Total veV Voting     │ 8,432,100 veV                   │
│ Eligible Validators  │ 94 / 128                        │
│ Active Gauge Voters  │ 312                             │
│ Pool APY             │ 5.84%                           │
├──────────────────────┼─────────────────────────────────┤
│ Epoch Phase          │ Vote Buying (3d 14h remaining)  │
│ Vote Deadline        │ 2026-02-18 00:00 UTC            │
│ Commit Date          │ 2026-02-19 (Monday)             │
└──────────────────────┴─────────────────────────────────┘

Top 10 Gauge Validators:
┌────┬──────────────────────┬────────────┬────────┬──────────────┐
│  # │ Validator            │ veV Weight │ Share  │ Projected SOL│
├────┼──────────────────────┼────────────┼────────┼──────────────┤
│  1 │ Helius               │  1,204,500 │ 14.29% │       17,757 │
│  2 │ Triton               │    892,300 │ 10.59% │       13,153 │
│  3 │ Coinbase Cloud       │    743,100 │  8.81% │       10,944 │
│  4 │ Everstake            │    621,800 │  7.38% │        9,167 │
│  5 │ Laine                │    534,200 │  6.34% │        7,873 │
│  6 │ Figment              │    412,700 │  4.89% │        6,081 │
│  7 │ Chorus One           │    389,400 │  4.62% │        5,737 │
│  8 │ Staking Facilities   │    301,500 │  3.58% │        4,442 │
│  9 │ P2P Validator        │    278,900 │  3.31% │        4,109 │
│ 10 │ Shinobi Systems      │    244,600 │  2.90% │        3,604 │
└────┴──────────────────────┴────────────┴────────┴──────────────┘
HHI: 612  Gini: 0.58  Top-5 Share: 47.4%

$ gauge-calc calculate --target-sol 5000 --strategy all

Target: 5,000 SOL from gauge reserve (124,266 SOL available)
veV needed: 353,892 veV (to capture 4.03% of gauge)

┌──────────────────┬───────────────┬────────────┬────────────┬──────────┐
│ Strategy         │ Cost (USDC)   │ $/SOL/year │ Annual ROI │ Payback  │
├──────────────────┼───────────────┼────────────┼────────────┼──────────┤
│ Buy on Votex     │  4,720/epoch  │    $49.02  │     +19.2% │ 41 weeks │
│ Lock $V (1 year) │ 88,473 once   │    $17.69  │     +230%  │ 15 weeks │
│ Lock $V (3 year) │ 29,491 once   │     $5.90  │   +4,847%  │  5 weeks │
│ Lock $V (5 year) │ 17,695 once   │     $3.54  │   +8,186%  │  3 weeks │
│ Hybrid (1y+Votex)│ 52,000 mixed  │    $11.40  │     +412%  │  9 weeks │
└──────────────────┴───────────────┴────────────┴────────────┴──────────┘
Recommended: Lock $V (3 year) — best capital efficiency for target size

$ gauge-calc epoch

Vault Epoch: 47
Phase:       Vote Buying
Started:     2026-02-12 00:00 UTC
Deadline:    2026-02-18 00:00 UTC (3d 14h)
Commit:      2026-02-19 00:00 UTC (Monday)
Stake moves: Epoch 48 (begins ~2026-02-19, full arrival ~5 Solana epochs)
```

---

## Future Extensions

- **Auto-bidder**: Place Votex bids programmatically when clearing price drops below threshold (requires wallet integration)
- **Multi-pool gauge comparison**: SolBlaze also uses Votex gauges — compare ROI across pools
- **Delegation-Oracle integration**: Feed gauge eligibility status into the unified eligibility dashboard
- **Whale CRM**: Track whale voter relationships over time, predict vote allocation shifts
- **Telegram/Discord bot**: Push epoch deadline reminders and displacement alerts to chat

---

## References

- [The Vault — Stakepool Delegation Overview](https://docs.thevault.finance/validators/stakepool-delegation-overview)
- [The Vault — Gauges](https://docs.thevault.finance/about/gauges)
- [The Vault — Delegation FAQs](https://docs.thevault.finance/validators/delegation-faqs)
- [Votex — The Vault DAO](https://docs.votex.so/daos/vault)
- [vSOL Stake Pool on Solana Compass](https://solanacompass.com/stake-pools/Fu9BYC6tWBo1KMKaP3CFoKfRhqv9akmy3DuYwnCyWiyC)
- [Tribeca Gauge Voting UI](https://tribeca.so/gov/vault/gauges)
- [$V Token TGE — Solana Floor](https://solanafloor.com/news/why-the-vaults-v-will-tge-using-1-token-supply)
