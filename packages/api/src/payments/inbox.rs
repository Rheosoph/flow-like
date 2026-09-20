use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
};
use chrono::Utc;
use flow_like_secrets::{ExposeSecret, SecretRef};
use sea_orm::ConnectionTrait;
use serde_json::{Value, json};

use super::{error, sql};
use crate::{
    error::ApiError,
    state::AppState,
    stripe_connect::{EventEnvelope, verify_webhook},
};

pub async fn connect(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    receive(
        &state,
        "connect",
        "STRIPE_CONNECT_WEBHOOK_SECRET",
        headers,
        body,
    )
    .await
}

pub async fn marketplace(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    receive(
        &state,
        "marketplace",
        "STRIPE_MARKETPLACE_WEBHOOK_SECRET",
        headers,
        body,
    )
    .await
}

fn supported(endpoint: &str, event_type: &str) -> bool {
    matches!(
        event_type,
        "checkout.session.completed"
            | "checkout.session.async_payment_succeeded"
            | "checkout.session.async_payment_failed"
            | "checkout.session.expired"
            | "payment_intent.succeeded"
            | "payment_intent.payment_failed"
            | "charge.updated"
            | "charge.refunded"
            | "refund.created"
            | "refund.updated"
            | "refund.failed"
            | "charge.dispute.created"
            | "charge.dispute.updated"
            | "charge.dispute.closed"
            | "transfer.created"
            | "transfer.updated"
            | "transfer.reversed"
            | "application_fee.created"
            | "application_fee.refunded"
    ) || endpoint == "connect"
        && matches!(
            event_type,
            "account.updated" | "account.application.deauthorized"
        )
}

async fn receive(
    state: &AppState,
    endpoint: &str,
    secret_name: &str,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, ApiError> {
    let signature = headers
        .get("stripe-signature")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ApiError::coded(
                StatusCode::BAD_REQUEST,
                "PAYMENT_SIGNATURE_INVALID",
                "A valid Stripe signature is required",
            )
        })?;
    let current = state
        .secrets
        .get_secret_string(&SecretRef::new(secret_name))
        .await
        .map_err(|_| {
            ApiError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "PAYMENT_WEBHOOK_UNCONFIGURED",
                "This payment webhook is not configured",
            )
        })?;
    let previous = state
        .secrets
        .get_secret_string(&SecretRef::new(format!("{secret_name}_PREVIOUS")))
        .await
        .ok();
    let mut secrets = vec![current.expose_secret()];
    if let Some(previous) = &previous {
        secrets.push(previous.expose_secret());
    }
    let mut event = verify_webhook(
        &body,
        signature,
        &secrets,
        Utc::now().timestamp() as u64,
        300,
    )
    .map_err(|_| {
        ApiError::coded(
            StatusCode::BAD_REQUEST,
            "PAYMENT_SIGNATURE_INVALID",
            "The Stripe signature or event envelope is invalid",
        )
    })?;
    let platform = state
        .platform_config
        .payments
        .platform_account_id
        .as_deref()
        .ok_or_else(|| {
            ApiError::coded(
                StatusCode::SERVICE_UNAVAILABLE,
                "PAYMENT_WEBHOOK_UNCONFIGURED",
                "The payment platform identity is not configured",
            )
        })?;
    let scope = event.account.clone().unwrap_or_else(|| "platform".into());
    let reason = if event.livemode != state.platform_config.payments.livemode {
        Some("WRONG_MODE")
    } else if (endpoint == "connect") != event.account.is_some() {
        Some("WRONG_SCOPE")
    } else if event
        .api_version
        .as_deref()
        .is_some_and(|version| version != crate::stripe_connect::STRIPE_API_VERSION)
    {
        Some("WRONG_API_VERSION")
    } else {
        None
    };
    let status = if reason.is_some() {
        "QUARANTINED"
    } else if !supported(endpoint, &event.event_type) {
        "IGNORED"
    } else {
        "PENDING"
    };
    let id = blake3::hash(
        format!(
            "{endpoint}:{platform}:{scope}:{}:{}",
            event.livemode, event.id
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    let now = Utc::now().timestamp_millis();
    minimize_replay_envelope(&mut event);
    state.db.execute_raw(sql(r#"INSERT INTO "PaymentWebhookInbox" (id,endpoint,"platformAccountId","scopeKey",livemode,"stripeEventId","eventType",payload,status,"quarantineReason","nextAttemptAt","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11,$11) ON CONFLICT DO NOTHING"#,
        vec![id.into(),endpoint.into(),platform.into(),scope.into(),event.livemode.into(),event.id.clone().into(),event.event_type.clone().into(),serde_json::to_value(event)?.into(),status.into(),reason.map(str::to_owned).into(),now.into()])).await?;
    Ok(StatusCode::OK)
}

// Financial state comes from canonical retrieval. Keep only scoped lookup hints for replay.
fn minimize_replay_envelope(event: &mut EventEnvelope) {
    let source = &event.data.object;
    let mut object = serde_json::Map::new();
    for key in ["id", "charge", "payment_intent", "fee", "source"] {
        if let Some(id) = source.get(key).and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("id").and_then(Value::as_str))
        }) {
            object.insert(key.to_owned(), Value::String(id.to_owned()));
        }
    }
    let mut metadata = serde_json::Map::new();
    for key in ["flowlike_attempt", "flowlike_kind", "flowlike_id"] {
        if let Some(value) = source
            .get("metadata")
            .and_then(|v| v.get(key))
            .and_then(Value::as_str)
        {
            metadata.insert(key.to_owned(), Value::String(value.to_owned()));
        }
    }
    if !metadata.is_empty() {
        object.insert("metadata".into(), Value::Object(metadata));
    }
    event.data.object = Value::Object(object);
    event.data.previous_attributes = None;
    event.request = event.request.as_ref().map(|request| {
        json!({"id":request.as_str().or_else(||request.get("id").and_then(Value::as_str)),"idempotency_key":request.get("idempotency_key").and_then(Value::as_str)})
    });
}

