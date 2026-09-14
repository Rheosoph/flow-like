use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Default, Deserialize)]
pub struct UsageQuery {
    days: Option<i64>,
    cursor: Option<String>,
    limit: Option<u64>,
}

pub async fn get_quotas(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>, ApiError> {
    let payer = user.sub()?;
    crate::quota::summary(&state, &payer, query.days.unwrap_or(30))
        .await
        .map(Json)
}

pub async fn get_operations(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<UsageQuery>,
) -> Result<Json<Value>, ApiError> {
    let payer = user.sub()?;
    crate::quota::operations(&state, &payer, query.cursor, query.limit.unwrap_or(50))
        .await
        .map(Json)
}
