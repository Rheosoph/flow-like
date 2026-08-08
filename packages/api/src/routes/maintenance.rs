//! Service-authenticated maintenance jobs for stateless deployments.
//!
//! The caller can select only an allowlisted job. It never receives database
//! credentials; all privileged work stays behind this API boundary.

use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, header},
    routing::post,
};
use flow_like_types::{
    cache::CacheCleanupResult,
    maintenance::{
        MaintenanceRunRequest, MaintenanceRunResponse, TelemetryAlertsMaintenanceResult,
    },
};

use crate::{
    cache::sweeper::sweep_once as sweep_cache_once,
    error::ApiError,
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
            let store = state.cache_store.clone().ok_or_else(|| {
                ApiError::service_unavailable(
                    "Cache backend is not configured or failed to initialize on this deployment",
                )
            })?;

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
    let authorization = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
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
    use axum::response::IntoResponse;

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
