//! Admin-triggered retention sweep for every bounded telemetry table.
//!
//! Long-running deployments (local, Kubernetes, docker-compose) host the
//! in-process ticker from `telemetry::sweeper`. Serverless deployments have no
//! such process, so a scheduler (e.g. an AWS EventBridge rule) calls this
//! endpoint instead. Both paths run the exact same `sweep_once`.

use axum::{Extension, Json, extract::State};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
    telemetry::sweeper::{TelemetrySweeperConfig, sweep_once},
};

#[derive(Clone, Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySweepResponse {
    /// Product events removed because they fell outside `FLOW_LIKE_EVENT_RETENTION_DAYS`.
    pub events_deleted: u64,
    /// Crash events removed because they fell outside `FLOW_LIKE_ERROR_RETENTION_DAYS`.
    pub errors_deleted: u64,
    /// Sessions removed because they fell outside `FLOW_LIKE_SESSION_RETENTION_DAYS`.
    pub sessions_deleted: u64,
    /// LLM calls removed because they fell outside `FLOW_LIKE_LLM_RETENTION_DAYS`.
    pub llm_deleted: u64,
    /// Spans removed because they fell outside `FLOW_LIKE_TRACE_RETENTION_DAYS`.
    pub spans_deleted: u64,
    /// Performance samples removed because they fell outside `FLOW_LIKE_PERF_RETENTION_DAYS`.
    pub perf_deleted: u64,
    /// Alert inbox rows removed because they fell outside `FLOW_LIKE_ALERT_EVENT_RETENTION_DAYS`.
    pub alert_events_deleted: u64,
    /// Daily rollup rows removed because they fell outside `FLOW_LIKE_ROLLUP_RETENTION_DAYS`.
    pub rollups_deleted: u64,
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/sweep",
    tag = "admin",
    responses(
        (status = 200, description = "Sweep completed, returns the rows removed per table", body = TelemetrySweepResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required")
    ),
    description = "Delete telemetry rows that are older than the configured retention windows. Raw tables that feed the daily rollups are only swept up to the last fully aggregated day. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/telemetry/sweep", skip(state, user))]
pub async fn sweep_telemetry(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<TelemetrySweepResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let config = TelemetrySweeperConfig::from_env();
    let result = sweep_once(&state.db, &config).await.map_err(|e| {
        tracing::error!(error = %e, "Admin telemetry sweep failed");
        ApiError::internal_error(flow_like_types::anyhow!("Telemetry sweep failed: {}", e))
    })?;

    if !result.is_empty() {
        tracing::info!(
            events_deleted = result.events_deleted,
            errors_deleted = result.errors_deleted,
            sessions_deleted = result.sessions_deleted,
            llm_deleted = result.llm_deleted,
            spans_deleted = result.spans_deleted,
            perf_deleted = result.perf_deleted,
            alert_events_deleted = result.alert_events_deleted,
            rollups_deleted = result.rollups_deleted,
            "Admin telemetry sweep removed expired rows"
        );
    }

    Ok(Json(TelemetrySweepResponse {
        events_deleted: result.events_deleted,
        errors_deleted: result.errors_deleted,
        sessions_deleted: result.sessions_deleted,
        llm_deleted: result.llm_deleted,
        spans_deleted: result.spans_deleted,
        perf_deleted: result.perf_deleted,
        alert_events_deleted: result.alert_events_deleted,
        rollups_deleted: result.rollups_deleted,
    }))
}
