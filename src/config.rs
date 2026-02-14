use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub rpc: RpcConfig,
    pub vault: VaultConfig,
    pub validator: ValidatorConfig,
    pub votex: VotexConfig,
    pub price: PriceConfig,
    pub alerts: AlertsConfig,
    pub snapshot: SnapshotConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcConfig {
    pub url: String,
    pub requests_per_second: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultConfig {
    pub pool_address: String,
    pub gauge_reserve_pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatorConfig {
    pub vote_account: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexConfig {
    pub scrape_url: String,
    pub scrape_interval_minutes: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceConfig {
    pub jupiter_api: String,
    pub birdeye_api: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertsConfig {
    pub displacement_threshold_sol: f64,
    pub deadline_warning_hours: u32,
    pub webhook_url: String,
    pub discord_webhook: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotConfig {
    pub db_path: String,
    pub retain_epochs: u32,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            rpc: RpcConfig {
                url: "https://api.mainnet-beta.solana.com".to_string(),
                requests_per_second: 5,
            },
            vault: VaultConfig {
                pool_address: "Fu9BYC6tWBo1KMKaP3CFoKfRhqv9akmy3DuYwnCyWiyC".to_string(),
                gauge_reserve_pct: 10.0,
            },
            validator: ValidatorConfig { vote_account: None },
            votex: VotexConfig {
                scrape_url: "https://votex.so/daos/vault/gauges".to_string(),
                scrape_interval_minutes: 30,
            },
            price: PriceConfig {
                jupiter_api: "https://price.jup.ag/v6/price".to_string(),
                birdeye_api: "https://public-api.birdeye.so/defi/price".to_string(),
            },
            alerts: AlertsConfig {
                displacement_threshold_sol: 1_000.0,
                deadline_warning_hours: 6,
                webhook_url: String::new(),
                discord_webhook: String::new(),
            },
            snapshot: SnapshotConfig {
                db_path: "~/.local/share/gauge-calc/snapshots.db".to_string(),
                retain_epochs: 52,
            },
        }
    }
}

impl AppConfig {
    pub fn load_or_default(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let raw = fs::read_to_string(path)
            .with_context(|| format!("failed to read config file from {}", path.display()))?;
        let parsed = toml::from_str::<Self>(&raw)
            .with_context(|| format!("failed to parse TOML config at {}", path.display()))?;
        Ok(parsed)
    }

    pub fn merge_cli_overrides(&mut self, rpc_override: Option<String>) {
        if let Some(rpc) = rpc_override {
            self.rpc.url = rpc;
        }
    }
}

pub fn default_config_path() -> PathBuf {
    let base = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("gauge-calc").join("config.toml")
}
