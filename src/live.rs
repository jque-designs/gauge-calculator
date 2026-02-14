use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const SOL_MINT: &str = "So11111111111111111111111111111111111111112";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveDataSnapshot {
    pub fetched_at: DateTime<Utc>,
    pub rpc_epoch: Option<RpcEpochInfo>,
    pub vote_accounts: Option<VoteAccountsSummary>,
    pub sol_price_usdc: Option<f64>,
    pub votex_probe: Option<VotexProbe>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcEpochInfo {
    pub epoch: u64,
    pub slot_index: u64,
    pub slots_in_epoch: u64,
    pub absolute_slot: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoteAccountsSummary {
    pub current_count: usize,
    pub delinquent_count: usize,
    pub avg_commission_current: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VotexProbe {
    pub url: String,
    pub status_code: u16,
    pub content_length: usize,
    pub contains_vault_keyword: bool,
}

pub struct LiveDataClient {
    http: Client,
}

impl LiveDataClient {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(15))
            .user_agent("gauge-calc/0.1 (cloud-agent)")
            .build()
            .context("failed to build HTTP client")?;
        Ok(Self { http })
    }

    pub fn fetch_all(
        &self,
        rpc_url: &str,
        jupiter_price_url: &str,
        votex_url: &str,
    ) -> LiveDataSnapshot {
        let mut warnings = Vec::new();

        let rpc_epoch = match self.fetch_rpc_epoch(rpc_url) {
            Ok(value) => Some(value),
            Err(err) => {
                warnings.push(format!("rpc_epoch_fetch_failed: {err:#}"));
                None
            }
        };

        let vote_accounts = match self.fetch_vote_accounts(rpc_url) {
            Ok(value) => Some(value),
            Err(err) => {
                warnings.push(format!("vote_accounts_fetch_failed: {err:#}"));
                None
            }
        };

        let sol_price_usdc = match self.fetch_sol_price_usdc(jupiter_price_url) {
            Ok(value) => Some(value),
            Err(err) => {
                warnings.push(format!("sol_price_fetch_failed: {err:#}"));
                None
            }
        };

        let votex_probe = match self.probe_votex(votex_url) {
            Ok(value) => Some(value),
            Err(err) => {
                warnings.push(format!("votex_probe_failed: {err:#}"));
                None
            }
        };

        LiveDataSnapshot {
            fetched_at: Utc::now(),
            rpc_epoch,
            vote_accounts,
            sol_price_usdc,
            votex_probe,
            warnings,
        }
    }

    pub fn fetch_rpc_epoch(&self, rpc_url: &str) -> Result<RpcEpochInfo> {
        let response = self.rpc_call(rpc_url, "getEpochInfo", json!([]))?;
        let result = response
            .get("result")
            .ok_or_else(|| anyhow!("missing result in getEpochInfo response"))?;
        Ok(RpcEpochInfo {
            epoch: json_u64(result, "epoch")?,
            slot_index: json_u64(result, "slotIndex")?,
            slots_in_epoch: json_u64(result, "slotsInEpoch")?,
            absolute_slot: json_u64(result, "absoluteSlot")?,
        })
    }

    pub fn fetch_vote_accounts(&self, rpc_url: &str) -> Result<VoteAccountsSummary> {
        let response = self.rpc_call(rpc_url, "getVoteAccounts", json!([]))?;
        let result = response
            .get("result")
            .ok_or_else(|| anyhow!("missing result in getVoteAccounts response"))?;
        let current = result
            .get("current")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing current vote account array"))?;
        let delinquent = result
            .get("delinquent")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("missing delinquent vote account array"))?;

        let total_commission: f64 = current
            .iter()
            .filter_map(|v| v.get("commission").and_then(Value::as_f64))
            .sum();
        let avg_commission_current = if current.is_empty() {
            0.0
        } else {
            total_commission / current.len() as f64
        };

        Ok(VoteAccountsSummary {
            current_count: current.len(),
            delinquent_count: delinquent.len(),
            avg_commission_current,
        })
    }

    pub fn fetch_sol_price_usdc(&self, jupiter_price_url: &str) -> Result<f64> {
        let url = if jupiter_price_url.contains('?') {
            format!("{jupiter_price_url}&ids={SOL_MINT}")
        } else {
            format!("{jupiter_price_url}?ids={SOL_MINT}")
        };
        let response = self
            .http
            .get(url)
            .send()
            .context("failed to request Jupiter price endpoint")?
            .error_for_status()
            .context("non-success status from Jupiter price endpoint")?;

        let body = response
            .json::<Value>()
            .context("failed to parse Jupiter price response")?;

        parse_price_response(&body)
            .ok_or_else(|| anyhow!("could not parse SOL price from response"))
    }

    pub fn probe_votex(&self, votex_url: &str) -> Result<VotexProbe> {
        let response = self
            .http
            .get(votex_url)
            .send()
            .context("failed to fetch Votex page")?;
        let status_code = response.status().as_u16();
        let response = response
            .error_for_status()
            .context("non-success status from Votex page")?;
        let text = response.text().context("failed to read Votex body")?;
        let lowered = text.to_ascii_lowercase();
        Ok(VotexProbe {
            url: votex_url.to_string(),
            status_code,
            content_length: text.len(),
            contains_vault_keyword: lowered.contains("vault") || lowered.contains("gauges"),
        })
    }

    fn rpc_call(&self, rpc_url: &str, method: &str, params: Value) -> Result<Value> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });
        let response = self
            .http
            .post(rpc_url)
            .json(&payload)
            .send()
            .with_context(|| format!("RPC request failed for method {method}"))?
            .error_for_status()
            .with_context(|| format!("RPC returned non-success for method {method}"))?;

        let value = response
            .json::<Value>()
            .with_context(|| format!("failed to parse RPC response for method {method}"))?;
        if value.get("error").is_some() {
            return Err(anyhow!("RPC returned error for method {method}: {value}"));
        }
        Ok(value)
    }
}

pub fn parse_price_response(body: &Value) -> Option<f64> {
    body.get("data")
        .and_then(Value::as_object)
        .and_then(|data| data.get(SOL_MINT))
        .and_then(|entry| entry.get("price"))
        .and_then(Value::as_f64)
        .or_else(|| {
            body.get(SOL_MINT)
                .and_then(|entry| entry.get("price"))
                .and_then(Value::as_f64)
        })
}

fn json_u64(value: &Value, key: &str) -> Result<u64> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("missing or invalid u64 key: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_price_shape_with_data_map() {
        let body = json!({
            "data": {
                "So11111111111111111111111111111111111111112": {
                    "id": "So11111111111111111111111111111111111111112",
                    "price": 212.45
                }
            }
        });
        let price = parse_price_response(&body).unwrap();
        assert!((price - 212.45).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_price_shape_without_data_key() {
        let body = json!({
            "So11111111111111111111111111111111111111112": {
                "price": 199.99
            }
        });
        let price = parse_price_response(&body).unwrap();
        assert!((price - 199.99).abs() < f64::EPSILON);
    }
}
