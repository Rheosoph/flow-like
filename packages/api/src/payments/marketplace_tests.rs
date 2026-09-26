use super::*;
use crate::db::{DbDialect, RetryPolicy, retry_transaction};
use flow_like::hub::{PaymentFeeBasis, PaymentTaxMode};
use flow_like_types::tokio;
use sea_orm::{Database, DatabaseConnection, TransactionTrait};

async fn database() -> DatabaseConnection {
    let url = std::env::var("PAYMENTS_MARKETPLACE_TEST_DATABASE_URL").expect(
        "PAYMENTS_MARKETPLACE_TEST_DATABASE_URL must point at an empty disposable database",
    );
    let db = Database::connect(url).await.unwrap();
    db.execute_unprepared(r#"
CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY, "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now());
CREATE TABLE "User" (id TEXT PRIMARY KEY, permission BIGINT NOT NULL DEFAULT 0);
CREATE TABLE "App" (id TEXT PRIMARY KEY, visibility TEXT NOT NULL, "ownerRoleId" TEXT NOT NULL);
CREATE TABLE "Membership" (id TEXT PRIMARY KEY,"userId" TEXT NOT NULL,"appId" TEXT NOT NULL,"roleId" TEXT NOT NULL,"joinedVia" TEXT,"createdAt" TIMESTAMPTZ NOT NULL,"updatedAt" TIMESTAMPTZ NOT NULL,UNIQUE("userId","appId"));
CREATE TABLE "WasmPackage" (id TEXT PRIMARY KEY, visibility TEXT NOT NULL DEFAULT 'PUBLIC', status TEXT NOT NULL DEFAULT 'ACTIVE', price BIGINT NOT NULL DEFAULT 0);
CREATE TABLE "WasmPackageUser" (id TEXT PRIMARY KEY,"packageId" TEXT NOT NULL,"userId" TEXT NOT NULL,permission BIGINT NOT NULL,"grantedBy" TEXT,"grantedAt" TIMESTAMPTZ NOT NULL,UNIQUE("packageId","userId"));
CREATE TABLE "AppDiscount" (id TEXT PRIMARY KEY);
CREATE TABLE "JoinQueue" (id TEXT PRIMARY KEY);
CREATE TABLE "AppPurchase" (id TEXT PRIMARY KEY,"userId" TEXT REFERENCES "User"(id) ON DELETE CASCADE,"appId" TEXT REFERENCES "App"(id) ON DELETE CASCADE,"discountId" TEXT REFERENCES "AppDiscount"(id) ON DELETE SET NULL,"stripeSessionId" TEXT NOT NULL,"stripePaymentIntentId" TEXT,"pricePaid" BIGINT,"originalPrice" BIGINT,"discountAmount" BIGINT,currency TEXT,status TEXT,"completedAt" TIMESTAMPTZ,"refundedAt" TIMESTAMPTZ,"refundReason" TEXT,"createdAt" TIMESTAMPTZ,"updatedAt" TIMESTAMPTZ);
CREATE TABLE "WasmPackagePurchase" (id TEXT PRIMARY KEY,"userId" TEXT REFERENCES "User"(id) ON DELETE CASCADE,"packageId" TEXT REFERENCES "WasmPackage"(id) ON DELETE CASCADE,"stripeSessionId" TEXT NOT NULL,"stripePaymentIntentId" TEXT,"pricePaid" BIGINT,"originalPrice" BIGINT,"discountAmount" BIGINT,currency TEXT,status TEXT,"completedAt" TIMESTAMPTZ,"refundedAt" TIMESTAMPTZ,"refundReason" TEXT,"createdAt" TIMESTAMPTZ,"updatedAt" TIMESTAMPTZ);
"#).await.expect("database must be empty");
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
    db.execute_unprepared(r#"
INSERT INTO "User" (id) VALUES ('seller');
INSERT INTO "ConnectedAccount" (id,"userId","platformAccountId",livemode,purpose,generation,"stripeAccountId","creationParams",country,"canSell","createdAt","updatedAt") VALUES ('seller-account','seller','acct_platform',false,'SELLER',1,'acct_seller','{}','DE',true,0,0);
INSERT INTO "PaymentAccountBinding" (id,"userId",livemode,purpose,"activeAccountId","createdAt","updatedAt") VALUES ('binding','seller',false,'SELLER','seller-account',0,0);
"#).await.unwrap();
    db
}

async fn seed(
    db: &DatabaseConnection,
    label: &str,
    buyer: &str,
    app: &str,
    cancel: bool,
) -> (
    payment_order::Model,
    payment_attempt::Model,
    Charge,
    PaymentIntent,
) {
    let offer = Offer {
        kind: ItemKind::App,
        item_id: app.into(),
        buyer: buyer.into(),
        buyer_email: None,
        payee: "seller".into(),
        platform_owned: false,
        account_id: Some("acct_seller".into()),
        account_row_id: Some("seller-account".into()),
        account_revision: 0,
        amount: 1000,
        fee: 100,
        fee_bps: 1000,
        fee_basis: PaymentFeeBasis::Gross,
        tax_mode: PaymentTaxMode::SellerSupplier,
        title: "Test purchase".into(),
        role_id: "buyer-role".into(),
        terms_version: "v1".into(),
        terms_hash: "terms-hash".into(),
        terms_text: "Purchase terms".into(),
        locale: "en".into(),
        withdrawal_waiver: false,
        withdrawal_days: Some(14),
        withdrawal_deadline: now() + 86_400_000,
        method_policy: crate::stripe_connect::PaymentMethodPolicy::cards_and_wallets(
            "v1",
            crate::stripe_connect::ChargeModel::Destination,
        ),
        product_tax_code: None,
    };
    let snapshot = serde_json::to_value(offer).unwrap();
    let order_id = format!("order_{label}");
    let attempt_id = format!("attempt_{label}");
    db.execute_raw(sql(
        r#"INSERT INTO "User" (id) VALUES ($1) ON CONFLICT DO NOTHING"#,
        vec![buyer.into()],
    ))
    .await
    .unwrap();
    db.execute_raw(sql(
        r#"INSERT INTO "App" VALUES ($1,'PUBLIC','owner-role') ON CONFLICT DO NOTHING"#,
        vec![app.into()],
    ))
    .await
    .unwrap();
    db.execute_raw(sql(r#"INSERT INTO "Membership" VALUES ($1,'seller',$2,'owner-role',NULL,now(),now()) ON CONFLICT DO NOTHING"#,vec![format!("owner_{app}").into(),app.into()])).await.unwrap();
    db.execute_raw(sql(r#"INSERT INTO "PaymentOrder" (id,kind,"userId","itemId","payeeUserId","connectedAccountId","platformAccountId",livemode,status,"chargeType",amount,currency,"applicationFeeAmount","feeBps",snapshot,"cancelRequested","expiresAt","nextCheckAt","createdAt","updatedAt") VALUES ($1,'APP',$2,$3,'seller','acct_seller','acct_platform',false,'OPEN','DESTINATION',1000,'eur',100,1000,$4,$5,$6,0,0,0)"#,vec![order_id.clone().into(),buyer.into(),app.into(),snapshot.clone().into(),cancel.into(),(now()+3_600_000).into()])).await.unwrap();
    db.execute_raw(sql(r#"INSERT INTO "PaymentAttempt" (id,"sourceType","sourceId",attempt,"platformAccountId","scopeKey",livemode,"operationId","stripeSessionId","payerUserId","payeeUserId","appId",amount,currency,"applicationFeeAmount","feeBps",snapshot,"createdAt","updatedAt") VALUES ($1,'MARKETPLACE',$2,1,'acct_platform','platform',false,$3,$4,$5,'seller',$6,1000,'eur',100,1000,$7,0,0)"#,vec![attempt_id.clone().into(),order_id.clone().into(),format!("op_{label}").into(),format!("cs_{label}").into(),buyer.into(),app.into(),snapshot.into()])).await.unwrap();
    let charge=serde_json::from_value(json!({"id":format!("ch_{label}"),"livemode":false,"amount":1000,"currency":"eur","paid":true,"captured":true,"amount_captured":1000,"application_fee_amount":100,"payment_intent":format!("pi_{label}"),"transfer":format!("tr_{label}"),"application_fee":format!("fee_{label}")})).unwrap();
    let intent=serde_json::from_value(json!({"id":format!("pi_{label}"),"livemode":false,"amount":1000,"currency":"eur","status":"succeeded","amount_received":1000,"application_fee_amount":100,"transfer_data":{"destination":"acct_seller"},"latest_charge":format!("ch_{label}")})).unwrap();
    (
        load_order(db, &order_id).await.unwrap(),
        payment_attempt::Entity::find_by_id(&attempt_id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
        charge,
        intent,
    )
}

/// An app sale fixture rewritten into a package sale by the package's owner.
async fn seed_package(
    db: &DatabaseConnection,
    label: &str,
    buyer: &str,
    package: &str,
) -> (
    payment_order::Model,
    payment_attempt::Model,
    Charge,
    PaymentIntent,
) {
    let (order, attempt, charge, intent) =
        seed(db, label, buyer, &format!("app_for_{label}"), false).await;
    let mut offer: Offer = serde_json::from_value(order.snapshot).unwrap();
    offer.kind = ItemKind::Package;
    offer.item_id = package.into();
    offer.role_id = String::new();
    let snapshot = serde_json::to_value(offer).unwrap();
    db.execute_raw(sql(r#"INSERT INTO "WasmPackage" (id,visibility,status,price) VALUES ($1,'PUBLIC','ACTIVE',1000) ON CONFLICT DO NOTHING"#,vec![package.into()])).await.unwrap();
    db.execute_raw(sql(r#"INSERT INTO "WasmPackageUser" (id,"packageId","userId",permission,"grantedAt") VALUES ($1,$2,'seller',1,now()) ON CONFLICT DO NOTHING"#,vec![format!("owner_{package}").into(),package.into()])).await.unwrap();
    db.execute_raw(sql(
        r#"UPDATE "PaymentOrder" SET kind='PACKAGE',"itemId"=$2,snapshot=$3 WHERE id=$1"#,
        vec![
            order.id.clone().into(),
            package.into(),
            snapshot.clone().into(),
        ],
    ))
    .await
    .unwrap();
    db.execute_raw(sql(
        r#"UPDATE "PaymentAttempt" SET "appId"=NULL,"packageId"=$2,snapshot=$3 WHERE id=$1"#,
        vec![attempt.id.clone().into(), package.into(), snapshot.into()],
    ))
    .await
    .unwrap();
    (
        load_order(db, &order.id).await.unwrap(),
        payment_attempt::Entity::find_by_id(&attempt.id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
        charge,
        intent,
    )
}

async fn seed_platform(
    db: &DatabaseConnection,
    label: &str,
) -> (
    payment_order::Model,
    payment_attempt::Model,
    Charge,
    PaymentIntent,
) {
    let app = format!("app_{label}");
    let (order, attempt, mut charge, mut intent) =
        seed(db, label, &format!("buyer_{label}"), &app, false).await;
    let mut offer: Offer = serde_json::from_value(order.snapshot).unwrap();
    offer.payee = "platform-admin".into();
    offer.platform_owned = true;
    offer.account_id = None;
    offer.account_row_id = None;
    offer.fee = 0;
    offer.fee_bps = 0;
    offer.tax_mode = PaymentTaxMode::PlatformSupplier;
    offer.method_policy.charge_model = crate::stripe_connect::ChargeModel::Platform;
    let mut snapshot = serde_json::to_value(offer).unwrap();
    snapshot["taxAmount"] = json!(200);
    db.execute_unprepared(r#"INSERT INTO "User" (id,permission) VALUES ('platform-admin',1) ON CONFLICT (id) DO UPDATE SET permission=1"#).await.unwrap();
    db.execute_raw(sql(r#"UPDATE "Membership" SET "userId"='platform-admin' WHERE "appId"=$1 AND "roleId"='owner-role'"#,vec![app.into()])).await.unwrap();
    db.execute_raw(sql(r#"UPDATE "PaymentOrder" SET "payeeUserId"='platform-admin',"connectedAccountId"=NULL,"chargeType"='PLATFORM',"applicationFeeAmount"=0,"feeBps"=0,snapshot=$2 WHERE id=$1"#,vec![order.id.clone().into(),snapshot.clone().into()])).await.unwrap();
    db.execute_raw(sql(r#"UPDATE "PaymentAttempt" SET "payeeUserId"='platform-admin',"applicationFeeAmount"=0,"feeBps"=0,snapshot=$2 WHERE id=$1"#,vec![attempt.id.clone().into(),snapshot.into()])).await.unwrap();
    charge.application_fee = None;
    charge.application_fee_amount = None;
    charge.transfer = None;
    intent.application_fee_amount = None;
    intent.transfer_data = None;
    (
        load_order(db, &order.id).await.unwrap(),
        payment_attempt::Entity::find_by_id(&attempt.id)
            .one(db)
            .await
            .unwrap()
            .unwrap(),
        charge,
        intent,
    )
}

async fn deliver(
    db: &DatabaseConnection,
    fixture: &(
        payment_order::Model,
        payment_attempt::Model,
        Charge,
        PaymentIntent,
    ),
) -> Result<(), ApiError> {
    retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &RetryPolicy::default(),
        |txn| {
            let (order, attempt, charge, intent) = fixture.clone();
            let mut snapshot = attempt.snapshot.clone();
            snapshot["taxAmount"] = json!(
                snapshot
                    .get("taxAmount")
                    .and_then(Value::as_i64)
                    .unwrap_or(0)
            );
            Box::pin(
                async move { settle_paid_txn(txn, order, attempt, charge, intent, snapshot).await },
            )
        },
    )
    .await
}
async fn reserve(
    db: &DatabaseConnection,
    attempt: &str,
    command: &str,
    amount: i64,
) -> Result<String, ApiError> {
    let id = format!("{attempt}:{command}");
    retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &RetryPolicy::default(),
        |txn| {
            let (attempt, id) = (attempt.to_owned(), id.clone());
            Box::pin(async move {
                reserve_refund_txn(
                    txn,
                    attempt,
                    id.clone(),
                    id,
                    Some(amount),
                    "requested_by_customer".into(),
                    "seller".into(),
                )
                .await
            })
        },
    )
    .await
}
async fn observe(
    db: &DatabaseConnection,
    attempt: &str,
    id: &str,
    amount: i64,
    status: &str,
) -> Refund {
    let attempt = payment_attempt::Entity::find_by_id(attempt)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let refund:Refund=serde_json::from_value(json!({"id":id,"amount":amount,"currency":"eur","status":status.to_ascii_lowercase(),"charge":attempt.stripe_charge_id})).unwrap();
    retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &RetryPolicy::default(),
        |txn| {
            let (attempt, refund, status) = (attempt.clone(), refund.clone(), status.to_owned());
            Box::pin(async move { observe_refund_txn(txn, attempt, refund, status).await })
        },
    )
    .await
    .unwrap();
    refund
}
async fn count(db: &DatabaseConnection, query: &str) -> i64 {
    db.query_one_raw(sql(query, vec![]))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "count")
        .unwrap()
}

async fn observe_fx_dispute(db: &DatabaseConnection, attempt_id: &str, status: &str) {
    let attempt = payment_attempt::Entity::find_by_id(attempt_id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let dispute: crate::stripe_connect::types::Dispute = serde_json::from_value(json!({
        "id":format!("dp_{attempt_id}"),"amount":1000,"currency":"eur","livemode":false,
        "status":status,"charge":attempt.stripe_charge_id,
        "balance_transactions":[{
            "id":format!("txn_{attempt_id}"),"amount":-1080,"currency":"usd",
            "fee":1500,"net":-2580,"type":"adjustment","status":"available",
            "exchange_rate":1.08
        }]
    }))
    .unwrap();
    retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &RetryPolicy::default(),
        |txn| {
            let (attempt, dispute) = (attempt.clone(), dispute.clone());
            Box::pin(async move { record_dispute_currency_review_txn(txn, attempt, dispute).await })
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires PAYMENTS_MARKETPLACE_TEST_DATABASE_URL pointing at an empty disposable PostgreSQL database"]
async fn settlement_refund_and_access_invariants_hold_in_real_transactions() {
    let db = database().await;
    let tax_pending = seed_platform(&db, "tax-pending").await;
    for _ in 0..2 {
        retry_transaction(
            &db,
            DbDialect::Postgres,
            None,
            &RetryPolicy::default(),
            |txn| {
                let (order, attempt, _, _) = tax_pending.clone();
                Box::pin(async move { defer_tax_check(txn, &order, &attempt).await })
            },
        )
        .await
        .unwrap();
    }
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentOutbox" WHERE "sourceId"='order_tax-pending' AND effect='marketplace_reconcile'"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_tax-pending' AND snapshot->>'taxCheckError'='PAYMENT_TAX_REVIEW' AND "capturedAmount"=0"#).await, 1);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_tax-pending'"#
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "PaymentLedgerEntry" WHERE "attemptId"='attempt_tax-pending'"#
        )
        .await,
        0
    );
    deliver(&db, &tax_pending).await.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_tax-pending' AND snapshot->>'taxCheckError' IS NULL AND "capturedAmount"=1000"#).await, 1);

    let mut changed_tax = tax_pending.clone();
    changed_tax.1.snapshot["taxAmount"] = json!(201);
    assert_eq!(
        deliver(&db, &changed_tax).await.unwrap_err().public_code(),
        "PAYMENT_TAX_REVIEW"
    );
    let mut invoiced = tax_pending.clone();
    invoiced.1.snapshot["invoiceId"] = json!("in_original");
    deliver(&db, &invoiced).await.unwrap();
    deliver(&db, &tax_pending).await.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_tax-pending' AND snapshot->>'invoiceId'='in_original' AND snapshot->>'taxAmount'='200'"#).await, 1);
    invoiced.1.snapshot["invoiceId"] = json!("in_other");
    assert_eq!(
        deliver(&db, &invoiced).await.unwrap_err().public_code(),
        "PAYMENT_BINDING_MISMATCH"
    );
    assert_eq!(tax_liability_perspective(&tax_pending.1), Some("PLATFORM"));
    let mut direct_node = tax_pending.1.clone();
    direct_node.source_type = "REQUEST".into();
    direct_node.connected_account_id = Some("acct_seller".into());
    direct_node.snapshot["automaticTax"] = json!(true);
    assert_eq!(tax_liability_perspective(&direct_node), Some("SELLER"));
    direct_node.source_type = "MARKETPLACE".into();
    assert_eq!(tax_liability_perspective(&direct_node), None);
    direct_node.source_type = "REQUEST".into();
    direct_node.snapshot["automaticTax"] = json!(false);
    assert_eq!(tax_liability_perspective(&direct_node), None);

    let note = json!({"id":"cn_partial","invoice":"in_tax","currency":"eur","livemode":false,
        "status":"issued","amount":500,"total_excluding_tax":400,
        "post_payment_amount":500,"pre_payment_amount":0});
    assert_eq!(
        verify_credit_note(&note, &tax_pending.1, "in_tax", Some(500)).unwrap(),
        100
    );
    assert!(verify_credit_note(&note, &tax_pending.1, "in_other", Some(500)).is_err());
    assert!(verify_credit_note(&note, &tax_pending.1, "in_tax", Some(501)).is_err());
    let mut excessive = note.clone();
    excessive["total_excluding_tax"] = json!(299);
    assert!(verify_credit_note(&excessive, &tax_pending.1, "in_tax", Some(500)).is_err());
    let mut full = note.clone();
    full["amount"] = json!(1000);
    full["post_payment_amount"] = json!(1000);
    full["total_excluding_tax"] = json!(800);
    assert_eq!(
        verify_credit_note(&full, &tax_pending.1, "in_tax", Some(1000)).unwrap(),
        200
    );
    let mut exempt = tax_pending.1.clone();
    exempt.snapshot["taxAmount"] = json!(0);
    full["total_excluding_tax"] = json!(1000);
    assert_eq!(
        verify_credit_note(&full, &exempt, "in_tax", Some(1000)).unwrap(),
        0
    );

    let sale = seed(&db, "sale", "buyer", "app", false).await;
    let (one, two) = tokio::join!(deliver(&db, &sale), deliver(&db, &sale));
    one.unwrap();
    two.unwrap();
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "Membership" WHERE "userId"='buyer'"#
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_sale'"#
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AppPurchase" WHERE id='order_sale'"#
        )
        .await,
        1
    );
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "PaymentLedgerEntry" WHERE "stripeObjectId"='ch_sale' AND perspective='CUSTOMER' AND kind='CHARGE'"#).await,1);
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "PaymentOutbox" WHERE "sourceId"='order_sale' AND effect='marketplace_paid'"#).await,1);
    let confirmed = load_order(&db, &sale.0.id).await.unwrap();
    let confirmed_at = confirmed.snapshot["confirmed_at"].as_i64().unwrap();
    let deadline = confirmed.snapshot["withdrawal_deadline"].as_i64().unwrap();
    assert_eq!(deadline - confirmed_at, 14 * 86_400_000);
    assert!(deadline > sale.0.snapshot["withdrawal_deadline"].as_i64().unwrap());
    deliver(&db, &sale).await.unwrap();
    let replayed = load_order(&db, &sale.0.id).await.unwrap();
    assert_eq!(
        replayed.snapshot["confirmed_at"],
        confirmed.snapshot["confirmed_at"]
    );
    assert_eq!(
        replayed.snapshot["withdrawal_deadline"],
        confirmed.snapshot["withdrawal_deadline"]
    );

    let duplicate = seed(&db, "duplicate", "buyer", "app", false).await;
    deliver(&db, &duplicate).await.unwrap();
    let orphan = payment_attempt::Entity::find_by_id(&duplicate.1.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(orphan.orphaned);
    observe(&db, &orphan.id, "re_duplicate", 1000, "SUCCEEDED").await;
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "Membership" WHERE "userId"='buyer'"#
        )
        .await,
        1
    );
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_sale' AND status='ACTIVE'"#).await,1);

    let canceled = seed(&db, "canceled", "canceled-buyer", "other-app", true).await;
    deliver(&db, &canceled).await.unwrap();
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_canceled'"#
        )
        .await,
        0
    );
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "PaymentOutbox" WHERE "sourceId"='attempt_canceled' AND effect='marketplace_orphan_refund'"#).await,1);

    let racing = seed(&db, "racing", "racing-buyer", "refund-app", false).await;
    deliver(&db, &racing).await.unwrap();
    let (first, second) = tokio::join!(
        reserve(&db, &racing.1.id, "first", 700),
        reserve(&db, &racing.1.id, "second", 700)
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let command = first
        .as_ref()
        .ok()
        .or(second.as_ref().ok())
        .unwrap()
        .clone();
    let local = payment_refund::Entity::find_by_id(&command)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let observed = observe(&db, &racing.1.id, "re_racing", 700, "SUCCEEDED").await;
    let txn = db.begin().await.unwrap();
    bind_refund_result(&txn, &local, &observed).await.unwrap();
    txn.commit().await.unwrap();
    let after = payment_attempt::Entity::find_by_id(&racing.1.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        (after.refunded_amount, after.reserved_refund_amount),
        (700, 0)
    );
    assert_eq!(
        payment_refund::Entity::find_by_id(&command)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "RECONCILED"
    );
    observe(&db, &racing.1.id, "re_racing", 700, "PENDING").await;
    assert_eq!(
        payment_attempt::Entity::find_by_id(&racing.1.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .refunded_amount,
        700
    );
    let suffix = if first.is_ok() { "first" } else { "second" };
    assert!(reserve(&db, &racing.1.id, suffix, 700).await.is_ok());
    assert_eq!(
        reserve(&db, &racing.1.id, suffix, 600)
            .await
            .unwrap_err()
            .public_code(),
        "PAYMENT_IDEMPOTENCY_CONFLICT"
    );

    db.execute_raw(sql(r#"INSERT INTO "AccessGrant" (id,"userId","itemKind","itemId","sourceType","sourceId","createdAt","updatedAt") VALUES ('manual','buyer','APP','app','COMP','manual',0,0)"#,vec![])).await.unwrap();
    observe(&db, &sale.1.id, "re_sale", 1000, "SUCCEEDED").await;
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "Membership" WHERE "userId"='buyer'"#
        )
        .await,
        1
    );
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_sale' AND status='REVOKED'"#).await,1);
    deliver(&db, &sale).await.unwrap();
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_sale' AND status='ACTIVE'"#).await,0);
    observe(&db, &racing.1.id, "re_racing", 700, "FAILED").await;
    let returned = payment_attempt::Entity::find_by_id(&racing.1.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(returned.refunded_amount, 0);
    assert_eq!(
        returned
            .snapshot
            .get("refundReviewRequired")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "PaymentLedgerEntry" WHERE "stripeObjectId"='re_racing' AND kind='REFUND_RETURNED'"#).await,1);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "PaymentOutbox" WHERE effect='payment_refund_review'"#
        )
        .await,
        1
    );
    observe(&db, &racing.1.id, "re_racing", 700, "SUCCEEDED").await;
    assert_eq!(
        payment_attempt::Entity::find_by_id(&racing.1.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .refunded_amount,
        0
    );

    let fx = seed(&db, "fx", "fx-buyer", "fx-app", false).await;
    deliver(&db, &fx).await.unwrap();
    observe_fx_dispute(&db, &fx.1.id, "lost").await;
    let reviewed = payment_attempt::Entity::find_by_id(&fx.1.id)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reviewed.snapshot["disputeReviewRequired"], json!(true));
    assert_eq!(
        reviewed.snapshot["disputeCurrencyReview"]["currency"],
        json!("eur")
    );
    assert_eq!(
        reviewed.snapshot["disputeCurrencyReview"]["balance_transactions"][0]["currency"],
        json!("usd")
    );
    assert_eq!(
        reviewed.snapshot["disputeCurrencyReview"]["balance_transactions"][0]["amount"],
        json!(-1080)
    );
    assert!(reviewed.snapshot.pointer("/dispute/outstanding").is_none());
    assert_eq!(
        require_dispute_settlement_currency(&reviewed.snapshot)
            .unwrap_err()
            .public_code(),
        "PAYMENT_DISPUTE_REQUIRES_REVIEW"
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_fx' AND status='REVOKED'"#
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "Membership" WHERE "userId"='fx-buyer'"#
        )
        .await,
        0
    );
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "PaymentLedgerEntry" WHERE "attemptId"='attempt_fx' AND kind='DISPUTE_CASH'"#).await,0);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "PaymentAdjustment" WHERE "attemptId"='attempt_fx'"#
        )
        .await,
        0
    );
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "PaymentOutbox" WHERE "sourceId"='attempt_fx' AND effect='payment_dispute_review'"#).await,1);
    observe_fx_dispute(&db, &fx.1.id, "needs_response").await;
    deliver(&db, &fx).await.unwrap();
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_fx' AND status='ACTIVE'"#
        )
        .await,
        0
    );
    assert_eq!(
        payment_attempt::Entity::find_by_id(&fx.1.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .snapshot["dispute"]["status"],
        json!("lost")
    );

    let fx_won = seed(&db, "fx-won", "fx-won-buyer", "fx-won-app", false).await;
    deliver(&db, &fx_won).await.unwrap();
    observe_fx_dispute(&db, &fx_won.1.id, "won").await;
    observe_fx_dispute(&db, &fx_won.1.id, "lost").await;
    assert_eq!(count(&db,r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_fx-won' AND status='ACTIVE'"#).await,1);
    assert_eq!(
        payment_attempt::Entity::find_by_id(&fx_won.1.id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .snapshot["dispute"]["status"],
        json!("won")
    );

    let platform = seed_platform(&db, "platform").await;
    let (first, second) = tokio::join!(deliver(&db, &platform), deliver(&db, &platform));
    first.unwrap();
    second.unwrap();
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "ConnectedAccount" WHERE "userId"='platform-admin'"#
        )
        .await,
        0
    );
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "AppPurchase" WHERE id='order_platform' AND "chargeType"='PLATFORM'"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentLedgerEntry" WHERE "attemptId"='attempt_platform' AND (perspective='SELLER' OR kind='APPLICATION_FEE')"#).await, 0);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_platform' AND "hydrationStatus"='READY'"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COALESCE(SUM(amount),0)::bigint AS count FROM "PaymentLedgerEntry" WHERE "attemptId"='attempt_platform' AND perspective='PLATFORM'"#).await, 800);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentLedgerEntry" WHERE "attemptId"='attempt_platform' AND perspective='TAX' AND amount=200"#).await, 1);

    // Refund authority follows the original recipient, including after admin removal.
    let txn = db.begin().await.unwrap();
    authorize_seller_refund(&txn, &platform.1.id, "platform-admin")
        .await
        .unwrap();
    txn.rollback().await.unwrap();
    db.execute_unprepared(r#"UPDATE "User" SET permission=0 WHERE id='platform-admin'"#)
        .await
        .unwrap();
    let txn = db.begin().await.unwrap();
    assert!(
        authorize_seller_refund(&txn, &platform.1.id, "platform-admin")
            .await
            .is_err()
    );
    txn.rollback().await.unwrap();
    deliver(&db, &platform).await.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentOrder" WHERE id='order_platform' AND "chargeType"='PLATFORM' AND "connectedAccountId" IS NULL"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_platform' AND status='ACTIVE'"#).await, 1);
    let refund = reserve(&db, &platform.1.id, "platform-refund", 1000)
        .await
        .unwrap();
    let refund = payment_refund::Entity::find_by_id(refund)
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert!(!refund.refund_application_fee);
    let operation = stripe_operation::Entity::find_by_id(refund.operation_id.unwrap())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    let command: StripeRequest = serde_json::from_value(operation.request_params).unwrap();
    assert!(command.scope.connected_account_id.is_none());
    assert!(command.parameters.get("refund_application_fee").is_none());
    observe(&db, &platform.1.id, "re_platform", 1000, "SUCCEEDED").await;
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentLedgerEntry" WHERE "attemptId"='attempt_platform' AND kind='REFUND' AND perspective='PLATFORM' AND amount=-1000"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_platform' AND status='REVOKED'"#).await, 1);
    deliver(&db, &platform).await.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_platform' AND status='ACTIVE'"#).await, 0);

    // A role transition before capture cancels delivery rather than changing the Stripe recipient.
    let removed = seed_platform(&db, "removed-admin").await;
    db.execute_unprepared(r#"UPDATE "User" SET permission=0 WHERE id='platform-admin'"#)
        .await
        .unwrap();
    deliver(&db, &removed).await.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_removed-admin' AND orphaned=true"#).await, 1);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_removed-admin'"#
        )
        .await,
        0
    );
    let promoted = seed(&db, "promoted", "promoted-buyer", "promoted-app", false).await;
    db.execute_unprepared(r#"UPDATE "User" SET permission=1 WHERE id='seller'"#)
        .await
        .unwrap();
    deliver(&db, &promoted).await.unwrap();
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_promoted' AND orphaned=true"#
        )
        .await,
        1
    );
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentOrder" WHERE id='order_promoted' AND "connectedAccountId"='acct_seller' AND "chargeType"='DESTINATION'"#).await, 1);
    db.execute_unprepared(r#"UPDATE "User" SET permission=0 WHERE id='seller'"#)
        .await
        .unwrap();

    let disputed = seed_platform(&db, "platform-disputed").await;
    deliver(&db, &disputed).await.unwrap();
    observe_fx_dispute(&db, &disputed.1.id, "lost").await;
    observe_fx_dispute(&db, &disputed.1.id, "needs_response").await;
    deliver(&db, &disputed).await.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_platform-disputed' AND status='REVOKED'"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAdjustment" WHERE "attemptId"='attempt_platform-disputed' AND purpose IN ('SELLER_RECOVERY','SELLER_RESTORE','FEE_REFUND')"#).await, 0);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_platform-disputed' AND snapshot->'dispute'->>'status'='lost' AND snapshot->>'platform_owned'='true'"#).await, 1);

    let package_sale = seed_package(&db, "package-sale", "package-buyer", "pkg").await;
    let (one, two) = tokio::join!(deliver(&db, &package_sale), deliver(&db, &package_sale));
    one.unwrap();
    two.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "WasmPackageUser" WHERE "userId"='package-buyer' AND "packageId"='pkg' AND permission=8 AND "grantedBy"='payment:order_package-sale'"#).await, 1);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "Membership" WHERE "userId"='package-buyer'"#
        )
        .await,
        0
    );
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "AccessGrant" WHERE "sourceId"='order_package-sale' AND "itemKind"='PACKAGE' AND status='ACTIVE'"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "WasmPackagePurchase" WHERE id='order_package-sale' AND "paymentOrderId"='order_package-sale' AND status='COMPLETED' AND "pricePaid"=1000"#).await, 1);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "AppPurchase" WHERE id='order_package-sale'"#
        )
        .await,
        0
    );
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentEntitlement" WHERE "userId"='package-buyer' AND "itemKind"='PACKAGE' AND "itemId"='pkg'"#).await, 1);

    observe(
        &db,
        &package_sale.1.id,
        "re_package-sale",
        1000,
        "SUCCEEDED",
    )
    .await;
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "WasmPackageUser" WHERE "userId"='package-buyer' AND "packageId"='pkg'"#).await, 0);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "WasmPackageUser" WHERE "userId"='seller' AND "packageId"='pkg' AND permission=1"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "WasmPackagePurchase" WHERE id='order_package-sale' AND status='REFUNDED'"#).await, 1);
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentOutbox" WHERE effect='package_access_changed' AND "sourceId"='pkg' AND payload->>'userId'='package-buyer'"#).await, 1);

    let owned = seed_package(&db, "package-owned", "package-holder", "pkg-owned").await;
    db.execute_unprepared(r#"INSERT INTO "WasmPackageUser" (id,"packageId","userId",permission,"grantedAt") VALUES ('holder','pkg-owned','package-holder',4,now())"#).await.unwrap();
    deliver(&db, &owned).await.unwrap();
    assert_eq!(count(&db, r#"SELECT COUNT(*) FROM "PaymentAttempt" WHERE id='attempt_package-owned' AND orphaned=true"#).await, 1);
    assert_eq!(
        count(
            &db,
            r#"SELECT COUNT(*) FROM "WasmPackagePurchase" WHERE id='order_package-owned'"#
        )
        .await,
        0
    );
}
