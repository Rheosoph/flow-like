//! Admin endpoints for run reconciliation.
//!
//! `POST /admin/runs/sweep` lets a scheduled job (e.g. an AWS
//! EventBridge Lambda) invoke `run_sweeper::sweep_once` against the
//! API's database, which is the only path that works in deployments
//! where the API runs on Lambda — there's no long-running process to
//! host the in-process ticker that lives in the local/k8s/docker-compose
//! mains.

use std::time::Duration;

use axum::{Extension, Json, extract::State};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    error::ApiError,
    execution::run_sweeper::{RunSweeperConfig, sweep_once},
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct SweepRunsRequest {
    /// Override the configured grace period (in seconds) for this
    /// invocation. If omitted, falls back to `RUN_SWEEPER_GRACE_SECS`
    /// or the default (3600s).
    pub grace_secs: Option<u64>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct SweepRunsResponse {
    /// Number of stuck runs that were marked as Timeout.
    pub swept: u64,
    /// Grace period (in seconds) actually used for this sweep.
    pub grace_secs: u64,
}

/// POST /admin/runs/sweep
///
/// Reconcile stuck workflow runs. Any run with status `Pending` or
/// `Running` whose `updated_at` is older than the grace period is
/// flipped to `Timeout` so it becomes visible in the UI as a failed
/// run instead of being lost.
///
/// Intended to be called on a schedule from outside the API process
/// (e.g. an AWS EventBridge schedule hitting this endpoint). Long-
/// running deployments (local, Kubernetes, docker-compose) already
/// run an in-process ticker and don't need to call this — but it's
/// available for ad-hoc reconciliation in any deployment.
#[utoipa::path(
    post,
    path = "/admin/runs/sweep",
    tag = "admin",
    request_body = SweepRunsRequest,
    responses(
        (status = 200, description = "Sweep completed, returns number of rows updated", body = SweepRunsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — Admin permission required")
    )
)]
#[tracing::instrument(name = "POST /admin/runs/sweep", skip(state, user, body))]
pub async fn sweep_runs(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<SweepRunsRequest>,
) -> Result<Json<SweepRunsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let configured = RunSweeperConfig::from_env();
    let grace = body
        .grace_secs
        .map(Duration::from_secs)
        .unwrap_or(configured.grace);

    let swept = sweep_once(&state.db, grace).await.map_err(|e| {
        tracing::error!(error = %e, "Admin run sweep failed");
        ApiError::internal_error(flow_like_types::anyhow!("Run sweep failed: {}", e))
    })?;

    Ok(Json(SweepRunsResponse {
        swept,
        grace_secs: grace.as_secs(),
    }))
}
