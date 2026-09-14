use crate::{entity::usage_invocation, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ConnectionTrait, DatabaseBackend, EntityTrait, Statement};
use serde_json::{Value, json};

/// Only the payer can inspect the operation. Provider payloads, prompts, endpoint
/// URLs and credentials are excluded from the fixed response fields below.
pub async fn get_operation_detail(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let payer = user.sub()?;
    let operation = state.db.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "SELECT id,kind,status,\"modelId\",provider,\"fundingClass\",used,reserved FROM \"QuotaOperation\" WHERE id=$1 AND \"payerId\"=$2",
        [id.clone().into(), payer.into()])).await?.ok_or(ApiError::NOT_FOUND)?;
    let status: String = operation.try_get("", "status")?;
    let invocation = usage_invocation::Entity::find_by_id(&id)
        .one(&state.db)
        .await?;
    let event = state.db.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "SELECT detail FROM \"QuotaEvent\" WHERE \"operationId\"=$1 ORDER BY \"createdAt\" DESC LIMIT 1", [id.clone().into()])).await?;
    let event_detail = event
        .and_then(|row| row.try_get::<String>("", "detail").ok())
        .and_then(|detail| serde_json::from_str::<Value>(&detail).ok());
    let detail = invocation
        .as_ref()
        .and_then(|row| row.raw_usage.as_ref())
        .or(event_detail.as_ref());
    let accounting = detail.and_then(|detail| detail.get("accounting"));
    let provider_cost = detail
        .and_then(|detail| detail.get("providerCostMicroUsd"))
        .and_then(Value::as_i64);
    let funding_bps = accounting
        .and_then(|rate| rate.get("funding_basis_points"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let funding_cost = provider_cost.map(|cost| {
        ((i128::from(cost.max(0)) * i128::from(funding_bps) + 9_999) / 10_000)
            .min(i128::from(i64::MAX)) as i64
    });
    let raw_usage = detail.and_then(|detail| detail.get("providerUsage"));
    let provider_reported_cost = raw_usage.is_some_and(|usage| {
        ["cost", "total_cost", "cost_micro_dollars", "cost_micros"]
            .iter()
            .any(|key| usage.get(*key).is_some())
    });
    Ok(Json(json!({
        "operationId": id,
        "kind": operation.try_get::<String>("", "kind")?,
        "modelId": operation.try_get::<Option<String>>("", "modelId")?,
        "provider": operation.try_get::<Option<String>>("", "provider")?,
        "fundingClass": operation.try_get::<String>("", "fundingClass")?,
        "status": status,
        "inputTokens": invocation.as_ref().map(|row| row.input_tokens),
        "outputTokens": invocation.as_ref().map(|row| row.output_tokens),
        "embeddingTokens": invocation.as_ref().map(|row| row.embedding_tokens),
        "tokenCountEstimated": raw_usage.and_then(|usage| usage.get("tokenCountEstimated")).and_then(Value::as_bool).unwrap_or(false),
        "meteringBasis": raw_usage.and_then(|usage| usage.get("meteringBasis")),
        "inputBytes": raw_usage.and_then(|usage| usage.get("inputBytes")),
        "providerReportedWords": raw_usage.and_then(|usage| usage.get("providerReportedWords")),
        "providerCostMicroUsd": provider_cost,
        "providerFundingCostMicroUsd": funding_cost,
        "servingCostMicroUsd": detail.and_then(|detail| detail.get("servingCostMicroUsd")),
        "costMicroEur": detail.and_then(|detail| detail.get("costMicroEur")),
        "rateVersion": accounting.and_then(|rate| rate.get("version")),
        "usdMicroPerEur": accounting.and_then(|rate| rate.get("usd_micro_per_eur")),
        "providerRequestId": invocation.as_ref().and_then(|row| row.provider_request_id.as_ref()),
        "estimated": status != "finalized" || !provider_reported_cost,
        "servingCostEstimated": true,
        "usage": serde_json::from_str::<Value>(&operation.try_get::<String>("", "used")?)?,
        "reserved": serde_json::from_str::<Value>(&operation.try_get::<String>("", "reserved")?)?,
    })))
}
