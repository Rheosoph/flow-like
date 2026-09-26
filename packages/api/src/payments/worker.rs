use axum::{Json, extract::State, http::HeaderMap};
use chrono::Utc;
use sea_orm::{ConnectionTrait, FromQueryResult};
use std::{
    sync::atomic::{AtomicI64, Ordering},
    time::{Duration, Instant},
};

use flow_like_types::tokio::{self, task::JoinHandle};
use serde_json::json;

use super::{error, operations::backoff_ms, sql};
use crate::{
    entity::{payment_outbox, payment_webhook_inbox},
    error::ApiError,
    state::AppState,
    stripe_connect::EventEnvelope,
};

pub use flow_like_types::maintenance::PaymentsMaintenanceResult as PassResult;
static LAST_HEALTH_MINUTE: AtomicI64 = AtomicI64::new(0);

pub fn spawn(state: AppState) -> Option<JoinHandle<()>> {
    if !state.platform_config.payments.servicing_enabled && !state.platform_config.features.premium
    {
        return None;
    }
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = run_once(&state, 5).await {
                tracing::error!(error=%error,"Payment recovery pass failed");
            }
        }
    }))
}

pub async fn maintenance(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<PassResult>, ApiError> {
    crate::routes::maintenance::authorize(&headers, state.maintenance_token.as_deref())?;
    run_once(&state, 10).await.map(Json)
}

pub async fn run_once(state: &AppState, limit: i64) -> Result<PassResult, ApiError> {
    let limit = limit.clamp(1, 25);
    let servicing = state.platform_config.payments.servicing_enabled;
    let mut report = PassResult::default();
    if !servicing && !state.platform_config.features.premium {
        return Ok(report);
    }
    let deadline = Instant::now() + Duration::from_secs(25);
    // Rotate the first queue so a slow provider cannot indefinitely starve refunds or reconciliation.
    let first = (Utc::now().timestamp() / 5).rem_euclid(7);
    for step in 0..7 {
        if Instant::now() >= deadline {
            break;
        }
        let phase = (first + step) % 7;
        let result = match phase {
            0 => super::legacy_checkout::reconcile(state, limit as u64).await,
            1 if servicing => drain_queue(state, true, limit, deadline, &mut report).await,
            2 => drain_queue(state, false, limit, deadline, &mut report).await,
            3 if servicing => reconcile_accounts(state, limit.min(3)).await,
            4 if servicing => super::node::reconcile(state, limit).await,
            5 if servicing => super::marketplace::reconcile(state, limit).await,
            6 if servicing => report_health(state).await,
            _ => Ok(()),
        };
        if let Err(error) = result {
            report.deferred += 1;
            tracing::warn!(phase,error=%error,"Payment recovery phase deferred");
        }
    }
    Ok(report)
}

async fn report_health(state: &AppState) -> Result<(), ApiError> {
    let now = Utc::now().timestamp_millis();
    let minute = now / 60_000;
    if LAST_HEALTH_MINUTE.swap(minute, Ordering::Relaxed) == minute {
        return Ok(());
    }
    let rows=state.db.query_all_raw(sql(r#"
        SELECT 'operations' AS queue,COUNT(*) AS count,MIN("createdAt") AS oldest FROM "StripeOperation" WHERE status IN ('INDETERMINATE','MANUAL_REVIEW') OR (status='PENDING' AND "createdAt"<$1)
        UNION ALL SELECT 'webhooks',COUNT(*),MIN("createdAt") FROM "PaymentWebhookInbox" WHERE status IN ('MANUAL_REVIEW','QUARANTINED') OR (status='PENDING' AND "createdAt"<$1)
        UNION ALL SELECT 'effects',COUNT(*),MIN("createdAt") FROM "PaymentOutbox" WHERE status='MANUAL_REVIEW' OR (status='PENDING' AND "createdAt"<$1)
        UNION ALL SELECT 'orphan_refunds',COUNT(*),MIN("createdAt") FROM "PaymentAttempt" WHERE orphaned=TRUE AND "capturedAmount">"refundedAmount" AND "createdAt"<$1
    "#,vec![(now-300_000).into()])).await?;
    for row in rows {
        let count: i64 = row.try_get("", "count")?;
        if count > 0 {
            let queue: String = row.try_get("", "queue")?;
            let oldest: i64 = row.try_get("", "oldest")?;
            tracing::error!(target:"payments",%queue,count,oldest_age_seconds=(now-oldest)/1000,"Payment recovery requires attention");
        }
    }
    Ok(())
}

async fn drain_queue(
    state: &AppState,
    inbox: bool,
    limit: i64,
    deadline: Instant,
    report: &mut PassResult,
) -> Result<(), ApiError> {
    let now = Utc::now().timestamp_millis();
    let rows = if inbox {
        state.db.query_all_raw(sql(r#"SELECT id FROM "PaymentWebhookInbox" WHERE status='PENDING' AND "nextAttemptAt"<=$1 AND livemode=$3 AND ("leaseExpiresAt" IS NULL OR "leaseExpiresAt"<$1) ORDER BY "nextAttemptAt",id LIMIT $2"#,vec![now.into(),limit.into(),state.platform_config.payments.livemode.into()])).await?
    } else {
        state.db.query_all_raw(sql(r#"SELECT id FROM "PaymentOutbox" WHERE status='PENDING' AND "nextAttemptAt"<=$1 AND ("leaseExpiresAt" IS NULL OR "leaseExpiresAt"<$1) AND ($3 OR effect IN ('legacy_purchase_orphan_refund','legacy_purchase_completed')) ORDER BY "nextAttemptAt",id LIMIT $2"#,vec![now.into(),limit.into(),state.platform_config.payments.servicing_enabled.into()])).await?
    };
    for row in rows {
        if Instant::now() >= deadline {
            break;
        }
        let id: String = row.try_get("", "id")?;
        let result = if inbox {
            process_event(state, &id).await
        } else {
            process_effect(state, &id).await
        };
        match result {
            Ok(true) if inbox => report.inbox_completed += 1,
            Ok(true) => report.effects_completed += 1,
            Ok(false) => {}
            Err(error) => {
                report.deferred += 1;
                tracing::warn!(%id,error=%error,"Payment delivery deferred");
            }
        }
    }
    Ok(())
}

async fn process_event(state: &AppState, id: &str) -> Result<bool, ApiError> {
    let now = Utc::now().timestamp_millis();
    let lease = flow_like_types::create_id();
    let row=state.db.query_one_raw(sql(r#"UPDATE "PaymentWebhookInbox" SET "leaseOwner"=$2,"leaseExpiresAt"=$3,revision=revision+1,attempts=attempts+1,"updatedAt"=$4 WHERE id=$1 AND status='PENDING' AND "nextAttemptAt"<=$4 AND ("leaseExpiresAt" IS NULL OR "leaseExpiresAt"<$4) RETURNING *"#,vec![id.into(),lease.clone().into(),(now+60_000).into(),now.into()])).await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let row = payment_webhook_inbox::Model::from_query_result(&row, "")?;
    let result = async {
        let event: EventEnvelope = serde_json::from_value(row.payload.clone())?;
        let mut handled=super::inbox::account_event(state, &event).await?;
        if !handled {
            if row.endpoint == "connect" {
                handled=super::node::handle_event(state, &event).await?;
            } else if row.endpoint == "marketplace" {
                handled=super::marketplace::handle_event(state, &event).await?;
                if !handled {
                    handled=super::node::handle_event(state, &event).await?;
                }
            }
        }
        if !handled && row.created_at > now-600_000 {
            let binding_pending=state.db.query_one_raw(sql(r#"SELECT id FROM "PaymentAttempt" WHERE "platformAccountId"=$1 AND "scopeKey"=$2 AND livemode=$3 AND "stripeSessionId" IS NULL AND "createdAt">$4 LIMIT 1"#,vec![row.platform_account_id.clone().into(),row.scope_key.clone().into(),row.livemode.into(),(now-600_000).into()])).await?.is_some();
            if binding_pending {return Err(error("PAYMENT_BINDING_PENDING","The payment response has not been bound yet"));}
        }
        Ok::<_, ApiError>(())
    }
    .await;
    finish(
        state,
        "PaymentWebhookInbox",
        id,
        &lease,
        row.revision,
        row.attempts,
        &result,
    )
    .await?;
    result.map(|_| true)
}

async fn process_effect(state: &AppState, id: &str) -> Result<bool, ApiError> {
    let now = Utc::now().timestamp_millis();
    let lease = flow_like_types::create_id();
    let row=state.db.query_one_raw(sql(r#"UPDATE "PaymentOutbox" SET "leaseOwner"=$2,"leaseExpiresAt"=$3,revision=revision+1,attempts=attempts+1,"updatedAt"=$4 WHERE id=$1 AND status='PENDING' AND "nextAttemptAt"<=$4 AND ("leaseExpiresAt" IS NULL OR "leaseExpiresAt"<$4) RETURNING *"#,vec![id.into(),lease.clone().into(),(now+60_000).into(),now.into()])).await?;
    let Some(row) = row else {
        return Ok(false);
    };
    let row = payment_outbox::Model::from_query_result(&row, "")?;
    let result = dispatch_effect(state, &row).await;
    finish(
        state,
        "PaymentOutbox",
        id,
        &lease,
        row.revision,
        row.attempts,
        &result,
    )
    .await?;
    result.map(|_| true)
}

async fn dispatch_effect(state: &AppState, row: &payment_outbox::Model) -> Result<(), ApiError> {
    match row.effect.as_str() {
        "PAYMENT_EMAIL" => {
            super::mail_confirmations::send_order_confirmation(
                state,
                &row.source_id,
                row.payload
                    .get("kind")
                    .and_then(|value| value.as_str())
                    .unwrap_or(""),
            )
            .await
        }
        "legacy_purchase_orphan_refund" | "legacy_purchase_completed" => {
            crate::routes::webhook::deliver_legacy_purchase_effect(state, &row.effect, &row.payload)
                .await
        }
        "package_access_changed" => {
            let user_id = row
                .payload
                .get("userId")
                .and_then(|value| value.as_str())
                .ok_or_else(|| ApiError::internal("Package access effect is missing userId"))?;
            state.invalidate_wasm_permission(user_id, &row.source_id);
            crate::package_license::refresh_package_access(state, user_id, &row.source_id).await;
            Ok(())
        }
        "PAYMENT_AUDIT" => {
            let action = row
                .payload
                .get("action")
                .and_then(|value| value.as_str())
                .unwrap_or("payments.changed");
            if crate::audit::records(&state.platform_config.audit, action) {
                let input=crate::audit::AuditRecordInput::system("payments",action,"PaymentEffect",&row.id).with_details(json!({"sourceType":row.source_type,"sourceId":row.source_id,"change":row.payload}));
                crate::audit::record::write(&state.db, input, crate::audit::WriteMode::Once)
                    .await?;
            }
            Ok(())
        }
        "CANCEL_APP_PAYMENTS" | "CANCEL_ACCOUNT_PAYMENTS" | "CANCEL_SELLER_PAYMENTS" => {
            let mut payload = row.payload.clone();
            payload
                .as_object_mut()
                .ok_or_else(|| ApiError::internal("Invalid payment cancellation payload"))?
                .insert("cutoffCreatedAt".into(), json!(row.created_at));
            let node =
                super::node::handle_effect(state, &row.effect, &row.source_id, &payload).await;
            let marketplace =
                super::marketplace::handle_effect(state, &row.effect, &row.source_id, &payload)
                    .await;
            node.and_then(require_handled)?;
            marketplace.and_then(require_handled)
        }
        "marketplace_adjustments"
        | "marketplace_orphan_refund"
        | "marketplace_refund"
        | "payment_refund_review"
        | "payment_dispute_review" => {
            super::marketplace::handle_effect(state, &row.effect, &row.source_id, &row.payload)
                .await
                .and_then(require_handled)
        }
        _ if row.source_type == "REQUEST" => {
            super::node::handle_effect(state, &row.effect, &row.source_id, &row.payload)
                .await
                .and_then(require_handled)
        }
        _ if row.source_type == "MARKETPLACE" => {
            super::marketplace::handle_effect(state, &row.effect, &row.source_id, &row.payload)
                .await
                .and_then(require_handled)
        }
        _ => Err(error(
            "PAYMENT_EFFECT_UNKNOWN",
            "This payment recovery effect requires operator review",
        )),
    }
}

fn require_handled(handled: bool) -> Result<(), ApiError> {
    if handled {
        Ok(())
    } else {
        Err(error(
            "PAYMENT_EFFECT_UNKNOWN",
            "This payment recovery effect requires operator review",
        ))
    }
}

async fn finish(
    state: &AppState,
    table: &str,
    id: &str,
    lease: &str,
    revision: i64,
    attempts: i32,
    result: &Result<(), ApiError>,
) -> Result<(), ApiError> {
    assert!(matches!(table, "PaymentWebhookInbox" | "PaymentOutbox"));
    let now = Utc::now().timestamp_millis();
    let status = if result.is_ok() {
        "COMPLETED"
    } else if attempts >= 20 {
        "MANUAL_REVIEW"
    } else {
        "PENDING"
    };
    let error = result
        .as_ref()
        .err()
        .map(|_| "Payment recovery failed; see the scoped operation and server trace".to_owned());
    state.db.execute_raw(sql(&format!(r#"UPDATE "{table}" SET status=$4,"lastError"=$5,"nextAttemptAt"=$6,"leaseOwner"=NULL,"leaseExpiresAt"=NULL,revision=revision+1,"updatedAt"=$7 WHERE id=$1 AND "leaseOwner"=$2 AND revision=$3"#),vec![id.into(),lease.into(),revision.into(),status.into(),error.into(),(now+backoff_ms(attempts)).into(),now.into()])).await?;
    Ok(())
}

async fn reconcile_accounts(state: &AppState, limit: i64) -> Result<(), ApiError> {
    let now = Utc::now().timestamp_millis();
    let rows=state.db.query_all_raw(sql(r#"SELECT id,"stripeAccountId" FROM "ConnectedAccount" WHERE "retiredAt" IS NULL AND livemode=$3 AND "syncNextAt"<=$4 AND ("syncLeaseUntil" IS NULL OR "syncLeaseUntil"<$4) AND ("syncedAt" IS NULL OR "syncedAt" < CASE WHEN state IN ('enabled','enabled_action_needed') THEN $1 WHEN "createdAt">$4-86400000 THEN $4-30000 ELSE $4-86400000 END) ORDER BY "syncNextAt",id LIMIT $2"#,vec![(now-300_000).into(),limit.into(),state.platform_config.payments.livemode.into(),now.into()])).await?;
    for row in rows {
        let id: String = row.try_get("", "id")?;
        let result = if row
            .try_get::<Option<String>>("", "stripeAccountId")?
            .is_none()
        {
            super::accounts::finish_account_creation(state, &id).await
        } else {
            super::accounts::sync_account(state, &id).await
        };
        if let Err(error) = result {
            state.db.execute_raw(sql(r#"UPDATE "ConnectedAccount" SET "syncNextAt"=$2,"syncAttempts"="syncAttempts"+1 WHERE id=$1 AND "stripeAccountId" IS NULL"#,vec![id.clone().into(),(now+60_000).into()])).await?;
            tracing::warn!(%id,error=%error,"Payment account reconciliation deferred");
        }
    }
    Ok(())
}
