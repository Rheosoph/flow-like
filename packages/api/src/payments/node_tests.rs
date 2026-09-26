use super::*;
use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::{payment_limit_counter, payment_limit_reservation, payment_refund, stripe_operation},
    stripe_connect::{
        RetryDisposition, StripeError, StripeResponse,
        fake::FakeGateway,
        types::{CheckoutSession, PaymentIntent},
    },
};
use flow_like_types::tokio;
use sea_orm::{Database, DatabaseConnection};

async fn database() -> DatabaseConnection {
    let url = std::env::var("PAYMENTS_NODE_TEST_DATABASE_URL")
        .expect("PAYMENTS_NODE_TEST_DATABASE_URL must point at an empty disposable database");
    let db = Database::connect(url).await.unwrap();
    db.execute_unprepared(
        r#"
CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY, "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE TABLE "User" (id TEXT PRIMARY KEY,permission BIGINT NOT NULL DEFAULT 0);
CREATE TABLE "App" (id TEXT PRIMARY KEY,"ownerRoleId" TEXT);
CREATE TABLE "Membership" ("appId" TEXT,"roleId" TEXT,"userId" TEXT);
CREATE TABLE "QuotaOperation" (id TEXT PRIMARY KEY,deadline BIGINT,generation BIGINT,kind TEXT,status TEXT,"cancelRequested" BOOLEAN);
CREATE TABLE "ExecutionRun" (id TEXT PRIMARY KEY,"appId" TEXT,"userId" TEXT,status TEXT,mode TEXT,"runVariant" TEXT,"technicalUserId" TEXT,"callerAppChain" JSONB,"completedAt" BIGINT);
CREATE TABLE "WasmPackage" (id TEXT PRIMARY KEY);
CREATE TABLE "AppDiscount" (id TEXT PRIMARY KEY);
CREATE TABLE "JoinQueue" (id TEXT PRIMARY KEY);
CREATE TABLE "AppPurchase" (id TEXT PRIMARY KEY,"userId" TEXT REFERENCES "User"(id),"appId" TEXT REFERENCES "App"(id),"discountId" TEXT REFERENCES "AppDiscount"(id));
CREATE TABLE "WasmPackagePurchase" (id TEXT PRIMARY KEY,"userId" TEXT REFERENCES "User"(id),"packageId" TEXT REFERENCES "WasmPackage"(id));
"#,
    )
    .await
    .expect("database must be empty");
    db.execute_unprepared(include_str!(
        "../../prisma/migrations/20260920120000_payments_foundations/migration.sql"
    ))
    .await
    .unwrap();
    db.execute_unprepared(include_str!(
        "../../prisma/migrations/20260920130000_platform_owned_payments/migration.sql"
    ))
    .await
    .unwrap();
    db
}

