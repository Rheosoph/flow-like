use chrono::Utc;
use sea_orm::{ConnectionTrait, EntityTrait, FromQueryResult};
use serde_json::json;

use super::{error, gateway_for_version, sql, stripe_error};
use crate::{
    entity::stripe_operation,
    error::ApiError,
    state::AppState,
    stripe_connect::{
        RequestMethod, RetryDisposition, StripeGateway, StripeRequest, StripeResponse,
    },
};

pub const REPLAY_WINDOW_MS: i64 = 23 * 60 * 60 * 1000;
const LEASE_MS: i64 = 60_000;

pub fn identity(request: &StripeRequest) -> Result<String, ApiError> {
    let key = request.idempotency_key.as_deref().ok_or_else(|| {
        error(
            "PAYMENT_OPERATION_INVALID",
            "Payment mutations require an idempotency key",
        )
    })?;
    let scope = request
        .scope
        .connected_account_id
        .as_deref()
        .unwrap_or("platform");
    Ok(blake3::hash(
        format!(
            "{}:{scope}:{}:{key}",
            request.scope.platform_account_id, request.scope.livemode
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string())
}

pub async fn prepare<C: ConnectionTrait>(
    db: &C,
    source_type: &str,
    source_id: &str,
    operation: &str,
    request: &StripeRequest,
) -> Result<String, ApiError> {
    if request.method != RequestMethod::Post {
        return Err(error(
            "PAYMENT_OPERATION_INVALID",
            "Only mutations are persisted as payment operations",
        ));
    }
    let digest = request.digest().map_err(stripe_error)?;
    let id = identity(request)?;
    let now = Utc::now().timestamp_millis();
    let scope = request
        .scope
        .connected_account_id
        .as_deref()
        .unwrap_or("platform");
    db.execute_raw(sql(r#"INSERT INTO "StripeOperation" (id,"sourceType","sourceId",operation,"platformAccountId","scopeKey","connectedAccountId",livemode,"requestParams","requestDigest","idempotencyKey","firstAttemptAt","lastAttemptAt","nextAttemptAt","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$12,$12,$12,$12) ON CONFLICT DO NOTHING"#,
        vec![id.clone().into(),source_type.into(),source_id.into(),operation.into(),request.scope.platform_account_id.clone().into(),scope.into(),request.scope.connected_account_id.clone().into(),request.scope.livemode.into(),serde_json::to_value(request)?.into(),digest.clone().into(),request.idempotency_key.clone().into(),now.into()])).await?;
    let recorded = stripe_operation::Entity::find_by_id(&id)
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if recorded.request_digest != digest
        || recorded.source_type != source_type
        || recorded.source_id != source_id
        || recorded.operation != operation
    {
        return Err(error(
            "PAYMENT_IDEMPOTENCY_CONFLICT",
            "This payment command was already used with different parameters",
        ));
    }
    Ok(id)
}

pub async fn execute(
    state: &AppState,
    source_type: &str,
    source_id: &str,
    operation: &str,
    request: StripeRequest,
) -> Result<StripeResponse, ApiError> {
    let id = prepare(&state.db, source_type, source_id, operation, &request).await?;
    execute_prepared(state, &id).await
}

pub async fn execute_prepared(state: &AppState, id: &str) -> Result<StripeResponse, ApiError> {
    let operation = stripe_operation::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let request: StripeRequest = serde_json::from_value(operation.request_params.clone())?;
    let gateway = gateway_for_version(state, &request.api_version).await?;
    execute_with_gateway(&state.db, id, gateway.as_ref()).await
}

pub async fn execute_with_gateway<C: ConnectionTrait>(
    db: &C,
    id: &str,
    gateway: &dyn StripeGateway,
) -> Result<StripeResponse, ApiError> {
    let operation = stripe_operation::Entity::find_by_id(id)
        .one(db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let connected_owner_operation = operation.source_type == "CONNECTED_ACCOUNT"
        && matches!(
            operation.operation.as_str(),
            "CREATE_ACCOUNT" | "ACCOUNT_LINK"
        );
    if connected_owner_operation
        && !(operation.operation == "CREATE_ACCOUNT" && operation.status == "SUCCEEDED")
    {
        let account = crate::entity::connected_account::Entity::find_by_id(&operation.source_id)
            .one(db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        if super::accounts::is_platform_admin(db, &account.user_id).await? {
            db.execute_raw(sql(r#"UPDATE "StripeOperation" SET status='FAILED',revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND status='PENDING' AND attempts=0 AND revision=$3"#,vec![id.into(),Utc::now().timestamp_millis().into(),operation.revision.into()])).await?;
            return Err(error(
                "PLATFORM_PAYMENTS_MANAGED",
                "Admin-owned apps collect payments in Flow-Like's Stripe account; connected account setup is unavailable",
            ));
        }
    }
    if operation.status == "SUCCEEDED" {
        return serde_json::from_value(
            operation.response.ok_or_else(|| {
                ApiError::internal("A completed payment operation has no response")
            })?,
        )
        .map_err(Into::into);
    }
    if matches!(operation.status.as_str(), "INDETERMINATE" | "MANUAL_REVIEW") {
        return Err(error(
            "PAYMENT_OPERATION_REVIEW",
            "The provider outcome is being reconciled; do not create another payment",
        ));
    }
    if operation.status == "FAILED" {
        return Err(error(
            "PAYMENT_OPERATION_FAILED",
            "The recorded payment operation failed",
        ));
    }
    let now = Utc::now().timestamp_millis();
    if operation.attempts > 0 && now.saturating_sub(operation.first_attempt_at) >= REPLAY_WINDOW_MS
    {
        db.execute_raw(sql(r#"UPDATE "StripeOperation" SET status='MANUAL_REVIEW',revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND revision=$3 AND status='PENDING'"#, vec![id.into(),now.into(),operation.revision.into()])).await?;
        return Err(error(
            "PAYMENT_OPERATION_REVIEW",
            "This operation is past its safe replay window",
        ));
    }
    let lease = flow_like_types::create_id();
    let claimed = db.query_one_raw(sql(r#"UPDATE "StripeOperation" SET "leaseOwner"=$2,"leaseExpiresAt"=$3,revision=revision+1,attempts=attempts+1,"firstAttemptAt"=CASE WHEN attempts=0 THEN $4 ELSE "firstAttemptAt" END,"lastAttemptAt"=$4,"updatedAt"=$4 WHERE id=$1 AND revision=$5 AND status='PENDING' AND "nextAttemptAt"<=$4 AND ("leaseExpiresAt" IS NULL OR "leaseExpiresAt"<$4) AND ($6=FALSE OR EXISTS (SELECT 1 FROM "ConnectedAccount" ca JOIN "User" u ON u.id=ca."userId" WHERE ca.id="StripeOperation"."sourceId" AND (u.permission & $7)=0)) RETURNING *"#,
        vec![id.into(),lease.clone().into(),(now+LEASE_MS).into(),now.into(),operation.revision.into(),connected_owner_operation.into(),crate::permission::global_permission::GlobalPermission::Admin.bits().into()])).await?
        .ok_or_else(|| error("PAYMENT_OPERATION_PENDING", "The payment operation is already running; check its status shortly"))?;
    let claimed = stripe_operation::Model::from_query_result(&claimed, "")?;
    let request: StripeRequest = serde_json::from_value(claimed.request_params)?;
    if request.digest().map_err(stripe_error)? != claimed.request_digest {
        db.execute_raw(sql(r#"UPDATE "StripeOperation" SET status='MANUAL_REVIEW',"leaseOwner"=NULL,"leaseExpiresAt"=NULL,revision=revision+1,"updatedAt"=$4 WHERE id=$1 AND "leaseOwner"=$2 AND revision=$3"#,vec![id.into(),lease.into(),claimed.revision.into(),now.into()])).await?;
        return Err(ApiError::internal(
            "Persisted Stripe operation failed its digest check",
        ));
    }
    let result = gateway.execute(&request).await;
    let finished = Utc::now().timestamp_millis();
    match result {
        Ok(response) => {
            let object_id = response
                .body
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_owned);
            let saved = db.execute_raw(sql(r#"UPDATE "StripeOperation" SET status='SUCCEEDED',response=$4,"stripeObjectId"=$5,"requestId"=$6,"leaseOwner"=NULL,"leaseExpiresAt"=NULL,revision=revision+1,"updatedAt"=$7 WHERE id=$1 AND "leaseOwner"=$2 AND revision=$3"#,
                vec![id.into(),lease.into(),claimed.revision.into(),serde_json::to_value(&response)?.into(),object_id.into(),response.request_id.clone().into(),finished.into()])).await?;
            if saved.rows_affected() != 1 {
                return Err(error(
                    "PAYMENT_OPERATION_PENDING",
                    "The payment result is being reconciled",
                ));
            }
            Ok(response)
        }
        Err(provider_error) => {
            let status = match provider_error.retry_disposition() {
                RetryDisposition::Never => "FAILED",
                RetryDisposition::SameOperation => "PENDING",
                RetryDisposition::Reconcile => "INDETERMINATE",
            };
            let retry_after = match &provider_error {
                crate::stripe_connect::StripeError::Api {
                    retry_after_seconds,
                    ..
                } => retry_after_seconds.unwrap_or(0).min(86_400) as i64 * 1_000,
                _ => 0,
            };
            let base = backoff_ms(claimed.attempts);
            let jitter = i64::from(
                blake3::hash(format!("{id}:{}", claimed.attempts).as_bytes()).as_bytes()[0],
            ) * base
                / 1024;
            let next = finished.saturating_add((base + jitter).max(retry_after));
            db.execute_raw(sql(r#"UPDATE "StripeOperation" SET status=$4,error=$5,"requestId"=$6,"leaseOwner"=NULL,"leaseExpiresAt"=NULL,"nextAttemptAt"=$7,revision=revision+1,"updatedAt"=$8 WHERE id=$1 AND "leaseOwner"=$2 AND revision=$3"#,
                vec![id.into(),lease.into(),claimed.revision.into(),status.into(),json!({"message":provider_error.to_string(),"retry":provider_error.retry_disposition()}).into(),provider_error.request_id().map(str::to_owned).into(),next.into(),finished.into()])).await?;
            Err(stripe_error(provider_error))
        }
    }
}

pub fn backoff_ms(attempt: i32) -> i64 {
    let base = 1_000_i64.saturating_mul(1_i64 << attempt.clamp(0, 10));
    base.min(900_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stripe_connect::StripeScope;

    #[test]
    fn operation_identity_separates_account_and_mode() {
        let one = StripeRequest::post(
            StripeScope::platform("acct_one", false),
            "/v1/refunds",
            json!({"charge":"ch_one"}),
            "refund:one",
        );
        let mut two = one.clone();
        two.scope.connected_account_id = Some("acct_seller".into());
        assert_ne!(identity(&one).unwrap(), identity(&two).unwrap());
        two = one.clone();
        two.scope.livemode = true;
        assert_ne!(identity(&one).unwrap(), identity(&two).unwrap());
    }

    #[test]
    fn retries_have_bounded_backoff() {
        assert!(backoff_ms(1) < backoff_ms(3));
        assert_eq!(backoff_ms(i32::MAX), 900_000);
    }
}
