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
    types::{GaugeEligibility, ValidatorGaugeEntry},
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
    let api = Router::new()
        .route("/docs", get(api_docs))
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/calculate", get(calculate))
        .route("/compare", get(compare))
        .route("/concentration", get(concentration))
        .route("/competitors", get(competitors))
        .route("/eligibility", get(eligibility))
        .route("/votex", get(votex))
        .route("/epoch", get(epoch))
        .route("/threats", get(threats))
        .route("/opportunities", get(opportunities))
        .route("/queue", get(queue))
        .route("/cohorts", get(cohorts))
        .route("/snapshot/save", post(snapshot_save))
        .route("/snapshot/load", get(snapshot_load))
        .route("/snapshot/list", get(snapshot_list));

    Router::new()
        .route("/", get(index))
        .nest("/api", api)
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

#[derive(Debug, Deserialize)]
struct ThreatsQuery {
    validator: Option<String>,
    live: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct QueueQuery {
    validator: Option<String>,
    pool: Option<String>,
    live: Option<bool>,
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
        "docs": "/api/docs"
    }))
}

async fn api_docs() -> Json<Value> {
    Json(json!({
        "name": "gauge-api",
        "routes": [
            {"method": "GET", "path": "/api/health", "description": "Basic health check"},
            {"method": "GET", "path": "/api/docs", "description": "List available routes"},
            {"method": "GET", "path": "/api/threats?validator=<pubkey>", "description": "Threat assessment for validator"},
            {"method": "GET", "path": "/api/opportunities", "description": "Decay/displacement opportunities"},
            {"method": "GET", "path": "/api/queue?validator=<pubkey>&pool=<pool>", "description": "Stake pool queue position"},
            {"method": "GET", "path": "/api/cohorts", "description": "Cohort flow analysis"},
            {"method": "GET", "path": "/api/status", "description": "Current gauge status"},
            {"method": "GET", "path": "/api/calculate?target_sol=<sol>", "description": "Strategy calculator"},
            {"method": "GET", "path": "/api/compare?target_sol=<sol>", "description": "Strategy comparison"},
            {"method": "GET", "path": "/api/concentration", "description": "Concentration metrics"},
            {"method": "GET", "path": "/api/competitors", "description": "Competitor classifier"},
            {"method": "GET", "path": "/api/eligibility?validator=<pubkey>", "description": "Eligibility check"},
            {"method": "GET", "path": "/api/votex", "description": "Votex market snapshot"},
            {"method": "GET", "path": "/api/epoch", "description": "Epoch timing"},
            {"method": "POST", "path": "/api/snapshot/save", "description": "Persist snapshot"},
            {"method": "GET", "path": "/api/snapshot/load", "description": "Load persisted snapshot"},
            {"method": "GET", "path": "/api/snapshot/list", "description": "List persisted snapshots"}
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

async fn threats(
    State(state): State<ApiState>,
    Query(query): Query<ThreatsQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let target = query
        .validator
        .ok_or_else(|| ApiError::bad_request("validator query parameter is required"))?;
    let validator = find_validator(&runtime.context.gauge.validators, &target)
        .ok_or_else(|| ApiError::bad_request(format!("validator not found: {target}")))?;
    let report = ConcentrationAnalyzer::analyze(
        &runtime.context.gauge,
        runtime.context.votex.clearing_price_per_vev,
    );
    let displacement = report
        .displacement_cost
        .iter()
        .find(|entry| entry.validator == validator.vote_account)
        .cloned();

    let mut ranked = runtime.context.gauge.validators.clone();
    ranked.sort_by(|a, b| b.projected_sol.total_cmp(&a.projected_sol));
    let rank = ranked
        .iter()
        .position(|v| v.vote_account == validator.vote_account)
        .map(|i| i + 1)
        .unwrap_or(0);

    let threat_level = displacement
        .as_ref()
        .map(|d| classify_threat_level(d.estimated_cost_usdc))
        .unwrap_or("unknown");

    let neighbors = ranking_neighbors(&ranked, rank.saturating_sub(1));

    Ok(Json(json!({
        "validator": validator,
        "rank": rank,
        "threat_level": threat_level,
        "estimated_displacement": displacement,
        "neighbors": neighbors,
        "live": runtime.live_data
    })))
}

async fn opportunities(
    State(state): State<ApiState>,
    Query(query): Query<LiveQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let report = ConcentrationAnalyzer::analyze(
        &runtime.context.gauge,
        runtime.context.votex.clearing_price_per_vev,
    );

    let mut cheapest = report.displacement_cost.clone();
    cheapest.sort_by(|a, b| a.estimated_cost_usdc.total_cmp(&b.estimated_cost_usdc));
    let cheapest = cheapest.into_iter().take(5).collect::<Vec<_>>();

    let fragile = runtime
        .context
        .gauge
        .validators
        .iter()
        .filter(|v| !v.eligibility_issues.is_empty() || !v.eligible)
        .map(|v| {
            json!({
                "validator": v.vote_account,
                "name": v.name,
                "eligible": v.eligible,
                "issues": v.eligibility_issues,
                "projected_sol": v.projected_sol
            })
        })
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "decay_opportunities": cheapest,
        "fragile_validators": fragile,
        "note": "Decay opportunities are approximated via displacement cost and eligibility fragility.",
        "live": runtime.live_data
    })))
}

async fn queue(State(state): State<ApiState>, Query(query): Query<QueueQuery>) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let target = query
        .validator
        .ok_or_else(|| ApiError::bad_request("validator query parameter is required"))?;
    let pool = query
        .pool
        .ok_or_else(|| ApiError::bad_request("pool query parameter is required"))?;

    let mut ranked = runtime.context.gauge.validators.clone();
    ranked.sort_by(|a, b| b.projected_sol.total_cmp(&a.projected_sol));
    let validator = find_validator(&ranked, &target)
        .ok_or_else(|| ApiError::bad_request(format!("validator not found: {target}")))?;
    let position = ranked
        .iter()
        .position(|v| v.vote_account == validator.vote_account)
        .map(|idx| idx + 1)
        .unwrap_or(0);

    Ok(Json(json!({
        "pool": pool,
        "validator": {
            "vote_account": validator.vote_account,
            "name": validator.name
        },
        "queue_position": position,
        "queue_size": ranked.len(),
        "projected_sol": validator.projected_sol,
        "share": validator.vote_share,
        "live": runtime.live_data
    })))
}

