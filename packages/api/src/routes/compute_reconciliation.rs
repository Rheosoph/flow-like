use crate::{
    compute_attempts::{self, AttemptReport},
    error::ApiError,
    state::AppState,
};
use axum::{
    Json,
    extract::{Query, State},
    http::HeaderMap,
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReconcileRequest {
    pub attempts: Vec<AttemptReport>,
}
#[derive(Deserialize)]
pub struct PendingQuery {
    pub cursor: Option<String>,
    pub limit: Option<u64>,
}

pub async fn reconcile(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<ReconcileRequest>,
) -> Result<Json<Value>, ApiError> {
    super::maintenance::authorize(&headers, state.maintenance_token.as_deref())?;
    if request.attempts.len() > 100 {
        return Err(ApiError::bad_request(
            "Reconcile at most 100 compute attempts per batch",
        ));
    }
    let mut ids = Vec::new();
    for report in request.attempts {
        ids.push(compute_attempts::record(&state.db, state.db_dialect, report).await?);
    }
    Ok(Json(json!({"attemptIds":ids})))
}

pub async fn pending(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PendingQuery>,
) -> Result<Json<Value>, ApiError> {
    super::maintenance::authorize(&headers, state.maintenance_token.as_deref())?;
    Ok(Json(
        compute_attempts::pending(&state.db, query.cursor, query.limit.unwrap_or(100)).await?,
    ))
}
