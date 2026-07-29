//! Admin-triggered daily rollup of the raw telemetry tables.
//!
//! Long-running deployments (local, Kubernetes, docker-compose) host the
//! in-process ticker from `telemetry::rollup`. Serverless deployments have no
//! such process, so a scheduler (e.g. an AWS EventBridge rule) calls this
//! endpoint instead. Both paths run the exact same `rollup_once`, which is
//! idempotent — triggering it twice never double-counts a day.

use axum::{Extension, Json, extract::State};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
    telemetry::rollup::{TelemetryRollupConfig, rollup_once},
};

#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryRollupResponse {
    /// Days recomputed by this pass, controlled by `FLOW_LIKE_TELEMETRY_ROLLUP_BACKFILL_DAYS`.
    pub days: u64,
    /// Rows upserted into `TelemetryInstallDaily` — one per install per day.
    pub installs_upserted: u64,
    /// Rows upserted into `TelemetryEventDaily`.
    pub events_upserted: u64,
    /// Rows upserted into `TelemetryDimensionDaily`.
    pub dimensions_upserted: u64,
    /// Rows upserted into `TelemetrySessionDaily`.
    pub sessions_upserted: u64,
    /// Rows upserted into `TelemetryLlmDaily`.
    pub llm_upserted: u64,
    /// Rows upserted into `TelemetryPerfDaily`.
    pub perf_upserted: u64,
    /// Rows upserted into `TelemetryFlowpilotDaily`.
    pub flowpilot_upserted: u64,
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/rollup",
    tag = "admin",
    responses(
        (status = 200, description = "Rollup completed, returns the rows upserted per table", body = TelemetryRollupResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required")
    ),
    description = "Recompute the daily telemetry aggregates that power long-range dashboards, for every day in the backfill window. Safe to run repeatedly. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/telemetry/rollup", skip(state, user))]
pub async fn rollup_telemetry(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<TelemetryRollupResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let config = TelemetryRollupConfig::from_env();
    let result = rollup_once(&state.db, &config).await.map_err(|e| {
        tracing::error!(error = %e, "Admin telemetry rollup failed");
        ApiError::internal_error(flow_like_types::anyhow!("Telemetry rollup failed: {}", e))
    })?;

    if !result.is_empty() {
        tracing::info!(
            days = result.days,
            installs_upserted = result.installs_upserted,
            events_upserted = result.events_upserted,
            dimensions_upserted = result.dimensions_upserted,
            sessions_upserted = result.sessions_upserted,
            llm_upserted = result.llm_upserted,
            perf_upserted = result.perf_upserted,
            flowpilot_upserted = result.flowpilot_upserted,
            "Admin telemetry rollup refreshed daily aggregates"
        );
    }

    Ok(Json(TelemetryRollupResponse {
        days: result.days,
        installs_upserted: result.installs_upserted,
        events_upserted: result.events_upserted,
        dimensions_upserted: result.dimensions_upserted,
        sessions_upserted: result.sessions_upserted,
        llm_upserted: result.llm_upserted,
        perf_upserted: result.perf_upserted,
        flowpilot_upserted: result.flowpilot_upserted,
    }))
}