async fn cohorts(
    State(state): State<ApiState>,
    Query(query): Query<LiveQuery>,
) -> ApiResult<Value> {
    let runtime = runtime_for(&state, query.live)?;
    let mut ranked = runtime.context.gauge.validators.clone();
    ranked.sort_by(|a, b| b.vote_share.total_cmp(&a.vote_share));
    let len = ranked.len();
    let first_cut = len.div_ceil(3);
    let second_cut = (len * 2).div_ceil(3);

    let top = cohort_summary("top", &ranked[..first_cut], runtime.context.pool.gauge_reserve_sol);
    let middle = cohort_summary(
        "middle",
        &ranked[first_cut..second_cut],
        runtime.context.pool.gauge_reserve_sol,
    );
    let tail = cohort_summary(
        "tail",
        &ranked[second_cut..],
        runtime.context.pool.gauge_reserve_sol,
    );

    Ok(Json(json!({
        "cohorts": [top, middle, tail],
        "total_validators": len,
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

fn find_validator<'a>(
    validators: &'a [ValidatorGaugeEntry],
    target: &str,
) -> Option<&'a ValidatorGaugeEntry> {
    validators
        .iter()
        .find(|v| v.vote_account == target || v.name.eq_ignore_ascii_case(target))
}

fn classify_threat_level(estimated_cost_usdc: f64) -> &'static str {
    if estimated_cost_usdc < 10_000.0 {
        "high"
    } else if estimated_cost_usdc < 25_000.0 {
        "medium"
    } else {
        "low"
    }
}

fn ranking_neighbors(ranked: &[ValidatorGaugeEntry], idx: usize) -> Value {
    let above = idx
        .checked_sub(1)
        .and_then(|i| ranked.get(i))
        .map(|v| json!({"vote_account": v.vote_account, "name": v.name, "projected_sol": v.projected_sol}));
    let below = ranked
        .get(idx + 1)
        .map(|v| json!({"vote_account": v.vote_account, "name": v.name, "projected_sol": v.projected_sol}));
    json!({
        "above": above,
        "below": below
    })
}

fn cohort_summary(name: &str, cohort: &[ValidatorGaugeEntry], gauge_reserve_sol: f64) -> Value {
    let vev: f64 = cohort.iter().map(|v| v.vev_weight).sum();
    let share: f64 = cohort.iter().map(|v| v.vote_share).sum();
    let projected_sol = gauge_reserve_sol * share;
    json!({
        "name": name,
        "count": cohort.len(),
        "vev_weight": vev,
        "vote_share": share,
        "projected_sol": projected_sol
    })
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
