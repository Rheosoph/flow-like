use crate::{error::ApiError, execution::verify_execution_jwt, state::AppState};
use axum::{Json, extract::State, http::HeaderMap};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct QuotaCallback {
    phase: String,
    attempt_id: String,
    runtime_ms: Option<u64>,
    status: Option<String>,
    compute: Option<crate::compute_attempts::AttemptReport>,
}

pub async fn report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<QuotaCallback>,
) -> Result<Json<Value>, ApiError> {
    let token = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(ApiError::UNAUTHORIZED)?;
    let claims = verify_execution_jwt(token).map_err(|_| ApiError::UNAUTHORIZED)?;
    let limit = claims
        .runtime_limit_ms
        .ok_or_else(|| ApiError::forbidden("This execution has no signed runtime allowance"))?;
    if body.attempt_id.is_empty() || body.attempt_id.len() > 128 {
        return Err(ApiError::bad_request("Invalid worker attempt identity"));
    }
    let accepted = match body.phase.as_str() {
        "attempt" => {
            let mut report = body
                .compute
                .ok_or_else(|| ApiError::bad_request("Compute attempt evidence is required"))?;
            report.operation_id = Some(claims.run_id.clone());
            report.payer_id = Some(crate::quota::operation_payer(&state, &claims.run_id).await?);
            report.role = "executor".into();
            report.cost_class = "workflow_compute".into();
            report.evidence = "measured_estimate".into();
            report.billed_duration_ms = None;
            report.cost_micro_usd = None;
            report.rate_version = crate::compute_attempts::RATE_VERSION.into();
            crate::compute_attempts::record(&state.db, state.db_dialect, report).await?;
            true
        }
        "start" => {
            crate::quota::claim_cloud(&state, &claims.run_id, &body.attempt_id, limit).await?
        }
        "poll" => {
            crate::quota::cloud_should_continue(&state, &claims.run_id, &body.attempt_id).await?
        }
        "finish" => {
            let duration = body
                .runtime_ms
                .ok_or_else(|| ApiError::bad_request("Measured runtime is required"))?;
            crate::quota::finish_cloud(
                &state,
                &claims.run_id,
                &body.attempt_id,
                duration,
                body.status.as_deref().unwrap_or("unknown"),
            )
            .await?;
            true
        }
        "reject" => {
            crate::quota::release_unstarted(
                &state,
                &claims.run_id,
                "executor rejected before workflow execution",
            )
            .await?;
            true
        }
        _ => return Err(ApiError::bad_request("Unknown quota callback phase")),
    };
    Ok(Json(json!({"accepted":accepted})))
}
