use chrono::Utc;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QueryOrder};
use serde_json::{Value, json};

use super::{
    Offer, SOURCE, coordinate_entitlement, error, load_order, now, operations, order_scope, outbox,
    sql, stripe_error,
};
use crate::{
    entity::{
        payment_adjustment, payment_attempt, payment_order, payment_refund, stripe_operation,
    },
    error::ApiError,
    state::AppState,
    stripe_connect::{
        EventEnvelope, ExpectedPayment, STRIPE_API_VERSION, StripeGateway, StripeRequest,
        StripeScope, VerifiedPayment,
        types::{Charge, CheckoutSession, PaymentIntent, Refund},
    },
};

pub async fn get(
    state: &AppState,
    scope: StripeScope,
    path: String,
    parameters: Value,
) -> Result<Value, ApiError> {
    let gateway = crate::payments::gateway_for_version(state, STRIPE_API_VERSION).await?;
    Ok(gateway
        .execute(&StripeRequest::get(scope, path, parameters))
        .await
        .map_err(stripe_error)?
        .body)
}

pub async fn reconcile_order(state: &AppState, id: &str) -> Result<(), ApiError> {
    let order = load_order(&state.db, id).await?;
    let attempts = payment_attempt::Entity::find()
        .filter(payment_attempt::Column::SourceType.eq(SOURCE))
        .filter(payment_attempt::Column::SourceId.eq(id))
        .order_by_asc(payment_attempt::Column::Attempt)
        .all(&state.db)
        .await?;
    for mut attempt in attempts {
        if attempt.stripe_session_id.is_none() {
            if stripe_operation::Entity::find_by_id(&attempt.operation_id)
                .one(&state.db)
                .await?
                .is_some_and(|operation| operation.status == "FAILED")
            {
                state.transaction(|txn|{let (order,attempt)=(order.clone(),attempt.clone());Box::pin(async move{
                    crate::db::coordination::coordinate(txn,"payments-charge",&[&attempt.id]).await?;
                    txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status='FAILED',revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND "stripeSessionId" IS NULL AND "capturedAmount"=0"#,vec![attempt.id.into(),now().into()])).await?;
                    txn.execute_raw(sql(r#"UPDATE "PaymentOrder" SET status=CASE WHEN "cancelRequested" THEN 'CANCELED' ELSE 'FAILED' END,"openKey"=NULL,revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND "acceptedAttemptId" IS NULL"#,vec![order.id.clone().into(),now().into()])).await?;
                    release_limit(txn,&order.id).await
                })}).await?;
                continue;
            }
            let response = operations::execute_prepared(state, &attempt.operation_id).await?;
            let session: CheckoutSession = response.decode().map_err(stripe_error)?;
            if session.livemode != order.livemode || session.mode != "payment" {
                return Err(error(
                    "PAYMENT_BINDING_MISMATCH",
                    "Checkout does not match the order",
                ));
            }
            state.db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "stripeSessionId"=$2,"checkoutUrl"=$3,status='OPEN',revision=revision+1,"updatedAt"=$4 WHERE id=$1 AND "stripeSessionId" IS NULL"#,vec![attempt.id.clone().into(),session.id.clone().into(),session.url.clone().into(),now().into()])).await?;
            state.db.execute_raw(sql(r#"UPDATE "PaymentOrder" SET status='OPEN',revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND status='OPENING' AND "cancelRequested"=false"#,vec![order.id.clone().into(),now().into()])).await?;
            attempt = payment_attempt::Entity::find_by_id(&attempt.id)
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
        }
        let session_id = attempt
            .stripe_session_id
            .as_ref()
            .ok_or_else(|| ApiError::internal("Unbound checkout attempt"))?;
        let scope = order_scope(&order);
        let mut session: CheckoutSession = serde_json::from_value(
            get(
                state,
                scope.clone(),
                format!("/v1/checkout/sessions/{session_id}"),
                json!({"expand":["payment_intent.latest_charge"]}),
            )
            .await?,
        )?;
        let latest_order = load_order(&state.db, id).await?;
        if session.status.as_deref() == Some("open")
            && (latest_order.cancel_requested || now() >= latest_order.expires_at)
        {
            let expired = operations::execute(
                state,
                SOURCE,
                id,
                "checkout_expire",
                StripeRequest::post(
                    scope.clone(),
                    format!("/v1/checkout/sessions/{session_id}/expire"),
                    json!({}),
                    format!("mkt:{id}:expire:{}", attempt.id),
                ),
            )
            .await;
            session = serde_json::from_value(
                get(
                    state,
                    scope.clone(),
                    format!("/v1/checkout/sessions/{session_id}"),
                    json!({"expand":["payment_intent.latest_charge"]}),
                )
                .await?,
            )?;
            if session.status.as_deref() == Some("open") {
                expired?;
                return Err(error(
                    "PAYMENT_OPERATION_PENDING",
                    "Checkout cancellation is pending",
                ));
            }
        }
        if session.status.as_deref() == Some("expired") {
            state.transaction(|txn|{let (order,attempt)=(latest_order.clone(),attempt.clone());Box::pin(async move{
                txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status='EXPIRED',revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND "capturedAmount"=0"#,vec![attempt.id.into(),now().into()])).await?;
                txn.execute_raw(sql(r#"UPDATE "PaymentOrder" SET status=$2,"openKey"=NULL,revision=revision+1,"updatedAt"=$3 WHERE id=$1 AND "acceptedAttemptId" IS NULL"#,vec![order.id.clone().into(),if order.cancel_requested{"CANCELED"}else{"EXPIRED"}.into(),now().into()])).await?;
                release_limit(txn,&order.id).await?;Ok::<_,ApiError>(())
            })}).await?;
            continue;
        }
        if session.status.as_deref() != Some("complete") {
            continue;
        }
        let intent_ref = session.payment_intent.as_ref().ok_or_else(|| {
            error(
                "PAYMENT_OPERATION_PENDING",
                "Stripe has not attached the payment intent",
            )
        })?;
        let intent: PaymentIntent = match intent_ref.expanded() {
            Some(value) => value.clone(),
            None => serde_json::from_value(
                get(
                    state,
                    scope.clone(),
                    format!("/v1/payment_intents/{}", intent_ref.id()),
                    json!({"expand":["latest_charge"]}),
                )
                .await?,
            )?,
        };
        let charge: Option<Charge> = match intent.latest_charge.as_ref() {
            Some(value) => Some(match value.expanded() {
                Some(charge) => charge.clone(),
                None => serde_json::from_value(
                    get(
                        state,
                        scope.clone(),
                        format!("/v1/charges/{}", value.id()),
                        json!({}),
                    )
                    .await?,
                )?,
            }),
            None => None,
        };
        let expected = ExpectedPayment {
            scope: scope.clone(),
            session_id: session_id.clone(),
            payment_intent_id: attempt.stripe_payment_intent_id.clone(),
            charge_id: attempt.stripe_charge_id.clone(),
            amount: u64::try_from(order.amount)
                .map_err(|_| ApiError::internal("Invalid stored amount"))?,
            currency: order.currency.clone(),
            application_fee_amount: u64::try_from(order.application_fee_amount)
                .map_err(|_| ApiError::internal("Invalid stored fee"))?,
            destination_account_id: order.connected_account_id.clone(),
        };
        match expected
            .verify(&scope, &session, &intent, charge.as_ref())
            .map_err(|_| {
                error(
                    "PAYMENT_BINDING_MISMATCH",
                    "The payment does not match the recorded order",
                )
            })? {
            VerifiedPayment::Processing => {
                if matches!(
                    intent.status.as_str(),
                    "canceled" | "requires_payment_method"
                ) {
                    state.transaction(|txn|{let (order,attempt)=(order.clone(),attempt.clone());Box::pin(async move{
                        txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status='FAILED',"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND "capturedAmount"=0"#,vec![attempt.id.into(),now().into()])).await?;
                        txn.execute_raw(sql(r#"UPDATE "PaymentOrder" SET status=CASE WHEN "cancelRequested" THEN 'CANCELED' ELSE 'FAILED' END,"openKey"=NULL,"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND "acceptedAttemptId" IS NULL"#,vec![order.id.clone().into(),now().into()])).await?;
                        release_limit(txn,&order.id).await
                    })}).await?;
                    continue;
                }
                state.db.execute_raw(sql(r#"UPDATE "PaymentOrder" SET status='PROCESSING',"nextCheckAt"=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1 AND "acceptedAttemptId" IS NULL"#,vec![order.id.clone().into(),(now()+60_000).into(),now().into()])).await?;
                state.db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status='PROCESSING',"stripePaymentIntentId"=$2,"nextCheckAt"=$3,revision=revision+1,"updatedAt"=$4 WHERE id=$1 AND "capturedAmount"=0"#,vec![attempt.id.clone().into(),intent.id.into(),(now()+60_000).into(),now().into()])).await?;
            }
            VerifiedPayment::Paid => {
                settle_paid(
                    state,
                    &order,
                    &attempt,
                    &session,
                    &intent,
                    charge
                        .as_ref()
                        .ok_or_else(|| ApiError::internal("Verified payment has no charge"))?,
                )
                .await?;
                let captured = payment_attempt::Entity::find_by_id(&attempt.id)
                    .one(&state.db)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;
                refresh_financials(state, &captured).await?;
            }
        }
    }
    Ok(())
}

async fn settle_paid(
    state: &AppState,
    order: &payment_order::Model,
    attempt: &payment_attempt::Model,
    session: &CheckoutSession,
    intent: &PaymentIntent,
    charge: &Charge,
) -> Result<(), ApiError> {
    let offer: Offer = serde_json::from_value(order.snapshot.clone())?;
    let tax = match checkout_tax_amount(&offer.tax_mode, offer.amount, session) {
        Ok(tax) => tax,
        Err(error) => {
            state
                .transaction(|txn| {
                    let (order, attempt) = (order.clone(), attempt.clone());
                    Box::pin(async move { defer_tax_check(txn, &order, &attempt).await })
                })
                .await?;
            return Err(error);
        }
    };
    let mut snapshot = attempt.snapshot.clone();
    snapshot["taxAmount"] = json!(tax);
    snapshot["taxCheckError"] = Value::Null;
    snapshot["receiptUrl"] = json!(charge.receipt_url);
    snapshot["invoiceId"] = json!(session.invoice.as_ref().map(|invoice| invoice.id()));
    snapshot["charge"] = serde_json::to_value(charge)?;
    state
        .transaction(|txn| {
            let (order, attempt, charge, intent, snapshot) = (
                order.clone(),
                attempt.clone(),
                charge.clone(),
                intent.clone(),
                snapshot.clone(),
            );
            Box::pin(
                async move { settle_paid_txn(txn, order, attempt, charge, intent, snapshot).await },
            )
        })
        .await
}

fn checkout_tax_amount(
    tax_mode: &flow_like::hub::PaymentTaxMode,
    amount: i64,
    session: &CheckoutSession,
) -> Result<i64, ApiError> {
    if *tax_mode == flow_like::hub::PaymentTaxMode::PlatformSupplier {
        let complete = session.automatic_tax.as_ref().is_some_and(|tax| {
            tax.enabled
                && tax.status.as_deref() == Some("complete")
                && tax.liability.as_ref().is_some_and(|liability| {
                    liability.liability_type == "self" && liability.account.is_none()
                })
        });
        if !complete || session.total_details.is_none() {
            return Err(error(
                "PAYMENT_TAX_REVIEW",
                "The purchase is waiting for a complete platform tax calculation",
            ));
        }
    }
    let tax = session
        .total_details
        .as_ref()
        .map_or(0, |details| details.amount_tax);
    i64::try_from(tax)
        .ok()
        .filter(|tax| *tax <= amount)
        .ok_or_else(|| {
            error(
                "PAYMENT_TAX_REVIEW",
                "The purchase has inconsistent tax totals",
            )
        })
}

async fn defer_tax_check(
    txn: &sea_orm::DatabaseTransaction,
    order: &payment_order::Model,
    attempt: &payment_attempt::Model,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "payments-charge", &[&attempt.id]).await?;
    let current = payment_attempt::Entity::find_by_id(&attempt.id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let mut snapshot = current.snapshot;
    snapshot["taxCheckError"] = json!("PAYMENT_TAX_REVIEW");
    txn.execute_raw(sql(
        r#"UPDATE "PaymentAttempt" SET snapshot=$2,"nextCheckAt"=$3,revision=revision+1,"updatedAt"=$4 WHERE id=$1"#,
        vec![attempt.id.clone().into(), snapshot.into(), (now()+60_000).into(), now().into()],
    )).await?;
    outbox::enqueue(
        txn,
        &format!("attempt:{}:tax-check", attempt.id),
        "marketplace_reconcile",
        SOURCE,
        &order.id,
        json!({}),
    )
    .await
}

fn validate_recorded_route(order: &payment_order::Model, offer: &Offer) -> Result<(), ApiError> {
    let valid = if offer.platform_owned {
        order.charge_type == "PLATFORM"
            && order.connected_account_id.is_none()
            && offer.account_id.is_none()
            && offer.account_row_id.is_none()
            && order.application_fee_amount == 0
            && order.fee_bps == 0
            && offer.fee == 0
            && offer.fee_bps == 0
    } else {
        order.charge_type == "DESTINATION"
            && order.connected_account_id.is_some()
            && order.connected_account_id == offer.account_id
            && offer.account_row_id.is_some()
    };
    if !valid {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "The recorded payment recipient changed",
        ));
    }
    Ok(())
}

async fn settle_paid_txn(
    txn: &sea_orm::DatabaseTransaction,
    order: payment_order::Model,
    attempt: payment_attempt::Model,
    charge: Charge,
    intent: PaymentIntent,
    snapshot: Value,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "payments-owner", &[&order.payee_user_id]).await?;
    crate::db::coordination::coordinate(txn, "payments-app", &[&order.item_id]).await?;
    crate::db::coordination::coordinate(txn, "payments-charge", &[&attempt.id]).await?;
    coordinate_entitlement(txn, &order.user_id, &order.item_id).await?;
    let current = load_order(txn, &order.id).await?;
    let offer: Offer = serde_json::from_value(current.snapshot.clone())?;
    let product_available=txn.query_one_raw(sql(r#"SELECT id FROM "App" WHERE id=$1 AND visibility IN ('PUBLIC','PUBLIC_REQUEST_ACCESS')"#,vec![order.item_id.clone().into()])).await?.is_some();
    let buyer_exists = txn
        .query_one_raw(sql(
            r#"SELECT id FROM "User" WHERE id=$1"#,
            vec![order.user_id.clone().into()],
        ))
        .await?
        .is_some();
    let blocked=txn.query_one_raw(sql(r#"SELECT id FROM "PaymentEntitlement" WHERE "userId"=$1 AND "itemKind"='APP' AND "itemId"=$2 AND blocked=true"#,vec![order.user_id.clone().into(),order.item_id.clone().into()])).await?.is_some();
    let member = txn
        .query_one_raw(sql(
            r#"SELECT id FROM "Membership" WHERE "userId"=$1 AND "appId"=$2"#,
            vec![order.user_id.clone().into(), order.item_id.clone().into()],
        ))
        .await?
        .is_some();
    let current_attempt = payment_attempt::Entity::find_by_id(&attempt.id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let mut latest_snapshot = current_attempt.snapshot.clone();
    if current_attempt.captured_amount > 0
        && latest_snapshot.get("taxAmount") != snapshot.get("taxAmount")
    {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "The captured payment tax amount changed",
        ));
    }
    if let (Some(previous), Some(observed)) = (
        latest_snapshot["invoiceId"].as_str(),
        snapshot["invoiceId"].as_str(),
    ) {
        if previous != observed {
            return Err(error(
                "PAYMENT_BINDING_MISMATCH",
                "The captured payment invoice changed",
            ));
        }
    }
    for key in [
        "taxAmount",
        "taxCheckError",
        "receiptUrl",
        "charge",
        "invoiceId",
    ] {
        if key != "invoiceId" || snapshot[key].is_string() {
            latest_snapshot[key] = snapshot[key].clone();
        }
    }
    let snapshot = latest_snapshot;
    let assessed_tax = match snapshot.get("taxAmount").and_then(Value::as_i64) {
        Some(tax) if (0..=current.amount).contains(&tax) => tax,
        None if offer.tax_mode == flow_like::hub::PaymentTaxMode::SellerSupplier => 0,
        _ => {
            return Err(error(
                "PAYMENT_TAX_REVIEW",
                "The captured payment has no valid tax amount",
            ));
        }
    };
    let delivered = current.accepted_attempt_id.as_deref() == Some(&attempt.id);
    validate_recorded_route(&current, &offer)?;
    let seller_admitted = if delivered {
        // Already delivered charges keep their original recipient after permission changes.
        true
    } else {
        let owner_matches = txn.query_one_raw(sql(
            r#"SELECT a.id FROM "App" a JOIN "Membership" m ON m."appId"=a.id AND m."roleId"=a."ownerRoleId" JOIN "User" u ON u.id=m."userId" WHERE a.id=$1 AND m."userId"=$2 AND (SELECT COUNT(*) FROM "Membership" owners WHERE owners."appId"=a.id AND owners."roleId"=a."ownerRoleId")=1 AND NOT EXISTS (SELECT 1 FROM "PaymentsBlock" p WHERE p."userId"=$2) AND NOT EXISTS (SELECT 1 FROM "AppPaymentSettings" s WHERE s."appId"=$1 AND s."adminBlockedAt" IS NOT NULL)"#,
            vec![order.item_id.clone().into(), order.payee_user_id.clone().into()],
        )).await?.is_some();
        let is_admin = if owner_matches {
            crate::payments::accounts::is_platform_admin(txn, &order.payee_user_id).await?
        } else {
            false
        };
        if !owner_matches || is_admin != offer.platform_owned {
            false
        } else if offer.platform_owned {
            true
        } else {
            txn.query_one_raw(sql(
                r#"SELECT c.id FROM "ConnectedAccount" c JOIN "PaymentAccountBinding" b ON b."activeAccountId"=c.id WHERE c.id=$1 AND c."userId"=$2 AND c."stripeAccountId"=$3 AND c."platformAccountId"=$4 AND c.livemode=$5 AND c."retiredAt" IS NULL AND c."canSell"=true"#,
                vec![offer.account_row_id.clone().into(), order.payee_user_id.clone().into(), order.connected_account_id.clone().into(), order.platform_account_id.clone().into(), order.livemode.into()],
            )).await?.is_some()
        }
    };
    let unavailable = !seller_admitted
        || !buyer_exists
        || blocked
        || current.withdrawn_at.is_some()
        || !product_available;
    let orphan = !delivered
        && (unavailable
            || member
            || current.accepted_attempt_id.is_some()
            || current.cancel_requested
            || now() > current.expires_at
            || current_attempt.orphaned
            || matches!(
                current.status.as_str(),
                "CANCELED" | "EXPIRED" | "FAILED" | "REFUND_REQUIRED"
            ));
    txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status=$2,"stripePaymentIntentId"=$3,"stripeChargeId"=$4,"capturedAmount"=amount,orphaned=$5,snapshot=$6,"hydrationStatus"=$7,revision=revision+1,"updatedAt"=$8 WHERE id=$1"#,vec![attempt.id.clone().into(),if orphan{"PAID_ORPHANED"}else{"PAID"}.into(),intent.id.into(),charge.id.clone().into(),orphan.into(),snapshot.into(),if offer.platform_owned || (charge.transfer.is_some() && (order.application_fee_amount == 0 || charge.application_fee.is_some())) {"READY"}else{"AWAITING_STRIPE_OBJECTS"}.into(),now().into()])).await?;
    ledger(
        txn,
        &attempt,
        &charge.id,
        "CHARGE",
        "CUSTOMER",
        order.amount,
    )
    .await?;
    if offer.platform_owned {
        ledger(txn, &attempt, &charge.id, "SALE", "PLATFORM", order.amount).await?;
        ledger(
            txn,
            &attempt,
            &charge.id,
            "TAX_LIABILITY",
            "PLATFORM",
            -assessed_tax,
        )
        .await?;
    } else {
        ledger(txn, &attempt, &charge.id, "SALE", "SELLER", order.amount).await?;
        ledger(
            txn,
            &attempt,
            &charge.id,
            "APPLICATION_FEE",
            "SELLER",
            -order.application_fee_amount,
        )
        .await?;
        ledger(
            txn,
            &attempt,
            &charge.id,
            "APPLICATION_FEE",
            "PLATFORM",
            order.application_fee_amount,
        )
        .await?;
    }
    if offer.tax_mode == flow_like::hub::PaymentTaxMode::PlatformSupplier {
        ledger(
            txn,
            &attempt,
            &charge.id,
            "TAX_ASSESSED",
            "TAX",
            assessed_tax,
        )
        .await?;
    }
    txn.execute_raw(sql(r#"UPDATE "PaymentLimitReservation" SET status='CONSUMED',"updatedAt"=$2 WHERE id=$1 AND status='RESERVED'"#,vec![order.id.clone().into(),now().into()])).await?;
    if !orphan && !delivered {
        // The withdrawal period starts when the purchase is confirmed, not while
        // an unpaid Checkout Session is waiting. Replays keep this first deadline.
        let confirmed_at = now();
        let mut confirmed_offer = current.snapshot.clone();
        if let Some(days) = offer.withdrawal_days {
            confirmed_offer["withdrawal_deadline"] =
                json!(confirmed_at + i64::from(days) * 86_400_000);
            confirmed_offer["confirmed_at"] = json!(confirmed_at);
        }
        let grant_id = format!("purchase:{}", order.id);
        txn.execute_raw(sql(r#"INSERT INTO "AccessGrant" (id,"userId","itemKind","itemId","sourceType","sourceId","createdAt","updatedAt") VALUES ($1,$2,'APP',$3,'PURCHASE',$4,$5,$5) ON CONFLICT DO NOTHING"#,vec![grant_id.into(),order.user_id.clone().into(),order.item_id.clone().into(),order.id.clone().into(),now().into()])).await?;
        txn.execute_raw(sql(r#"INSERT INTO "Membership" (id,"userId","appId","roleId","joinedVia","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$6)"#,vec![flow_like_types::create_id().into(),order.user_id.clone().into(),order.item_id.clone().into(),offer.role_id.into(),format!("payment:{}",order.id).into(),Utc::now().fixed_offset().into()])).await?;
        txn.execute_raw(sql(r#"UPDATE "PaymentOrder" SET status='COMPLETED',"acceptedAttemptId"=$2,"openKey"=NULL,snapshot=$4,revision=revision+1,"updatedAt"=$3 WHERE id=$1"#,vec![order.id.clone().into(),attempt.id.clone().into(),confirmed_at.into(),confirmed_offer.into()])).await?;
        txn.execute_raw(sql(r#"INSERT INTO "AppPurchase" (id,"paymentOrderId","chargeType","userId","appId","pricePaid","originalPrice","discountAmount",currency,"stripeSessionId","stripePaymentIntentId",status,"completedAt","createdAt","updatedAt") VALUES ($1,$1,$9,$2,$3,$4,$4,0,$5,$6,$7,'COMPLETED',$8,$8,$8) ON CONFLICT (id) DO NOTHING"#,vec![order.id.clone().into(),order.user_id.clone().into(),order.item_id.clone().into(),order.amount.into(),order.currency.clone().into(),attempt.stripe_session_id.clone().into(),charge.payment_intent.as_ref().map(|id|id.id().to_owned()).into(),Utc::now().fixed_offset().into(),current.charge_type.clone().into()])).await?;
        outbox::enqueue(
            txn,
            &format!("mkt:{}:paid", order.id),
            "marketplace_paid",
            SOURCE,
            &order.id,
            json!({}),
        )
        .await?;
        outbox::enqueue(
            txn,
            &format!("mkt:{}:email:purchased", order.id),
            "PAYMENT_EMAIL",
            SOURCE,
            &order.id,
            json!({"kind":"purchased"}),
        )
        .await?;
    }
    if orphan {
        if current.accepted_attempt_id.is_none() {
            txn.execute_raw(sql(r#"UPDATE "PaymentOrder" SET status='REFUND_REQUIRED',"openKey"=NULL,revision=revision+1,"updatedAt"=$2 WHERE id=$1"#,vec![order.id.clone().into(),now().into()])).await?;
        }
        outbox::enqueue(
            txn,
            &format!("attempt:{}:orphan", attempt.id),
            "marketplace_orphan_refund",
            "PAYMENT_ATTEMPT",
            &attempt.id,
            json!({}),
        )
        .await?;
    }
    outbox::enqueue(
        txn,
        &format!("attempt:{}:capture-adjustments", attempt.id),
        "marketplace_adjustments",
        "PAYMENT_ATTEMPT",
        &attempt.id,
        json!({}),
    )
    .await?;
    Ok::<_, ApiError>(())
}

pub(super) async fn ledger<C: ConnectionTrait>(
    db: &C,
    attempt: &payment_attempt::Model,
    object: &str,
    kind: &str,
    perspective: &str,
    amount: i64,
) -> Result<(), ApiError> {
    let id = blake3::hash(
        serde_json::to_vec(&json!([
            attempt.platform_account_id,
            attempt.scope_key,
            attempt.livemode,
            object,
            kind,
            perspective
        ]))?
        .as_slice(),
    )
    .to_hex()
    .to_string();
    db.execute_raw(sql(r#"INSERT INTO "PaymentLedgerEntry" (id,"attemptId","sourceType","sourceId","platformAccountId","scopeKey",livemode,perspective,kind,amount,currency,"payeeUserId","payerUserId","appId","packageId","stripeObjectId","occurredAt","createdAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$17) ON CONFLICT DO NOTHING"#,vec![id.into(),attempt.id.clone().into(),attempt.source_type.clone().into(),attempt.source_id.clone().into(),attempt.platform_account_id.clone().into(),attempt.scope_key.clone().into(),attempt.livemode.into(),perspective.into(),kind.into(),amount.into(),attempt.currency.clone().into(),attempt.payee_user_id.clone().into(),attempt.payer_user_id.clone().into(),attempt.app_id.clone().into(),attempt.package_id.clone().into(),object.into(),now().into()])).await?;
    Ok(())
}

pub async fn request_refund(
    state: &AppState,
    attempt_id: &str,
    command_id: &str,
    amount: Option<i64>,
    reason: &str,
    requested_by: &str,
) -> Result<String, ApiError> {
    request_refund_inner(
        state,
        attempt_id,
        command_id,
        amount,
        reason,
        requested_by,
        false,
    )
    .await
}

pub async fn request_seller_refund(
    state: &AppState,
    attempt_id: &str,
    command_id: &str,
    amount: Option<i64>,
    reason: &str,
    requested_by: &str,
) -> Result<String, ApiError> {
    request_refund_inner(
        state,
        attempt_id,
        command_id,
        amount,
        reason,
        requested_by,
        true,
    )
    .await
}

async fn request_refund_inner(
    state: &AppState,
    attempt_id: &str,
    command_id: &str,
    amount: Option<i64>,
    reason: &str,
    requested_by: &str,
    require_seller: bool,
) -> Result<String, ApiError> {
    if command_id.is_empty() || command_id.len() > 200 {
        return Err(error(
            "PAYMENT_REFUND_INVALID",
            "Invalid refund command or reason",
        ));
    }
    let command = format!("{attempt_id}:{command_id}");
    let refund_id = blake3::hash(command.as_bytes()).to_hex().to_string();
    state
        .transaction(|txn| {
            let (attempt_id, command, refund_id, reason, requested_by) = (
                attempt_id.to_owned(),
                command.clone(),
                refund_id.clone(),
                reason.to_owned(),
                requested_by.to_owned(),
            );
            Box::pin(async move {
                if require_seller {
                    authorize_seller_refund(txn, &attempt_id, &requested_by).await?;
                }
                reserve_refund_txn(
                    txn,
                    attempt_id,
                    command,
                    refund_id,
                    amount,
                    reason,
                    requested_by,
                )
                .await
            })
        })
        .await
}

async fn authorize_seller_refund(
    txn: &sea_orm::DatabaseTransaction,
    attempt_id: &str,
    requested_by: &str,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "payments-owner", &[requested_by]).await?;
    let attempt = payment_attempt::Entity::find_by_id(attempt_id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let authorized = if platform_retains_proceeds(&attempt) {
        crate::payments::accounts::is_platform_admin(txn, requested_by).await?
    } else {
        attempt.payee_user_id == requested_by
    };
    if !authorized {
        return Err(ApiError::forbidden(
            "Only the payment recipient can issue this refund",
        ));
    }
    Ok(())
}

pub(crate) async fn reserve_refund_txn(
    txn: &sea_orm::DatabaseTransaction,
    attempt_id: String,
    command: String,
    refund_id: String,
    amount: Option<i64>,
    reason: String,
    requested_by: String,
) -> Result<String, ApiError> {
    if !matches!(
        reason.as_str(),
        "requested_by_customer" | "duplicate" | "fraudulent"
    ) {
        return Err(error("PAYMENT_REFUND_INVALID", "Invalid refund reason"));
    }
    crate::db::coordination::coordinate(txn, "payments-charge", &[&attempt_id]).await?;
    if let Some(existing) = payment_refund::Entity::find()
        .filter(payment_refund::Column::CommandId.eq(&command))
        .one(txn)
        .await?
    {
        if existing.attempt_id != attempt_id
            || amount.is_some_and(|a| a != existing.amount)
            || existing.reason != reason
            || existing.requested_by != requested_by
        {
            return Err(error(
                "PAYMENT_IDEMPOTENCY_CONFLICT",
                "Refund command was reused with different parameters",
            ));
        }
        return Ok::<_, ApiError>(existing.id);
    }
    let attempt = payment_attempt::Entity::find_by_id(&attempt_id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let amount = amount.unwrap_or(
        attempt.captured_amount - attempt.refunded_amount - attempt.reserved_refund_amount,
    );
    crate::payments::domain::remaining_refund(
        attempt.captured_amount,
        attempt.refunded_amount,
        attempt.reserved_refund_amount,
        amount,
    )?;
    let charge = attempt.stripe_charge_id.clone().ok_or_else(|| {
        error(
            "PAYMENT_NOT_PAID",
            "The charge is not available for refund yet",
        )
    })?;
    let scope = StripeScope {
        platform_account_id: attempt.platform_account_id.clone(),
        connected_account_id: attempt.connected_account_id.clone(),
        livemode: attempt.livemode,
    };
    let direct = scope.connected_account_id.is_some() && attempt.application_fee_amount > 0;
    let mut parameters = json!({"charge":charge,"amount":amount,"reason":reason});
    if direct {
        parameters["refund_application_fee"] = json!(true);
    }
    let request = StripeRequest::post(
        scope,
        "/v1/refunds",
        parameters,
        format!("refund:{refund_id}"),
    );
    let operation =
        operations::prepare(txn, "PAYMENT_REFUND", &refund_id, "refund_create", &request).await?;
    txn.execute_raw(sql(r#"INSERT INTO "PaymentRefund" (id,"attemptId","commandId","platformAccountId","scopeKey",livemode,amount,currency,status,reason,"requestedBy","operationId","refundApplicationFee","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,'RESERVED',$9,$10,$11,$12,$13,$13)"#,vec![refund_id.clone().into(),attempt_id.clone().into(),command.into(),attempt.platform_account_id.into(),attempt.scope_key.into(),attempt.livemode.into(),amount.into(),attempt.currency.into(),reason.into(),requested_by.into(),operation.into(),direct.into(),now().into()])).await?;
    txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "reservedRefundAmount"="reservedRefundAmount"+$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1"#,vec![attempt_id.into(),amount.into(),now().into()])).await?;
    outbox::enqueue(
        txn,
        &format!("refund:{refund_id}:send"),
        "marketplace_refund",
        "PAYMENT_REFUND",
        &refund_id,
        json!({}),
    )
    .await?;
    Ok(refund_id)
}

pub async fn reconcile_refund(state: &AppState, id: &str) -> Result<(), ApiError> {
    let refund = payment_refund::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let attempt = payment_attempt::Entity::find_by_id(&refund.attempt_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let scope = StripeScope {
        platform_account_id: attempt.platform_account_id.clone(),
        connected_account_id: attempt.connected_account_id.clone(),
        livemode: attempt.livemode,
    };
    let observed: Refund = if let Some(stripe_id) = &refund.stripe_refund_id {
        serde_json::from_value(
            get(state, scope, format!("/v1/refunds/{stripe_id}"), json!({})).await?,
        )?
    } else {
        let operation_id = refund
            .operation_id
            .as_ref()
            .ok_or_else(|| ApiError::internal("Unbound refund operation"))?;
        match operations::execute_prepared(state, operation_id).await {
            Ok(response) => response.decode().map_err(stripe_error)?,
            Err(err) => {
                if stripe_operation::Entity::find_by_id(operation_id)
                    .one(&state.db)
                    .await?
                    .is_some_and(|op| op.status == "FAILED")
                {
                    state.transaction(|txn|{let refund=refund.clone();Box::pin(async move{crate::db::coordination::coordinate(txn,"payments-charge",&[&refund.attempt_id]).await?;txn.execute_raw(sql(r#"UPDATE "PaymentRefund" SET status='FAILED',"failureReason"='provider_rejected',"updatedAt"=$2 WHERE id=$1 AND status='RESERVED'"#,vec![refund.id.into(),now().into()])).await?;recompute_refund_totals(txn,&refund.attempt_id).await})}).await?;
                    return Ok(());
                }
                return Err(err);
            }
        }
    };
    state
        .transaction(|txn| {
            let (refund, observed) = (refund.clone(), observed.clone());
            Box::pin(async move { bind_refund_result(txn, &refund, &observed).await })
        })
        .await?;
    observe_refund(state, &attempt.id, &observed).await?;
    if matches!(
        observed.status.as_deref(),
        Some("pending" | "requires_action") | None
    ) {
        return Err(error(
            "PAYMENT_REFUND_PENDING",
            "The provider is still processing the refund",
        ));
    }
    Ok(())
}

async fn bind_refund_result(
    txn: &sea_orm::DatabaseTransaction,
    local: &payment_refund::Model,
    observed: &Refund,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "payments-charge", &[&local.attempt_id]).await?;
    if observed.amount != local.amount as u64 || observed.currency != local.currency {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "Refund response differs from the saved command",
        ));
    }
    let canonical = payment_refund::Entity::find()
        .filter(payment_refund::Column::AttemptId.eq(&local.attempt_id))
        .filter(payment_refund::Column::StripeRefundId.eq(&observed.id))
        .one(txn)
        .await?;
    if let Some(canonical) = canonical
        && canonical.id != local.id
    {
        // Keep the command as audit evidence; one canonical row owns its amount.
        txn.execute_raw(sql(r#"UPDATE "PaymentRefund" SET status='RECONCILED',"failureReason"=$2,"updatedAt"=$3 WHERE id=$1 AND "stripeRefundId" IS NULL"#,vec![local.id.clone().into(),format!("canonical_refund:{}",canonical.id).into(),now().into()])).await?;
    } else {
        txn.execute_raw(sql(r#"UPDATE "PaymentRefund" SET "stripeRefundId"=$2,"updatedAt"=$3 WHERE id=$1 AND ("stripeRefundId" IS NULL OR "stripeRefundId"=$2)"#,vec![local.id.clone().into(),observed.id.clone().into(),now().into()])).await?;
    }
    recompute_refund_totals(txn, &local.attempt_id).await
}

async fn release_limit<C: ConnectionTrait>(txn: &C, id: &str) -> Result<(), ApiError> {
    let reservations=txn.query_all_raw(sql(r#"UPDATE "PaymentLimitReservation" SET status='RELEASED',"updatedAt"=$2 WHERE id=$1 AND status='RESERVED' RETURNING "counterKey",amount"#,vec![id.into(),now().into()])).await?;
    for row in reservations {
        txn.execute_raw(sql(r#"UPDATE "PaymentLimitCounter" SET amount=GREATEST(0,amount-$2),count=GREATEST(0,count-1),revision=revision+1,"updatedAt"=$3 WHERE key=$1"#,vec![row.try_get::<String>("","counterKey")?.into(),row.try_get::<i64>("","amount")?.into(),now().into()])).await?;
    }
    Ok(())
}

pub async fn observe_refund(
    state: &AppState,
    attempt_id: &str,
    observed: &Refund,
) -> Result<(), ApiError> {
    let attempt = payment_attempt::Entity::find_by_id(attempt_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if attempt.stripe_charge_id.as_deref() != Some(observed.charge.id())
        || observed.currency != attempt.currency
        || observed.amount > u64::try_from(attempt.captured_amount).unwrap_or(0)
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "Refund does not belong to this charge",
        ));
    }
    let status = observed
        .status
        .as_deref()
        .unwrap_or("pending")
        .to_ascii_uppercase();
    if !matches!(
        status.as_str(),
        "SUCCEEDED" | "PENDING" | "REQUIRES_ACTION" | "FAILED" | "CANCELED"
    ) {
        return Err(error("PAYMENT_OPERATION_REVIEW", "Unknown refund state"));
    }
    state
        .transaction(|txn| {
            let (attempt, observed, status) = (attempt.clone(), observed.clone(), status.clone());
            Box::pin(async move { observe_refund_txn(txn, attempt, observed, status).await })
        })
        .await
}

fn platform_retains_proceeds(attempt: &payment_attempt::Model) -> bool {
    attempt.connected_account_id.is_none()
        && attempt.application_fee_amount == 0
        && (attempt
            .snapshot
            .get("platform_owned")
            .and_then(Value::as_bool)
            == Some(true)
            || attempt
                .snapshot
                .get("platformOwned")
                .and_then(Value::as_bool)
                == Some(true))
}

fn tax_liability_perspective(attempt: &payment_attempt::Model) -> Option<&'static str> {
    if platform_retains_proceeds(attempt) {
        Some("PLATFORM")
    } else if attempt.source_type == "REQUEST"
        && attempt.connected_account_id.is_some()
        && attempt.snapshot["automaticTax"].as_bool() == Some(true)
    {
        Some("SELLER")
    } else {
        None
    }
}

async fn observe_refund_txn(
    txn: &sea_orm::DatabaseTransaction,
    attempt: payment_attempt::Model,
    observed: Refund,
    status: String,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "payments-charge", &[&attempt.id]).await?;
    let existing = payment_refund::Entity::find()
        .filter(payment_refund::Column::AttemptId.eq(&attempt.id))
        .filter(payment_refund::Column::StripeRefundId.eq(&observed.id))
        .one(txn)
        .await?;
    let pending = if existing.is_none() {
        txn.query_one_raw(sql(r#"SELECT r.id FROM "PaymentRefund" r JOIN "StripeOperation" o ON o.id=r."operationId" WHERE r."attemptId"=$1 AND o."stripeObjectId"=$2"#,vec![attempt.id.clone().into(),observed.id.clone().into()])).await?.map(|row|row.try_get::<String>("","id")).transpose()?
    } else {
        None
    };
    let id = existing
        .as_ref()
        .map(|r| r.id.clone())
        .or(pending)
        .unwrap_or_else(|| {
            blake3::hash(
                format!(
                    "{}:{}:{}:{}",
                    attempt.platform_account_id, attempt.scope_key, attempt.livemode, observed.id
                )
                .as_bytes(),
            )
            .to_hex()
            .to_string()
        });
    if existing.as_ref().is_some_and(|r| {
        (matches!(r.status.as_str(), "FAILED" | "CANCELED") && r.status != status)
            || (r.status == "SUCCEEDED" && matches!(status.as_str(), "PENDING" | "REQUIRES_ACTION"))
    }) {
        return Ok::<_, ApiError>(());
    }
    txn.execute_raw(sql(r#"INSERT INTO "PaymentRefund" (id,"attemptId","commandId","platformAccountId","scopeKey",livemode,"stripeRefundId",amount,currency,status,reason,"requestedBy","failureReason","pendingReason","createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,'requested_by_customer','stripe',$11,$12,$13,$13) ON CONFLICT (id) DO UPDATE SET "stripeRefundId"=EXCLUDED."stripeRefundId",status=EXCLUDED.status,"failureReason"=EXCLUDED."failureReason","pendingReason"=EXCLUDED."pendingReason","updatedAt"=EXCLUDED."updatedAt""#,vec![id.clone().into(),attempt.id.clone().into(),format!("stripe:{}:{}:{}:{}",attempt.platform_account_id,attempt.scope_key,attempt.livemode,observed.id).into(),attempt.platform_account_id.clone().into(),attempt.scope_key.clone().into(),attempt.livemode.into(),observed.id.clone().into(),(observed.amount as i64).into(),observed.currency.clone().into(),status.clone().into(),observed.failure_reason.into(),observed.pending_reason.into(),now().into()])).await?;
    recompute_refund_totals(txn, &attempt.id).await?;
    if status == "SUCCEEDED" {
        ledger(
            txn,
            &attempt,
            &observed.id,
            "REFUND",
            "CUSTOMER",
            -(observed.amount as i64),
        )
        .await?;
        if platform_retains_proceeds(&attempt) {
            ledger(
                txn,
                &attempt,
                &observed.id,
                "REFUND",
                "PLATFORM",
                -(observed.amount as i64),
            )
            .await?;
        }
    }
    if matches!(status.as_str(), "FAILED" | "CANCELED") {
        if txn.query_one_raw(sql(r#"SELECT id FROM "PaymentLedgerEntry" WHERE "attemptId"=$1 AND "stripeObjectId"=$2 AND perspective='CUSTOMER' AND kind='REFUND'"#,vec![attempt.id.clone().into(),observed.id.clone().into()])).await?.is_some(){
            ledger(txn,&attempt,&observed.id,"REFUND_RETURNED","CUSTOMER",observed.amount as i64).await?;
            if platform_retains_proceeds(&attempt) {
                ledger(txn,&attempt,&observed.id,"REFUND_RETURNED","PLATFORM",observed.amount as i64).await?;
            }
        }
        let current = payment_attempt::Entity::find_by_id(&attempt.id)
            .one(txn)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        let mut snapshot = current.snapshot;
        snapshot["refundReviewRequired"] = json!(true);
        txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET snapshot=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1"#,vec![attempt.id.clone().into(),snapshot.into(),now().into()])).await?;
        outbox::enqueue(
            txn,
            &format!("refund:{id}:review"),
            "payment_refund_review",
            "PAYMENT_REFUND",
            &id,
            json!({"attemptId":attempt.id,"status":status}),
        )
        .await?;
    }
    if attempt.source_type == SOURCE {
        let order = load_order(txn, &attempt.source_id).await?;
        let current = payment_attempt::Entity::find_by_id(&attempt.id)
            .one(txn)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        if order.accepted_attempt_id.as_deref() == Some(&attempt.id) && current.refunded_amount > 0
        {
            txn.execute_raw(sql(r#"UPDATE "AppPurchase" SET status=$2,"refundedAt"=CASE WHEN $3 THEN COALESCE("refundedAt",$4) ELSE "refundedAt" END,"refundReason"='stripe_refund',"updatedAt"=$4 WHERE "paymentOrderId"=$1"#,vec![order.id.clone().into(),if current.refunded_amount>=current.captured_amount{"REFUNDED"}else{"PARTIALLY_REFUNDED"}.into(),(current.refunded_amount>=current.captured_amount).into(),Utc::now().fixed_offset().into()])).await?;
        }
        if current.refunded_amount >= current.captured_amount
            && order.accepted_attempt_id.as_deref() == Some(&attempt.id)
        {
            coordinate_entitlement(txn, &order.user_id, &order.item_id).await?;
            revoke_grant(txn, &order, "full_refund").await?;
        }
        outbox::enqueue(
            txn,
            &format!("refund:{id}:{status}:adjust"),
            "marketplace_adjustments",
            "PAYMENT_ATTEMPT",
            &attempt.id,
            json!({}),
        )
        .await?;
    }
    Ok(())
}

async fn recompute_refund_totals<C: ConnectionTrait>(
    db: &C,
    attempt: &str,
) -> Result<(), ApiError> {
    db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "refundedAmount"=(SELECT COALESCE(SUM(amount),0) FROM "PaymentRefund" WHERE "attemptId"=$1 AND status='SUCCEEDED'),"reservedRefundAmount"=(SELECT COALESCE(SUM(amount),0) FROM "PaymentRefund" WHERE "attemptId"=$1 AND status IN ('RESERVED','PENDING','REQUIRES_ACTION')),revision=revision+1,"updatedAt"=$2 WHERE id=$1"#,vec![attempt.into(),now().into()])).await?;
    Ok(())
}

pub(super) async fn revoke_grant<C: ConnectionTrait>(
    db: &C,
    order: &payment_order::Model,
    reason: &str,
) -> Result<(), ApiError> {
    db.execute_raw(sql(r#"UPDATE "AccessGrant" SET status='REVOKED',reason=$2,revision=revision+1,"updatedAt"=$3 WHERE "sourceType"='PURCHASE' AND "sourceId"=$1"#,vec![order.id.clone().into(),reason.into(),now().into()])).await?;
    let offer: Offer = serde_json::from_value(order.snapshot.clone())?;
    db.execute_raw(sql(r#"DELETE FROM "Membership" WHERE "userId"=$1 AND "appId"=$2 AND "roleId"=$3 AND "joinedVia"=$4 AND NOT EXISTS (SELECT 1 FROM "AccessGrant" WHERE "userId"=$1 AND "itemKind"='APP' AND "itemId"=$2 AND status='ACTIVE')"#,vec![order.user_id.clone().into(),order.item_id.clone().into(),offer.role_id.into(),format!("payment:{}",order.id).into()])).await?;
    Ok(())
}

pub async fn handle_event(state: &AppState, event: &EventEnvelope) -> Result<bool, ApiError> {
    if event.account.is_some() {
        return Ok(false);
    }
    let Some(platform) = state
        .platform_config
        .payments
        .platform_account_id
        .as_deref()
    else {
        return Ok(false);
    };
    let object = &event.data.object;
    let id = object.get("id").and_then(Value::as_str).unwrap_or("");
    let charge = id_of(object.get("charge"));
    let intent = id_of(object.get("payment_intent"));
    let attempt = payment_attempt::Entity::find()
        .filter(payment_attempt::Column::SourceType.eq(SOURCE))
        .filter(payment_attempt::Column::PlatformAccountId.eq(platform))
        .filter(payment_attempt::Column::Livemode.eq(event.livemode))
        .filter(
            sea_orm::Condition::any()
                .add(payment_attempt::Column::StripeSessionId.eq(id))
                .add(payment_attempt::Column::StripeChargeId.eq(id))
                .add(payment_attempt::Column::StripeChargeId.eq(charge))
                .add(payment_attempt::Column::StripePaymentIntentId.eq(intent)),
        )
        .one(&state.db)
        .await?;
    let Some(attempt) = attempt else {
        return Ok(false);
    };
    let scope = StripeScope::platform(&attempt.platform_account_id, attempt.livemode);
    if event.event_type.starts_with("refund.") {
        let refund: Refund = serde_json::from_value(
            get(state, scope, format!("/v1/refunds/{id}"), json!({})).await?,
        )?;
        observe_refund(state, &attempt.id, &refund).await?;
    } else if event.event_type.starts_with("charge.dispute.") {
        observe_dispute(state, &attempt, id).await?;
    } else {
        reconcile_order(state, &attempt.source_id).await?;
    }
    Ok(true)
}

fn id_of(value: Option<&Value>) -> &str {
    value
        .and_then(|v| v.as_str().or_else(|| v.get("id").and_then(Value::as_str)))
        .unwrap_or("")
}

pub async fn handle_effect(
    state: &AppState,
    effect: &str,
    source_id: &str,
    payload: &Value,
) -> Result<bool, ApiError> {
    match effect {
        "CANCEL_APP_PAYMENTS" | "CANCEL_ACCOUNT_PAYMENTS" | "CANCEL_SELLER_PAYMENTS" => {
            cancel_scope(state, effect, source_id, payload).await?
        }
        "marketplace_reconcile" => reconcile_order(state, source_id).await?,
        "marketplace_refund" => reconcile_refund(state, source_id).await?,
        "payment_refund_review" => {
            return Err(error(
                "PAYMENT_REFUND_REQUIRES_REVIEW",
                "The refund failed or was canceled. Repayment requires operator review",
            ));
        }
        "payment_dispute_review" => {
            return Err(error(
                "PAYMENT_DISPUTE_REQUIRES_REVIEW",
                "Dispute settlement currency requires operator review",
            ));
        }
        "marketplace_orphan_refund" => {
            request_refund(
                state,
                source_id,
                &format!("orphan:{source_id}"),
                None,
                "duplicate",
                "system",
            )
            .await?;
        }
        "marketplace_adjustments" => adjust_attempt(state, source_id).await?,
        "marketplace_withdrawal" => {
            deliver_notice(state, effect, source_id).await?;
            let order = load_order(&state.db, source_id).await?;
            let attempt = payment_attempt::Entity::find_by_id(
                order.accepted_attempt_id.ok_or(ApiError::NOT_FOUND)?,
            )
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
            if attempt.refunded_amount < attempt.captured_amount {
                if attempt.captured_amount
                    - attempt.refunded_amount
                    - attempt.reserved_refund_amount
                    <= 0
                {
                    return Err(error(
                        "PAYMENT_REFUND_PENDING",
                        "An existing refund must finish before withdrawal repayment",
                    ));
                }
                request_refund(
                    state,
                    &attempt.id,
                    &format!("withdrawal:{source_id}"),
                    None,
                    "requested_by_customer",
                    &order.user_id,
                )
                .await?;
            }
        }
        "marketplace_paid" | "marketplace_comp" => deliver_notice(state, effect, source_id).await?,
        _ => return Ok(false),
    }
    Ok(true)
}

async fn cancel_scope(
    state: &AppState,
    effect: &str,
    source_id: &str,
    payload: &Value,
) -> Result<(), ApiError> {
    let cutoff = payload
        .get("cutoffCreatedAt")
        .and_then(Value::as_i64)
        .unwrap_or_else(now);
    let account_id = payload
        .get("accountId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let rows = if effect == "CANCEL_APP_PAYMENTS" {
        state.db.query_all_raw(sql(r#"SELECT id,"itemId" FROM "PaymentOrder" WHERE kind='APP' AND "itemId"=$1 AND "createdAt"<=$2 AND "acceptedAttemptId" IS NULL AND "cancelRequested"=false AND "openKey" IS NOT NULL ORDER BY "itemId",id LIMIT 25"#,vec![source_id.into(),cutoff.into()])).await?
    } else if effect == "CANCEL_SELLER_PAYMENTS" {
        state.db.query_all_raw(sql(r#"SELECT id,"itemId" FROM "PaymentOrder" WHERE "payeeUserId"=$1 AND "createdAt"<=$2 AND "acceptedAttemptId" IS NULL AND "cancelRequested"=false AND "openKey" IS NOT NULL ORDER BY "itemId",id LIMIT 25"#,vec![source_id.into(),cutoff.into()])).await?
    } else {
        state.db.query_all_raw(sql(r#"SELECT id,"itemId" FROM "PaymentOrder" WHERE (snapshot->>'account_row_id'=$1 OR "connectedAccountId"=$2) AND "createdAt"<=$3 AND "acceptedAttemptId" IS NULL AND "cancelRequested"=false AND "openKey" IS NOT NULL ORDER BY "itemId",id LIMIT 25"#,vec![source_id.into(),account_id.into(),cutoff.into()])).await?
    };
    let count = rows.len();
    let ids = rows
        .into_iter()
        .map(|row| {
            Ok::<_, ApiError>((
                row.try_get::<String>("", "id")?,
                row.try_get::<String>("", "itemId")?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    state.transaction(|txn|{let ids=ids.clone();Box::pin(async move{
        for (id,app) in ids {
            crate::db::coordination::coordinate(txn,"payments-app",&[&app]).await?;
            let changed=txn.execute_raw(sql(r#"UPDATE "PaymentOrder" SET "cancelRequested"=true,status='CANCEL_PENDING',"nextCheckAt"=$2,revision=revision+1,"updatedAt"=$2 WHERE id=$1 AND "acceptedAttemptId" IS NULL AND "cancelRequested"=false AND "openKey" IS NOT NULL"#,vec![id.clone().into(),now().into()])).await?;
            if changed.rows_affected()==1{outbox::enqueue(txn,&format!("mkt:{id}:cancel"),"marketplace_reconcile",SOURCE,&id,json!({})).await?;}
        }
        Ok::<_,ApiError>(())
    })}).await?;
    if count == 25 {
        return Err(error(
            "PAYMENT_OPERATION_PENDING",
            "More checkouts are awaiting cancellation",
        ));
    }
    Ok(())
}

async fn list_objects(
    state: &AppState,
    scope: StripeScope,
    path: &str,
    mut params: Value,
) -> Result<Vec<Value>, ApiError> {
    params["limit"] = json!(100);
    let mut records = Vec::new();
    for _ in 0..10 {
        let response = get(state, scope.clone(), path.to_owned(), params.clone()).await?;
        let data = response
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                error(
                    "PAYMENT_OPERATION_REVIEW",
                    "Stripe returned an incomplete collection",
                )
            })?;
        records.extend(data.iter().cloned());
        if response.get("has_more").and_then(Value::as_bool) == Some(false) {
            return Ok(records);
        }
        let last = data
            .last()
            .and_then(|v| v.get("id"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                error(
                    "PAYMENT_OPERATION_REVIEW",
                    "Stripe collection pagination stopped",
                )
            })?;
        params["starting_after"] = json!(last);
    }
    Err(error(
        "PAYMENT_OPERATION_REVIEW",
        "The financial collection requires operator reconciliation",
    ))
}

async fn refresh_financials(
    state: &AppState,
    attempt: &payment_attempt::Model,
) -> Result<(), ApiError> {
    let charge = attempt
        .stripe_charge_id
        .as_deref()
        .ok_or(ApiError::NOT_FOUND)?;
    let scope = StripeScope::platform(&attempt.platform_account_id, attempt.livemode);
    for record in list_objects(
        state,
        scope.clone(),
        "/v1/refunds",
        json!({"charge":charge}),
    )
    .await?
    {
        let refund: Refund = serde_json::from_value(record)?;
        observe_refund(state, &attempt.id, &refund).await?;
    }
    for record in list_objects(state, scope, "/v1/disputes", json!({"charge":charge})).await? {
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| error("PAYMENT_OPERATION_REVIEW", "A dispute has no id"))?;
        observe_dispute(state, attempt, id).await?;
    }
    Ok(())
}

pub async fn reconcile(state: &AppState, limit: i64) -> Result<(), ApiError> {
    let rows=state.db.query_all_raw(sql(r#"SELECT o.id,MAX(a."capturedAmount") AS captured FROM "PaymentOrder" o LEFT JOIN "PaymentAttempt" a ON a."sourceType"='MARKETPLACE' AND a."sourceId"=o.id WHERE o."nextCheckAt"<=$1 AND (o.status IN ('CREATED','OPENING','OPEN','PROCESSING','CANCEL_PENDING') OR a."capturedAmount">0 OR a."hydrationStatus"='AWAITING_STRIPE_OBJECTS') GROUP BY o.id,o."nextCheckAt" ORDER BY o."nextCheckAt",o.id LIMIT $2"#,vec![now().into(),limit.clamp(1,100).into()])).await?;
    for row in rows {
        let id: String = row.try_get("", "id")?;
        let delay = if row.try_get::<Option<i64>>("", "captured")?.unwrap_or(0) > 0 {
            3_600_000
        } else {
            60_000
        };
        if let Err(err) = reconcile_order(state, &id).await {
            tracing::warn!(order_id=%id,error=%err,"Marketplace order reconciliation deferred");
        }
        state
            .db
            .execute_raw(sql(
                r#"UPDATE "PaymentOrder" SET "nextCheckAt"=$2 WHERE id=$1"#,
                vec![id.into(), (now() + delay).into()],
            ))
            .await?;
    }
    Ok(())
}

async fn observe_dispute(
    state: &AppState,
    attempt: &payment_attempt::Model,
    id: &str,
) -> Result<(), ApiError> {
    let scope = StripeScope::platform(&attempt.platform_account_id, attempt.livemode);
    let dispute: crate::stripe_connect::types::Dispute =
        serde_json::from_value(get(state, scope, format!("/v1/disputes/{id}"), json!({})).await?)?;
    if attempt.stripe_charge_id.as_deref() != Some(dispute.charge.id())
        || dispute.currency != attempt.currency
        || dispute.livemode != attempt.livemode
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "Dispute does not belong to this charge",
        ));
    }
    if dispute
        .balance_transactions
        .iter()
        .any(|movement| movement.currency != attempt.currency)
    {
        return state
            .transaction(|txn| {
                let (attempt, dispute) = (attempt.clone(), dispute.clone());
                Box::pin(
                    async move { record_dispute_currency_review_txn(txn, attempt, dispute).await },
                )
            })
            .await;
    }
    let cash = dispute
        .balance_transactions
        .iter()
        .try_fold(0_i64, |sum, tx| sum.checked_add(tx.amount))
        .ok_or_else(|| ApiError::internal("Dispute amount overflow"))?;
    let outstanding = cash.saturating_neg().max(0).min(attempt.captured_amount);
    state.transaction(|txn|{let (attempt,dispute)=(attempt.clone(),dispute.clone());Box::pin(async move{
        crate::db::coordination::coordinate(txn,"payments-charge",&[&attempt.id]).await?;
        let current=payment_attempt::Entity::find_by_id(&attempt.id).one(txn).await?.ok_or(ApiError::NOT_FOUND)?;
        let previous=current.snapshot.pointer("/dispute/status").and_then(Value::as_str);
        if previous.is_some_and(|status|matches!(status,"won"|"lost"|"warning_closed") && status!=dispute.status){return Ok::<_,ApiError>(());}
        let mut snapshot=current.snapshot;snapshot["dispute"]=json!({"id":dispute.id,"status":dispute.status,"outstanding":outstanding,"evidence":dispute.evidence_details});
        txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET snapshot=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1"#,vec![attempt.id.clone().into(),snapshot.into(),now().into()])).await?;
        for movement in &dispute.balance_transactions{ledger(txn,&attempt,&movement.id,"DISPUTE_CASH","PLATFORM",movement.amount).await?;}
        if dispute.status=="lost"{let order=load_order(txn,&attempt.source_id).await?;if order.accepted_attempt_id.as_deref()==Some(&attempt.id){coordinate_entitlement(txn,&order.user_id,&order.item_id).await?;revoke_grant(txn,&order,"lost_dispute").await?;}}
        outbox::enqueue(txn,&format!("dispute:{}:{}:{outstanding}",dispute.id,dispute.status),"marketplace_adjustments","PAYMENT_ATTEMPT",&attempt.id,json!({})).await?;
        let audit=crate::audit::AuditRecordInput::system("stripe","marketplace.dispute.updated","PaymentAttempt",&format!("{}:{}:{outstanding}",dispute.id,dispute.status)).on_scope(attempt.app_id.as_deref().unwrap_or("platform")).with_details(json!({"attemptId":attempt.id,"status":dispute.status}));
        crate::audit::record::write(txn,audit,crate::audit::WriteMode::Once).await?;Ok(())
    })}).await
}

async fn record_dispute_currency_review_txn(
    txn: &sea_orm::DatabaseTransaction,
    attempt: payment_attempt::Model,
    dispute: crate::stripe_connect::types::Dispute,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "payments-charge", &[&attempt.id]).await?;
    let current = payment_attempt::Entity::find_by_id(&attempt.id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if current
        .snapshot
        .pointer("/dispute/status")
        .and_then(Value::as_str)
        .is_some_and(|status| {
            matches!(status, "won" | "lost" | "warning_closed") && status != dispute.status
        })
    {
        return Ok(());
    }
    let mut snapshot = current.snapshot;
    snapshot["dispute"] = json!({
        "id": dispute.id,
        "status": dispute.status,
        "evidence": dispute.evidence_details,
    });
    // Retain settlement amounts in their actual currencies without deriving a sale-currency loss.
    snapshot["disputeCurrencyReview"] = serde_json::to_value(&dispute)?;
    snapshot["disputeReviewRequired"] = json!(true);
    txn.execute_raw(sql(
        r#"UPDATE "PaymentAttempt" SET snapshot=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1"#,
        vec![attempt.id.clone().into(), snapshot.into(), now().into()],
    ))
    .await?;
    if dispute.status == "lost" {
        let order = load_order(txn, &attempt.source_id).await?;
        if order.accepted_attempt_id.as_deref() == Some(&attempt.id) {
            coordinate_entitlement(txn, &order.user_id, &order.item_id).await?;
            revoke_grant(txn, &order, "lost_dispute").await?;
        }
    }
    outbox::enqueue(
        txn,
        &format!("dispute:{}:currency-review", dispute.id),
        "payment_dispute_review",
        "PAYMENT_ATTEMPT",
        &attempt.id,
        json!({"disputeId":dispute.id,"reason":"settlement_currency_mismatch"}),
    )
    .await?;
    Ok(())
}

fn require_dispute_settlement_currency(snapshot: &Value) -> Result<(), ApiError> {
    if snapshot
        .get("disputeReviewRequired")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(error(
            "PAYMENT_DISPUTE_REQUIRES_REVIEW",
            "Seller recovery is paused until dispute settlement currency is reviewed",
        ));
    }
    Ok(())
}

/// Cumulative targets prevent a refund and a dispute each recovering the full sale.
fn targets(
    gross: i64,
    tax: i64,
    refunded: i64,
    disputed: i64,
    bps: u16,
    net_fee: bool,
) -> Result<(i64, i64), ApiError> {
    if gross <= 0 || tax < 0 || tax > gross || refunded < 0 || refunded > gross || disputed < 0 {
        return Err(error(
            "PAYMENT_OPERATION_REVIEW",
            "Invalid settlement totals",
        ));
    }
    let customer_remaining = (gross - refunded - disputed.min(gross - refunded)).max(0);
    let tax_remaining =
        (i128::from(tax) * i128::from(customer_remaining) / i128::from(gross)) as i64;
    let fee_gross = gross - refunded;
    let fee_tax = (i128::from(tax) * i128::from(fee_gross) / i128::from(gross)) as i64;
    let base = if net_fee {
        fee_gross - fee_tax
    } else {
        fee_gross
    };
    let retained_fee = ((i128::from(base) * i128::from(bps) + 9999) / 10000) as i64;
    Ok((gross - customer_remaining + tax_remaining, retained_fee))
}

fn targets_with_refunded_tax(
    gross: i64,
    tax: i64,
    tax_refunded: i64,
    refunded: i64,
    disputed: i64,
    bps: u16,
    net_fee: bool,
) -> Result<(i64, i64), ApiError> {
    targets(gross, tax, refunded, disputed, bps, net_fee)?;
    if tax_refunded < 0 || tax_refunded > tax {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "Tax credits exceed the assessed tax",
        ));
    }
    let remaining = gross - refunded;
    let disputed = disputed.min(remaining);
    let tax_remaining = tax - tax_refunded;
    let retained_tax = if remaining == 0 {
        0
    } else {
        (i128::from(tax_remaining) * i128::from(remaining - disputed) / i128::from(remaining))
            as i64
    };
    let fee_base = if net_fee {
        remaining - tax_remaining
    } else {
        remaining
    };
    if fee_base < 0 {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "Tax credits have not caught up with the refund",
        ));
    }
    Ok((
        refunded + disputed + retained_tax,
        ((i128::from(fee_base) * i128::from(bps) + 9999) / 10000) as i64,
    ))
}

fn credit_note_tax(note: &Value) -> Result<i64, ApiError> {
    let total = required_i64(note, "amount")?;
    let net = required_i64(note, "total_excluding_tax")?;
    total
        .checked_sub(net)
        .filter(|tax| total >= 0 && net >= 0 && *tax >= 0 && *tax <= total)
        .ok_or_else(|| {
            error(
                "PAYMENT_TAX_REVIEW",
                "The credit note has inconsistent tax totals",
            )
        })
}

fn verify_credit_note(
    note: &Value,
    attempt: &payment_attempt::Model,
    invoice: &str,
    expected_amount: Option<i64>,
) -> Result<i64, ApiError> {
    let amount = required_i64(note, "amount")?;
    if id_of(note.get("invoice")) != invoice
        || note.get("currency").and_then(Value::as_str) != Some(&attempt.currency)
        || note.get("livemode").and_then(Value::as_bool) != Some(attempt.livemode)
        || note.get("status").and_then(Value::as_str) != Some("issued")
        || expected_amount.is_some_and(|expected| expected != amount)
        || required_i64(note, "post_payment_amount")? != amount
        || required_i64(note, "pre_payment_amount")? != 0
    {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "The credit note does not match the paid invoice and refund",
        ));
    }
    let tax = credit_note_tax(note)?;
    if tax > recorded_tax(attempt)? {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "Tax credits exceed the original tax",
        ));
    }
    Ok(tax)
}

fn recorded_tax(attempt: &payment_attempt::Model) -> Result<i64, ApiError> {
    attempt
        .snapshot
        .get("taxAmount")
        .and_then(Value::as_i64)
        .filter(|tax| (0..=attempt.amount).contains(tax))
        .ok_or_else(|| {
            error(
                "PAYMENT_TAX_REVIEW",
                "The original tax amount is unavailable",
            )
        })
}

pub async fn reconcile_node_tax(
    state: &AppState,
    attempt: &payment_attempt::Model,
) -> Result<(), ApiError> {
    if attempt.source_type != "REQUEST" {
        return Err(ApiError::internal("Expected a node payment"));
    }
    if let Some(pending) = payment_adjustment::Entity::find()
        .filter(payment_adjustment::Column::AttemptId.eq(&attempt.id))
        .filter(payment_adjustment::Column::Status.eq("PENDING"))
        .one(&state.db)
        .await?
    {
        if pending.purpose != "TAX_CREDIT_NOTE" {
            return Err(error(
                "PAYMENT_TAX_REVIEW",
                "The node payment has an unexpected adjustment",
            ));
        }
        finish_adjustment(state, attempt, &pending).await?;
    }
    credit_refunds(state, attempt).await?;
    Ok(())
}

async fn credit_refunds(
    state: &AppState,
    attempt: &payment_attempt::Model,
) -> Result<i64, ApiError> {
    let refunds = payment_refund::Entity::find()
        .filter(payment_refund::Column::AttemptId.eq(&attempt.id))
        .filter(payment_refund::Column::Status.eq("SUCCEEDED"))
        .all(&state.db)
        .await?;
    if attempt
        .snapshot
        .get("refundReviewRequired")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "A failed refund requires invoice and repayment review",
        ));
    }
    if refunds.is_empty() {
        return Ok(0);
    }
    let invoice = attempt
        .snapshot
        .get("invoiceId")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                "PAYMENT_OPERATION_PENDING",
                "The invoice is still being created",
            )
        })?;
    let scope = StripeScope {
        platform_account_id: attempt.platform_account_id.clone(),
        connected_account_id: attempt.connected_account_id.clone(),
        livemode: attempt.livemode,
    };
    let record = get(
        state,
        scope.clone(),
        format!("/v1/invoices/{invoice}"),
        json!({}),
    )
    .await?;
    if record.get("id").and_then(Value::as_str) != Some(invoice)
        || record.get("status").and_then(Value::as_str) != Some("paid")
        || required_i64(&record, "total")? != attempt.amount
        || record.get("currency").and_then(Value::as_str) != Some(&attempt.currency)
        || record.get("livemode").and_then(Value::as_bool) != Some(attempt.livemode)
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "The invoice does not match the purchase",
        ));
    }
    if record
        .pointer("/automatic_tax/enabled")
        .and_then(Value::as_bool)
        != Some(true)
        || record
            .pointer("/automatic_tax/status")
            .and_then(Value::as_str)
            != Some("complete")
        || record
            .pointer("/automatic_tax/liability/type")
            .and_then(Value::as_str)
            != Some("self")
        || record.pointer("/issuer/type").and_then(Value::as_str) != Some("self")
        || required_i64(&record, "total_excluding_tax")?.checked_add(recorded_tax(attempt)?)
            != Some(attempt.amount)
    {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "The invoice tax does not match the purchase",
        ));
    }
    let notes = list_objects(
        state,
        scope.clone(),
        "/v1/credit_notes",
        json!({"invoice":invoice}),
    )
    .await?;
    let mut tax_refunded = 0_i64;
    let mut linked = std::collections::HashMap::<String, i64>::new();
    for note in notes {
        if note.get("status").and_then(Value::as_str) == Some("void") {
            continue;
        }
        let tax = verify_credit_note(&note, attempt, invoice, None)?;
        let allocations = note
            .get("refunds")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                error(
                    "PAYMENT_TAX_REVIEW",
                    "Credit note refund allocations are unavailable",
                )
            })?;
        let mut total = 0_i64;
        for allocation in allocations {
            let refund = id_of(allocation.get("refund"));
            if !refunds
                .iter()
                .any(|r| r.stripe_refund_id.as_deref() == Some(refund))
            {
                return Err(error(
                    "PAYMENT_TAX_REVIEW",
                    "An invoice credit has no reconciled refund",
                ));
            }
            let amount = required_i64(allocation, "amount_refunded")?;
            if amount <= 0 {
                return Err(error(
                    "PAYMENT_TAX_REVIEW",
                    "Invalid credit note refund allocation",
                ));
            }
            total = total
                .checked_add(amount)
                .ok_or_else(|| ApiError::internal("Credit allocation overflow"))?;
            let entry = linked.entry(refund.to_owned()).or_default();
            *entry = entry
                .checked_add(amount)
                .ok_or_else(|| ApiError::internal("Credit allocation overflow"))?;
        }
        if total != required_i64(&note, "amount")? {
            return Err(error(
                "PAYMENT_TAX_REVIEW",
                "Non-refund invoice credits require operator review",
            ));
        }
        tax_refunded = tax_refunded
            .checked_add(tax)
            .ok_or_else(|| ApiError::internal("Tax credit overflow"))?;
        if tax_refunded > recorded_tax(attempt)? {
            return Err(error(
                "PAYMENT_TAX_REVIEW",
                "Tax credits exceed the original tax",
            ));
        }
        ledger(
            &state.db,
            attempt,
            id_of(Some(&note)),
            "CREDIT_NOTE",
            "TAX",
            -tax,
        )
        .await?;
        if let Some(perspective) = tax_liability_perspective(attempt) {
            ledger(
                &state.db,
                attempt,
                id_of(Some(&note)),
                "TAX_LIABILITY_RELEASE",
                perspective,
                tax,
            )
            .await?;
        }
    }
    for refund in refunds {
        let provider_id = refund.stripe_refund_id.as_deref().ok_or_else(|| {
            error(
                "PAYMENT_TAX_REVIEW",
                "A completed refund has no provider id",
            )
        })?;
        let baseline = linked.get(provider_id).copied().unwrap_or(0);
        if baseline > refund.amount {
            return Err(error(
                "PAYMENT_TAX_REVIEW",
                "Credit notes exceed the refund",
            ));
        }
        if baseline < refund.amount {
            let amount = refund.amount - baseline;
            // One gross invoice amount lets Stripe allocate tax and rounding for its original line.
            let parameters = json!({"invoice":invoice,"amount":amount,"refunds":[{"refund":provider_id,"amount_refunded":amount}],"reason":"order_change","email_type":"credit_note"});
            let preview = get(
                state,
                scope.clone(),
                "/v1/credit_notes/preview".into(),
                json!({"invoice":invoice,"amount":amount,"refunds":[{"refund":provider_id,"amount_refunded":amount}]}),
            )
            .await?;
            if required_i64(&preview, "amount")? != amount
                || required_i64(&preview, "post_payment_amount")? != amount
                || credit_note_tax(&preview)? > recorded_tax(attempt)? - tax_refunded
            {
                return Err(error(
                    "PAYMENT_TAX_REVIEW",
                    "The proposed credit does not match the refund",
                ));
            }
            plan_adjustment(
                state,
                attempt,
                "TAX_CREDIT_NOTE",
                invoice,
                baseline,
                refund.amount,
                amount,
                StripeRequest::post(
                    scope.clone(),
                    "/v1/credit_notes",
                    parameters,
                    format!("refund:{}:credit:{baseline}", refund.id),
                ),
            )
            .await?;
        }
    }
    Ok(tax_refunded)
}

async fn adjust_attempt(state: &AppState, id: &str) -> Result<(), ApiError> {
    let attempt = payment_attempt::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if attempt.source_type != SOURCE {
        return Ok(());
    }
    require_dispute_settlement_currency(&attempt.snapshot)?;
    let offer: Offer = serde_json::from_value(attempt.snapshot.clone())?;
    validate_recorded_route(&load_order(&state.db, &attempt.source_id).await?, &offer)?;
    if let Some(pending) = payment_adjustment::Entity::find()
        .filter(payment_adjustment::Column::AttemptId.eq(id))
        .filter(payment_adjustment::Column::Status.eq("PENDING"))
        .one(&state.db)
        .await?
    {
        if offer.platform_owned && pending.purpose != "TAX_CREDIT_NOTE" {
            return Err(error(
                "PAYMENT_BINDING_MISMATCH",
                "A platform sale cannot adjust seller balances",
            ));
        }
        finish_adjustment(state, &attempt, &pending).await?;
        return Err(error(
            "PAYMENT_ADJUSTMENT_PENDING",
            "Refreshing seller balances after an adjustment",
        ));
    }
    let charge_id = attempt.stripe_charge_id.as_ref().ok_or_else(|| {
        error(
            "PAYMENT_OPERATION_PENDING",
            "Charge details are still arriving",
        )
    })?;
    let scope = StripeScope::platform(&attempt.platform_account_id, attempt.livemode);
    let charge: Charge = serde_json::from_value(
        get(
            state,
            scope.clone(),
            format!("/v1/charges/{charge_id}"),
            json!({}),
        )
        .await?,
    )?;
    if charge.id != *charge_id
        || charge.amount != attempt.amount as u64
        || charge.payment_intent.as_ref().map(|id| id.id())
            != attempt.stripe_payment_intent_id.as_deref()
        || charge.currency != attempt.currency
        || charge.livemode != attempt.livemode
    {
        return Err(error("PAYMENT_BINDING_MISMATCH", "Charge details changed"));
    }
    if offer.platform_owned {
        if charge.application_fee_amount.is_some()
            || charge.application_fee.is_some()
            || charge.transfer.is_some()
            || attempt.connected_account_id.is_some()
            || attempt.application_fee_amount != 0
        {
            return Err(error(
                "PAYMENT_BINDING_MISMATCH",
                "A platform sale has connected-account movements",
            ));
        }
    }
    let tax = if offer.tax_mode == flow_like::hub::PaymentTaxMode::PlatformSupplier {
        attempt
            .snapshot
            .get("taxAmount")
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                error(
                    "PAYMENT_OPERATION_PENDING",
                    "The final tax amount is not available",
                )
            })?
    } else {
        0
    };
    let tax_refunded = if offer.tax_mode == flow_like::hub::PaymentTaxMode::PlatformSupplier {
        credit_refunds(state, &attempt).await?
    } else {
        0
    };
    if offer.platform_owned {
        mark_adjustments_ready(state, &attempt).await?;
        return Ok(());
    }
    let transfer = charge.transfer.as_ref().ok_or_else(|| {
        error(
            "PAYMENT_OPERATION_PENDING",
            "Seller transfer is still being created",
        )
    })?;
    let fee_id = charge.application_fee.as_ref().map(|fee| fee.id());
    let fee_refunded = match fee_id {
        Some(id) => required_i64(
            &get(
                state,
                scope.clone(),
                format!("/v1/application_fees/{id}"),
                json!({}),
            )
            .await?,
            "amount_refunded",
        )?,
        None if attempt.application_fee_amount == 0 => 0,
        None => {
            return Err(error(
                "PAYMENT_OPERATION_PENDING",
                "Platform fee is still being created",
            ));
        }
    };
    let disputed = attempt
        .snapshot
        .pointer("/dispute/outstanding")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let (mut recovery_target, retained_fee) = targets_with_refunded_tax(
        attempt.amount,
        tax,
        tax_refunded,
        attempt.refunded_amount,
        disputed,
        offer.fee_bps,
        offer.fee_basis == flow_like::hub::PaymentFeeBasis::NetOfTax,
    )?;
    let fee_target = attempt.application_fee_amount - retained_fee;
    recovery_target = recovery_target
        .checked_add((fee_refunded - fee_target).max(0))
        .ok_or_else(|| ApiError::internal("Fee recovery overflow"))?;
    if fee_target > fee_refunded {
        let fee_id = fee_id.ok_or_else(|| {
            error(
                "PAYMENT_OPERATION_PENDING",
                "Platform fee is still being created",
            )
        })?;
        let amount = fee_target - fee_refunded;
        return plan_adjustment(
            state,
            &attempt,
            "FEE_REFUND",
            fee_id,
            fee_refunded,
            fee_target,
            amount,
            StripeRequest::post(
                scope,
                format!("/v1/application_fees/{fee_id}/refunds"),
                json!({"amount":amount}),
                format!("attempt:{id}:fee:{fee_target}"),
            ),
        )
        .await;
    }
    let original = get(
        state,
        scope.clone(),
        format!("/v1/transfers/{}", transfer.id()),
        json!({}),
    )
    .await?;
    let original_amount = required_i64(&original, "amount")?;
    let original_reversed = required_i64(&original, "amount_reversed")?;
    let mut recovered = original_reversed;
    let mut candidates = vec![(
        transfer.id().to_owned(),
        original_amount - original_reversed,
        original_reversed,
    )];
    let restores = payment_adjustment::Entity::find()
        .filter(payment_adjustment::Column::AttemptId.eq(id))
        .filter(payment_adjustment::Column::Purpose.eq("SELLER_RESTORE"))
        .filter(payment_adjustment::Column::Status.eq("SUCCEEDED"))
        .all(&state.db)
        .await?;
    let restoration_generation = restores.len();
    for restore in restores {
        let transfer_id = restore.stripe_object_id.ok_or_else(|| {
            error(
                "PAYMENT_OPERATION_REVIEW",
                "A seller restoration has no transfer id",
            )
        })?;
        let record = get(
            state,
            scope.clone(),
            format!("/v1/transfers/{transfer_id}"),
            json!({}),
        )
        .await?;
        let amount = required_i64(&record, "amount")?;
        let reversed = required_i64(&record, "amount_reversed")?;
        recovered = recovered
            .checked_sub(amount - reversed)
            .ok_or_else(|| ApiError::internal("Recovery total overflow"))?;
        candidates.push((transfer_id, amount - reversed, reversed));
    }
    if recovered < recovery_target {
        let (transfer_id, available, baseline) = candidates
            .into_iter()
            .find(|(_, available, _)| *available > 0)
            .ok_or_else(|| {
                error(
                    "PAYMENT_SELLER_EXPOSURE",
                    "Seller recovery requires operator review",
                )
            })?;
        let amount = (recovery_target - recovered).min(available);
        let purpose = if attempt.refunded_amount == 0 && disputed == 0 {
            "TAX_WITHHOLD"
        } else {
            "SELLER_RECOVERY"
        };
        return plan_adjustment(
            state,
            &attempt,
            purpose,
            &transfer_id,
            baseline,
            recovery_target,
            amount,
            StripeRequest::post(
                scope,
                format!("/v1/transfers/{transfer_id}/reversals"),
                json!({"amount":amount}),
                format!("attempt:{id}:recover:{transfer_id}:{baseline}:{recovery_target}"),
            ),
        )
        .await;
    }
    if recovered > recovery_target {
        let amount = recovered - recovery_target;
        return plan_adjustment(state,&attempt,"SELLER_RESTORE",transfer.id(),recovered,recovery_target,amount,StripeRequest::post(scope,"/v1/transfers",json!({"amount":amount,"currency":attempt.currency,"destination":offer.account_id,"transfer_group":format!("mkt:{}",attempt.source_id)}),format!("attempt:{id}:restore:{restoration_generation}:{recovered}:{recovery_target}"))).await;
    }
    mark_adjustments_ready(state, &attempt).await
}

async fn mark_adjustments_ready(
    state: &AppState,
    attempt: &payment_attempt::Model,
) -> Result<(), ApiError> {
    state.db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "hydrationStatus"='READY',"nextCheckAt"=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1 AND revision=$4"#,vec![attempt.id.clone().into(),(now()+3_600_000).into(),now().into(),attempt.revision.into()])).await?;
    Ok(())
}

fn required_i64(value: &Value, field: &str) -> Result<i64, ApiError> {
    value
        .get(field)
        .and_then(Value::as_i64)
        .filter(|v| *v >= 0)
        .ok_or_else(|| {
            error(
                "PAYMENT_OPERATION_REVIEW",
                "Stripe returned incomplete financial details",
            )
        })
}

#[allow(clippy::too_many_arguments)]
async fn plan_adjustment(
    state: &AppState,
    attempt: &payment_attempt::Model,
    purpose: &str,
    parent: &str,
    baseline: i64,
    target: i64,
    amount: i64,
    request: StripeRequest,
) -> Result<(), ApiError> {
    let id = operations::identity(&request)?;
    state.transaction(|txn|{let (attempt,purpose,parent,request,id)=(attempt.clone(),purpose.to_owned(),parent.to_owned(),request.clone(),id.clone());Box::pin(async move{
        crate::db::coordination::coordinate(txn,"payments-charge",&[&attempt.id]).await?;
        let current=payment_attempt::Entity::find_by_id(&attempt.id).one(txn).await?.ok_or(ApiError::NOT_FOUND)?;
        if current.revision!=attempt.revision{return Err(error("PAYMENT_ADJUSTMENT_PENDING","Settlement changed while balances were retrieved"));}
        if payment_adjustment::Entity::find().filter(payment_adjustment::Column::AttemptId.eq(&attempt.id)).filter(payment_adjustment::Column::Status.eq("PENDING")).one(txn).await?.is_some(){return Err(error("PAYMENT_ADJUSTMENT_PENDING","Another adjustment is already running"));}
        let operation=operations::prepare(txn,"PAYMENT_ADJUSTMENT",&id,&purpose,&request).await?;
        txn.execute_raw(sql(r#"INSERT INTO "PaymentAdjustment" (id,"attemptId",purpose,"platformAccountId","scopeKey",livemode,"parentObjectId","operationId",amount,currency,"createdAt","updatedAt") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$11) ON CONFLICT DO NOTHING"#,vec![id.into(),attempt.id.into(),purpose.into(),attempt.platform_account_id.into(),attempt.scope_key.into(),attempt.livemode.into(),parent.into(),operation.into(),amount.into(),attempt.currency.into(),now().into()])).await?;Ok::<_,ApiError>(())
    })}).await?;
    let adjustment = payment_adjustment::Entity::find_by_id(&id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    finish_adjustment(state, attempt, &adjustment).await?;
    tracing::debug!(attempt_id=%attempt.id,baseline,target,"Seller adjustment submitted");
    Err(error(
        "PAYMENT_ADJUSTMENT_PENDING",
        "Refreshing seller balances after an adjustment",
    ))
}

async fn finish_adjustment(
    state: &AppState,
    attempt: &payment_attempt::Model,
    adjustment: &payment_adjustment::Model,
) -> Result<(), ApiError> {
    if adjustment.status == "SUCCEEDED" {
        return Ok(());
    }
    let result = operations::execute_prepared(state, &adjustment.operation_id).await?;
    let object = result
        .body
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                "PAYMENT_OPERATION_REVIEW",
                "The adjustment has no provider id",
            )
        })?
        .to_owned();
    let credit_tax = if adjustment.purpose == "TAX_CREDIT_NOTE" {
        Some(verify_credit_note(
            &result.body,
            attempt,
            adjustment.parent_object_id.as_deref().ok_or_else(|| {
                error(
                    "PAYMENT_TAX_REVIEW",
                    "The credit note has no recorded invoice",
                )
            })?,
            Some(adjustment.amount),
        )?)
    } else {
        None
    };
    state.transaction(|txn|{let (attempt,adjustment,object)=(attempt.clone(),adjustment.clone(),object.clone());Box::pin(async move{
        txn.execute_raw(sql(r#"UPDATE "PaymentAdjustment" SET status='SUCCEEDED',"stripeObjectId"=$2,"updatedAt"=$3 WHERE id=$1"#,vec![adjustment.id.into(),object.clone().into(),now().into()])).await?;
        if let Some(tax)=credit_tax{
            ledger(txn,&attempt,&object,"CREDIT_NOTE","TAX",-tax).await?;
            if let Some(perspective)=tax_liability_perspective(&attempt) {ledger(txn,&attempt,&object,"TAX_LIABILITY_RELEASE",perspective,tax).await?;}
            return Ok(());
        }
        let amount=if matches!(adjustment.purpose.as_str(),"SELLER_RESTORE"|"FEE_REFUND"){adjustment.amount}else{-adjustment.amount};
        ledger(txn,&attempt,&object,&adjustment.purpose,"SELLER",amount).await?;
        if adjustment.purpose=="FEE_REFUND"{ledger(txn,&attempt,&object,"FEE_REFUND","PLATFORM",-adjustment.amount).await?;}
        Ok::<_,ApiError>(())
    })}).await
}

async fn deliver_notice(state: &AppState, effect: &str, id: &str) -> Result<(), ApiError> {
    let (user_id, app_id, title, description) = if effect == "marketplace_comp" {
        let grant = crate::entity::access_grant::Entity::find_by_id(id)
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        (
            grant.user_id,
            grant.item_id,
            "App access granted".to_owned(),
            "The owner gave you complimentary access to this app.".to_owned(),
        )
    } else {
        let order = load_order(&state.db, id).await?;
        let offer: Offer = serde_json::from_value(order.snapshot)?;
        if effect == "marketplace_withdrawal" {
            (
                order.user_id,
                order.item_id,
                "Withdrawal received".to_owned(),
                format!(
                    "Your withdrawal for {} was received. Repayment is being processed. Order: {id}",
                    offer.title
                ),
            )
        } else {
            (
                order.user_id,
                order.item_id,
                format!("Purchase complete: {}", offer.title),
                format!("You now have access to {}. Order: {id}", offer.title),
            )
        }
    };
    let audit = crate::audit::AuditRecordInput::system(
        "payments",
        if effect == "marketplace_comp" {
            "membership.comp"
        } else if effect == "marketplace_withdrawal" {
            "marketplace.purchase.withdrawn"
        } else {
            "marketplace.purchase.paid"
        },
        "PaymentOrder",
        id,
    )
    .on_scope(&app_id)
    .with_details(json!({"userId":user_id}));
    crate::audit::record::write(&state.db, audit, crate::audit::WriteMode::Once).await?;
    if crate::entity::user::Entity::find_by_id(&user_id)
        .one(&state.db)
        .await?
        .is_none()
    {
        return Ok(());
    }
    crate::push_notifications::dispatch_notification_idempotent(
        state,
        &format!("{effect}:{id}"),
        crate::push_notifications::DispatchNotificationInput {
            user_id,
            app_id: None,
            title,
            description: Some(description),
            icon: Some("shopping-bag".into()),
            image: None,
            link: Some("/account/purchases".into()),
            notification_type: crate::entity::sea_orm_active_enums::NotificationType::System,
            source_run_id: None,
            source_node_id: None,
        },
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tax_session(tax: u64) -> CheckoutSession {
        serde_json::from_value(json!({
            "id":"cs_tax","livemode":false,"mode":"payment","payment_status":"paid",
            "automatic_tax":{"enabled":true,"status":"complete","liability":{"type":"self"}},
            "total_details":{"amount_tax":tax}
        }))
        .unwrap()
    }

    #[test]
    fn platform_tax_requires_complete_liability_and_totals() {
        let mode = flow_like::hub::PaymentTaxMode::PlatformSupplier;
        for tax in [0, 190, 1190] {
            assert_eq!(
                checkout_tax_amount(&mode, 1190, &tax_session(tax)).unwrap(),
                tax as i64
            );
        }
        assert!(checkout_tax_amount(&mode, 1190, &tax_session(1191)).is_err());
        assert!(checkout_tax_amount(&mode, 1190, &tax_session(u64::MAX)).is_err());
        let mut session = tax_session(190);
        for status in [None, Some("failed"), Some("requires_location_inputs")] {
            session.automatic_tax.as_mut().unwrap().status = status.map(str::to_owned);
            assert!(checkout_tax_amount(&mode, 1190, &session).is_err());
        }
        session = tax_session(190);
        session.automatic_tax.as_mut().unwrap().enabled = false;
        assert!(checkout_tax_amount(&mode, 1190, &session).is_err());
        session = tax_session(190);
        session
            .automatic_tax
            .as_mut()
            .unwrap()
            .liability
            .as_mut()
            .unwrap()
            .liability_type = "account".into();
        assert!(checkout_tax_amount(&mode, 1190, &session).is_err());
        session = tax_session(190);
        session.automatic_tax = None;
        assert!(checkout_tax_amount(&mode, 1190, &session).is_err());
        session = tax_session(190);
        session.total_details = None;
        assert!(checkout_tax_amount(&mode, 1190, &session).is_err());
    }

    #[test]
    fn credit_note_tax_preserves_stripe_rounding_and_rejects_inconsistent_totals() {
        assert_eq!(
            credit_note_tax(&json!({"amount":595,"total_excluding_tax":500})).unwrap(),
            95
        );
        assert_eq!(
            credit_note_tax(&json!({"amount":1,"total_excluding_tax":1})).unwrap(),
            0
        );
        assert_eq!(
            credit_note_tax(&json!({"amount":1190,"total_excluding_tax":1000})).unwrap(),
            190
        );
        for note in [
            json!({"amount":100,"total_excluding_tax":101}),
            json!({"amount":100,"total_excluding_tax":-1}),
            json!({"amount":-1,"total_excluding_tax":-1}),
            json!({"amount":100,"total_excluding_tax":null}),
        ] {
            assert!(credit_note_tax(&note).is_err());
        }
    }
    #[test]
    fn net_tax_refund_and_dispute_targets_do_not_double_recover() {
        assert_eq!(
            targets(11900, 1900, 0, 0, 1000, true).unwrap(),
            (1900, 1000)
        );
        assert_eq!(
            targets(11900, 1900, 11900, 0, 1000, true).unwrap(),
            (11900, 0)
        );
        assert_eq!(
            targets(11900, 1900, 0, 11900, 1000, true).unwrap(),
            (11900, 1000)
        );
        assert_eq!(
            targets(11900, 1900, 5950, 11900, 1000, true).unwrap(),
            (11900, 500)
        );
        assert_eq!(
            targets(11900, 1900, 0, 0, 1000, false).unwrap(),
            (1900, 1190)
        );
        assert!(targets(100, 101, 0, 0, 1000, true).is_err());
    }
    #[test]
    fn partial_refunds_keep_rounding_in_minor_units() {
        assert_eq!(targets(300, 0, 299, 0, 1000, false).unwrap(), (299, 1));
        assert_eq!(
            targets(11900, 1900, 5950, 0, 1000, true).unwrap(),
            (6900, 500)
        );
    }
}

#[cfg(test)]
#[path = "marketplace_tests.rs"]
mod database_tests;
