//! Service-authenticated recovery of unresolved workflow usage.
use crate::{error::ApiError, quota::QuotaAmounts, state::AppState};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::HeaderMap,
    routing::{get, post},
};
use sea_orm::{ConnectionTrait, Statement};
use serde::Deserialize;
use serde_json::{Value, json};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/pending", get(pending))
        .route("/adjust", post(adjust))
}
fn sql(query: &str, args: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres, query, args)
}
#[derive(Deserialize)]
struct PendingQuery {
    after_deadline: Option<i64>,
    after_id: Option<String>,
    limit: Option<u64>,
}
async fn pending(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PendingQuery>,
) -> Result<Json<Value>, ApiError> {
    super::maintenance::authorize(&headers, state.maintenance_token.as_deref())?;
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let rows=state.db.query_all_raw(sql("SELECT id,\"payerId\",\"appId\",kind,used,reserved,deadline,\"ownerId\" FROM \"QuotaOperation\" WHERE status='unknown' AND (deadline>$1 OR (deadline=$1 AND id>$2)) ORDER BY deadline,id LIMIT $3",vec![query.after_deadline.unwrap_or(0).into(),query.after_id.unwrap_or_default().into(),(limit as i64+1).into()])).await?;
    let more = rows.len() > limit as usize;
    let mut items = Vec::new();
    for row in rows.into_iter().take(limit as usize) {
        items.push(json!({"operationId":row.try_get::<String>("","id")?,"payerId":row.try_get::<String>("","payerId")?,"appId":row.try_get::<Option<String>>("","appId")?,"kind":row.try_get::<String>("","kind")?,"used":serde_json::from_str::<QuotaAmounts>(&row.try_get::<String>("","used")?)?,"reserved":serde_json::from_str::<QuotaAmounts>(&row.try_get::<String>("","reserved")?)?,"deadline":row.try_get::<i64>("","deadline")?,"worker":row.try_get::<Option<String>>("","ownerId")?}));
    }
    let next = if more {
        items
            .last()
            .map(|item| json!({"after_deadline":item["deadline"],"after_id":item["operationId"]}))
    } else {
        None
    };
    Ok(Json(json!({"items":items,"nextCursor":next})))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Adjustment {
    operation_id: String,
    runtime_ms: i64,
    cloud_starts: i64,
    revision: String,
    reason: String,
    actor: String,
    evidence_attempt_id: String,
}
async fn adjust(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<Adjustment>,
) -> Result<Json<Value>, ApiError> {
    super::maintenance::authorize(&headers, state.maintenance_token.as_deref())?;
    if body.runtime_ms < 0
        || !(0..=1).contains(&body.cloud_starts)
        || body.reason.trim().len() < 12
        || body.reason.len() > 2048
        || body.actor.trim().is_empty()
        || body.actor.len() > 256
        || body.revision.is_empty()
        || body.revision.len() > 128
    {
        return Err(ApiError::bad_request(
            "An adjustment requires bounded usage, a revision, an actor and a reason",
        ));
    }
    let op = state
        .db
        .query_one_raw(sql(
            "SELECT kind,status FROM \"QuotaOperation\" WHERE id=$1",
            vec![body.operation_id.clone().into()],
        ))
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if op.try_get::<String>("", "kind")? != "workflow" {
        return Err(ApiError::bad_request(
            "This endpoint adjusts workflow runtime only",
        ));
    }
    let evidence=state.db.query_one_raw(sql("SELECT id FROM \"ComputeAttempt\" WHERE id=$1 AND \"operationId\"=$2 AND role='executor' AND \"evidenceRank\">=1 AND status<>'started'",vec![body.evidence_attempt_id.clone().into(),body.operation_id.clone().into()])).await?;
    if evidence.is_none() {
        return Err(ApiError::conflict(
            "A terminal AWS or invoice attempt report is required before resolving workflow usage",
        ));
    }
    crate::quota::settle(&state,&body.operation_id,&format!("adjustment:{}",body.revision),QuotaAmounts {runtime_ms:body.runtime_ms,cloud_starts:body.cloud_starts,..Default::default()},true,json!({"administrative":true,"adjustment":true,"reason":body.reason,"actor":body.actor,"evidenceAttemptId":body.evidence_attempt_id})).await?;
    Ok(Json(json!({"resolved":true})))
}
