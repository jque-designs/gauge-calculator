use anyhow::Result;

use crate::{
    config::AppConfig,
    live::{LiveDataClient, LiveDataSnapshot},
    types::SampleContext,
};

#[derive(Debug, Clone)]
pub struct RuntimeSnapshot {
    pub context: SampleContext,
    pub live_data: Option<LiveDataSnapshot>,
    pub sol_price_usdc: f64,
}

pub fn build_runtime_context(config: &AppConfig, live_enabled: bool) -> Result<RuntimeSnapshot> {
    let mut context = SampleContext::new();
    let live_data = if live_enabled {
        let client = LiveDataClient::new()?;
        let snapshot = client.fetch_all(
            &config.rpc.url,
            &config.price.jupiter_api,
            &config.votex.scrape_url,
        );
        apply_live_overlay(&mut context, &snapshot);
        Some(snapshot)
    } else {
        None
    };
    let sol_price_usdc = live_data
        .as_ref()
        .and_then(|snapshot| snapshot.sol_price_usdc)
        .unwrap_or(200.0);

    Ok(RuntimeSnapshot {
        context,
        live_data,
        sol_price_usdc,
    })
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
