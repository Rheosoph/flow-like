use super::*;
use crate::stripe_connect::{
    EventEnvelope, ExpectedPayment, StripeGateway, VerifiedPayment,
    types::{
        ApplicationFee, BalanceTransaction, Charge, CheckoutSession, Dispute, FeeRefund,
        PaymentIntent, Refund, StripeList,
    },
};

fn stripe_scope(row: &payment_request::Model) -> Result<StripeScope, ApiError> {
    request_stripe_scope(row)
}
async fn retrieve<T: serde::de::DeserializeOwned>(
    state: &AppState,
    row: &payment_request::Model,
    path: String,
    parameters: Value,
) -> Result<T, ApiError> {
    super::super::gateway_for_version(state, crate::stripe_connect::STRIPE_API_VERSION)
        .await?
        .execute(&StripeRequest::get(stripe_scope(row)?, path, parameters))
        .await
        .map_err(stripe_error)?
        .decode()
        .map_err(stripe_error)
}

pub async fn reconcile_request(state: &AppState, id: &str) -> Result<(), ApiError> {
    let claimed = state
        .db
        .execute_raw(sql(
            r#"UPDATE "PaymentRequest" SET "nextCheckAt"=$2 WHERE id=$1 AND "nextCheckAt"<=$3"#,
            vec![id.into(), (now() + 60_000).into(), now().into()],
        ))
        .await?;
    if claimed.rows_affected() != 1 {
        return Ok(());
    }
    let row = load(state, id).await?;
    let pending = matches!(
        row.status.as_str(),
        "CREATED" | "OPENING" | "OPEN" | "PROCESSING" | "CANCEL_PENDING"
    );
    if pending
        && (row.expires_at <= now()
            || run_record(&state.db, &row.run_id, &row.app_id, &row.payer_user_id)
                .await?
                .is_none())
    {
        cancel(
            state,
            id,
            if row.expires_at <= now() {
                "DEADLINE_EXPIRED"
            } else {
                "RUN_ENDED"
            },
        )
        .await?;
    }
    let row = load(state, id).await?;
    let Some(mut attempt) = attempt(state, id).await? else {
        if row.cancel_requested {
            close_request(state, &row).await?;
        }
        return Ok(());
    };
    if attempt.stripe_session_id.is_none() {
        if crate::entity::stripe_operation::Entity::find_by_id(&attempt.operation_id)
            .one(&state.db)
            .await?
            .is_some_and(|operation| operation.status == "FAILED")
        {
            fail_request(state, &row.id, &attempt.id, "PAYMENT_CHECKOUT_FAILED").await?;
            return Ok(());
        }
        let response = operations::execute_prepared(state, &attempt.operation_id).await?;
        let session: CheckoutSession = response.decode().map_err(stripe_error)?;
        if !crate::stripe_connect::request::valid_id(&session.id, "cs_")
            || session.mode != "payment"
            || session.livemode != row.livemode
            || session.amount_total != Some(row.amount as u64)
            || session.currency.as_deref() != Some(row.currency.as_str())
        {
            return Err(error(
                "PAYMENT_BINDING_MISMATCH",
                "The provider session does not match the payment",
            ));
        }
        verify_tax_configuration(&row, &session)?;
        state.db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "stripeSessionId"=$2,"checkoutUrl"=$3,status='OPEN',"updatedAt"=$4,revision=revision+1 WHERE id=$1 AND ("stripeSessionId" IS NULL OR "stripeSessionId"=$2)"#,vec![attempt.id.clone().into(),session.id.clone().into(),session.url.into(),now().into()])).await?;
        state.db.execute_raw(sql(r#"UPDATE "PaymentRequest" SET status='OPEN',"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND status='OPENING' AND "cancelRequested"=false"#,vec![id.into(),now().into()])).await?;
        attempt.stripe_session_id = Some(session.id);
    }
    let session_id = attempt
        .stripe_session_id
        .as_deref()
        .ok_or_else(|| ApiError::internal("Payment session has no identifier"))?;
    let mut session: CheckoutSession = retrieve(
        state,
        &row,
        format!("/v1/checkout/sessions/{session_id}"),
        json!({}),
    )
    .await?;
    if row.cancel_requested && session.status.as_deref() == Some("open") {
        let expire = StripeRequest::post(
            stripe_scope(&row)?,
            format!("/v1/checkout/sessions/{session_id}/expire"),
            json!({}),
            format!("request:{id}:expire:{session_id}"),
        );
        match operations::execute(state, SOURCE, id, "checkout_expire", expire).await {
            Ok(response) => session = response.decode().map_err(stripe_error)?,
            Err(_) => {
                session = retrieve(
                    state,
                    &row,
                    format!("/v1/checkout/sessions/{session_id}"),
                    json!({}),
                )
                .await?
            }
        }
    }
    let intent_id = session
        .payment_intent
        .as_ref()
        .map(|value| value.id())
        .or(attempt.stripe_payment_intent_id.as_deref());
    let intent: Option<PaymentIntent> = match intent_id {
        Some(intent_id) => Some(
            retrieve(
                state,
                &row,
                format!("/v1/payment_intents/{intent_id}"),
                json!({}),
            )
            .await?,
        ),
        None => None,
    };
    if session.status.as_deref() == Some("expired") {
        if intent.as_ref().is_some_and(|intent| {
            !intent_failed_without_capture(&intent.status, intent.amount_received)
        }) {
            state
                .db
                .execute_raw(sql(
                    r#"UPDATE "PaymentRequest" SET "nextCheckAt"=$2 WHERE id=$1"#,
                    vec![id.into(), (now() + 5000).into()],
                ))
                .await?;
            return Ok(());
        }
        state.db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status='EXPIRED',"checkoutUrl"=NULL,"nextCheckAt"=NULL,"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND "capturedAmount"=0"#,vec![attempt.id.clone().into(),now().into()])).await?;
        close_request(state, &row).await?;
        return Ok(());
    }
    if let Some(intent) = intent.filter(|_| session.status.as_deref() == Some("complete")) {
        let expected = ExpectedPayment {
            scope: stripe_scope(&row)?,
            session_id: session_id.into(),
            payment_intent_id: attempt.stripe_payment_intent_id.clone(),
            charge_id: attempt.stripe_charge_id.clone(),
            amount: row.amount as u64,
            currency: row.currency.clone(),
            application_fee_amount: row.application_fee_amount as u64,
            destination_account_id: None,
        };
        if intent_failed_without_capture(&intent.status, intent.amount_received)
            && session.payment_status == "unpaid"
        {
            expected
                .verify(&stripe_scope(&row)?, &session, &intent, None)
                .map_err(|_| {
                    error(
                        "PAYMENT_BINDING_MISMATCH",
                        "The failed payment does not match the request",
                    )
                })?;
            fail_request(state, &row.id, &attempt.id, "PAYMENT_FAILED").await?;
            return Ok(());
        }
        let charge: Option<Charge> = match intent.latest_charge.as_ref() {
            Some(charge) => Some(
                retrieve(
                    state,
                    &row,
                    format!("/v1/charges/{}", charge.id()),
                    json!({}),
                )
                .await?,
            ),
            None => None,
        };
        let observation = expected
            .verify(&stripe_scope(&row)?, &session, &intent, charge.as_ref())
            .map_err(|_| {
                error(
                    "PAYMENT_BINDING_MISMATCH",
                    "The provider payment does not match the request",
                )
            })?;
        if observation == VerifiedPayment::Paid {
            let charge = charge.ok_or_else(|| {
                error(
                    "PAYMENT_HYDRATING",
                    "The captured charge is not available yet",
                )
            })?;
            settle_paid(state, &row, &attempt, &session, &intent, &charge).await?;
            return Ok(());
        }
        if session.status.as_deref() == Some("complete") {
            state.db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status='PROCESSING',"stripePaymentIntentId"=$2,"checkoutUrl"=NULL,"updatedAt"=$3,revision=revision+1 WHERE id=$1 AND "capturedAmount"=0"#,vec![attempt.id.clone().into(),intent.id.into(),now().into()])).await?;
            state.db.execute_raw(sql(r#"UPDATE "PaymentRequest" SET status='PROCESSING',"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND status IN ('OPEN','OPENING') AND "cancelRequested"=false"#,vec![id.into(),now().into()])).await?;
        }
    }
    state
        .db
        .execute_raw(sql(
            r#"UPDATE "PaymentRequest" SET "nextCheckAt"=$2 WHERE id=$1"#,
            vec![id.into(), (now() + 5000).into()],
        ))
        .await?;
    Ok(())
}

pub(super) const ORPHAN_REFUND_REASON: &str = "requested_by_customer";

pub(super) fn intent_failed_without_capture(status: &str, amount_received: u64) -> bool {
    amount_received == 0 && matches!(status, "canceled" | "requires_payment_method")
}

async fn fail_request(
    state: &AppState,
    request_id: &str,
    attempt_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    state
        .transaction(|txn| {
            let (request_id, attempt_id, reason) = (
                request_id.to_owned(),
                attempt_id.to_owned(),
                reason.to_owned(),
            );
            Box::pin(async move { fail_request_txn(txn, &request_id, &attempt_id, &reason).await })
        })
        .await
}

pub(super) async fn fail_request_txn(
    txn: &sea_orm::DatabaseTransaction,
    request_id: &str,
    attempt_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    crate::db::coordination::coordinate(txn, "payment-request", &[request_id]).await?;
    crate::db::coordination::coordinate(txn, "payments-charge", &[attempt_id]).await?;
    let attempt = payment_attempt::Entity::find_by_id(attempt_id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if attempt.source_type != SOURCE
        || attempt.source_id != request_id
        || attempt.captured_amount != 0
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "Only an uncaptured request attempt can fail",
        ));
    }
    let request = payment_request::Entity::find_by_id(request_id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if request.accepted_attempt_id.is_some() {
        return Ok(());
    }
    txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET status='FAILED',"checkoutUrl"=NULL,"nextCheckAt"=NULL,"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND "capturedAmount"=0"#, vec![attempt_id.into(), now().into()])).await?;
    txn.execute_raw(sql(r#"UPDATE "PaymentRequest" SET status=CASE WHEN "expiresAt"<=$2 THEN 'EXPIRED' WHEN "cancelRequested" THEN 'CANCELED' ELSE 'FAILED' END,reason=COALESCE(reason,$3),"nextCheckAt"=$2,"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND "acceptedAttemptId" IS NULL"#, vec![request_id.into(), now().into(), reason.into()])).await?;
    release_limit(txn, request_id).await
}

async fn release_limit(
    txn: &sea_orm::DatabaseTransaction,
    request_id: &str,
) -> Result<(), ApiError> {
    let reservations = txn.query_all_raw(sql(r#"UPDATE "PaymentLimitReservation" SET status='RELEASED',"updatedAt"=$2 WHERE id=$1 AND status='RESERVED' RETURNING "counterKey",amount"#,vec![request_id.into(),now().into()])).await?;
    for reservation in reservations {
        txn.execute_raw(sql(r#"UPDATE "PaymentLimitCounter" SET amount=GREATEST(0,amount-$2),count=GREATEST(0,count-1),revision=revision+1,"updatedAt"=$3 WHERE key=$1"#,vec![reservation.try_get::<String>("","counterKey")?.into(),reservation.try_get::<i64>("","amount")?.into(),now().into()])).await?;
    }
    Ok(())
}

fn classify_request_payment(
    request: &payment_request::Model,
    attempt_id: &str,
    running: bool,
    admitted: bool,
    observed_at: i64,
) -> domain::PaidDisposition {
    domain::classify_paid(
        request.accepted_attempt_id.as_deref(),
        attempt_id,
        request.cancel_requested || !admitted,
        !running || request.expires_at <= observed_at,
    )
}

async fn settle_paid(
    state: &AppState,
    row: &payment_request::Model,
    attempt: &payment_attempt::Model,
    session: &CheckoutSession,
    intent: &PaymentIntent,
    charge: &Charge,
) -> Result<(), ApiError> {
    state
        .transaction(|txn| {
            let (row, attempt, session, intent, charge) = (
                row.clone(),
                attempt.clone(),
                session.clone(),
                intent.clone(),
                charge.clone(),
            );
            Box::pin(async move {
                settle_paid_txn(txn, &row, &attempt, &session, &intent, &charge).await
            })
        })
        .await
}

fn verify_tax_configuration(
    row: &payment_request::Model,
    session: &CheckoutSession,
) -> Result<(), ApiError> {
    if !automatic_tax(row) {
        return Ok(());
    }
    product_tax_code(row)?;
    if !session.automatic_tax.as_ref().is_some_and(|tax| {
        tax.enabled
            && tax.liability.as_ref().is_some_and(|liability| {
                liability.liability_type == "self" && liability.account.is_none()
            })
    }) {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "The payment tax liability does not match its recipient",
        ));
    }
    Ok(())
}

fn tax_observation(
    row: &payment_request::Model,
    session: &CheckoutSession,
) -> Result<Option<(i64, Option<String>)>, ApiError> {
    verify_tax_configuration(row, session)?;
    if !automatic_tax(row) {
        return Ok(None);
    }
    let details = session.total_details.as_ref().ok_or_else(|| {
        error(
            "PAYMENT_TAX_REVIEW",
            "The completed payment has no tax calculation",
        )
    })?;
    if session
        .automatic_tax
        .as_ref()
        .and_then(|tax| tax.status.as_deref())
        != Some("complete")
        || session.amount_total != Some(row.amount as u64)
        || details.amount_tax > row.amount as u64
        || details.amount_discount != 0
        || details.amount_shipping != 0
    {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "The completed tax calculation does not match the approved payment",
        ));
    }
    let invoice = session
        .invoice
        .as_ref()
        .map(|invoice| invoice.id().to_owned());
    if invoice
        .as_deref()
        .is_some_and(|id| !crate::stripe_connect::request::valid_id(id, "in_"))
    {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "The payment invoice identifier is invalid",
        ));
    }
    Ok(Some((details.amount_tax as i64, invoice)))
}

fn payment_perspective(row: &payment_request::Model) -> &'static str {
    if platform_owned(row) {
        "PLATFORM"
    } else {
        "SELLER"
    }
}

fn awaiting_stripe_objects(row: &payment_request::Model, charge: &Charge) -> bool {
    charge.balance_transaction.is_none()
        || (row.application_fee_amount > 0 && charge.application_fee.is_none())
}

async fn settlement_admitted<C: ConnectionTrait>(
    db: &C,
    row: &payment_request::Model,
) -> Result<bool, ApiError> {
    let current_admin = match accounts::is_platform_admin(db, &row.payee_user_id).await {
        Ok(current_admin) => current_admin,
        Err(error) if error.status() == axum::http::StatusCode::NOT_FOUND => return Ok(false),
        Err(error) => return Err(error),
    };
    if current_admin != platform_owned(row) {
        return Ok(false);
    }
    let settings_valid = db.query_one_raw(sql(r#"SELECT a.id FROM "App" a JOIN "Membership" m ON m."appId"=a.id AND m."roleId"=a."ownerRoleId" JOIN "AppPaymentSettings" s ON s."appId"=a.id WHERE a.id=$1 AND m."userId"=$2 AND s."ownerUserId"=$2 AND s."paymentsEnabled"=true AND s.revision=$3 AND s."adminBlockedAt" IS NULL AND NOT EXISTS (SELECT 1 FROM "PaymentsBlock" p WHERE p."userId"=$2) AND NOT EXISTS (SELECT 1 FROM "Membership" other WHERE other."appId"=a.id AND other."roleId"=a."ownerRoleId" AND other."userId"<>$2) LIMIT 1"#, vec![row.app_id.clone().into(),row.payee_user_id.clone().into(),row.settings_revision.into()])).await?.is_some();
    if !settings_valid || platform_owned(row) {
        return Ok(settings_valid);
    }
    Ok(db.query_one_raw(sql(r#"SELECT c.id FROM "ConnectedAccount" c JOIN "PaymentAccountBinding" b ON b."activeAccountId"=c.id AND b."userId"=$2 AND b.livemode=$3 WHERE c."stripeAccountId"=$1 AND c."userId"=$2 AND c.livemode=$3 AND c."platformAccountId"=$4 AND c."retiredAt" IS NULL AND c."canAcceptPayments"=true"#, vec![row.connected_account_id.clone().into(),row.payee_user_id.clone().into(),row.livemode.into(),row.platform_account_id.clone().into()])).await?.is_some())
}

pub(super) async fn settle_paid_txn(
    txn: &sea_orm::DatabaseTransaction,
    row: &payment_request::Model,
    attempt: &payment_attempt::Model,
    session: &CheckoutSession,
    intent: &PaymentIntent,
    charge: &Charge,
) -> Result<(), ApiError> {
    request_stripe_scope(row)?;
    let tax = tax_observation(row, session)?;
    crate::db::coordination::coordinate(txn, "payments-owner", &[&row.payee_user_id]).await?;
    crate::db::coordination::coordinate(txn, "payments-app", &[&row.app_id]).await?;
    crate::db::coordination::coordinate(txn, "payment-request", &[&row.id]).await?;
    crate::db::coordination::coordinate(txn, "payments-charge", &[&attempt.id]).await?;
    let current_attempt = payment_attempt::Entity::find_by_id(&attempt.id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let mut snapshot = current_attempt.snapshot;
    if let Some((amount, invoice)) = &tax {
        if current_attempt.captured_amount > 0 && snapshot["taxAmount"].as_i64() != Some(*amount) {
            return Err(error(
                "PAYMENT_TAX_REVIEW",
                "The captured payment tax amount changed",
            ));
        }
        if let (Some(previous), Some(observed)) =
            (snapshot["invoiceId"].as_str(), invoice.as_deref())
            && previous != observed
        {
            return Err(error(
                "PAYMENT_BINDING_MISMATCH",
                "The payment invoice changed",
            ));
        }
        snapshot["taxAmount"] = json!(amount);
        snapshot["automaticTaxStatus"] = json!("complete");
        if let Some(invoice) = invoice {
            snapshot["invoiceId"] = json!(invoice);
        }
    }
    snapshot["receiptUrl"] = json!(charge.receipt_url);
    snapshot["paymentMethod"] = charge.payment_method_details["type"].clone();
    snapshot["wallet"] = charge.payment_method_details["card"]["wallet"]["type"].clone();
    txn.execute_raw(sql(
        r#"UPDATE "PaymentAttempt" SET snapshot=$2 WHERE id=$1"#,
        vec![attempt.id.clone().into(), snapshot.into()],
    ))
    .await?;
    let current = payment_request::Entity::find_by_id(&row.id)
        .one(txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let running = run_record(txn, &row.run_id, &row.app_id, &row.payer_user_id)
        .await?
        .is_some();
    let admitted = settlement_admitted(txn, row).await?;
    let disposition = classify_request_payment(&current, &attempt.id, running, admitted, now());
    let orphan = disposition != domain::PaidDisposition::Accept;
    txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "stripePaymentIntentId"=$2,"stripeChargeId"=$3,"capturedAmount"=$4,status='PAID',orphaned=$5,"checkoutUrl"=NULL,"hydrationStatus"=$6,"updatedAt"=$7,revision=revision+1 WHERE id=$1 AND ("stripeChargeId" IS NULL OR "stripeChargeId"=$3)"#,vec![attempt.id.clone().into(),intent.id.clone().into(),charge.id.clone().into(),row.amount.into(),orphan.into(),if awaiting_stripe_objects(row, charge){Some("AWAITING_STRIPE_OBJECTS")}else{None}.map(str::to_owned).into(),now().into()])).await?;
    if !orphan {
        txn.execute_raw(sql(r#"UPDATE "PaymentRequest" SET status='PAID',reason=NULL,"acceptedAttemptId"=$2,"nextCheckAt"=$3,"updatedAt"=$3,revision=revision+1 WHERE id=$1 AND ("acceptedAttemptId" IS NULL OR "acceptedAttemptId"=$2)"#,vec![row.id.clone().into(),attempt.id.clone().into(),now().into()])).await?;
    } else {
        txn.execute_raw(sql(r#"UPDATE "PaymentRequest" SET status=CASE WHEN "expiresAt"<=$2 THEN 'EXPIRED' ELSE 'CANCELED' END,"cancelRequested"=true,reason=COALESCE(reason,'LATE_PAYMENT'),"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND "acceptedAttemptId" IS NULL"#,vec![row.id.clone().into(),now().into()])).await?;
        outbox::enqueue(
            txn,
            &format!("request:{}:orphan:{}", row.id, attempt.id),
            "node_orphan_refund",
            SOURCE,
            &attempt.id,
            json!({}),
        )
        .await?;
    }
    let ledger_id = blake3::hash(
        format!(
            "{}:{}:{}:{}:charge",
            row.platform_account_id, attempt.scope_key, row.livemode, charge.id
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    txn.execute_raw(sql(r#"INSERT INTO "PaymentLedgerEntry" (id,"attemptId","sourceType","sourceId","platformAccountId","scopeKey",livemode,perspective,kind,amount,currency,"payeeUserId","payerUserId","appId","stripeObjectId","stripeBalanceTransactionId","occurredAt","createdAt") VALUES ($1,$2,'REQUEST',$3,$4,$5,$6,$15,'CHARGE',$7,$8,$9,$10,$11,$12,$13,$14,$14) ON CONFLICT DO NOTHING"#,vec![ledger_id.into(),attempt.id.clone().into(),row.id.clone().into(),row.platform_account_id.clone().into(),attempt.scope_key.clone().into(),row.livemode.into(),row.amount.into(),row.currency.clone().into(),row.payee_user_id.clone().into(),row.payer_user_id.clone().into(),row.app_id.clone().into(),charge.id.clone().into(),charge.balance_transaction.as_ref().map(|value|value.id().to_owned()).into(),occurred_ms(charge.created).into(),payment_perspective(row).into()])).await?;
    if let Some((tax_amount, _)) = tax {
        for (kind, perspective, amount) in [
            ("TAX_ASSESSED", "TAX", tax_amount),
            ("TAX_LIABILITY", payment_perspective(row), -tax_amount),
        ] {
            let id = blake3::hash(
                format!(
                    "{}:{}:{}:{}:{kind}:{perspective}",
                    row.platform_account_id, attempt.scope_key, row.livemode, charge.id
                )
                .as_bytes(),
            )
            .to_hex()
            .to_string();
            txn.execute_raw(sql(r#"INSERT INTO "PaymentLedgerEntry" (id,"attemptId","sourceType","sourceId","platformAccountId","scopeKey",livemode,perspective,kind,amount,currency,"payeeUserId","payerUserId","appId","stripeObjectId","occurredAt","createdAt") VALUES ($1,$2,'REQUEST',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$15) ON CONFLICT DO NOTHING"#, vec![id.into(),attempt.id.clone().into(),row.id.clone().into(),row.platform_account_id.clone().into(),attempt.scope_key.clone().into(),row.livemode.into(),perspective.into(),kind.into(),amount.into(),row.currency.clone().into(),row.payee_user_id.clone().into(),row.payer_user_id.clone().into(),row.app_id.clone().into(),charge.id.clone().into(),occurred_ms(charge.created).into()])).await?;
        }
    }
    txn.execute_raw(sql(r#"UPDATE "PaymentLimitReservation" SET status='CONSUMED',"updatedAt"=$2 WHERE id=$1 AND status='RESERVED'"#,vec![row.id.clone().into(),now().into()])).await?;
    Ok(())
}
async fn close_request(state: &AppState, row: &payment_request::Model) -> Result<(), ApiError> {
    state.transaction(|txn|{let row=row.clone();Box::pin(async move{
        crate::db::coordination::coordinate(txn,"payment-request",&[&row.id]).await?;
        txn.execute_raw(sql(r#"UPDATE "PaymentRequest" SET status=CASE WHEN "expiresAt"<=$2 THEN 'EXPIRED' ELSE 'CANCELED' END,"cancelRequested"=true,"nextCheckAt"=$3,"updatedAt"=$2,revision=revision+1 WHERE id=$1 AND "acceptedAttemptId" IS NULL AND status IN ('CREATED','OPENING','OPEN','PROCESSING','CANCEL_PENDING')"#,vec![row.id.clone().into(),now().into(),(now()+60_000).into()])).await?;
        // Only a never-opened or confirmed expired session releases its capacity.
        release_limit(txn, &row.id).await?;
        Ok::<_,ApiError>(())
    })}).await
}

pub async fn handle_event(state: &AppState, event: &EventEnvelope) -> Result<bool, ApiError> {
    if event.account.is_none() && event.event_type.starts_with("application_fee.") {
        let id = if event.event_type.starts_with("application_fee.refund.") {
            event.data.object["fee"].as_str()
        } else {
            event.data.object["id"].as_str()
        }
        .ok_or_else(|| ApiError::bad_request("Missing application fee identifier"))?;
        return observe_application_fee(state, id, None).await;
    }
    let configured = super::super::connect_scope(state).await?;
    if event.livemode != configured.livemode {
        return Err(error(
            "PAYMENT_SCOPE_MISMATCH",
            "The event belongs to another payment mode",
        ));
    }
    let object_id = event.data.object["id"].as_str().unwrap_or("");
    let rows = event_attempts(&state.db, event, &configured.platform_account_id).await?;
    if rows.is_empty() {
        return Ok(false);
    };
    for row in rows {
        let source_id: String = row.try_get("", "sourceId")?;
        let attempt_id: String = row.try_get("", "id")?;
        let request = load(state, &source_id).await?;
        request_stripe_scope(&request)?;
        if request.connected_account_id != event.account
            || request.platform_account_id != configured.platform_account_id
            || request.livemode != configured.livemode
        {
            return Err(error(
                "PAYMENT_SCOPE_MISMATCH",
                "The event belongs to another payment scope",
            ));
        }
        if event.event_type.starts_with("charge.dispute.") {
            observe_dispute(state, &request, &attempt_id, object_id).await?;
        }
        if event.event_type.starts_with("refund.") {
            let refund_id = event.data.object["id"]
                .as_str()
                .ok_or_else(|| ApiError::bad_request("Missing refund identifier"))?;
            let refund: Refund = retrieve(
                state,
                &request,
                format!("/v1/refunds/{refund_id}"),
                json!({}),
            )
            .await?;
            super::super::marketplace::observe_refund(state, &attempt_id, &refund).await?;
        }
        reconcile_request(state, &source_id).await?;
    }
    Ok(true)
}
pub(super) async fn event_attempts<C: ConnectionTrait>(
    db: &C,
    event: &EventEnvelope,
    platform_account_id: &str,
) -> Result<Vec<sea_orm::QueryResult>, ApiError> {
    let object_id = event.data.object["id"].as_str().unwrap_or("");
    let metadata_id = event.data.object["metadata"]["flowlike_attempt"]
        .as_str()
        .unwrap_or("");
    let rows=db.query_all_raw(sql(r#"SELECT "sourceId",id FROM "PaymentAttempt" WHERE "sourceType"='REQUEST' AND "connectedAccountId" IS NOT DISTINCT FROM $1 AND livemode=$2 AND "platformAccountId"=$6 AND ("stripeSessionId"=$3 OR "stripePaymentIntentId"=$3 OR "stripeChargeId"=$3 OR id=$4 OR "stripeChargeId"=$5) LIMIT 2"#,vec![event.account.clone().into(),event.livemode.into(),object_id.into(),metadata_id.into(),event.data.object["charge"].as_str().unwrap_or("").into(),platform_account_id.into()])).await?;
    Ok(rows)
}

async fn observe_dispute(
    state: &AppState,
    row: &payment_request::Model,
    attempt_id: &str,
    id: &str,
) -> Result<(), ApiError> {
    let attempt = payment_attempt::Entity::find_by_id(attempt_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let dispute: Dispute = retrieve(state, row, format!("/v1/disputes/{id}"), json!({})).await?;
    if attempt.stripe_charge_id.as_deref() != Some(dispute.charge.id())
        || dispute.currency != attempt.currency
        || dispute.livemode != attempt.livemode
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "Dispute does not belong to the payment",
        ));
    }
    state.transaction(|txn|{let (attempt,dispute,perspective)=(attempt.clone(),dispute.clone(),payment_perspective(row));Box::pin(async move{
        crate::db::coordination::coordinate(txn,"payments-charge",&[&attempt.id]).await?;
        let current=payment_attempt::Entity::find_by_id(&attempt.id).one(txn).await?.ok_or(ApiError::NOT_FOUND)?;
        if current.snapshot["dispute"]["status"].as_str().is_some_and(|s|matches!(s,"won"|"lost"|"warning_closed") && s!=dispute.status){return Ok::<_,ApiError>(());}
        let mut snapshot=current.snapshot;
        snapshot["dispute"]=json!({"id":dispute.id,"status":dispute.status,"evidence":dispute.evidence_details});
        txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET snapshot=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1"#,vec![attempt.id.clone().into(),snapshot.into(),now().into()])).await?;
        for movement in &dispute.balance_transactions {
            let ledger=blake3::hash(format!("{}:{}:{}:{}:dispute",attempt.platform_account_id,attempt.scope_key,attempt.livemode,movement.id).as_bytes()).to_hex().to_string();
            txn.execute_raw(sql(r#"INSERT INTO "PaymentLedgerEntry" (id,"attemptId","sourceType","sourceId","platformAccountId","scopeKey",livemode,perspective,kind,amount,currency,"payeeUserId","payerUserId","appId","stripeObjectId","stripeBalanceTransactionId","occurredAt","createdAt") VALUES ($1,$2,'REQUEST',$3,$4,$5,$6,$14,'DISPUTE_CASH',$7,$8,$9,$10,$11,$12,$12,$13,$13) ON CONFLICT DO NOTHING"#,vec![ledger.into(),attempt.id.clone().into(),attempt.source_id.clone().into(),attempt.platform_account_id.clone().into(),attempt.scope_key.clone().into(),attempt.livemode.into(),movement.amount.into(),movement.currency.clone().into(),attempt.payee_user_id.clone().into(),attempt.payer_user_id.clone().into(),attempt.app_id.clone().into(),movement.id.clone().into(),occurred_ms(movement.created).into(),perspective.into()])).await?;
        }
        outbox::enqueue(txn,&format!("node-dispute:{}:{}",dispute.id,dispute.status),"PAYMENT_AUDIT",SOURCE,&attempt.id,json!({"action":"payments.dispute.updated","sellerUserId":attempt.payee_user_id,"stripeAccountId":attempt.connected_account_id,"disputeId":dispute.id,"status":dispute.status,"evidence":dispute.evidence_details})).await?;
        Ok::<_,ApiError>(())
    })}).await
}
async fn platform_get<T: serde::de::DeserializeOwned>(
    state: &AppState,
    path: String,
    parameters: Value,
) -> Result<T, ApiError> {
    let scope = super::super::connect_scope(state).await?;
    super::super::gateway_for_version(state, crate::stripe_connect::STRIPE_API_VERSION)
        .await?
        .execute(&StripeRequest::get(scope, path, parameters))
        .await
        .map_err(stripe_error)?
        .decode()
        .map_err(stripe_error)
}
fn occurred_ms(created: u64) -> i64 {
    if created == 0 {
        now()
    } else {
        created.saturating_mul(1000).min(i64::MAX as u64) as i64
    }
}
async fn fee_journal(
    state: &AppState,
    attempt: &payment_attempt::Model,
    object: &str,
    kind: &str,
    amount: i64,
    balance: Option<&str>,
    occurred: i64,
) -> Result<(), ApiError> {
    for (perspective, scope, amount) in [
        ("PLATFORM", "platform", amount),
        ("SELLER", attempt.scope_key.as_str(), -amount),
    ] {
        let id = blake3::hash(
            format!(
                "{}:{}:{scope}:{object}:{perspective}:{kind}",
                attempt.platform_account_id, attempt.livemode
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        state.db.execute_raw(sql(r#"INSERT INTO "PaymentLedgerEntry" (id,"attemptId","sourceType","sourceId","platformAccountId","scopeKey",livemode,perspective,kind,amount,currency,"payeeUserId","payerUserId","appId","stripeObjectId","stripeBalanceTransactionId","occurredAt","createdAt") VALUES ($1,$2,'REQUEST',$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$16) ON CONFLICT DO NOTHING"#,vec![id.into(),attempt.id.clone().into(),attempt.source_id.clone().into(),attempt.platform_account_id.clone().into(),scope.into(),attempt.livemode.into(),perspective.into(),kind.into(),amount.into(),attempt.currency.clone().into(),attempt.payee_user_id.clone().into(),attempt.payer_user_id.clone().into(),attempt.app_id.clone().into(),object.into(),balance.map(str::to_owned).into(),occurred.into()])).await?;
    }
    Ok(())
}
async fn observe_application_fee(
    state: &AppState,
    id: &str,
    expected: Option<&payment_attempt::Model>,
) -> Result<bool, ApiError> {
    let fee: ApplicationFee =
        platform_get(state, format!("/v1/application_fees/{id}"), json!({})).await?;
    let scope = super::super::connect_scope(state).await?;
    let attempt = match expected {
        Some(attempt) => Some(attempt.clone()),
        None => {
            payment_attempt::Entity::find()
                .filter(payment_attempt::Column::SourceType.eq(SOURCE))
                .filter(payment_attempt::Column::PlatformAccountId.eq(&scope.platform_account_id))
                .filter(payment_attempt::Column::ConnectedAccountId.eq(fee.account.id()))
                .filter(payment_attempt::Column::StripeChargeId.eq(fee.charge.id()))
                .filter(payment_attempt::Column::Livemode.eq(fee.livemode))
                .one(&state.db)
                .await?
        }
    };
    let Some(attempt) = attempt else {
        return Ok(false);
    };
    if attempt.source_type != SOURCE
        || attempt.platform_account_id != scope.platform_account_id
        || attempt.livemode != fee.livemode
        || attempt.connected_account_id.as_deref() != Some(fee.account.id())
        || attempt.stripe_charge_id.as_deref() != Some(fee.charge.id())
        || attempt.application_fee_amount as u64 != fee.amount
        || attempt.currency != fee.currency
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "Application fee does not belong to this payment",
        ));
    }
    fee_journal(
        state,
        &attempt,
        &fee.id,
        "APPLICATION_FEE",
        fee.amount as i64,
        fee.balance_transaction.as_ref().map(|value| value.id()),
        occurred_ms(fee.created),
    )
    .await?;
    let refunds: StripeList<FeeRefund> = platform_get(
        state,
        format!("/v1/application_fees/{id}/refunds"),
        json!({"limit":100}),
    )
    .await?;
    for refund in refunds.data {
        if refund.fee.id() != fee.id
            || refund.currency != fee.currency
            || refund.amount > fee.amount
        {
            return Err(error(
                "PAYMENT_BINDING_MISMATCH",
                "Application fee refund does not match",
            ));
        }
        fee_journal(
            state,
            &attempt,
            &refund.id,
            "FEE_REFUND",
            -(refund.amount as i64),
            refund.balance_transaction.as_ref().map(|value| value.id()),
            occurred_ms(refund.created),
        )
        .await?;
    }
    if refunds.has_more {
        return Err(error(
            "PAYMENT_OPERATION_REVIEW",
            "Fee refund reconciliation requires further pages",
        ));
    }
    Ok(true)
}

async fn reconcile_paid(
    state: &AppState,
    attempt: &payment_attempt::Model,
) -> Result<(), ApiError> {
    let row = load(state, &attempt.source_id).await?;
    let Some(charge) = attempt.stripe_charge_id.as_deref() else {
        return Ok(());
    };
    let canonical: Charge =
        retrieve(state, &row, format!("/v1/charges/{charge}"), json!({})).await?;
    if canonical.id != charge
        || canonical.amount != attempt.captured_amount as u64
        || canonical.currency != attempt.currency
        || canonical.livemode != attempt.livemode
    {
        return Err(error("PAYMENT_BINDING_MISMATCH", "Captured charge changed"));
    }
    if platform_owned(&row)
        && (canonical.application_fee_amount.is_some()
            || canonical.application_fee.is_some()
            || canonical.transfer.is_some()
            || canonical.transfer_data.is_some())
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "Platform payments cannot contain connected-account transfers or fees",
        ));
    }
    if let Some(fee) = canonical.application_fee.as_ref() {
        observe_application_fee(state, fee.id(), Some(attempt)).await?;
    }
    if let Some(balance) = canonical.balance_transaction.as_ref() {
        let movement: BalanceTransaction = retrieve(
            state,
            &row,
            format!("/v1/balance_transactions/{}", balance.id()),
            json!({}),
        )
        .await?;
        if movement.id != balance.id() {
            return Err(error(
                "PAYMENT_BINDING_MISMATCH",
                "Charge balance transaction changed",
            ));
        }
        let id = blake3::hash(
            format!(
                "{}:{}:{}:{}:CHARGE_CASH",
                attempt.platform_account_id, attempt.scope_key, attempt.livemode, movement.id
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string();
        state.db.execute_raw(sql(r#"INSERT INTO "PaymentLedgerEntry" (id,"attemptId","sourceType","sourceId","platformAccountId","scopeKey",livemode,perspective,kind,amount,currency,"payeeUserId","payerUserId","appId","stripeObjectId","stripeBalanceTransactionId","occurredAt","createdAt") VALUES ($1,$2,'REQUEST',$3,$4,$5,$6,$15,'CHARGE_CASH',$7,$8,$9,$10,$11,$12,$12,$13,$14) ON CONFLICT DO NOTHING"#,vec![id.into(),attempt.id.clone().into(),attempt.source_id.clone().into(),attempt.platform_account_id.clone().into(),attempt.scope_key.clone().into(),attempt.livemode.into(),movement.net.into(),movement.currency.into(),attempt.payee_user_id.clone().into(),attempt.payer_user_id.clone().into(),attempt.app_id.clone().into(),movement.id.into(),(movement.created.saturating_mul(1000).min(i64::MAX as u64) as i64).into(),now().into(),payment_perspective(&row).into()])).await?;
    }
    let refunds: StripeList<Refund> = retrieve(
        state,
        &row,
        "/v1/refunds".into(),
        json!({"charge":charge,"limit":100}),
    )
    .await?;
    for refund in refunds.data {
        super::super::marketplace::observe_refund(state, &attempt.id, &refund).await?;
    }
    if refunds.has_more {
        return Err(error(
            "PAYMENT_OPERATION_REVIEW",
            "Refund reconciliation requires further pages",
        ));
    }
    let disputes: StripeList<Dispute> = retrieve(
        state,
        &row,
        "/v1/disputes".into(),
        json!({"charge":charge,"limit":100}),
    )
    .await?;
    for dispute in disputes.data {
        observe_dispute(state, &row, &attempt.id, &dispute.id).await?;
    }
    if disputes.has_more {
        return Err(error(
            "PAYMENT_OPERATION_REVIEW",
            "Dispute reconciliation requires further pages",
        ));
    }
    if automatic_tax(&row) {
        refresh_tax_invoice(state, &row, attempt).await?;
        let current = payment_attempt::Entity::find_by_id(&attempt.id)
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;
        super::super::marketplace::reconcile_node_tax(state, &current).await?;
    }
    if attempt.hydration_status.as_deref() == Some("AWAITING_STRIPE_OBJECTS") {
        reconcile_request(state, &row.id).await?;
    }
    state
        .db
        .execute_raw(sql(
            r#"UPDATE "PaymentAttempt" SET "nextCheckAt"=$2 WHERE id=$1"#,
            vec![attempt.id.clone().into(), (now() + 900_000).into()],
        ))
        .await?;
    Ok(())
}

async fn refresh_tax_invoice(
    state: &AppState,
    row: &payment_request::Model,
    attempt: &payment_attempt::Model,
) -> Result<(), ApiError> {
    if attempt.snapshot["invoiceId"].as_str().is_some() {
        return Ok(());
    }
    let session_id = attempt.stripe_session_id.as_deref().ok_or_else(|| {
        error(
            "PAYMENT_TAX_REVIEW",
            "The taxed payment has no Checkout session",
        )
    })?;
    let session: CheckoutSession = retrieve(
        state,
        row,
        format!("/v1/checkout/sessions/{session_id}"),
        json!({}),
    )
    .await?;
    if session.id != session_id
        || session.livemode != attempt.livemode
        || session.currency.as_deref() != Some(attempt.currency.as_str())
        || session.payment_intent.as_ref().map(|intent| intent.id())
            != attempt.stripe_payment_intent_id.as_deref()
    {
        return Err(error(
            "PAYMENT_BINDING_MISMATCH",
            "The payment invoice session changed",
        ));
    }
    let Some((tax, invoice)) = tax_observation(row, &session)? else {
        return Err(error(
            "PAYMENT_TAX_REVIEW",
            "The payment tax policy changed",
        ));
    };
    let invoice = invoice.ok_or_else(|| {
        error(
            "PAYMENT_OPERATION_PENDING",
            "The payment invoice is still being created",
        )
    })?;
    state.transaction(|txn| {
        let (attempt, invoice) = (attempt.clone(), invoice.clone());
        Box::pin(async move {
            crate::db::coordination::coordinate(txn, "payments-charge", &[&attempt.id]).await?;
            let current = payment_attempt::Entity::find_by_id(&attempt.id).one(txn).await?.ok_or(ApiError::NOT_FOUND)?;
            if current.snapshot["taxAmount"].as_i64() != Some(tax)
                || current.snapshot["invoiceId"].as_str().is_some_and(|id| id != invoice)
            {
                return Err(error("PAYMENT_TAX_REVIEW", "The captured payment tax or invoice changed"));
            }
            let mut snapshot = current.snapshot;
            snapshot["invoiceId"] = json!(invoice);
            txn.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET snapshot=$2,revision=revision+1,"updatedAt"=$3 WHERE id=$1"#, vec![attempt.id.into(), snapshot.into(), now().into()])).await?;
            Ok::<_,ApiError>(())
        })
    }).await
}

pub async fn handle_effect(
    state: &AppState,
    effect: &str,
    source_id: &str,
    payload: &Value,
) -> Result<bool, ApiError> {
    match effect {
        "node_reconcile" => {
            reconcile_request(state, source_id).await?;
            Ok(true)
        }
        "node_orphan_refund" => {
            let attempt = payment_attempt::Entity::find_by_id(source_id)
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            if attempt.source_type != SOURCE || !attempt.orphaned {
                return Err(error(
                    "PAYMENT_REFUND_INVALID",
                    "Only an orphaned request can use automatic recovery",
                ));
            }
            if attempt.refunded_amount + attempt.reserved_refund_amount < attempt.captured_amount {
                super::super::marketplace::request_refund(
                    state,
                    source_id,
                    &format!("node-orphan:{source_id}"),
                    Some(
                        attempt.captured_amount
                            - attempt.refunded_amount
                            - attempt.reserved_refund_amount,
                    ),
                    ORPHAN_REFUND_REASON,
                    "system",
                )
                .await?;
            }
            Ok(true)
        }
        "CANCEL_APP_PAYMENTS" | "CANCEL_ACCOUNT_PAYMENTS" | "CANCEL_SELLER_PAYMENTS" => {
            let cutoff = payload["cutoffCreatedAt"].as_i64().ok_or_else(|| {
                ApiError::internal("Payment cancellation requires an event cutoff")
            })?;
            let mut query = payment_request::Entity::find()
                .filter(payment_request::Column::CreatedAt.lte(cutoff))
                .filter(payment_request::Column::CancelRequested.eq(false))
                .filter(payment_request::Column::Status.is_in([
                    "CREATED",
                    "OPENING",
                    "OPEN",
                    "PROCESSING",
                    "CANCEL_PENDING",
                ]));
            query = if effect == "CANCEL_APP_PAYMENTS" {
                query.filter(payment_request::Column::AppId.eq(source_id))
            } else if effect == "CANCEL_SELLER_PAYMENTS" {
                query.filter(payment_request::Column::PayeeUserId.eq(source_id))
            } else {
                query.filter(
                    payment_request::Column::ConnectedAccountId
                        .eq(payload["accountId"].as_str().unwrap_or("")),
                )
            };
            let rows = query
                .order_by_asc(payment_request::Column::CreatedAt)
                .limit(25)
                .all(&state.db)
                .await?;
            let full_batch = rows.len() == 25;
            for row in rows {
                cancel(
                    state,
                    &row.id,
                    payload["reason"].as_str().unwrap_or("PAYMENTS_DISABLED"),
                )
                .await?;
            }
            if full_batch {
                return Err(ApiError::internal(
                    "Payment cancellation has another pending batch",
                ));
            }
            Ok(true)
        }
        "PAYMENT_REPORT" => {
            crate::audit::record::write(
                &state.db,
                crate::audit::AuditRecordInput::system(
                    "payments",
                    "payments.reported",
                    "PaymentRequest",
                    source_id,
                )
                .with_details(payload.clone()),
                crate::audit::WriteMode::Once,
            )
            .await?;
            Ok(true)
        }
        _ => Ok(false),
    }
}
pub async fn reconcile(state: &AppState, limit: i64) -> Result<(), ApiError> {
    let rows = payment_request::Entity::find()
        .filter(payment_request::Column::NextCheckAt.lte(now()))
        .filter(payment_request::Column::Status.is_in([
            "CREATED",
            "OPENING",
            "OPEN",
            "PROCESSING",
            "CANCEL_PENDING",
        ]))
        .order_by_asc(payment_request::Column::NextCheckAt)
        .limit(limit.clamp(1, 100) as u64)
        .all(&state.db)
        .await?;
    for row in rows {
        if let Err(error) = reconcile_request(state, &row.id).await {
            tracing::warn!(payment_id=%row.id,%error,"Payment request reconciliation deferred");
            state
                .db
                .execute_raw(sql(
                    r#"UPDATE "PaymentRequest" SET "nextCheckAt"=$2 WHERE id=$1"#,
                    vec![row.id.into(), (now() + 30_000).into()],
                ))
                .await?;
        }
    }
    let captured = payment_attempt::Entity::find()
        .filter(payment_attempt::Column::SourceType.eq(SOURCE))
        .filter(payment_attempt::Column::CapturedAmount.gt(0))
        .filter(payment_attempt::Column::NextCheckAt.lte(now()))
        .order_by_asc(payment_attempt::Column::NextCheckAt)
        .limit(limit.clamp(1, 100) as u64)
        .all(&state.db)
        .await?;
    for attempt in captured {
        if let Err(error) = reconcile_paid(state, &attempt).await {
            tracing::warn!(attempt_id=%attempt.id,%error,"Captured payment reconciliation deferred");
        }
    }
    let orphans = payment_attempt::Entity::find()
        .filter(payment_attempt::Column::SourceType.eq(SOURCE))
        .filter(payment_attempt::Column::Orphaned.eq(true))
        .filter(payment_attempt::Column::CapturedAmount.gt(0))
        .limit(limit.clamp(1, 100) as u64)
        .all(&state.db)
        .await?;
    for attempt in orphans {
        if attempt.refunded_amount + attempt.reserved_refund_amount < attempt.captured_amount {
            handle_effect(state, "node_orphan_refund", &attempt.id, &json!({})).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::PaidDisposition::{Accept, Duplicate, Orphan};

    #[test]
    fn taxed_capture_requires_completed_calculation_with_the_approved_total() {
        let mut row = super::super::tests::request_fixture();
        row.snapshot = json!({"automaticTax":true,"productTaxCode":"txcd_10000000"});
        let base = json!({"id":"cs_tax","livemode":false,"mode":"payment","payment_status":"paid","amount_total":100,"invoice":"in_tax","automatic_tax":{"enabled":true,"status":"complete","liability":{"type":"self"}},"total_details":{"amount_tax":19,"amount_discount":0,"amount_shipping":0}});
        let session: CheckoutSession = serde_json::from_value(base.clone()).unwrap();
        assert_eq!(
            tax_observation(&row, &session).unwrap(),
            Some((19, Some("in_tax".into())))
        );
        let mut zero = base.clone();
        zero["total_details"]["amount_tax"] = json!(0);
        zero["invoice"] = Value::Null;
        assert_eq!(
            tax_observation(&row, &serde_json::from_value(zero).unwrap()).unwrap(),
            Some((0, None))
        );
        for (pointer, value) in [
            ("/automatic_tax/status", json!("requires_location_inputs")),
            ("/automatic_tax/status", json!("failed")),
            ("/automatic_tax/enabled", json!(false)),
            ("/automatic_tax/liability/type", json!("account")),
            ("/amount_total", json!(119)),
            ("/total_details/amount_tax", json!(101)),
            ("/total_details/amount_shipping", json!(10)),
            ("/total_details/amount_discount", json!(10)),
            ("/invoice", json!("cs_wrong")),
        ] {
            let mut changed = base.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(
                tax_observation(&row, &serde_json::from_value(changed).unwrap()).is_err(),
                "accepted invalid field {pointer}"
            );
        }
        row.snapshot = json!({});
        let legacy: CheckoutSession = serde_json::from_value(
            json!({"id":"cs_old","livemode":false,"mode":"payment","payment_status":"paid"}),
        )
        .unwrap();
        assert_eq!(tax_observation(&row, &legacy).unwrap(), None);
    }
    #[test]
    fn paid_observation_requires_live_unchanged_admission_before_deadline() {
        let mut request = super::super::tests::request_fixture();
        assert_eq!(
            classify_request_payment(&request, "attempt", true, true, 999),
            Accept
        );
        assert_eq!(
            classify_request_payment(&request, "attempt", true, true, 1000),
            Orphan
        );
        assert_eq!(
            classify_request_payment(&request, "attempt", false, true, 999),
            Orphan
        );
        assert_eq!(
            classify_request_payment(&request, "attempt", true, false, 999),
            Orphan
        );
        request.cancel_requested = true;
        assert_eq!(
            classify_request_payment(&request, "attempt", true, true, 999),
            Orphan
        );
    }
    #[test]
    fn replay_of_accepted_attempt_survives_later_run_end_but_second_charge_is_duplicate() {
        let mut request = super::super::tests::request_fixture();
        request.accepted_attempt_id = Some("accepted".into());
        request.cancel_requested = true;
        assert_eq!(
            classify_request_payment(&request, "accepted", false, false, 2000),
            Accept
        );
        assert_eq!(
            classify_request_payment(&request, "another-charge", false, false, 2000),
            Duplicate
        );
    }
    #[test]
    fn cash_dates_use_provider_seconds_with_checked_millisecond_conversion() {
        assert_eq!(occurred_ms(1_700_000_000), 1_700_000_000_000);
        assert_eq!(occurred_ms(u64::MAX), i64::MAX);
    }
    #[test]
    fn checkout_expiry_does_not_release_processing_or_unknown_payment() {
        for status in [
            "processing",
            "requires_action",
            "requires_capture",
            "succeeded",
            "future_status",
        ] {
            assert!(!intent_failed_without_capture(status, 0));
        }
        assert!(intent_failed_without_capture("canceled", 0));
        assert!(!intent_failed_without_capture("canceled", 100));
        assert!(intent_failed_without_capture("requires_payment_method", 0));
    }
}
