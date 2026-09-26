use async_trait::async_trait;
use chrono::Utc;
use flow_like_api::{
    entity::{payment_outbox, stripe_operation},
    payments::{operations, outbox},
    stripe_connect::{
        RetryDisposition, StripeError, StripeGateway, StripeRequest, StripeResponse, StripeScope,
    },
};
use flow_like_types::tokio::{
    self,
    sync::{Mutex, Notify, OnceCell},
};
use sea_orm::{
    ConnectionTrait, Database, DatabaseConnection, EntityTrait, Statement, TransactionTrait,
};
use serde_json::json;
use std::{collections::VecDeque, sync::Arc};

static SCHEMA: OnceCell<()> = OnceCell::const_new();

async fn database() -> DatabaseConnection {
    let url = std::env::var("PAYMENTS_TEST_DATABASE_URL")
        .expect("PAYMENTS_TEST_DATABASE_URL must point at an empty disposable PostgreSQL database");
    let db = Database::connect(url).await.unwrap();
    SCHEMA.get_or_init(||async {
        db.execute_unprepared(r#"
CREATE TABLE "User" (id TEXT PRIMARY KEY, permission BIGINT NOT NULL DEFAULT 0);
CREATE TABLE "App" (id TEXT PRIMARY KEY);
CREATE TABLE "WasmPackage" (id TEXT PRIMARY KEY);
CREATE TABLE "AppDiscount" (id TEXT PRIMARY KEY);
CREATE TABLE "JoinQueue" (id TEXT PRIMARY KEY);
CREATE TABLE "AppPurchase" (id TEXT PRIMARY KEY,"userId" TEXT REFERENCES "User"(id) ON DELETE CASCADE,"appId" TEXT REFERENCES "App"(id) ON DELETE CASCADE,"discountId" TEXT REFERENCES "AppDiscount"(id) ON DELETE SET NULL,"stripeSessionId" TEXT NOT NULL);
CREATE TABLE "WasmPackagePurchase" (id TEXT PRIMARY KEY,"userId" TEXT REFERENCES "User"(id) ON DELETE CASCADE,"packageId" TEXT REFERENCES "WasmPackage"(id) ON DELETE CASCADE,"stripeSessionId" TEXT NOT NULL);
"#).await.expect("database must be empty");
        db.execute_unprepared(include_str!("../prisma/migrations/20260920120000_payments_foundations/migration.sql")).await.unwrap();
    }).await;
    db
}

struct Fake {
    requests: Mutex<Vec<StripeRequest>>,
    replies: Mutex<VecDeque<Result<StripeResponse, StripeError>>>,
    pause: Option<(Arc<Notify>, Arc<Notify>)>,
}
impl Fake {
    fn new(replies: Vec<Result<StripeResponse, StripeError>>) -> Self {
        Self {
            requests: Mutex::new(vec![]),
            replies: Mutex::new(replies.into()),
            pause: None,
        }
    }
}
#[async_trait]
impl StripeGateway for Fake {
    async fn execute(&self, request: &StripeRequest) -> Result<StripeResponse, StripeError> {
        self.requests.lock().await.push(request.clone());
        if let Some((started, resume)) = &self.pause {
            started.notify_one();
            resume.notified().await;
        }
        self.replies
            .lock()
            .await
            .pop_front()
            .expect("unexpected extra Stripe request")
    }
}
fn request(key: &str) -> StripeRequest {
    StripeRequest::post(
        StripeScope::platform("acct_test", false),
        "/v1/refunds",
        json!({"charge":"ch_test","amount":321}),
        key,
    )
}
fn response() -> Result<StripeResponse, StripeError> {
    Ok(StripeResponse {
        body: json!({"id":"re_test","status":"succeeded"}),
        request_id: Some("req_test".into()),
    })
}
async fn due(db: &DatabaseConnection, id: &str) {
    db.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        r#"UPDATE "StripeOperation" SET "nextAttemptAt"=0 WHERE id=$1"#,
        [id.into()],
    ))
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn concurrent_delivery_holds_one_lease_and_replays_the_recorded_response() {
    let connection = database().await;
    let db = &connection;
    let request = request("test:concurrent");
    let id = operations::prepare(db, "TEST", "concurrent", "REFUND", &request)
        .await
        .unwrap();
    let started = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    let fake = Fake {
        pause: Some((started.clone(), resume.clone())),
        ..Fake::new(vec![response()])
    };
    let first = operations::execute_with_gateway(db, &id, &fake);
    let second = async {
        started.notified().await;
        let error = operations::execute_with_gateway(db, &id, &fake)
            .await
            .unwrap_err();
        assert_eq!(error.public_code(), "PAYMENT_OPERATION_PENDING");
        resume.notify_one();
    };
    let (first, ()) = tokio::join!(first, second);
    assert!(first.is_ok());
    assert!(
        operations::execute_with_gateway(db, &id, &fake)
            .await
            .is_ok()
    );
    assert_eq!(fake.requests.lock().await.len(), 1);
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn a_lost_response_retries_the_same_key_and_exact_parameters() {
    let connection = database().await;
    let db = &connection;
    let request = request("test:lost");
    let id = operations::prepare(db, "TEST", "lost", "REFUND", &request)
        .await
        .unwrap();
    let fake = Fake::new(vec![
        Err(StripeError::Transport {
            retry: RetryDisposition::SameOperation,
        }),
        response(),
    ]);
    assert!(
        operations::execute_with_gateway(db, &id, &fake)
            .await
            .is_err()
    );
    due(db, &id).await;
    assert!(
        operations::execute_with_gateway(db, &id, &fake)
            .await
            .is_ok()
    );
    let requests = fake.requests.lock().await;
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].idempotency_key, requests[1].idempotency_key);
    assert_eq!(requests[0].digest().unwrap(), requests[1].digest().unwrap());
    let mut conflicting = request.clone();
    conflicting.parameters = json!({"charge":"ch_test","amount":999});
    assert_eq!(
        operations::prepare(db, "TEST", "lost", "REFUND", &conflicting)
            .await
            .unwrap_err()
            .public_code(),
        "PAYMENT_IDEMPOTENCY_CONFLICT"
    );
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn indeterminate_provider_errors_are_not_automatically_replayed() {
    let connection = database().await;
    let db = &connection;
    let id = operations::prepare(db, "TEST", "unknown", "REFUND", &request("test:unknown"))
        .await
        .unwrap();
    let fake = Fake::new(vec![Err(StripeError::Api {
        status: 500,
        code: None,
        request_id: Some("req_unknown".into()),
        retry: RetryDisposition::Reconcile,
        retry_after_seconds: None,
    })]);
    assert!(
        operations::execute_with_gateway(db, &id, &fake)
            .await
            .is_err()
    );
    assert_eq!(
        operations::execute_with_gateway(db, &id, &fake)
            .await
            .unwrap_err()
            .public_code(),
        "PAYMENT_OPERATION_REVIEW"
    );
    assert_eq!(fake.requests.lock().await.len(), 1);
    let row = stripe_operation::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, "INDETERMINATE");
    assert_eq!(row.request_id.as_deref(), Some("req_unknown"));
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn idempotency_expiry_never_creates_a_fresh_charge() {
    let connection = database().await;
    let db = &connection;
    let id = operations::prepare(db, "TEST", "expired", "REFUND", &request("test:expired"))
        .await
        .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        r#"UPDATE "StripeOperation" SET attempts=1,"firstAttemptAt"=$2 WHERE id=$1"#,
        vec![
            id.clone().into(),
            (Utc::now().timestamp_millis() - operations::REPLAY_WINDOW_MS - 1).into(),
        ],
    ))
    .await
    .unwrap();
    let fake = Fake::new(vec![]);
    assert_eq!(
        operations::execute_with_gateway(db, &id, &fake)
            .await
            .unwrap_err()
            .public_code(),
        "PAYMENT_OPERATION_REVIEW"
    );
    assert!(fake.requests.lock().await.is_empty());
    assert_eq!(
        stripe_operation::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "MANUAL_REVIEW"
    );
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn outbox_commit_rollback_and_unique_identity_are_atomic() {
    let connection = database().await;
    let db = &connection;
    let txn = db.begin().await.unwrap();
    outbox::enqueue(
        &txn,
        "test:rollback",
        "REFUND",
        "TEST",
        "rollback",
        json!({"amount":300}),
    )
    .await
    .unwrap();
    txn.rollback().await.unwrap();
    let id = blake3::hash(b"test:rollback").to_hex().to_string();
    assert!(
        payment_outbox::Entity::find_by_id(&id)
            .one(db)
            .await
            .unwrap()
            .is_none()
    );
    let txn = db.begin().await.unwrap();
    outbox::enqueue(
        &txn,
        "test:rollback",
        "REFUND",
        "TEST",
        "rollback",
        json!({"amount":300}),
    )
    .await
    .unwrap();
    txn.commit().await.unwrap();
    outbox::enqueue(
        db,
        "test:rollback",
        "REFUND",
        "TEST",
        "rollback",
        json!({"amount":300}),
    )
    .await
    .unwrap();
    assert!(
        outbox::enqueue(
            db,
            "test:rollback",
            "REFUND",
            "TEST",
            "rollback",
            json!({"amount":301})
        )
        .await
        .is_err()
    );
    assert_eq!(
        payment_outbox::Entity::find_by_id(id)
            .one(db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "PENDING"
    );
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn global_admin_cannot_create_accounts_or_replay_onboarding_links() {
    let db = database().await;
    db.execute_unprepared(r#"
INSERT INTO "User" (id,permission) VALUES ('onboarding-owner',1);
INSERT INTO "ConnectedAccount" (id,"userId","platformAccountId",livemode,purpose,generation,"creationParams",country,"createdAt","updatedAt") VALUES ('onboarding-account','onboarding-owner','acct_test',false,'shared',1,'{}','DE',0,0);
"#).await.unwrap();
    let create = StripeRequest::post(
        StripeScope::platform("acct_test", false),
        "/v1/accounts",
        json!({"country":"DE"}),
        "test:admin-create",
    );
    let create_id = operations::prepare(
        &db,
        "CONNECTED_ACCOUNT",
        "onboarding-account",
        "CREATE_ACCOUNT",
        &create,
    )
    .await
    .unwrap();
    let fake = Fake::new(vec![response()]);
    assert_eq!(
        operations::execute_with_gateway(&db, &create_id, &fake)
            .await
            .unwrap_err()
            .public_code(),
        "PLATFORM_PAYMENTS_MANAGED"
    );
    assert!(fake.requests.lock().await.is_empty());
    assert_eq!(
        stripe_operation::Entity::find_by_id(&create_id)
            .one(&db)
            .await
            .unwrap()
            .unwrap()
            .status,
        "FAILED"
    );

    db.execute_unprepared(r#"UPDATE "User" SET permission=0 WHERE id='onboarding-owner'"#)
        .await
        .unwrap();
    let link = StripeRequest::post(
        StripeScope::platform("acct_test", false),
        "/v1/account_links",
        json!({"account":"acct_seller","type":"account_onboarding","return_url":"https://flow-like.com/payments/return","refresh_url":"https://flow-like.com/payments/return?r=test"}),
        "test:admin-link",
    );
    let link_id = operations::prepare(
        &db,
        "CONNECTED_ACCOUNT",
        "onboarding-account",
        "ACCOUNT_LINK",
        &link,
    )
    .await
    .unwrap();
    operations::execute_with_gateway(&db, &link_id, &fake)
        .await
        .unwrap();
    db.execute_unprepared(r#"UPDATE "User" SET permission=1 WHERE id='onboarding-owner'"#)
        .await
        .unwrap();
    assert_eq!(
        operations::execute_with_gateway(&db, &link_id, &fake)
            .await
            .unwrap_err()
            .public_code(),
        "PLATFORM_PAYMENTS_MANAGED"
    );
    assert_eq!(fake.requests.lock().await.len(), 1);

    let refund = request("test:promoted-owner-refund");
    let refund_id = operations::prepare(
        &db,
        "PAYMENT_ATTEMPT",
        "historical-payment",
        "REFUND",
        &refund,
    )
    .await
    .unwrap();
    let fake = Fake::new(vec![response()]);
    operations::execute_with_gateway(&db, &refund_id, &fake)
        .await
        .unwrap();
    assert_eq!(fake.requests.lock().await.len(), 1);
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn deleted_owner_cached_account_creation_remains_readable() {
    let db = database().await;
    db.execute_unprepared(r#"
INSERT INTO "User" (id,permission) VALUES ('deleted-onboarding-owner',0);
INSERT INTO "ConnectedAccount" (id,"userId","platformAccountId",livemode,purpose,generation,"creationParams",country,"createdAt","updatedAt") VALUES ('deleted-onboarding-account','deleted-onboarding-owner','acct_test',false,'shared',1,'{}','DE',0,0);
"#).await.unwrap();
    let create = StripeRequest::post(
        StripeScope::platform("acct_test", false),
        "/v1/accounts",
        json!({"country":"DE"}),
        "test:deleted-owner-create",
    );
    let id = operations::prepare(
        &db,
        "CONNECTED_ACCOUNT",
        "deleted-onboarding-account",
        "CREATE_ACCOUNT",
        &create,
    )
    .await
    .unwrap();
    let fake = Fake::new(vec![Ok(StripeResponse {
        body: json!({"id":"acct_created"}),
        request_id: Some("req_account_created".into()),
    })]);
    operations::execute_with_gateway(&db, &id, &fake)
        .await
        .unwrap();
    db.execute_unprepared(r#"DELETE FROM "User" WHERE id='deleted-onboarding-owner'"#)
        .await
        .unwrap();
    let cached = operations::execute_with_gateway(&db, &id, &fake)
        .await
        .unwrap();
    assert_eq!(cached.body["id"], "acct_created");
    assert_eq!(fake.requests.lock().await.len(), 1);
}
