use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::{
    analysis::ConcentrationAnalyzer,
    calculator::{compare_strategies, AcquisitionStrategy, StrategyComparison, StrategyRequest},
    config::AppConfig,
    runtime::build_runtime_context,
    snapshot::{resolve_path as resolve_snapshot_path, SnapshotStore},
    types::GaugeEligibility,
};

#[derive(Clone)]
pub struct ApiState {
    pub config: Arc<AppConfig>,
    pub default_live: bool,
}

impl ApiState {
    pub fn new(config: AppConfig, default_live: bool) -> Self {
        Self {
            config: Arc::new(config),
            default_live,
        }
    }
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/calculate", get(calculate))
        .route("/compare", get(compare))
        .route("/concentration", get(concentration))
        .route("/competitors", get(competitors))
        .route("/eligibility", get(eligibility))
        .route("/votex", get(votex))
        .route("/epoch", get(epoch))
        .route("/snapshot/save", post(snapshot_save))
        .route("/snapshot/load", get(snapshot_load))
        .route("/snapshot/list", get(snapshot_list))
        .with_state(state)
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    fn bad_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            message: message.into(),
        }
    }

    fn internal(error: anyhow::Error) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: error.to_string(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        Self::internal(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(json!({
                "error": self.message
            })),
        )
            .into_response()
    }
}

type ApiResult<T> = std::result::Result<Json<T>, ApiError>;