pub async fn account_event(state: &AppState, event: &EventEnvelope) -> Result<bool, ApiError> {
    if !matches!(
        event.event_type.as_str(),
        "account.updated" | "account.application.deauthorized"
    ) {
        return Ok(false);
    }
    let Some(account_id) = &event.account else {
        return Err(error(
            "PAYMENT_BINDING_INVALID",
            "Account event has no connected account scope",
        ));
    };
    let row=state.db.query_one_raw(sql(r#"SELECT id FROM "ConnectedAccount" WHERE "stripeAccountId"=$1 AND livemode=$2 AND "platformAccountId"=$3"#,vec![account_id.clone().into(),event.livemode.into(),state.platform_config.payments.platform_account_id.clone().into()])).await?;
    let Some(row) = row else {
        return Ok(true);
    };
    let id: String = row.try_get("", "id")?;
    if event.event_type == "account.updated" {
        if event.data.object.get("id").and_then(Value::as_str) != Some(account_id) {
            return Err(error(
                "PAYMENT_BINDING_INVALID",
                "Account event identity does not match its scope",
            ));
        }
        super::accounts::sync_account(state, &id).await?;
    } else {
        let now = Utc::now().timestamp_millis();
        state.transaction(|txn| {let id=id.clone();let account_id=account_id.clone();Box::pin(async move {
            txn.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET state='disconnected',"retiredAt"=COALESCE("retiredAt",$2),"retiredReason"='DEAUTHORIZED',"canAcceptPayments"=FALSE,"canSell"=FALSE,revision=revision+1,"syncRevision"="syncRevision"+1,"updatedAt"=$2 WHERE id=$1"#,vec![id.clone().into(),now.into()])).await?;
            super::outbox::enqueue(txn,&format!("account:{id}:deauthorized"),"CANCEL_ACCOUNT_PAYMENTS","CONNECTED_ACCOUNT",&id,json!({"accountId":account_id,"reason":"DEAUTHORIZED"})).await?;
            super::outbox::enqueue(txn,&format!("account:{id}:deauthorized:audit"),"PAYMENT_AUDIT","CONNECTED_ACCOUNT",&id,json!({"action":"payments.account.deauthorized"})).await?;
            Ok::<_,ApiError>(())
        })}).await?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replay_payload_removes_customer_and_bank_details() {
        let mut event:EventEnvelope=serde_json::from_value(json!({"id":"evt_test","type":"charge.updated","livemode":false,"created":1,"data":{"object":{"id":"ch_test","payment_intent":{"id":"pi_test","customer":"cus_private"},"billing_details":{"email":"private@example.com"},"metadata":{"flowlike_attempt":"attempt_test","email":"private@example.com"}},"previous_attributes":{"email":"private@example.com"}}})).unwrap();
        minimize_replay_envelope(&mut event);
        assert_eq!(
            event.data.object,
            json!({"id":"ch_test","payment_intent":"pi_test","metadata":{"flowlike_attempt":"attempt_test"}})
        );
        assert!(event.data.previous_attributes.is_none());
        assert!(!serde_json::to_string(&event).unwrap().contains("private"));
    }
    #[test]
    fn unknown_events_never_schedule_provider_work() {
        assert!(!supported("connect", "customer.updated"));
        assert!(!supported("marketplace", "account.updated"));
        assert!(supported("connect", "account.updated"));
        assert!(supported("marketplace", "checkout.session.completed"));
    }
}