async fn seed(
    db: &DatabaseConnection,
    label: &str,
) -> (payment_request::Model, payment_attempt::Model) {
    let request_id = format!("request_{label}");
    let attempt_id = format!("attempt_{label}");
    let counter = format!("counter_{label}");
    let scope = StripeScope::platform("acct_platform", false).connected("acct_seller");
    let request = StripeRequest::post(
        scope,
        "/v1/checkout/sessions",
        json!({"mode":"payment","line_items":[{"quantity":1,"price_data":{"currency":"eur","unit_amount":1000}}]}),
        format!("request:{request_id}:session:1"),
    );
    let operation = operations::prepare(db, SOURCE, &request_id, "checkout_create", &request)
        .await
        .unwrap();
    db.execute_raw(sql(
        r#"INSERT INTO "PaymentRequest" (id,"runId","nodeId",nonce,"requestDigest","appId","boardId","payerUserId","payeeUserId","connectedAccountId","platformAccountId",livemode,amount,currency,"applicationFeeAmount","feeBps","productName",description,snapshot,status,"expiresAt","nextCheckAt","createdAt","updatedAt") VALUES ($1,'run','node',$1,'digest','app','board','payer','seller','acct_seller','acct_platform',false,1000,'eur',100,1000,'Product','','{}','OPENING',$2,0,0,0)"#,
        vec![request_id.clone().into(), (now() + 300_000).into()],
    ))
    .await
    .unwrap();
    db.execute_raw(sql(
        r#"INSERT INTO "PaymentAttempt" (id,"sourceType","sourceId",attempt,"platformAccountId","scopeKey","connectedAccountId",livemode,"operationId","payerUserId","payeeUserId","appId",amount,currency,"applicationFeeAmount","feeBps",snapshot,"nextCheckAt","createdAt","updatedAt") VALUES ($1,'REQUEST',$2,1,'acct_platform','acct_seller','acct_seller',false,$3,'payer','seller','app',1000,'eur',100,1000,'{}',0,0,0)"#,
        vec![attempt_id.clone().into(), request_id.clone().into(), operation.into()],
    ))
    .await
    .unwrap();
    db.execute_raw(sql(
        r#"INSERT INTO "PaymentLimitCounter" (key,amount,count,currency,"windowEnd","createdAt","updatedAt") VALUES ($1,1000,1,'eur',$2,0,0)"#,
        vec![counter.clone().into(), (now() + 86_400_000).into()],
    ))
    .await
    .unwrap();
    db.execute_raw(sql(
        r#"INSERT INTO "PaymentLimitReservation" (id,"counterKey","sourceType","sourceId",amount,"expiresAt","createdAt","updatedAt") VALUES ($1,$2,'REQUEST',$1,1000,$3,0,0)"#,
        vec![request_id.clone().into(), counter.into(), (now() + 300_000).into()],
    ))
    .await
    .unwrap();
    (
        payment_request::Entity::find_by_id(request_id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
        payment_attempt::Entity::find_by_id(attempt_id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
    )
}

async fn reservation_state(db: &DatabaseConnection, id: &str) -> (String, i64, i64) {
    let reservation = payment_limit_reservation::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let counter = payment_limit_counter::Entity::find_by_id(&reservation.counter_key)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    (reservation.status, counter.amount, counter.count)
}

async fn failed_checkout(db: &DatabaseConnection, attempt: &payment_attempt::Model) -> FakeGateway {
    let gateway = FakeGateway::default();
    gateway.script(Err(StripeError::Api {
        status: 400,
        code: Some("payment_method_not_available".into()),
        request_id: Some("req_declined".into()),
        retry: RetryDisposition::Never,
        retry_after_seconds: None,
    }));
    assert!(
        operations::execute_with_gateway(db, &attempt.operation_id, &gateway)
            .await
            .is_err()
    );
    assert_eq!(
        stripe_operation::Entity::find_by_id(&attempt.operation_id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "FAILED"
    );
    assert!(
        operations::execute_with_gateway(db, &attempt.operation_id, &gateway)
            .await
            .is_err()
    );
    assert_eq!(gateway.calls().len(), 1);
    gateway
}

fn completed_unpaid(label: &str, intent_status: &str) -> (CheckoutSession, PaymentIntent) {
    (
        serde_json::from_value(json!({
            "id":format!("cs_{label}"),"livemode":false,"mode":"payment",
            "status":"complete","payment_status":"unpaid","amount_total":1000,
            "currency":"eur","payment_intent":format!("pi_{label}")
        }))
        .unwrap(),
        serde_json::from_value(json!({
            "id":format!("pi_{label}"),"livemode":false,"amount":1000,
            "amount_received":0,"currency":"eur","application_fee_amount":100,
            "status":intent_status
        }))
        .unwrap(),
    )
}

async fn fail(
    db: &DatabaseConnection,
    request_id: &str,
    attempt_id: &str,
    reason: &str,
) -> Result<(), ApiError> {
    retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &RetryPolicy::default(),
        |txn| {
            let (request_id, attempt_id, reason) = (
                request_id.to_owned(),
                attempt_id.to_owned(),
                reason.to_owned(),
            );
            Box::pin(async move {
                settlement::fail_request_txn(txn, &request_id, &attempt_id, &reason).await
            })
        },
    )
    .await
}

async fn reserve_orphan(db: &DatabaseConnection, attempt_id: &str) -> Result<String, ApiError> {
    reserve_orphan_reason(db, attempt_id, settlement::ORPHAN_REFUND_REASON).await
}

async fn reserve_orphan_reason(
    db: &DatabaseConnection,
    attempt_id: &str,
    reason: &str,
) -> Result<String, ApiError> {
    let command = format!("{attempt_id}:node-orphan:{attempt_id}");
    let refund_id = blake3::hash(command.as_bytes()).to_hex().to_string();
    retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &RetryPolicy::default(),
        |txn| {
            let (attempt_id, command, refund_id, reason) = (
                attempt_id.to_owned(),
                command.clone(),
                refund_id.clone(),
                reason.to_owned(),
            );
            Box::pin(async move {
                crate::payments::marketplace::reserve_refund_txn(
                    txn,
                    attempt_id,
                    command,
                    refund_id,
                    Some(1000),
                    reason,
                    "system".into(),
                )
                .await
            })
        },
    )
    .await
}

#[test]
fn complete_unpaid_can_fail_only_after_the_intent_confirms_no_capture() {
    let scope = StripeScope::platform("acct_platform", false).connected("acct_seller");
    for status in ["canceled", "requires_payment_method"] {
        let (session, intent) = completed_unpaid("failed", status);
        let expected = crate::stripe_connect::ExpectedPayment {
            scope: scope.clone(),
            session_id: session.id.clone(),
            payment_intent_id: None,
            charge_id: None,
            amount: 1000,
            currency: "eur".into(),
            application_fee_amount: 100,
            destination_account_id: None,
        };
        assert_eq!(
            expected.verify(&scope, &session, &intent, None).unwrap(),
            crate::stripe_connect::VerifiedPayment::Processing
        );
        assert!(settlement::intent_failed_without_capture(
            &intent.status,
            intent.amount_received
        ));
        assert!(!settlement::intent_failed_without_capture(status, 1));
    }
    for status in [
        "processing",
        "requires_action",
        "requires_confirmation",
        "requires_capture",
        "succeeded",
        "future_provider_state",
    ] {
        assert!(!settlement::intent_failed_without_capture(status, 0));
    }
}

#[tokio::test]
#[ignore = "requires PAYMENTS_NODE_TEST_DATABASE_URL pointing at an empty disposable PostgreSQL database"]
async fn direct_failure_and_orphan_refund_recovery_preserve_money_invariants() {
    let db = database().await;
    let rejected = seed(&db, "rejected").await;
    failed_checkout(&db, &rejected.1).await;
    let (first, second) = tokio::join!(
        fail(&db, &rejected.0.id, &rejected.1.id, "CHECKOUT_FAILED"),
        fail(&db, &rejected.0.id, &rejected.1.id, "CHECKOUT_FAILED")
    );
    first.unwrap();
    second.unwrap();
    assert_eq!(
        reservation_state(&db, &rejected.0.id).await,
        ("RELEASED".into(), 0, 0)
    );
    assert_eq!(
        payment_request::Entity::find_by_id(&rejected.0.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "FAILED"
    );
    assert_eq!(
        payment_attempt::Entity::find_by_id(&rejected.1.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "FAILED"
    );

    for status in ["canceled", "requires_payment_method"] {
        let fixture = seed(&db, status).await;
        let (_, intent) = completed_unpaid(status, status);
        db.execute_raw(sql(
            r#"UPDATE "PaymentRequest" SET status='CANCEL_PENDING',"cancelRequested"=true WHERE id=$1"#,
            vec![fixture.0.id.clone().into()],
        )).await.unwrap();
        assert!(settlement::intent_failed_without_capture(
            &intent.status,
            intent.amount_received
        ));
        fail(&db, &fixture.0.id, &fixture.1.id, "PAYMENT_FAILED")
            .await
            .unwrap();
        assert_eq!(
            reservation_state(&db, &fixture.0.id).await,
            ("RELEASED".into(), 0, 0)
        );
        assert_eq!(
            payment_request::Entity::find_by_id(&fixture.0.id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            "CANCELED"
        );
    }

    let orphan = seed(&db, "orphan").await;
    db.execute_raw(sql(
        r#"UPDATE "PaymentAttempt" SET status='PAID',"capturedAmount"=1000,"stripeChargeId"='ch_orphan',orphaned=true WHERE id=$1"#,
        vec![orphan.1.id.clone().into()],
    )).await.unwrap();
    db.execute_raw(sql(
        r#"UPDATE "PaymentLimitReservation" SET status='CONSUMED' WHERE id=$1"#,
        vec![orphan.0.id.clone().into()],
    ))
    .await
    .unwrap();
    assert_eq!(
        fail(&db, &orphan.0.id, &orphan.1.id, "PAYMENT_FAILED")
            .await
            .unwrap_err()
            .public_code(),
        "PAYMENT_BINDING_MISMATCH"
    );
    assert_eq!(
        reservation_state(&db, &orphan.0.id).await,
        ("CONSUMED".into(), 1000, 1)
    );

    assert_eq!(
        reserve_orphan_reason(&db, &orphan.1.id, "orphan_payment")
            .await
            .unwrap_err()
            .public_code(),
        "PAYMENT_REFUND_INVALID"
    );

    let (first, second) = tokio::join!(
        reserve_orphan(&db, &orphan.1.id),
        reserve_orphan(&db, &orphan.1.id)
    );
    let refund_id = first.unwrap();
    assert_eq!(second.unwrap(), refund_id);
    let refund = payment_refund::Entity::find_by_id(&refund_id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(refund.amount, 1000);
    assert_eq!(refund.reason, "requested_by_customer");
    assert!(refund.refund_application_fee);
    assert_eq!(
        payment_attempt::Entity::find_by_id(&orphan.1.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .reserved_refund_amount,
        1000
    );
    let operation = stripe_operation::Entity::find_by_id(refund.operation_id.as_ref().unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let request: StripeRequest = serde_json::from_value(operation.request_params).unwrap();
    assert_eq!(
        request.scope,
        StripeScope::platform("acct_platform", false).connected("acct_seller")
    );
    assert_eq!(request.path, "/v1/refunds");
    assert_eq!(
        request.parameters,
        json!({
            "charge":"ch_orphan","amount":1000,"reason":"requested_by_customer","refund_application_fee":true
        })
    );

    let gateway = FakeGateway::default();
    gateway.script(Ok(StripeResponse {
        body: json!({"id":"re_orphan","object":"refund","charge":"ch_orphan","amount":1000,"currency":"eur","status":"succeeded"}),
        request_id: Some("req_refund".into()),
    }));
    for _ in 0..2 {
        let response = operations::execute_with_gateway(&db, &operation.id, &gateway)
            .await
            .unwrap();
        assert_eq!(response.body["id"], "re_orphan");
        assert_eq!(response.body["status"], "succeeded");
    }
    assert_eq!(gateway.calls().len(), 1);
    assert_eq!(
        gateway.calls()[0].scope.connected_account_id.as_deref(),
        Some("acct_seller")
    );
    platform_owned_capture_and_refund(&db).await;
}

async fn capture(
    db: &DatabaseConnection,
    row: &payment_request::Model,
    attempt: &payment_attempt::Model,
    intent: &PaymentIntent,
    charge: &crate::stripe_connect::types::Charge,
) {
    retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &RetryPolicy::default(),
        |txn| {
            let (row, attempt, intent, charge) =
                (row.clone(), attempt.clone(), intent.clone(), charge.clone());
            Box::pin(async move {
                let session: CheckoutSession = serde_json::from_value(json!({
                    "id":"cs_node","livemode":row.livemode,"mode":"payment","payment_status":"paid",
                    "status":"complete","amount_total":row.amount,"currency":row.currency,
                    "payment_intent":intent.id,"invoice":"in_node",
                    "automatic_tax":{"enabled":true,"status":"complete","liability":{"type":"self"}},
                    "total_details":{"amount_tax":200,"amount_discount":0,"amount_shipping":0}
                })).unwrap();
                settlement::settle_paid_txn(txn, &row, &attempt, &session, &intent, &charge).await
            })
        },
    )
    .await
    .unwrap();
}

async fn platform_owned_capture_and_refund(db: &DatabaseConnection) {
    db.execute_unprepared(r#"
INSERT INTO "User" (id,permission) VALUES ('admin',1);
INSERT INTO "App" (id,"ownerRoleId") VALUES ('platform-app','owner');
INSERT INTO "Membership" ("appId","roleId","userId") VALUES ('platform-app','owner','admin');
INSERT INTO "AppPaymentSettings" ("appId","ownerUserId","paymentsEnabled","updatedBy","createdAt","updatedAt") VALUES ('platform-app','admin',true,'admin',0,0);
INSERT INTO "ExecutionRun" (id,"appId","userId",status,mode,"runVariant") VALUES ('platform-run','platform-app','payer','RUNNING','REMOTE','PRIMARY');
"#).await.unwrap();
    db.execute_raw(sql(r#"INSERT INTO "QuotaOperation" (id,deadline,generation,kind,status,"cancelRequested") VALUES ('platform-run',$1,1,'workflow','running',false)"#,vec![(now()+600_000).into()])).await.unwrap();
    let (old_row, old_attempt) = seed(db, "platform").await;
    db.execute_raw(sql(r#"UPDATE "PaymentRequest" SET "runId"='platform-run',"appId"='platform-app',"payeeUserId"='admin',"connectedAccountId"=NULL,"applicationFeeAmount"=0,"feeBps"=0,snapshot='{"platformOwned":true,"automaticTax":true,"productTaxCode":"txcd_10000000"}',"settingsRevision"=0 WHERE id=$1"#,vec![old_row.id.clone().into()])).await.unwrap();
    db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "appId"='platform-app',"payeeUserId"='admin',"connectedAccountId"=NULL,"scopeKey"='platform',"applicationFeeAmount"=0,"feeBps"=0,snapshot='{"platformOwned":true,"automaticTax":true,"productTaxCode":"txcd_10000000"}' WHERE id=$1"#,vec![old_attempt.id.clone().into()])).await.unwrap();
    let row = payment_request::Entity::find_by_id(&old_row.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let attempt = payment_attempt::Entity::find_by_id(&old_attempt.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let intent: PaymentIntent=serde_json::from_value(json!({"id":"pi_platform","livemode":false,"amount":1000,"amount_received":1000,"currency":"eur","status":"succeeded","latest_charge":"ch_platform"})).unwrap();
    let charge=serde_json::from_value(json!({"id":"ch_platform","livemode":false,"amount":1000,"amount_captured":1000,"currency":"eur","paid":true,"captured":true,"payment_intent":"pi_platform","balance_transaction":"txn_platform"})).unwrap();
    capture(db, &row, &attempt, &intent, &charge).await;
    capture(db, &row, &attempt, &intent, &charge).await;
    let accepted = payment_request::Entity::find_by_id(&row.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(accepted.status, "PAID");
    assert_eq!(
        accepted.accepted_attempt_id.as_deref(),
        Some(attempt.id.as_str())
    );
    let captured = payment_attempt::Entity::find_by_id(&attempt.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert!(!captured.orphaned);
    assert!(
        captured.hydration_status.is_none(),
        "platform capture never waits for an application fee"
    );
    assert!(captured.connected_account_id.is_none());
    let mut event:crate::stripe_connect::EventEnvelope=serde_json::from_value(json!({"id":"evt_platform","type":"charge.updated","created":1,"livemode":false,"data":{"object":{"id":"ch_platform"}}})).unwrap();
    let matches = settlement::event_attempts(db, &event, "acct_platform")
        .await
        .unwrap();
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].try_get::<String>("", "id").unwrap(), attempt.id);
    event.account = Some("acct_seller".into());
    assert!(
        settlement::event_attempts(db, &event, "acct_platform")
            .await
            .unwrap()
            .is_empty()
    );
    event.account = None;
    event.livemode = true;
    assert!(
        settlement::event_attempts(db, &event, "acct_platform")
            .await
            .unwrap()
            .is_empty()
    );
    event.livemode = false;
    assert!(
        settlement::event_attempts(db, &event, "acct_other")
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(captured.snapshot["taxAmount"], 200);
    assert_eq!(captured.snapshot["invoiceId"], "in_node");
    let ledger=db.query_all_raw(sql(r#"SELECT perspective,"scopeKey",amount FROM "PaymentLedgerEntry" WHERE "attemptId"=$1 AND kind='CHARGE'"#,vec![attempt.id.clone().into()])).await.unwrap();
    assert_eq!(ledger.len(), 1);
    assert_eq!(
        ledger[0].try_get::<String>("", "perspective").unwrap(),
        "PLATFORM"
    );
    assert_eq!(
        ledger[0].try_get::<String>("", "scopeKey").unwrap(),
        "platform"
    );
    assert_eq!(ledger[0].try_get::<i64>("", "amount").unwrap(), 1000);
    let net = db.query_one_raw(sql(r#"SELECT sum(amount)::BIGINT AS amount,count(*)::BIGINT AS entries FROM "PaymentLedgerEntry" WHERE "attemptId"=$1 AND perspective='PLATFORM'"#,vec![attempt.id.clone().into()])).await.unwrap().unwrap();
    assert_eq!(net.try_get::<i64>("", "amount").unwrap(), 800);
    assert_eq!(
        net.try_get::<i64>("", "entries").unwrap(),
        2,
        "duplicate capture must not duplicate tax liability"
    );

    // Historical platform captures retain their recipient after an administrator is demoted.
    db.execute_unprepared(r#"UPDATE "User" SET permission=0 WHERE id='admin'"#)
        .await
        .unwrap();
    capture(db, &row, &attempt, &intent, &charge).await;
    assert_eq!(
        payment_request::Entity::find_by_id(&row.id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "PAID"
    );
    let refund_id = reserve_orphan(db, &attempt.id).await.unwrap();
    let refund = payment_refund::Entity::find_by_id(refund_id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert!(!refund.refund_application_fee);
    let operation = stripe_operation::Entity::find_by_id(refund.operation_id.unwrap())
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let refund_request: StripeRequest = serde_json::from_value(operation.request_params).unwrap();
    assert_eq!(
        refund_request.scope,
        StripeScope::platform("acct_platform", false)
    );
    assert_eq!(
        refund_request.parameters,
        json!({"charge":"ch_platform","amount":1000,"reason":"requested_by_customer"})
    );

    // A newly captured pending platform payment is orphaned after demotion, without rerouting it.
    let pending = seed(db, "demoted").await;
    db.execute_raw(sql(r#"UPDATE "PaymentRequest" SET "runId"='platform-run',"appId"='platform-app',"payeeUserId"='admin',"connectedAccountId"=NULL,"applicationFeeAmount"=0,"feeBps"=0,snapshot='{"platformOwned":true}',"settingsRevision"=0 WHERE id=$1"#,vec![pending.0.id.clone().into()])).await.unwrap();
    db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "appId"='platform-app',"payeeUserId"='admin',"connectedAccountId"=NULL,"scopeKey"='platform',"applicationFeeAmount"=0,"feeBps"=0,snapshot='{"platformOwned":true}' WHERE id=$1"#,vec![pending.1.id.clone().into()])).await.unwrap();
    let row = payment_request::Entity::find_by_id(&pending.0.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let attempt = payment_attempt::Entity::find_by_id(&pending.1.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let mut intent = intent.clone();
    intent.id = "pi_demoted".into();
    let mut charge = charge.clone();
    charge.id = "ch_demoted".into();
    capture(db, &row, &attempt, &intent, &charge).await;
    let current = payment_attempt::Entity::find_by_id(&attempt.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert!(current.orphaned);
    assert!(current.connected_account_id.is_none());
    assert!(
        db.query_one_raw(sql(
            r#"SELECT id FROM "PaymentOutbox" WHERE effect='node_orphan_refund' AND "sourceId"=$1"#,
            vec![attempt.id.clone().into()]
        ))
        .await
        .unwrap()
        .is_some()
    );
    let refund_id = reserve_orphan(db, &attempt.id).await.unwrap();
    let refund = payment_refund::Entity::find_by_id(refund_id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert!(!refund.refund_application_fee);

    // Connected captures retain their seller route and fee, and promotion cancels pending ones.
    db.execute_unprepared(r#"
INSERT INTO "User" (id,permission) VALUES ('seller',0);
INSERT INTO "App" (id,"ownerRoleId") VALUES ('app','seller-owner');
INSERT INTO "Membership" ("appId","roleId","userId") VALUES ('app','seller-owner','seller');
INSERT INTO "AppPaymentSettings" ("appId","ownerUserId","paymentsEnabled","updatedBy","createdAt","updatedAt") VALUES ('app','seller',true,'seller',0,0);
INSERT INTO "ConnectedAccount" (id,"userId","platformAccountId",livemode,purpose,generation,"stripeAccountId","creationParams",country,"canAcceptPayments","createdAt","updatedAt") VALUES ('seller-account','seller','acct_platform',false,'shared',0,'acct_seller','{}','DE',true,0,0);
INSERT INTO "PaymentAccountBinding" (id,"userId",livemode,purpose,"activeAccountId","createdAt","updatedAt") VALUES ('seller-binding','seller',false,'shared','seller-account',0,0);
INSERT INTO "ExecutionRun" (id,"appId","userId",status,mode,"runVariant") VALUES ('run','app','payer','RUNNING','REMOTE','PRIMARY');
"#).await.unwrap();
    db.execute_raw(sql(r#"INSERT INTO "QuotaOperation" (id,deadline,generation,kind,status,"cancelRequested") VALUES ('run',$1,1,'workflow','running',false)"#,vec![(now()+600_000).into()])).await.unwrap();
    let seller = seed(db, "seller_capture").await;
    db.execute_raw(sql(
        r#"UPDATE "PaymentRequest" SET "settingsRevision"=0 WHERE id=$1"#,
        vec![seller.0.id.clone().into()],
    ))
    .await
    .unwrap();
    let seller = (
        payment_request::Entity::find_by_id(&seller.0.id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
        seller.1,
    );
    let mut intent = intent.clone();
    intent.id = "pi_seller_capture".into();
    let mut charge = charge.clone();
    charge.id = "ch_seller_capture".into();
    charge.application_fee_amount = Some(100);
    charge.application_fee = Some(crate::stripe_connect::types::Expandable::Id(
        "fee_seller".into(),
    ));
    capture(db, &seller.0, &seller.1, &intent, &charge).await;
    assert_eq!(
        payment_request::Entity::find_by_id(&seller.0.id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "PAID"
    );
    let ledger = db
        .query_one_raw(sql(
            r#"SELECT perspective,"scopeKey" FROM "PaymentLedgerEntry" WHERE "attemptId"=$1"#,
            vec![seller.1.id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        ledger.try_get::<String>("", "perspective").unwrap(),
        "SELLER"
    );
    assert_eq!(
        ledger.try_get::<String>("", "scopeKey").unwrap(),
        "acct_seller"
    );
    let taxed = seed(db, "taxed_seller").await;
    for (table, id) in [
        ("PaymentRequest", &taxed.0.id),
        ("PaymentAttempt", &taxed.1.id),
    ] {
        db.execute_raw(sql(&format!(r#"UPDATE "{table}" SET snapshot='{{"automaticTax":true,"productTaxCode":"txcd_10000000"}}' WHERE id=$1"#), vec![id.clone().into()])).await.unwrap();
    }
    db.execute_raw(sql(
        r#"UPDATE "PaymentRequest" SET "settingsRevision"=0 WHERE id=$1"#,
        vec![taxed.0.id.clone().into()],
    ))
    .await
    .unwrap();
    let taxed = (
        payment_request::Entity::find_by_id(&taxed.0.id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
        payment_attempt::Entity::find_by_id(&taxed.1.id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
    );
    intent.id = "pi_taxed_seller".into();
    charge.id = "ch_taxed_seller".into();
    capture(db, &taxed.0, &taxed.1, &intent, &charge).await;
    capture(db, &taxed.0, &taxed.1, &intent, &charge).await;
    let tax_ledger = db.query_one_raw(sql(r#"SELECT amount,"scopeKey",perspective FROM "PaymentLedgerEntry" WHERE "attemptId"=$1 AND kind='TAX_LIABILITY'"#, vec![taxed.1.id.clone().into()])).await.unwrap().unwrap();
    assert_eq!(tax_ledger.try_get::<i64>("", "amount").unwrap(), -200);
    assert_eq!(
        tax_ledger.try_get::<String>("", "scopeKey").unwrap(),
        "acct_seller"
    );
    assert_eq!(
        tax_ledger.try_get::<String>("", "perspective").unwrap(),
        "SELLER"
    );
    let captured = payment_attempt::Entity::find_by_id(&taxed.1.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(captured.snapshot["taxAmount"], 200);
    assert_eq!(captured.snapshot["invoiceId"], "in_node");
    let promoted = seed(db, "promoted").await;
    db.execute_unprepared(r#"UPDATE "User" SET permission=1 WHERE id='seller'"#)
        .await
        .unwrap();
    intent.id = "pi_promoted".into();
    charge.id = "ch_promoted".into();
    capture(db, &promoted.0, &promoted.1, &intent, &charge).await;
    let attempt = payment_attempt::Entity::find_by_id(&promoted.1.id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert!(attempt.orphaned);
    assert_eq!(attempt.connected_account_id.as_deref(), Some("acct_seller"));
    let refund_id = reserve_orphan(db, &attempt.id).await.unwrap();
    let refund = payment_refund::Entity::find_by_id(refund_id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert!(refund.refund_application_fee);
    let operation = stripe_operation::Entity::find_by_id(refund.operation_id.unwrap())
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let request: StripeRequest = serde_json::from_value(operation.request_params).unwrap();
    assert_eq!(
        request.scope.connected_account_id.as_deref(),
        Some("acct_seller")
    );
}

#[test]
fn platform_checkout_uses_immutable_platform_scope_and_omits_connect_parameters() {
    let mut row = super::tests::request_fixture();
    row.connected_account_id = None;
    row.application_fee_amount = 0;
    row.fee_bps = 0;
    row.snapshot = json!({"platformOwned":true});
    let policy = PaymentMethodPolicy {
        version: "interactive-v1".into(),
        charge_model: ChargeModel::Platform,
        allowed_methods: vec!["card".into(), "link".into()],
        configuration_id: None,
        allow_delayed: false,
    };
    let params = checkout_parameters(
        &row,
        "attempt",
        2_000_000,
        "https://example.com/payments/return",
        &policy,
    )
    .unwrap();
    assert_eq!(
        request_stripe_scope(&row).unwrap(),
        StripeScope::platform("acct_platform", false)
    );
    assert_eq!(
        params["payment_intent_data"],
        json!({"metadata":{"flowlike_attempt":"attempt","flowlike_kind":"request"}})
    );
    assert_eq!(
        params["allowed_payment_method_types"],
        json!(["card", "link"])
    );
    assert_eq!(params["line_items"][0]["price_data"]["unit_amount"], 100);
    row.connected_account_id = Some("acct_other".into());
    assert!(request_stripe_scope(&row).is_err());
    row.connected_account_id = None;
    row.application_fee_amount = 1;
    assert!(request_stripe_scope(&row).is_err());

    let connected = super::tests::request_fixture();
    let direct_policy = PaymentMethodPolicy {
        charge_model: ChargeModel::Direct,
        ..policy
    };
    let params = checkout_parameters(
        &connected,
        "attempt",
        2_000_000,
        "https://example.com/payments/return",
        &direct_policy,
    )
    .unwrap();
    assert_eq!(params["payment_intent_data"]["application_fee_amount"], 1);
    assert_eq!(
        request_stripe_scope(&connected)
            .unwrap()
            .connected_account_id
            .as_deref(),
        Some("acct_seller")
    );
}

#[test]
fn taxed_node_checkout_preserves_total_and_uses_the_recorded_supplier_scope() {
    for platform in [false, true] {
        let mut row = super::tests::request_fixture();
        row.snapshot = json!({"platformOwned":platform,"automaticTax":true,"productTaxCode":"txcd_10000000","shippingCountries":["DE","FR"]});
        if platform {
            row.connected_account_id = None;
            row.application_fee_amount = 0;
            row.fee_bps = 0;
        }
        let policy = PaymentMethodPolicy {
            version: "interactive-v1".into(),
            charge_model: if platform {
                ChargeModel::Platform
            } else {
                ChargeModel::Direct
            },
            allowed_methods: vec!["card".into(), "link".into()],
            configuration_id: None,
            allow_delayed: false,
        };
        let params = checkout_parameters(
            &row,
            "attempt",
            2_000_000,
            "https://example.com/payments/return",
            &policy,
        )
        .unwrap();
        assert_eq!(
            params["line_items"][0]["price_data"]["unit_amount"],
            row.amount
        );
        assert_eq!(
            params["line_items"][0]["price_data"]["tax_behavior"],
            "inclusive"
        );
        assert_eq!(
            params["line_items"][0]["price_data"]["product_data"]["tax_code"],
            "txcd_10000000"
        );
        assert_eq!(
            params["automatic_tax"],
            json!({"enabled":true,"liability":{"type":"self"}})
        );
        assert_eq!(
            params["invoice_creation"],
            json!({"enabled":true,"invoice_data":{"issuer":{"type":"self"}}})
        );
        assert_eq!(params["customer_creation"], "always");
        assert_eq!(params["billing_address_collection"], "required");
        assert!(params.get("tax_id_collection").is_none());
        assert_eq!(
            params["shipping_address_collection"],
            json!({"allowed_countries":["DE","FR"]})
        );
        assert_eq!(
            request_stripe_scope(&row).unwrap().connected_account_id,
            if platform {
                None
            } else {
                Some("acct_seller".into())
            }
        );
        row.snapshot["productTaxCode"] = json!("");
        assert!(
            checkout_parameters(
                &row,
                "attempt",
                2_000_000,
                "https://example.com/payments/return",
                &policy
            )
            .is_err()
        );
    }
    let row = super::tests::request_fixture();
    assert!(
        !automatic_tax(&row),
        "historical snapshots must preserve their original tax behavior"
    );
}
