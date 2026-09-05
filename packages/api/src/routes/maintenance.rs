//! Service-authenticated maintenance jobs for stateless deployments.
//!
//! The caller can select only an allowlisted job. It never receives database
//! credentials; all privileged work stays behind this API boundary.

use axum::{Json, Router, extract::State, http::HeaderMap, routing::post};
use flow_like_types::{
    cache::CacheCleanupResult,
    maintenance::{
        DeletionQueueMaintenanceResult, MaintenanceRunRequest, MaintenanceRunResponse,
        RegressionSuitesMaintenanceResult, RunSweepMaintenanceResult,
        StateCleanupMaintenanceResult, TelemetryAlertsMaintenanceResult,
    },
};

use crate::{
    cache::{require_cache_store, sweeper::sweep_once as sweep_cache_once},
    channel::sweep_expired as sweep_channels_once,
    error::ApiError,
    execution::run_sweeper::{RunSweeperConfig, sweep_once as sweep_runs_once},
    state::AppState,
    telemetry::alerts::{TelemetryAlertConfig, evaluate_once},
};

pub fn routes() -> Router<AppState> {
    Router::new().route("/run", post(run_maintenance_job))
}

#[tracing::instrument(
    name = "POST /maintenance/run",
    skip(state, headers),
    fields(job = request.job().as_str(), idempotency_key)
)]
async fn run_maintenance_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<MaintenanceRunRequest>,
) -> Result<Json<MaintenanceRunResponse>, ApiError> {
    authorize(&headers, state.maintenance_token.as_deref())?;

    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("<none>");
    tracing::Span::current().record("idempotency_key", idempotency_key);

    match request {
        MaintenanceRunRequest::TelemetryAlerts => {
            let config = TelemetryAlertConfig::from_env();
            let result = evaluate_once(&state, &config).await.map_err(|error| {
                tracing::error!(error = %error, "Scheduled telemetry alert evaluation failed");
                ApiError::internal_error(flow_like_types::anyhow!(
                    "Telemetry alert evaluation failed: {}",
                    error
                ))
            })?;

            tracing::info!(
                evaluated = result.evaluated,
                triggered = result.triggered,
                resolved = result.resolved,
                "Maintenance telemetry alert evaluation completed"
            );

            Ok(Json(MaintenanceRunResponse::TelemetryAlerts(
                TelemetryAlertsMaintenanceResult {
                    evaluated: result.evaluated,
                    triggered: result.triggered,
                    resolved: result.resolved,
                },
            )))
        }
        MaintenanceRunRequest::CacheCleanup => {
            // Channel rows are expiring coordination state with no native TTL either; they ride
            // the same scheduled job so serverless deployments need no second trigger.
            match sweep_channels_once(&state.db, state.db_dialect).await {
                Ok(deleted) => {
                    tracing::info!(deleted, "Maintenance channel sweep completed")
                }
                Err(error) => {
                    tracing::error!(error = %error, "Scheduled channel sweep failed")
                }
            }

            let store = require_cache_store(&state.cache).await?;

            let deleted = sweep_cache_once(store.as_ref()).await.map_err(|error| {
                tracing::error!(error = %error, "Scheduled cache cleanup failed");
                ApiError::internal_error(flow_like_types::anyhow!(
                    "Cache cleanup failed: {}",
                    error
                ))
            })?;

            tracing::info!(
                deleted,
                backend = store.backend_name(),
                "Maintenance cache cleanup completed"
            );

            Ok(Json(MaintenanceRunResponse::CacheCleanup(
                CacheCleanupResult {
                    deleted: deleted.max(0) as u64,
                },
            )))
        }
        MaintenanceRunRequest::RunSweep => {
            let config = RunSweeperConfig::from_env();
            let swept = sweep_runs_once(
                &crate::audit::ExecutionAuditContext::from(&state),
                config.grace,
                config.batch_size,
            )
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "Scheduled run sweep failed");
                ApiError::internal_error(flow_like_types::anyhow!("Run sweep failed: {}", error))
            })?;

            tracing::info!(
                swept,
                grace_secs = config.grace.as_secs(),
                batch_size = config.batch_size,
                batch_full = swept >= config.batch_size,
                "Maintenance run sweep completed"
            );

            Ok(Json(MaintenanceRunResponse::RunSweep(
                RunSweepMaintenanceResult {
                    swept,
                    grace_secs: config.grace.as_secs(),
                    batch_size: config.batch_size,
                },
            )))
        }
        MaintenanceRunRequest::StateCleanup => {
            let store = crate::routes::execution::progress::get_state_store(&state).await?;

            let deleted_runs = store.delete_expired_runs().await.map_err(|error| {
                tracing::error!(error = %error, "Scheduled state-store run cleanup failed");
                ApiError::internal_error(flow_like_types::anyhow!(
                    "Expired execution run cleanup failed: {}",
                    error
                ))
            })?;
            let deleted_events = store.delete_expired_events().await.map_err(|error| {
                tracing::error!(error = %error, "Scheduled state-store event cleanup failed");
                ApiError::internal_error(flow_like_types::anyhow!(
                    "Expired execution event cleanup failed: {}",
                    error
                ))
            })?;

            // Staged event payloads are written to the content store before the
            // row that references them, so a write that fails — or a
            // multi-chunk insert that only partly applies — leaves an object no
            // row will ever name. Age is the only property such an object still
            // carries, and this is the only pass that looks at it. It rides the
            // state-cleanup schedule rather than a second trigger, the way the
            // channel sweep rides the cache job, and a failure here must not
            // discard the row cleanup that already succeeded.
            let min_age_secs = crate::execution::state::staged_payload_min_age_secs();
            match store.sweep_staged_payloads(min_age_secs).await {
                Ok(sweep) => tracing::info!(
                    scanned = sweep.scanned,
                    deleted = sweep.deleted,
                    stopped_early = sweep.stopped_early,
                    min_age_secs,
                    backend = store.backend_name(),
                    "Maintenance staged-payload sweep completed"
                ),
                Err(error) => {
                    tracing::error!(error = %error, "Scheduled staged-payload sweep failed")
                }
            }

            tracing::info!(
                deleted_runs,
                deleted_events,
                backend = store.backend_name(),
                "Maintenance execution-state cleanup completed"
            );

            Ok(Json(MaintenanceRunResponse::StateCleanup(
                StateCleanupMaintenanceResult {
                    deleted_runs,
                    deleted_events,
                },
            )))
        }
        MaintenanceRunRequest::RegressionSuites => {
            let outcome = crate::execution::regression::maintenance_tick(&state)
                .await
                .map_err(|error| {
                    tracing::error!(error = %error, "Scheduled regression-suite maintenance failed");
                    error
                })?;

            tracing::info!(
                dispatched = outcome.dispatched,
                swept = outcome.swept,
                executed = outcome.executed,
                "Maintenance regression-suite pass completed"
            );

            Ok(Json(MaintenanceRunResponse::RegressionSuites(
                RegressionSuitesMaintenanceResult {
                    dispatched: outcome.dispatched,
                    swept: outcome.swept,
                    executed: outcome.executed,
                },
            )))
        }
        MaintenanceRunRequest::DeletionQueue => {
            let budget = crate::deletion::PassBudget::from_env();
            let report = crate::deletion::run_queue(&state, budget)
                .await
                .map_err(|error| {
                    tracing::error!(error = %error, "Scheduled deletion queue pass failed");
                    error
                })?;

            tracing::info!(
                claimed = report.claimed,
                completed = report.completed,
                suspended = report.suspended,
                failed = report.failed,
                max_chunks = budget.max_chunks,
                "Maintenance deletion queue pass completed"
            );

            Ok(Json(MaintenanceRunResponse::DeletionQueue(
                DeletionQueueMaintenanceResult {
                    claimed: report.claimed,
                    completed: report.completed,
                    suspended: report.suspended,
                    failed: report.failed,
                },
            )))
        }
    }
}