#[derive(Debug, Deserialize)]
struct LiveQuery {
    live: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct StatusQuery {
    validator: Option<String>,
    live: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CalculateQuery {
    target_sol: f64,
    strategy: Option<ApiStrategyFilter>,
    lock_years: Option<f64>,
    live: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CompareQuery {
    target_sol: f64,
    lock_years: Option<f64>,
    v_price: Option<f64>,
    live: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct CompetitorsQuery {
    validator: Option<String>,
    top: Option<usize>,
    live: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct EligibilityQuery {
    validator: Option<String>,
    live: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct SnapshotLoadQuery {
    epoch: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SnapshotListQuery {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum ApiStrategyFilter {
    Lock,
    Buy,
    Hybrid,
    All,
}

async fn index() -> Json<Value> {
    Json(json!({
        "name": "gauge-api",
        "endpoints": [
            "GET /health",
            "GET /status",
            "GET /calculate",
            "GET /compare",
            "GET /concentration",
            "GET /competitors",
            "GET /eligibility",
            "GET /votex",
            "GET /epoch",
            "POST /snapshot/save",
            "GET /snapshot/load",
            "GET /snapshot/list"
        ]
    }))
}

async fn health() -> Json<Value> {
    Json(json!({
        "ok": true
    }))
}

async fn status(
    State(state): State<ApiState>,
    Query(query): Query<StatusQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    if let Some(target) = query.validator {
        let validator = runtime
            .context
            .gauge
            .validators
            .iter()
            .find(|v| v.vote_account == target || v.name.eq_ignore_ascii_case(&target))
            .ok_or_else(|| ApiError::bad_request(format!("validator not found: {target}")))?;
        return Ok(Json(json!({
            "validator": validator,
            "live": runtime.live_data,
            "sol_price_usdc": runtime.sol_price_usdc
        })));
    }

    Ok(Json(json!({
        "context": runtime.context,
        "live": runtime.live_data,
        "sol_price_usdc": runtime.sol_price_usdc
    })))
}

async fn calculate(
    State(state): State<ApiState>,
    Query(query): Query<CalculateQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let mut comparison = compare_strategies(&StrategyRequest {
        target_sol: query.target_sol,
        gauge_reserve_sol: runtime.context.pool.gauge_reserve_sol,
        total_vev: runtime.context.gauge.total_vev_voting,
        pool_apy: runtime.context.pool.apy,
        sol_price_usdc: runtime.sol_price_usdc,
        votex_clearing_price: runtime.context.votex.clearing_price_per_vev,
        v_price_usdc: runtime.context.vtoken.price_usdc,
        lock_years: query.lock_years.unwrap_or(3.0),
    })?;

    filter_strategies(
        &mut comparison,
        query.strategy.unwrap_or(ApiStrategyFilter::All),
    );
    Ok(Json(json!({
        "comparison": comparison,
        "sol_price_usdc": runtime.sol_price_usdc,
        "live": runtime.live_data
    })))
}

async fn compare(
    State(state): State<ApiState>,
    Query(query): Query<CompareQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let comparison = compare_strategies(&StrategyRequest {
        target_sol: query.target_sol,
        gauge_reserve_sol: runtime.context.pool.gauge_reserve_sol,
        total_vev: runtime.context.gauge.total_vev_voting,
        pool_apy: runtime.context.pool.apy,
        sol_price_usdc: runtime.sol_price_usdc,
        votex_clearing_price: runtime.context.votex.clearing_price_per_vev,
        v_price_usdc: query.v_price.unwrap_or(runtime.context.vtoken.price_usdc),
        lock_years: query.lock_years.unwrap_or(3.0),
    })?;

    Ok(Json(json!({
        "comparison": comparison,
        "sol_price_usdc": runtime.sol_price_usdc,
        "live": runtime.live_data
    })))
}

async fn concentration(
    State(state): State<ApiState>,
    Query(query): Query<LiveQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let report = ConcentrationAnalyzer::analyze(
        &runtime.context.gauge,
        runtime.context.votex.clearing_price_per_vev,
    );
    Ok(Json(json!({
        "report": report,
        "live": runtime.live_data
    })))
}

async fn competitors(
    State(state): State<ApiState>,
    Query(query): Query<CompetitorsQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let mut profiles =
        crate::analysis::classify_competitors(&runtime.context.gauge, &runtime.context.votex.bids);
    if let Some(target) = query.validator {
        profiles.retain(|p| p.validator == target || p.name.eq_ignore_ascii_case(&target));
    }
    profiles.truncate(query.top.unwrap_or(10));
    Ok(Json(json!({
        "competitors": profiles,
        "live": runtime.live_data
    })))
}

async fn eligibility(
    State(state): State<ApiState>,
    Query(query): Query<EligibilityQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let target = query
        .validator
        .or_else(|| state.config.validator.vote_account.clone())
        .ok_or_else(|| ApiError::bad_request("validator is required"))?;

    let entry = runtime
        .context
        .gauge
        .validators
        .iter()
        .find(|v| v.vote_account == target || v.name.eq_ignore_ascii_case(&target))
        .ok_or_else(|| ApiError::bad_request(format!("validator not found: {target}")))?;
    let eligibility = GaugeEligibility {
        vote_account: entry.vote_account.clone(),
        eligible: entry.eligible,
        issues: entry.eligibility_issues.clone(),
        evaluated_over_epochs: 10,
    };
    Ok(Json(json!({
        "eligibility": eligibility,
        "live": runtime.live_data
    })))
}

async fn votex(State(state): State<ApiState>, Query(query): Query<LiveQuery>) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    Ok(Json(json!({
        "market": runtime.context.votex,
        "live": runtime.live_data
    })))
}

async fn epoch(State(state): State<ApiState>, Query(query): Query<LiveQuery>) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    Ok(Json(json!({
        "epoch": runtime.context.gauge.epoch,
        "live": runtime.live_data
    })))
}

async fn snapshot_save(
    State(state): State<ApiState>,
    Query(query): Query<LiveQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let epoch = runtime
        .live_data
        .as_ref()
        .and_then(|l| l.rpc_epoch.as_ref())
        .map(|e| e.epoch)
        .unwrap_or(runtime.context.gauge.epoch.epoch_number as u64)
        .min(u32::MAX as u64) as u32;
    let store = SnapshotStore::open(&state.config.snapshot.db_path)?;
    let payload = json!({
        "context": runtime.context,
        "live": runtime.live_data,
    });
    let saved = store.save_payload(epoch, "state", &payload)?;
    Ok(Json(json!({
        "saved": saved,
        "db_path": resolve_snapshot_path(&state.config.snapshot.db_path)
    })))
}

async fn snapshot_load(
    State(state): State<ApiState>,
    Query(query): Query<SnapshotLoadQuery>,
) -> ApiResult<Value> {
    let store = SnapshotStore::open(&state.config.snapshot.db_path)?;
    let snapshot = store.load("state", query.epoch)?;
    Ok(Json(json!({
        "snapshot": snapshot,
        "db_path": resolve_snapshot_path(&state.config.snapshot.db_path)
    })))
}

async fn snapshot_list(
    State(state): State<ApiState>,
    Query(query): Query<SnapshotListQuery>,
) -> ApiResult<Value> {
    let store = SnapshotStore::open(&state.config.snapshot.db_path)?;
    let limit = query.limit.unwrap_or(50).min(500);
    let snapshots = store.list(Some("state"), limit)?;
    Ok(Json(json!({
        "snapshots": snapshots,
        "db_path": resolve_snapshot_path(&state.config.snapshot.db_path)
    })))
}

fn runtime_for(
    state: &ApiState,
    live_override: Option<bool>,
) -> Result<crate::runtime::RuntimeSnapshot> {
    let live_enabled = live_override.unwrap_or(state.default_live);
    build_runtime_context(&state.config, live_enabled)
}

fn filter_strategies(comparison: &mut StrategyComparison, strategy: ApiStrategyFilter) {
    if matches!(strategy, ApiStrategyFilter::All) {
        return;
    }
    comparison.strategies.retain(|s| match strategy {
        ApiStrategyFilter::All => true,
        ApiStrategyFilter::Lock => matches!(s.strategy, AcquisitionStrategy::LockVTokens { .. }),
        ApiStrategyFilter::Buy => matches!(s.strategy, AcquisitionStrategy::BuyOnVotex { .. }),
        ApiStrategyFilter::Hybrid => matches!(s.strategy, AcquisitionStrategy::Hybrid { .. }),
    });
    comparison.recommended = 0;
    comparison.recommendation_reason = "Filtered to requested strategy type".to_string();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn filter_strategies_keeps_requested_kind() {
        let req = StrategyRequest {
            target_sol: 1_000.0,
            gauge_reserve_sol: 124_266.0,
            total_vev: 5_000_000.0,
            pool_apy: 0.058,
            sol_price_usdc: 200.0,
            votex_clearing_price: 0.05,
            v_price_usdc: 0.5,
            lock_years: 3.0,
        };
        let mut comparison = compare_strategies(&req).unwrap();
        filter_strategies(&mut comparison, ApiStrategyFilter::Buy);
        assert!(comparison
            .strategies
            .iter()
            .all(|s| matches!(s.strategy, AcquisitionStrategy::BuyOnVotex { .. })));
    }

    #[test]
    fn runtime_for_respects_live_override() {
        let state = ApiState::new(AppConfig::default(), false);
        let snapshot = runtime_for(&state, Some(false)).unwrap();
        assert!(snapshot.live_data.is_none());
    }
}
