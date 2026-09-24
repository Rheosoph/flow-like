use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use chrono::Utc;
use sea_orm::ConnectionTrait;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{accounts, error, outbox, sql};
use crate::{
    auth::AppUser, error::ApiError, permission::global_permission::GlobalPermission,
    state::AppState,
};

#[derive(Deserialize)]
pub struct QueueQuery {
    pub kind: Option<String>,
}

#[utoipa::path(get,path="/admin/payments/queue",tag="payments",params(("kind"=Option<String>,Query)),security(("bearer_auth"=[])),responses((status=200,description="Payment recovery queue",body=Object)))]
pub async fn queue(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<QueueQuery>,
) -> Result<Json<Value>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    let (table, fields) = match query.kind.as_deref().unwrap_or("operations") {
        "operations" => (
            "StripeOperation",
            r#""sourceType","sourceId",operation AS action,"requestId","stripeObjectId" AS "objectId",NULL::text AS reason"#,
        ),
        "events" => (
            "PaymentWebhookInbox",
            r#"'WEBHOOK' AS "sourceType","stripeEventId" AS "sourceId","eventType" AS action,NULL::text AS "requestId",NULL::text AS "objectId","quarantineReason" AS reason"#,
        ),
        "effects" => (
            "PaymentOutbox",
            r#""sourceType","sourceId",effect AS action,NULL::text AS "requestId",NULL::text AS "objectId","lastError" AS reason"#,
        ),
        _ => {
            return Err(error(
                "PAYMENT_QUEUE_INVALID",
                "Choose operations, events or effects",
            ));
        }
    };
    let rows=state.db.query_all_raw(sql(&format!(r#"SELECT id,status,attempts,"createdAt",{fields} FROM "{table}" WHERE status IN ('PENDING','INDETERMINATE','MANUAL_REVIEW','QUARANTINED') ORDER BY CASE WHEN status='PENDING' THEN 1 ELSE 0 END,"createdAt",id LIMIT 100"#),vec![])).await?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(json!({"id":row.try_get::<String>("","id")?,"status":row.try_get::<String>("","status")?,"attempts":row.try_get::<i32>("","attempts")?,"createdAt":row.try_get::<i64>("","createdAt")?,"sourceType":row.try_get::<String>("","sourceType")?,"sourceId":row.try_get::<String>("","sourceId")?,"action":row.try_get::<String>("","action")?,"requestId":row.try_get::<Option<String>>("","requestId")?,"objectId":row.try_get::<Option<String>>("","objectId")?,"reason":row.try_get::<Option<String>>("","reason")?}));
    }
    Ok(Json(json!({"entries":entries})))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockInput {
    pub blocked: bool,
    pub reason: String,
}

#[utoipa::path(post,path="/admin/payments/sellers/{user_id}/block",tag="payments",params(("user_id"=String,Path)),request_body=BlockInput,security(("bearer_auth"=[])),responses((status=200,description="Seller block state",body=Object)))]
pub async fn block_seller(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(user_id): Path<String>,
    Json(body): Json<BlockInput>,
) -> Result<Json<Value>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    let actor = accounts::recent_session(&state, &user).await?;
    if body.reason.trim().is_empty() || body.reason.len() > 1000 {
        return Err(error(
            "PAYMENT_REASON_REQUIRED",
            "Supply a reason of at most 1000 characters",
        ));
    }
    let now = Utc::now().timestamp_millis();
    let effect_id = flow_like_types::create_id();
    state.transaction(|txn| {let user_id=user_id.clone();let actor=actor.clone();let reason=body.reason.clone();let effect_id=effect_id.clone();Box::pin(async move {
        if body.blocked {
            txn.execute_raw(sql(r#"INSERT INTO "PaymentsBlock" ("userId",reason,"blockedBy","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$4) ON CONFLICT ("userId") DO UPDATE SET reason=EXCLUDED.reason,"blockedBy"=EXCLUDED."blockedBy",revision="PaymentsBlock".revision+1,"updatedAt"=EXCLUDED."updatedAt""#,vec![user_id.clone().into(),reason.clone().into(),actor.clone().into(),now.into()])).await?;
            let accounts=txn.query_all_raw(sql(r#"UPDATE "ConnectedAccount" SET revision=revision+1,"updatedAt"=$2 WHERE "userId"=$1 RETURNING id,"stripeAccountId""#,vec![user_id.clone().into(),now.into()])).await?;
            for account in accounts {
                let id:String=account.try_get("","id")?;
                outbox::enqueue(txn,&format!("block:{effect_id}:{id}"),"CANCEL_ACCOUNT_PAYMENTS","CONNECTED_ACCOUNT",&id,json!({"accountId":account.try_get::<Option<String>>("","stripeAccountId")?,"reason":"SELLER_BLOCKED"})).await?;
            }
        } else {
            txn.execute_raw(sql(r#"DELETE FROM "PaymentsBlock" WHERE "userId"=$1"#,vec![user_id.clone().into()])).await?;
        }
        outbox::enqueue(txn,&effect_id,"PAYMENT_AUDIT","USER",&user_id,json!({"action":"payments.seller.blocked","actor":actor,"blocked":body.blocked,"reason":reason})).await?;
        Ok::<_,ApiError>(())
    })}).await?;
    Ok(Json(json!({"blocked":body.blocked})))
}

#[derive(Deserialize, utoipa::ToSchema)]
pub struct RetryInput {
    pub reason: String,
}

#[utoipa::path(post,path="/admin/payments/effects/{id}/resume",tag="payments",params(("id"=String,Path)),request_body=RetryInput,security(("bearer_auth"=[])),responses((status=200,description="Recovery effect queued",body=Object)))]
pub async fn retry_effect(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Json(body): Json<RetryInput>,
) -> Result<Json<Value>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;
    let actor = accounts::recent_session(&state, &user).await?;
    if body.reason.trim().is_empty() || body.reason.len() > 1000 {
        return Err(error(
            "PAYMENT_REASON_REQUIRED",
            "Supply a reason of at most 1000 characters",
        ));
    }
    let now = Utc::now().timestamp_millis();
    let audit_id = flow_like_types::create_id();
    state.transaction(|txn| {let id=id.clone();let actor=actor.clone();let reason=body.reason.clone();let audit_id=audit_id.clone();Box::pin(async move {
        let changed=txn.execute_raw(sql(r#"UPDATE "PaymentOutbox" SET status='PENDING',attempts=0,"nextAttemptAt"=$2,revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND status='MANUAL_REVIEW' AND ("leaseExpiresAt" IS NULL OR "leaseExpiresAt"<$2)"#,vec![id.clone().into(),now.into()])).await?;
        if changed.rows_affected()!=1 {return Err(error("PAYMENT_NOT_RETRYABLE","Only a suspended recovery effect can be resumed"));}
        outbox::enqueue(txn,&audit_id,"PAYMENT_AUDIT","PAYMENT_EFFECT",&id,json!({"action":"payments.effect.resumed","actor":actor,"reason":reason})).await?;
        Ok::<_,ApiError>(())
    })}).await?;
    Ok(Json(json!({"queued":true})))
}