fn authorize(headers: &HeaderMap, expected_token: Option<&str>) -> Result<(), ApiError> {
    let expected_token = expected_token.ok_or_else(|| {
        ApiError::service_unavailable("MAINTENANCE_TOKEN is not configured on the API")
    })?;
    let provided_token = bearer_token(headers)
        .ok_or_else(|| ApiError::unauthorized("Invalid maintenance credentials"))?;

    if !constant_time_eq(provided_token.as_bytes(), expected_token.as_bytes()) {
        return Err(ApiError::unauthorized("Invalid maintenance credentials"));
    }

    Ok(())
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let authorization = crate::middleware::jwt::viewer_authorization(headers)?;
    let (scheme, token) = authorization.split_once(' ')?;
    let token = token.trim();

    if !scheme.eq_ignore_ascii_case("bearer") || token.is_empty() {
        return None;
    }

    Some(token)
}

/// Constant-time byte comparison. Unequal lengths cannot represent the same
/// credential and are rejected before comparing contents.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut difference = 0u8;
    for (left, right) in a.iter().zip(b.iter()) {
        difference |= left ^ right;
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{http::header, response::IntoResponse};

    const TOKEN: &str = "0123456789abcdef0123456789abcdef";

    fn headers(authorization: Option<&str>) -> HeaderMap {
        let mut headers = HeaderMap::new();
        if let Some(value) = authorization {
            headers.insert(header::AUTHORIZATION, value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn bearer_auth_accepts_the_configured_token() {
        let headers = headers(Some(&format!("Bearer {TOKEN}")));
        assert!(authorize(&headers, Some(TOKEN)).is_ok());
    }

    #[test]
    fn bearer_scheme_is_case_insensitive() {
        let headers = headers(Some(&format!("bEaReR {TOKEN}")));
        assert!(authorize(&headers, Some(TOKEN)).is_ok());
    }

    #[test]
    fn missing_and_wrong_tokens_are_unauthorized() {
        let missing = authorize(&HeaderMap::new(), Some(TOKEN))
            .unwrap_err()
            .into_response();
        let wrong = authorize(
            &headers(Some("Bearer fedcba9876543210fedcba9876543210")),
            Some(TOKEN),
        )
        .unwrap_err()
        .into_response();

        assert_eq!(missing.status(), axum::http::StatusCode::UNAUTHORIZED);
        assert_eq!(wrong.status(), axum::http::StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn missing_server_token_fails_closed() {
        let response = authorize(&headers(Some(&format!("Bearer {TOKEN}"))), None)
            .unwrap_err()
            .into_response();

        assert_eq!(
            response.status(),
            axum::http::StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[test]
    fn constant_time_comparison_handles_equal_content_and_length_mismatches() {
        assert!(constant_time_eq(TOKEN.as_bytes(), TOKEN.as_bytes()));
        assert!(!constant_time_eq(TOKEN.as_bytes(), b"0123456789abcdef"));
        assert!(!constant_time_eq(
            TOKEN.as_bytes(),
            b"fedcba9876543210fedcba9876543210"
        ));
    }
}
