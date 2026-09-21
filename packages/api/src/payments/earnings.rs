use super::{accounts, connect_scope, error, sql, stripe_error};
use crate::{
    auth::AppUser,
    error::ApiError,
    state::AppState,
    stripe_connect::{StripeGateway, StripeRequest},
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use sea_orm::ConnectionTrait;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
pub struct PageQuery {
    pub before: Option<i64>,
}

#[utoipa::path(get,path="/user/payments/earnings",tag="payments",params(("before"=Option<i64>,Query)),security(("bearer_auth"=[])),responses((status=200,description="Seller payment history",body=Object)))]
pub async fn history(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<PageQuery>,
) -> Result<Json<Value>, ApiError> {
    let owner = accounts::session_user(&user)?;
    let platform_admin = accounts::is_platform_admin(&state.db, owner).await?;
    let rows=state.db.query_all_raw(sql(r#"SELECT id,"sourceType","sourceId",status,amount,currency,"capturedAmount","refundedAmount","reservedRefundAmount","applicationFeeAmount","hydrationStatus",orphaned,"createdAt",(COALESCE(snapshot->>'platform_owned','false')='true' OR COALESCE(snapshot->>'platformOwned','false')='true') AS "platformOwned" FROM "PaymentAttempt" WHERE (((COALESCE(snapshot->>'platform_owned','false')='true' OR COALESCE(snapshot->>'platformOwned','false')='true') AND $4=TRUE) OR ("payeeUserId"=$1 AND COALESCE(snapshot->>'platform_owned','false')!='true' AND COALESCE(snapshot->>'platformOwned','false')!='true')) AND livemode=$2 AND "createdAt"<$3 ORDER BY "createdAt" DESC,id DESC LIMIT 50"#,vec![owner.into(),state.platform_config.payments.livemode.into(),query.before.unwrap_or(i64::MAX).into(),platform_admin.into()])).await?;
    let mut entries = Vec::new();
    for row in rows {
        entries.push(json!({"platformOwned":row.try_get::<bool>("","platformOwned")?,"id":row.try_get::<String>("","id")?,"sourceType":row.try_get::<String>("","sourceType")?,"sourceId":row.try_get::<String>("","sourceId")?,"status":row.try_get::<String>("","status")?,"amount":row.try_get::<i64>("","amount")?,"currency":row.try_get::<String>("","currency")?,"capturedAmount":row.try_get::<i64>("","capturedAmount")?,"refundedAmount":row.try_get::<i64>("","refundedAmount")?,"reservedRefundAmount":row.try_get::<i64>("","reservedRefundAmount")?,"applicationFeeAmount":row.try_get::<i64>("","applicationFeeAmount")?,"settlement":row.try_get::<Option<String>>("","hydrationStatus")?,"orphaned":row.try_get::<bool>("","orphaned")?,"createdAt":row.try_get::<i64>("","createdAt")?}));
    }
    Ok(Json(
        json!({"platformOwned":platform_admin,"entries":entries,"nextBefore":entries.last().and_then(|row|row.get("createdAt"))}),
    ))
}

#[utoipa::path(get,path="/user/payments/balance",tag="payments",security(("bearer_auth"=[])),responses((status=200,description="Current recipient account balance",body=Object)))]
pub async fn balance(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Value>, ApiError> {
    let owner = accounts::session_user(&user)?;
    if accounts::is_platform_admin(&state.db, owner).await? {
        let request = StripeRequest::get(connect_scope(&state).await?, "/v1/balance", json!({}));
        let gateway = super::gateway_for_version(&state, &request.api_version).await?;
        let body = gateway.execute(&request).await.map_err(stripe_error)?.body;
        if !accounts::is_platform_admin(&state.db, owner).await? {
            return Err(ApiError::FORBIDDEN);
        }
        // This is the entire platform balance, including other apps and billing activity.
        return Ok(Json(
            json!({"available":body.get("available"),"pending":body.get("pending"),"payoutsEnabled":null,"platformOwned":true,"scope":"platform","includesExternalActivity":true}),
        ));
    }
    let account = accounts::account_for_user(&state, owner)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if account.retired_reason.as_deref() == Some("DEAUTHORIZED") {
        return Err(error(
            "REFUND_BLOCKED_DISCONNECTED",
            "Reconnect Stripe authorization to view this account balance",
        ));
    }
    let id = account.stripe_account_id.ok_or(ApiError::NOT_FOUND)?;
    let scope = connect_scope(&state).await?.connected(id);
    let request = StripeRequest::get(scope, "/v1/balance", json!({}));
    let gateway = super::gateway_for_version(&state, &request.api_version).await?;
    let body = gateway.execute(&request).await.map_err(stripe_error)?.body;
    // The provider's balance includes activity outside Flow-Like on a full-dashboard account.
    Ok(Json(
        json!({"available":body.get("available"),"pending":body.get("pending"),"payoutsEnabled":account.payouts_enabled,"platformOwned":false,"scope":"stripe_account","includesExternalActivity":true}),
    ))
}
