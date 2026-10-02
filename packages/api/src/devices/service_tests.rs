use super::*;
use axum::http::{HeaderValue, StatusCode};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use management::AccessRole;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
    Value,
};
use std::collections::BTreeSet;
use view::{
    DeviceEnrollmentState, DeviceLimits, DeviceRelationship, DeviceUsage, DeviceUsageView,
    DeviceView, EnrollmentFilter,
};

const API_BASE: &str = "https://device-test.example/api/v1";
const HEARTBEAT: &str = "/devices/device/heartbeat";

const VIEW_FIELDS: [&str; 15] = [
    "device_id",
    "owner_id",
    "name",
    "display_name",
    "identity",
    "status",
    "registered_at",
    "last_seen_at",
    "auth_epoch",
    "revoked_at",
    "relationship",
    "access_expires_at",
    "access_rules_expire_at",
    "cloud_approvals",
    "auth_rejection",
];

fn sql(query: &str, values: impl IntoIterator<Item = Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, query, values)
}

fn assert_status<T>(result: Result<T, ApiError>, expected: StatusCode) {
    assert_eq!(result.err().expect("request must fail").status(), expected);
}

fn assertion_at(device_id: &str, path: &str, key: &SigningKey, issued_at: i64) -> String {
    sign_client_assertion(
        &ClientAssertion {
            iss: device_id.into(),
            sub: device_id.into(),
            aud: endpoint_url(API_BASE, path).unwrap(),
            iat: issued_at,
            nbf: issued_at,
            exp: issued_at + 60,
            jti: uuid::Uuid::new_v4().to_string(),
        },
        key,
    )
    .unwrap()
}

fn assertion(device_id: &str, path: &str, key: &SigningKey) -> String {
    assertion_at(device_id, path, key, chrono::Utc::now().timestamp())
}

fn signed_request(
    token: &str,
    key: &SigningKey,
    method: &str,
    path: &str,
    issued_at: i64,
) -> HeaderMap {
    let proof = sign_dpop(
        &DpopProof {
            jti: uuid::Uuid::new_v4().to_string(),
            htm: method.into(),
            htu: endpoint_url(API_BASE, path).unwrap(),
            iat: issued_at,
            ath: Some(access_token_hash(token)),
            nonce: None,
        },
        key,
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_str(&format!("DPoP {token}")).unwrap(),
    );
    headers.insert("dpop", HeaderValue::from_str(&proof).unwrap());
    headers
}

fn request_headers_at(token: &str, key: &SigningKey, path: &str, issued_at: i64) -> HeaderMap {
    signed_request(token, key, "POST", path, issued_at)
}

fn request_headers(token: &str, key: &SigningKey, path: &str) -> HeaderMap {
    request_headers_at(token, key, path, chrono::Utc::now().timestamp())
}

/// Rewrites the signed issue time without the key, which breaks the signature.
fn with_forged_issue_time(mut headers: HeaderMap, issued_at: i64) -> HeaderMap {
    let proof = headers["dpop"].to_str().unwrap().to_owned();
    let mut segments = proof.split('.').map(str::to_owned).collect::<Vec<_>>();
    let mut claims: DpopProof =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(&segments[1]).unwrap()).unwrap();
    claims.iat = issued_at;
    segments[1] = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims).unwrap());
    headers.insert("dpop", HeaderValue::from_str(&segments.join(".")).unwrap());
    headers
}

/// A disposable schema holding the registry, access-rule, cloud-approval and device
/// console tables. Every pool connection selects it before its first query.
struct TestDatabase {
    admin: DatabaseConnection,
    db: DatabaseConnection,
    schema: String,
}

impl TestDatabase {
    async fn create(active_users: &[&str]) -> Self {
        let url = std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL")
            .expect("set FLOW_LIKE_DEVICE_TEST_DATABASE_URL to a disposable PostgreSQL server");
        let admin = Database::connect(&url).await.unwrap();
        let schema = format!("device_service_test_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute_unprepared(&format!("CREATE SCHEMA {schema}"))
            .await
            .unwrap();
        let mut scoped_url = reqwest::Url::parse(&url).unwrap();
        scoped_url
            .query_pairs_mut()
            .append_pair("options", &format!("-c search_path={schema}"));
        let mut options = ConnectOptions::new(scoped_url.to_string());
        options.max_connections(2).min_connections(1);
        let db = Database::connect(options).await.unwrap();
        for statement in [
            include_str!("../../prisma/migrations/20260921120000_standalone_devices/migration.sql"),
            include_str!("../../prisma/migrations/20260921140000_instance_resources/migration.sql"),
            include_str!("../../prisma/migrations/20260922120000_device_management/migration.sql"),
            include_str!("../../prisma/migrations/20261001120000_device_console/migration.sql"),
        ]
        .into_iter()
        .flat_map(|migration| migration.split(';'))
        .filter(|statement| !statement.trim().is_empty())
        {
            db.execute_unprepared(statement).await.unwrap();
        }
        db.execute_unprepared(r#"CREATE TABLE "User" (id TEXT PRIMARY KEY, status TEXT NOT NULL, "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#).await.unwrap();
        for user in active_users {
            db.execute_raw(sql(
                r#"INSERT INTO "User" (id,status) VALUES ($1,'ACTIVE')"#,
                [(*user).into()],
            ))
            .await
            .unwrap();
        }
        Self { admin, db, schema }
    }

    async fn discard(self) {
        self.db.close().await.unwrap();
        self.admin
            .execute_unprepared(&format!("DROP SCHEMA {} CASCADE", self.schema))
            .await
            .unwrap();
        self.admin.close().await.unwrap();
    }
}

fn hub(enabled: bool) -> StandaloneConfig {
    StandaloneConfig {
        enabled,
        api_base_url: Some(API_BASE.into()),
        ..StandaloneConfig::default()
    }
}

fn device_context<'a>(
    db: &'a DatabaseConnection,
    config: &'a StandaloneConfig,
) -> DeviceContext<'a> {
    DeviceContext {
        db,
        dialect: crate::db::DbDialect::Postgres,
        config,
        domain: "unused.example",
        secure: true,
    }
}

fn setup_name(device_id: &str) -> String {
    format!("{device_id} as set up")
}

/// Stores a registration the way a redeemed enrollment leaves it and returns the
/// device's authentication key.
async fn register(
    db: &DatabaseConnection,
    device_id: &str,
    owner: &str,
    registered_at: i64,
) -> SigningKey {
    let auth = SigningKey::generate();
    let identity = DeviceIdentity {
        auth_key: auth.public_key(),
        management_key: [42; 32],
        telemetry_key: SigningKey::generate().public_key(),
    };
    let receipt = DeviceReceipt {
        enrollment_id: format!("enrollment-{device_id}"),
        device_id: device_id.into(),
        owner_id: owner.into(),
        name: setup_name(device_id),
        identity: identity.clone(),
        manifest_jws: "signed-manifest-fixture".into(),
        binding_jws: "signed-binding-fixture".into(),
        registered_at,
        auth_epoch: 1,
    };
    db.execute_raw(sql(
        r#"INSERT INTO "ManagedDevice" (id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES ($1,$2,$3,'active',1,$4,$5,$6)"#,
        [
            device_id.into(),
            owner.into(),
            setup_name(device_id).into(),
            serde_json::to_string(&identity).unwrap().into(),
            serde_json::to_string(&receipt).unwrap().into(),
            registered_at.into(),
        ],
    ))
    .await
    .unwrap();
    auth
}

/// One version of a device's access rules as `persist_policy` stores it.
async fn access_rules(
    db: &DatabaseConnection,
    device_id: &str,
    version: i64,
    expires_at: i64,
    recipients: &[(&str, i64)],
) {
    stored_rules(
        db,
        device_id,
        version,
        "policy-jws-fixture",
        expires_at,
        recipients,
    )
    .await;
}

/// Rows written past `persist_policy`, for states it refuses to store.
async fn stored_rules(
    db: &DatabaseConnection,
    device_id: &str,
    version: i64,
    policy_jws: &str,
    expires_at: i64,
    recipients: &[(&str, i64)],
) {
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceManagementPolicy" ("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES ($1,$2,'digest',$3,$4,$4)"#,
        [
            device_id.into(),
            version.into(),
            policy_jws.into(),
            expires_at.into(),
        ],
    ))
    .await
    .unwrap();
    for (user, grant_expires_at) in recipients {
        db.execute_raw(sql(
            r#"INSERT INTO "DeviceManagementRecipient" ("deviceId",version,"grantId","userId","expiresAt") VALUES ($1,$2,$3,$4,$5)"#,
            [
                device_id.into(),
                version.into(),
                format!("grant-{user}").into(),
                (*user).into(),
                (*grant_expires_at).into(),
            ],
        ))
        .await
        .unwrap();
    }
}

async fn cloud_access(
    db: &DatabaseConnection,
    grant_id: &str,
    device_id: &str,
    delegator: &str,
    status: &str,
    expires_at: i64,
) {
    db.execute_raw(sql(
        r#"INSERT INTO "PlacementResourceGrant" (id,"deviceId","placementId","deploymentId","projectId","appId","delegatingUserId","approvedByUserId",status,"authzVersion","modelIds","maxInstances","expiresAt","createdAt") VALUES ($1,$2,'placement','deployment','project',NULL,$3,$3,$4,1,'[]',1,$5,$5)"#,
        [
            grant_id.into(),
            device_id.into(),
            delegator.into(),
            status.into(),
            expires_at.into(),
        ],
    ))
    .await
    .unwrap();
}

async fn spending(db: &DatabaseConnection, grant_id: &str, payer: &str, expires_at: i64) {
    db.execute_raw(sql(
        r#"INSERT INTO "PlacementBillingGrant" (id,"grantId","payerId","approvedByUserId",status,"authzVersion","limitMicros","usedMicros","reservedMicros","expiresAt","createdAt") VALUES ($1,$2,$3,$3,'active',1,1000,0,0,$4,$4)"#,
        [
            format!("billing-{grant_id}-{payer}").into(),
            grant_id.into(),
            payer.into(),
            expires_at.into(),
        ],
    ))
    .await
    .unwrap();
}

#[derive(Debug, PartialEq)]
struct StoredRejection {
    code: String,
    skew_seconds: Option<i64>,
    count: i64,
    first_at: i64,
    last_at: i64,
}

async fn stored_rejection(db: &DatabaseConnection, device_id: &str) -> Option<StoredRejection> {
    db.query_one_raw(sql(
        r#"SELECT code,"skewSeconds",count,"firstAt","lastAt" FROM "DeviceAuthRejection" WHERE "deviceId" = $1"#,
        [device_id.into()],
    ))
    .await
    .unwrap()
    .map(|row| StoredRejection {
        code: row.try_get("", "code").unwrap(),
        skew_seconds: row.try_get("", "skewSeconds").unwrap(),
        count: row.try_get("", "count").unwrap(),
        first_at: row.try_get("", "firstAt").unwrap(),
        last_at: row.try_get("", "lastAt").unwrap(),
    })
}

/// Moves the stored rejection into the past, beyond the write throttle.
async fn age_rejection(db: &DatabaseConnection, seconds: i64) {
    db.execute_raw(sql(
        r#"UPDATE "DeviceAuthRejection" SET "firstAt" = "firstAt" - $1, "lastAt" = "lastAt" - $1"#,
        [seconds.into()],
    ))
    .await
    .unwrap();
}

async fn set_last_seen(db: &DatabaseConnection, device_id: &str, last_seen_at: i64) {
    db.execute_raw(sql(
        r#"UPDATE "ManagedDevice" SET "lastSeenAt" = $1 WHERE id = $2"#,
        [last_seen_at.into(), device_id.into()],
    ))
    .await
    .unwrap();
}

fn find<'a>(devices: &'a [DeviceView], device_id: &str) -> &'a DeviceView {
    devices
        .iter()
        .find(|device| device.device_id == device_id)
        .unwrap_or_else(|| panic!("{device_id} is not listed"))
}

fn ids(devices: &[DeviceView]) -> Vec<&str> {
    devices
        .iter()
        .map(|device| device.device_id.as_str())
        .collect()
}

fn fields(value: &serde_json::Value) -> BTreeSet<&str> {
    value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect()
}

/// Exercise the production service and signed wire protocol against the actual
/// registry migration. No cloud storage, runtime dispatchers or identity provider
/// are initialized; human authentication is covered at its separate boundary.
#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn signed_enrollment_session_presence_and_revocation() {
    backend_jwt::init_for_tests();
    let database = TestDatabase::create(&["owner"]).await;
    let db = &database.db;
    let policy = hub(true);
    let state = device_context(db, &policy);
    let disabled_policy = hub(false);
    let disabled = device_context(db, &disabled_policy);
    let bootstrap = SigningKey::generate();
    let controller = SigningKey::generate();
    let request = CreateEnrollmentRequest {
        name: "Signed service test".into(),
        api_base_url: API_BASE.into(),
        bootstrap_key: bootstrap.public_key(),
        controller_key: controller.public_key(),
        owner_invitation_key: SigningKey::generate().public_key(),
    };
    assert_status(
        create_enrollment(&disabled, "owner", request.clone()).await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    let package = create_enrollment(&state, "owner", request).await.unwrap();
    let manifest = &package.manifest;
    let enrollment_id = &manifest.enrollment_id;
    let device_id = &manifest.device_id;
    let manifest_jws = sign_manifest(manifest, &controller).unwrap();
    let challenge = challenge(
        &state,
        enrollment_id,
        ChallengeRequest {
            enrollment_token: package.enrollment_token.clone(),
        },
    )
    .await
    .unwrap();
    // A second request, including one made with a leaked enrollment token, is
    // answered with the same challenge and cannot invalidate the device's redemption.
    assert_eq!(
        super::challenge(
            &state,
            enrollment_id,
            ChallengeRequest {
                enrollment_token: package.enrollment_token.clone(),
            },
        )
        .await
        .unwrap(),
        challenge
    );
    let auth = SigningKey::generate();
    let now = chrono::Utc::now().timestamp();
    let binding = EnrollmentBinding {
        version: PROTOCOL_VERSION,
        enrollment_id: enrollment_id.clone(),
        device_id: device_id.clone(),
        identity: DeviceIdentity {
            auth_key: auth.public_key(),
            management_key: [42; 32],
            telemetry_key: SigningKey::generate().public_key(),
        },
        manifest_digest: compact_digest(&manifest_jws),
        challenge_id: challenge.challenge_id,
        challenge_nonce: challenge.nonce,
        issued_at: now,
        expires_at: challenge.expires_at,
    };
    let binding_jws = sign_binding(&binding, &bootstrap).unwrap();
    let proof = EnrollmentProof {
        iss: device_id.clone(),
        sub: device_id.clone(),
        aud: endpoint_url(
            API_BASE,
            &format!("/devices/enrollments/{enrollment_id}/redeem"),
        )
        .unwrap(),
        iat: now,
        nbf: now,
        exp: challenge.expires_at,
        jti: uuid::Uuid::new_v4().to_string(),
        challenge_id: binding.challenge_id.clone(),
        challenge_nonce: binding.challenge_nonce.clone(),
        binding_digest: compact_digest(&binding_jws),
        manifest_digest: compact_digest(&manifest_jws),
    };
    let mut redemption = RedeemEnrollmentRequest {
        enrollment_token: package.enrollment_token,
        manifest_jws,
        binding_jws,
        proof_jws: "malformed-proof".into(),
    };
    assert_status(
        redeem(&state, enrollment_id, redemption.clone()).await,
        StatusCode::UNAUTHORIZED,
    );
    // A rejected proof cannot spend the enrollment or the pending challenge.
    assert_eq!(
        repository(&state)
            .enrollment(enrollment_id)
            .await
            .unwrap()
            .status,
        "pending"
    );
    redemption.proof_jws = sign_enrollment_proof(&proof, &auth).unwrap();
    let receipt = redeem(&state, enrollment_id, redemption.clone())
        .await
        .unwrap();
    assert_eq!(receipt.identity, binding.identity);
    assert_eq!(receipt.manifest_jws, redemption.manifest_jws);
    assert_eq!(receipt.binding_jws, redemption.binding_jws);
    assert_status(
        redeem(&state, enrollment_id, redemption).await,
        StatusCode::UNAUTHORIZED,
    );
    // A package that became a device is no longer a pending setup.
    assert!(
        view::enrollments(&state, "owner", EnrollmentFilter::Recent)
            .await
            .unwrap()
            .is_empty()
    );

    let token_request = DeviceTokenRequest {
        device_id: device_id.clone(),
        client_assertion: assertion(device_id, "/devices/token", &auth),
    };
    let session = token(&state, token_request.clone()).await.unwrap();
    assert_eq!(session.token_type, "DPoP");
    assert!(jwt::is_device_credential(&session.access_token));
    assert_status(token(&state, token_request).await, StatusCode::UNAUTHORIZED);
    let heartbeat_path = format!("/devices/{device_id}/heartbeat");
    let headers = request_headers(&session.access_token, &auth, &heartbeat_path);
    assert_status(
        device_principal(
            &disabled,
            device_id,
            &headers,
            "POST",
            &heartbeat_path,
            true,
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    let wrong_headers = request_headers(
        &session.access_token,
        &SigningKey::generate(),
        &heartbeat_path,
    );
    let rejected = device_principal(
        &state,
        device_id,
        &wrong_headers,
        "POST",
        &heartbeat_path,
        true,
    )
    .await
    .err()
    .expect("a proof from another key must fail");
    assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(rejected.public_code(), DEVICE_PROOF_INVALID);
    let mut malformed_headers = headers.clone();
    malformed_headers.insert("dpop", HeaderValue::from_static("malformed-proof"));
    assert_status(
        device_principal(
            &state,
            device_id,
            &malformed_headers,
            "POST",
            &heartbeat_path,
            true,
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    let principal = device_principal(&state, device_id, &headers, "POST", &heartbeat_path, true)
        .await
        .unwrap();
    assert_eq!(principal.status.device_id, *device_id);
    assert!(principal.status.last_seen_at.is_some());
    let replayed = device_principal(&state, device_id, &headers, "POST", &heartbeat_path, true)
        .await
        .err()
        .expect("a replayed proof must fail");
    assert_eq!(replayed.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(replayed.public_code(), DEVICE_PROOF_INVALID);

    // The owner's label never reaches the agent: its heartbeat and its own status read
    // keep the strict registration status with the name from the signed setup package.
    let renamed = rename(&state, "owner", device_id, Some("Lab GPU (rack 2)"))
        .await
        .unwrap();
    assert_eq!(renamed.display_name.as_deref(), Some("Lab GPU (rack 2)"));
    assert_eq!(renamed.name, "Signed service test");
    let status_path = format!("/devices/{device_id}");
    for (method, path, heartbeat) in [
        ("POST", &heartbeat_path, true),
        ("GET", &status_path, false),
    ] {
        let headers = signed_request(
            &session.access_token,
            &auth,
            method,
            path,
            chrono::Utc::now().timestamp(),
        );
        let status = device_principal(&state, device_id, &headers, method, path, heartbeat)
            .await
            .unwrap()
            .status;
        assert_eq!(status.name, "Signed service test");
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(fields(&wire).len(), 8);
        assert_eq!(
            serde_json::from_value::<DeviceStatus>(wire).unwrap(),
            status
        );
    }

    let receipt_path = format!("/devices/{device_id}/receipt");
    let recover = ReceiptRequest {
        client_assertion: assertion(device_id, &receipt_path, &auth),
    };
    assert_eq!(
        recover_receipt(&state, device_id, recover.clone())
            .await
            .unwrap(),
        receipt
    );
    assert_status(
        recover_receipt(&state, device_id, recover).await,
        StatusCode::UNAUTHORIZED,
    );
    repository(&state).revoke("owner", device_id).await.unwrap();
    let headers = request_headers(&session.access_token, &auth, &heartbeat_path);
    let revoked = device_principal(&state, device_id, &headers, "POST", &heartbeat_path, true)
        .await
        .err()
        .expect("a revoked device must be denied");
    assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(revoked.public_code(), DEVICE_PROOF_INVALID);
    assert_status(
        token(
            &state,
            DeviceTokenRequest {
                device_id: device_id.clone(),
                client_assertion: assertion(device_id, "/devices/token", &auth),
            },
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        recover_receipt(
            &state,
            device_id,
            ReceiptRequest {
                client_assertion: assertion(device_id, &receipt_path, &auth),
            },
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    let listed = view::list(&state, "owner").await.unwrap();
    assert_eq!(listed[0].status, DeviceRegistrationStatus::Revoked);
    assert_eq!(listed[0].display_name.as_deref(), Some("Lab GPU (rack 2)"));

    database.discard().await;
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn device_views_mark_relationship_and_effective_expiry() {
    backend_jwt::init_for_tests();
    let database = TestDatabase::create(&[
        "owner",
        "reader",
        "colleague",
        "payer",
        "stranger",
        "suspended",
    ])
    .await;
    let db = &database.db;
    let config = hub(true);
    let state = device_context(db, &config);
    let now = chrono::Utc::now().timestamp();

    for (device_id, owner, age) in [
        ("own-old", "owner", 300),
        ("own-new", "owner", 100),
        ("own-revoked", "owner", 50),
        ("shared-full", "owner", 40),
        ("shared-capped", "owner", 30),
        ("shared-ended", "owner", 20),
        ("shared-lapsed", "owner", 19),
        ("shared-revoked", "owner", 18),
        ("cloud-only", "owner", 10),
        ("cloud-revoked", "owner", 9),
        ("cloud-ended", "owner", 8),
        ("suspended-shared", "suspended", 5),
    ] {
        register(db, device_id, owner, now - age).await;
    }
    db.execute_unprepared(r#"UPDATE "User" SET status = 'SUSPENDED' WHERE id = 'suspended'"#)
        .await
        .unwrap();

    access_rules(
        db,
        "shared-full",
        1,
        now + 600,
        &[("reader", now + 300), ("colleague", now + 400)],
    )
    .await;
    // Only the latest rules count, and they end access before the grant inside them does.
    access_rules(db, "shared-capped", 1, now + 900, &[("reader", now + 700)]).await;
    access_rules(db, "shared-capped", 2, now + 100, &[("reader", now + 300)]).await;
    access_rules(db, "shared-ended", 1, now + 600, &[("reader", now + 300)]).await;
    access_rules(
        db,
        "shared-ended",
        2,
        now + 600,
        &[("colleague", now + 300)],
    )
    .await;
    access_rules(db, "shared-lapsed", 1, now + 600, &[("reader", now - 1)]).await;
    access_rules(db, "shared-revoked", 1, now + 600, &[("reader", now + 300)]).await;
    access_rules(
        db,
        "suspended-shared",
        1,
        now + 600,
        &[("reader", now + 300)],
    )
    .await;

    cloud_access(
        db,
        "payer-grant",
        "cloud-only",
        "payer",
        "active",
        now + 500,
    )
    .await;
    spending(db, "payer-grant", "payer", now + 400).await;
    cloud_access(
        db,
        "revoked-device-grant",
        "cloud-revoked",
        "payer",
        "active",
        now + 200,
    )
    .await;
    cloud_access(
        db,
        "withdrawn-grant",
        "cloud-ended",
        "payer",
        "revoked",
        now + 500,
    )
    .await;
    cloud_access(
        db,
        "lapsed-grant",
        "cloud-ended",
        "payer",
        "active",
        now - 1,
    )
    .await;
    cloud_access(
        db,
        "reader-grant",
        "shared-full",
        "reader",
        "active",
        now + 250,
    )
    .await;
    cloud_access(db, "owner-grant", "own-new", "owner", "active", now + 150).await;
    spending(db, "owner-grant", "payer", now + 120).await;

    for device_id in ["own-revoked", "shared-revoked", "cloud-revoked"] {
        repository(&state).revoke("owner", device_id).await.unwrap();
    }
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceAuthRejection" ("deviceId",code,"skewSeconds",count,"firstAt","lastAt") VALUES ('own-new','clock_skew',300,3,$1,$2),('shared-full','clock_skew',-90,1,$2,$2),('own-old','reason_of_a_newer_hub',NULL,1,$2,$2)"#,
        [(now - 30).into(), (now - 10).into()],
    ))
    .await
    .unwrap();

    let owned = view::list(&state, "owner").await.unwrap();
    assert_eq!(
        ids(&owned),
        [
            "cloud-ended",
            "cloud-only",
            "shared-lapsed",
            "shared-ended",
            "shared-capped",
            "shared-full",
            "own-new",
            "own-old",
            "cloud-revoked",
            "shared-revoked",
            "own-revoked",
        ],
        "active devices come first, each group newest first"
    );
    for device in &owned {
        assert_eq!(device.relationship, DeviceRelationship::Owner);
        assert_eq!(device.owner_id, "owner");
        assert_eq!(device.access_expires_at, None);
        assert_eq!(device.name, setup_name(&device.device_id));
        assert_eq!(
            device.revoked_at.is_some(),
            device.status == DeviceRegistrationStatus::Revoked
        );
    }
    assert!(
        find(&owned, "own-revoked")
            .revoked_at
            .is_some_and(|revoked_at| (now..=now + 60).contains(&revoked_at))
    );
    assert_eq!(
        find(&owned, "shared-full").access_rules_expire_at,
        Some(now + 600)
    );
    assert_eq!(
        find(&owned, "shared-capped").access_rules_expire_at,
        Some(now + 100)
    );
    assert_eq!(find(&owned, "own-old").access_rules_expire_at, None);
    let own_new = find(&owned, "own-new");
    assert_eq!(
        own_new.cloud_approvals,
        Some(view::CloudApprovals {
            resource_grants: 1,
            billing_grants: 0,
            expires_at: now + 150,
        })
    );
    assert_eq!(
        own_new.auth_rejection,
        Some(view::AuthRejection {
            code: AuthRejectionCode::ClockSkew,
            skew_seconds: Some(300),
            count: 3,
            first_at: now - 30,
            last_at: now - 10,
        })
    );
    // Approvals other people gave are theirs to list, and a reason this hub does not
    // know reads as no reason.
    assert_eq!(find(&owned, "cloud-only").cloud_approvals, None);
    assert_eq!(find(&owned, "own-old").auth_rejection, None);
    assert!(find(&owned, "shared-full").auth_rejection.is_some());

    let shared = view::list(&state, "reader").await.unwrap();
    assert_eq!(ids(&shared), ["shared-capped", "shared-full"]);
    let shared_full = find(&shared, "shared-full");
    assert_eq!(shared_full.relationship, DeviceRelationship::Shared);
    assert_eq!(shared_full.owner_id, "owner");
    assert_eq!(shared_full.access_expires_at, Some(now + 300));
    assert_eq!(shared_full.access_rules_expire_at, Some(now + 600));
    assert_eq!(shared_full.auth_rejection, None, "owner rows only");
    assert_eq!(
        shared_full.cloud_approvals,
        Some(view::CloudApprovals {
            resource_grants: 1,
            billing_grants: 0,
            expires_at: now + 250,
        })
    );
    let capped = find(&shared, "shared-capped");
    assert_eq!(capped.access_expires_at, Some(now + 100));
    assert_eq!(capped.access_rules_expire_at, Some(now + 100));
    assert_eq!(capped.cloud_approvals, None);

    let colleague = view::list(&state, "colleague").await.unwrap();
    assert_eq!(ids(&colleague), ["shared-ended", "shared-full"]);
    assert_eq!(
        find(&colleague, "shared-full").access_expires_at,
        Some(now + 400)
    );

    let approved = view::list(&state, "payer").await.unwrap();
    assert_eq!(ids(&approved), ["cloud-revoked", "cloud-only", "own-new"]);
    for device in &approved {
        assert_eq!(device.relationship, DeviceRelationship::CloudApproval);
        assert_eq!(device.access_expires_at, None);
        assert_eq!(device.access_rules_expire_at, None);
        assert_eq!(device.auth_rejection, None);
    }
    assert_eq!(
        find(&approved, "cloud-only").cloud_approvals,
        Some(view::CloudApprovals {
            resource_grants: 1,
            billing_grants: 1,
            expires_at: now + 500,
        })
    );
    assert_eq!(
        find(&approved, "own-new").cloud_approvals,
        Some(view::CloudApprovals {
            resource_grants: 0,
            billing_grants: 1,
            expires_at: now + 120,
        })
    );
    let still_paid = find(&approved, "cloud-revoked");
    assert_eq!(still_paid.status, DeviceRegistrationStatus::Revoked);
    assert!(still_paid.revoked_at.is_some());

    assert!(view::list(&state, "stranger").await.unwrap().is_empty());

    for (user, listed) in [
        ("owner", &owned),
        ("reader", &shared),
        ("colleague", &colleague),
        ("payer", &approved),
    ] {
        let visible = view::visible_devices(&state, user).await.unwrap();
        assert_eq!(
            visible
                .iter()
                .map(|device| (device.device_id.as_str(), device.relationship))
                .collect::<Vec<_>>(),
            listed
                .iter()
                .map(|device| (device.device_id.as_str(), device.relationship))
                .collect::<Vec<_>>(),
            "{user}"
        );
        for device in listed {
            assert_eq!(
                &view::get(&state, user, &device.device_id).await.unwrap(),
                device,
                "{user} reads {} alone as in the list",
                device.device_id
            );
            let wire = serde_json::to_value(device).unwrap();
            assert_eq!(fields(&wire), BTreeSet::from(VIEW_FIELDS));
            assert!(
                serde_json::from_value::<DeviceStatus>(wire).is_err(),
                "a view must never parse as the agent's strict status"
            );
        }
        let wire = serde_json::to_string(listed).unwrap();
        for secret in [
            "signed-manifest-fixture",
            "signed-binding-fixture",
            "policy-jws-fixture",
        ] {
            assert!(!wire.contains(secret), "{user} must not receive {secret}");
        }
    }
    assert!(
        view::visible_devices(&state, "stranger")
            .await
            .unwrap()
            .is_empty()
    );
    // A recipient learns neither who else has access nor any grant.
    let wire = serde_json::to_string(&shared).unwrap();
    assert!(!wire.contains("colleague") && !wire.contains("grant-"));

    // A device the caller may not see answers exactly like one that does not exist.
    for (user, device_id) in [
        ("stranger", "own-new"),
        ("stranger", "no-such-device"),
        ("reader", "own-new"),
        ("reader", "shared-ended"),
        ("reader", "shared-lapsed"),
        ("reader", "shared-revoked"),
        ("reader", "suspended-shared"),
        ("payer", "cloud-ended"),
    ] {
        let hidden = view::get(&state, user, device_id).await.unwrap_err();
        assert_eq!(hidden.status(), StatusCode::NOT_FOUND, "{user} {device_id}");
        assert_eq!(
            hidden.public_message(),
            ApiError::NOT_FOUND.public_message()
        );
    }

    let renamed = rename(
        &state,
        "owner",
        "shared-full",
        Some("  Cafe\u{301} rack 2 "),
    )
    .await
    .unwrap();
    assert_eq!(renamed.display_name.as_deref(), Some("Caf\u{e9} rack 2"));
    assert_eq!(renamed.name, setup_name("shared-full"));
    assert_eq!(renamed.relationship, DeviceRelationship::Owner);
    assert_eq!(
        view::get(&state, "reader", "shared-full")
            .await
            .unwrap()
            .display_name
            .as_deref(),
        Some("Caf\u{e9} rack 2"),
        "recipients see the owner's label"
    );
    for (user, device_id) in [
        ("reader", "shared-full"),
        ("stranger", "shared-full"),
        ("payer", "cloud-only"),
        ("owner", "own-revoked"),
        ("owner", "no-such-device"),
    ] {
        assert_status(
            rename(&state, user, device_id, Some("Mine")).await,
            StatusCode::NOT_FOUND,
        );
    }
    let longest = "x".repeat(MAX_DISPLAY_NAME_CHARS);
    assert_eq!(
        rename(&state, "owner", "own-old", Some(longest.as_str()))
            .await
            .unwrap()
            .display_name
            .as_deref(),
        Some(longest.as_str())
    );
    for invalid in ["", "   ", &format!("{longest}x"), "tab\tinside"] {
        assert_status(
            rename(&state, "owner", "own-old", Some(invalid)).await,
            StatusCode::BAD_REQUEST,
        );
    }
    let kept = view::get(&state, "owner", "own-old").await.unwrap();
    assert_eq!(kept.display_name.as_deref(), Some(longest.as_str()));
    assert_eq!(kept.name, setup_name("own-old"));
    assert_eq!(
        rename(&state, "owner", "own-old", None)
            .await
            .unwrap()
            .display_name,
        None
    );

    // A revocation made before the hub recorded its time has none.
    db.execute_unprepared(
        r#"UPDATE "ManagedDevice" SET "revokedAt" = NULL WHERE id = 'own-revoked'"#,
    )
    .await
    .unwrap();
    let undated = view::get(&state, "owner", "own-revoked").await.unwrap();
    assert_eq!(undated.status, DeviceRegistrationStatus::Revoked);
    assert_eq!(undated.revoked_at, None);

    let disabled_config = hub(false);
    let disabled = device_context(db, &disabled_config);
    assert_status(
        view::list(&disabled, "owner").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    assert_status(
        view::get(&disabled, "owner", "own-new").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    assert_status(
        rename(&disabled, "owner", "own-new", Some("Label")).await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    assert_status(
        view::enrollments(&disabled, "owner", EnrollmentFilter::Open).await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    assert_eq!(
        view::get(&state, "owner", "own-new")
            .await
            .unwrap()
            .display_name,
        None,
        "a refused rename changes nothing"
    );

    database.discard().await;
}

async fn beat(state: &DeviceContext<'_>, headers: &HeaderMap) -> Result<DevicePrincipal, ApiError> {
    device_principal(state, "device", headers, "POST", HEARTBEAT, true).await
}

fn assert_refused_proof<T>(result: Result<T, ApiError>) {
    let refused = result.err().expect("the proof must be refused");
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(refused.public_code(), DEVICE_PROOF_INVALID);
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn rejected_proofs_record_only_attributable_reasons() {
    backend_jwt::init_for_tests();
    let database = TestDatabase::create(&["owner", "reader"]).await;
    let db = &database.db;
    let config = hub(true);
    let state = device_context(db, &config);
    let now = chrono::Utc::now().timestamp();
    let auth = register(db, "device", "owner", now - 600).await;
    access_rules(db, "device", 1, now + 600, &[("reader", now + 300)]).await;
    let session = token(
        &state,
        DeviceTokenRequest {
            device_id: "device".into(),
            client_assertion: assertion("device", "/devices/token", &auth),
        },
    )
    .await
    .unwrap()
    .access_token;

    // Nothing is recorded for a proof nobody can attribute to the device, for an
    // unknown device, or for a replay.
    assert_refused_proof(
        beat(
            &state,
            &request_headers_at(&session, &SigningKey::generate(), HEARTBEAT, now + 300),
        )
        .await,
    );
    assert_refused_proof(
        beat(
            &state,
            &with_forged_issue_time(request_headers(&session, &auth, HEARTBEAT), now + 300),
        )
        .await,
    );
    assert_status(
        token(
            &state,
            DeviceTokenRequest {
                device_id: "unknown-device".into(),
                client_assertion: assertion_at(
                    "unknown-device",
                    "/devices/token",
                    &auth,
                    now - 600,
                ),
            },
        )
        .await,
        StatusCode::NOT_FOUND,
    );
    let accepted = request_headers(&session, &auth, HEARTBEAT);
    beat(&state, &accepted).await.unwrap();
    assert_refused_proof(beat(&state, &accepted).await);
    let recorded = db
        .query_one_raw(sql(
            r#"SELECT COUNT(*) AS count FROM "DeviceAuthRejection""#,
            [],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap();
    assert_eq!(recorded, 0);

    // A proof the device signed while its clock runs five minutes ahead.
    set_last_seen(db, "device", now - 120).await;
    assert_refused_proof(
        beat(
            &state,
            &request_headers_at(&session, &auth, HEARTBEAT, now + 300),
        )
        .await,
    );
    let first = stored_rejection(db, "device").await.unwrap();
    assert_eq!(first.code, "clock_skew");
    assert!(
        first
            .skew_seconds
            .is_some_and(|skew| (290..=300).contains(&skew)),
        "{first:?}"
    );
    assert_eq!(first.count, 1);
    assert_eq!(first.first_at, first.last_at);
    assert!((now..=now + 30).contains(&first.last_at));
    let reported = view::get(&state, "owner", "device")
        .await
        .unwrap()
        .auth_rejection
        .expect("the owner sees why the hub refuses the device");
    assert_eq!(reported.code, AuthRejectionCode::ClockSkew);
    assert_eq!(reported.skew_seconds, first.skew_seconds);
    assert_eq!((reported.count, reported.last_at), (1, first.last_at));
    let recipient = view::get(&state, "reader", "device").await.unwrap();
    assert_eq!(recipient.relationship, DeviceRelationship::Shared);
    assert_eq!(recipient.auth_rejection, None);

    // A retrying device writes at most once per throttle window.
    assert_refused_proof(
        beat(
            &state,
            &request_headers_at(&session, &auth, HEARTBEAT, now + 300),
        )
        .await,
    );
    assert_eq!(stored_rejection(db, "device").await.unwrap(), first);

    // The same reason without a check-in in between continues the count, here from a
    // token request signed while the clock runs ten minutes behind.
    age_rejection(db, 60).await;
    assert_refused_proof(
        token(
            &state,
            DeviceTokenRequest {
                device_id: "device".into(),
                client_assertion: assertion_at("device", "/devices/token", &auth, now - 600),
            },
        )
        .await,
    );
    let second = stored_rejection(db, "device").await.unwrap();
    assert_eq!(second.code, "clock_skew");
    assert!(
        second
            .skew_seconds
            .is_some_and(|skew| (-630..=-600).contains(&skew)),
        "{second:?}"
    );
    assert_eq!(second.count, 2);
    assert_eq!(second.first_at, first.first_at - 60);
    assert!(second.last_at >= first.last_at);

    // Once the device checks in again the reason is no longer reported, and a later
    // refusal starts a new count.
    beat(&state, &request_headers(&session, &auth, HEARTBEAT))
        .await
        .unwrap();
    assert_eq!(
        view::get(&state, "owner", "device")
            .await
            .unwrap()
            .auth_rejection,
        None
    );
    age_rejection(db, 60).await;
    assert_refused_proof(
        beat(
            &state,
            &request_headers_at(&session, &auth, HEARTBEAT, now + 300),
        )
        .await,
    );
    let restarted = stored_rejection(db, "device").await.unwrap();
    assert_eq!(restarted.count, 1);
    assert_eq!(restarted.first_at, restarted.last_at);

    // A session issued before the revocation is attributable through the hub's own
    // signature. The device still receives the plain denial.
    age_rejection(db, 60).await;
    set_last_seen(db, "device", now - 120).await;
    repository(&state).revoke("owner", "device").await.unwrap();
    let denied = beat(&state, &request_headers(&session, &auth, HEARTBEAT))
        .await
        .err()
        .expect("a revoked device must be denied");
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(denied.public_code(), DEVICE_PROOF_INVALID);
    let revoked = stored_rejection(db, "device").await.unwrap();
    assert_eq!(revoked.code, "revoked_credential");
    assert_eq!(revoked.skew_seconds, None);
    assert_eq!(revoked.count, 1);
    assert_eq!(revoked.first_at, revoked.last_at);
    let reported = view::get(&state, "owner", "device")
        .await
        .unwrap()
        .auth_rejection
        .unwrap();
    assert_eq!(reported.code, AuthRejectionCode::RevokedCredential);
    assert_eq!(reported.skew_seconds, None);

    // The record is best effort: without its table the device gets the same answer.
    let other = register(db, "other-device", "owner", now - 600).await;
    db.execute_unprepared(r#"DROP TABLE "DeviceAuthRejection""#)
        .await
        .unwrap();
    assert_refused_proof(
        token(
            &state,
            DeviceTokenRequest {
                device_id: "other-device".into(),
                client_assertion: assertion_at("other-device", "/devices/token", &other, now - 600),
            },
        )
        .await,
    );

    database.discard().await;
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn enrollment_lists_show_only_the_owners_unstarted_packages() {
    backend_jwt::init_for_tests();
    let database = TestDatabase::create(&["owner", "other", "bulk", "stranger"]).await;
    let db = &database.db;
    let config = hub(true);
    let state = device_context(db, &config);
    let now = chrono::Utc::now().timestamp();
    let controller = SigningKey::generate();
    let mut packages = Vec::new();
    for (owner, name, age) in [
        ("owner", "waiting", 10),
        ("owner", "lapsed", 20),
        ("owner", "cancelled", 30),
        ("owner", "started", 40),
        ("other", "someone else's", 5),
    ] {
        let package = create_enrollment(
            &state,
            owner,
            CreateEnrollmentRequest {
                name: name.into(),
                api_base_url: API_BASE.into(),
                bootstrap_key: SigningKey::generate().public_key(),
                controller_key: controller.public_key(),
                owner_invitation_key: SigningKey::generate().public_key(),
            },
        )
        .await
        .unwrap();
        db.execute_raw(sql(
            r#"UPDATE "DeviceEnrollment" SET "createdAt" = $1 WHERE id = $2"#,
            [
                (now - age).into(),
                package.manifest.enrollment_id.clone().into(),
            ],
        ))
        .await
        .unwrap();
        packages.push(package);
    }
    let id = |index: usize| packages[index].manifest.enrollment_id.as_str();
    db.execute_raw(sql(
        r#"UPDATE "DeviceEnrollment" SET "expiresAt" = $1 WHERE id = $2"#,
        [(now - 5).into(), id(1).into()],
    ))
    .await
    .unwrap();
    repository(&state)
        .cancel_enrollment("owner", id(2))
        .await
        .unwrap();
    db.execute_raw(sql(
        r#"UPDATE "DeviceEnrollment" SET status = 'consumed', "consumedAt" = $1 WHERE id = $2"#,
        [now.into(), id(3).into()],
    ))
    .await
    .unwrap();

    let states = |listed: &[view::DeviceEnrollmentView]| {
        listed
            .iter()
            .map(|package| (package.name.clone(), package.state))
            .collect::<Vec<_>>()
    };
    let open = view::enrollments(&state, "owner", EnrollmentFilter::Open)
        .await
        .unwrap();
    assert_eq!(
        states(&open),
        [
            ("waiting".to_owned(), DeviceEnrollmentState::Pending),
            ("lapsed".to_owned(), DeviceEnrollmentState::Expired),
        ]
    );
    let waiting = &packages[0].manifest;
    assert_eq!(open[0].enrollment_id, waiting.enrollment_id);
    assert_eq!(open[0].device_id, waiting.device_id);
    assert_eq!(open[0].created_at, now - 10);
    assert_eq!(open[0].expires_at, waiting.expires_at);
    assert_eq!(
        open[0].controller_key_thumbprint,
        controller.public_key().thumbprint().unwrap()
    );
    assert_eq!(open[1].expires_at, now - 5);

    let recent = view::enrollments(&state, "owner", EnrollmentFilter::Recent)
        .await
        .unwrap();
    assert_eq!(
        states(&recent),
        [
            ("waiting".to_owned(), DeviceEnrollmentState::Pending),
            ("lapsed".to_owned(), DeviceEnrollmentState::Expired),
            ("cancelled".to_owned(), DeviceEnrollmentState::Cancelled),
        ],
        "a package that became a device is never listed"
    );
    assert_eq!(
        states(
            &view::enrollments(&state, "other", EnrollmentFilter::Recent)
                .await
                .unwrap()
        ),
        [("someone else's".to_owned(), DeviceEnrollmentState::Pending)]
    );
    assert!(
        view::enrollments(&state, "stranger", EnrollmentFilter::Recent)
            .await
            .unwrap()
            .is_empty()
    );

    let wire = serde_json::to_value(&recent).unwrap();
    for package in wire.as_array().unwrap() {
        assert_eq!(
            fields(package),
            BTreeSet::from([
                "enrollment_id",
                "device_id",
                "name",
                "state",
                "created_at",
                "expires_at",
                "controller_key_thumbprint",
            ])
        );
    }
    let wire = wire.to_string();
    for package in &packages[..3] {
        let stored = repository(&state)
            .enrollment(&package.manifest.enrollment_id)
            .await
            .unwrap();
        let bootstrap_key = serde_json::to_value(&package.manifest.bootstrap_key).unwrap()["x"]
            .as_str()
            .unwrap()
            .to_owned();
        for secret in [&stored.jwt_id, &bootstrap_key, &package.enrollment_token] {
            assert!(!wire.contains(secret.as_str()));
        }
    }

    // The list is bounded and keeps the newest packages.
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceEnrollment" (id,"deviceId","ownerId","jwtId",manifest,status,"expiresAt","createdAt")
        SELECT 'bulk-' || sequence, 'bulk-device-' || sequence, 'bulk', 'bulk-jwt-' || sequence, $1, 'pending', $2, $3 + sequence
        FROM generate_series(1,205) AS sequence"#,
        [
            serde_json::to_string(waiting).unwrap().into(),
            (now + 600).into(),
            now.into(),
        ],
    ))
    .await
    .unwrap();
    let bulk = view::enrollments(&state, "bulk", EnrollmentFilter::Open)
        .await
        .unwrap();
    assert_eq!(bulk.len(), repository::MAX_LISTED_ENROLLMENTS);
    assert_eq!(bulk[0].enrollment_id, "bulk-205");
    assert_eq!(bulk[199].enrollment_id, "bulk-6");

    database.discard().await;
}

const BACKUP_CIPHERTEXT: &str = "RkxWQVVMVDEtZW5jcnlwdGVkLWtleXMtZml4dHVyZQ";

async fn request_setup(
    state: &DeviceContext<'_>,
    owner: &str,
) -> Result<CreateEnrollmentResponse, ApiError> {
    create_enrollment(
        state,
        owner,
        CreateEnrollmentRequest {
            name: "Usage fixture".into(),
            api_base_url: API_BASE.into(),
            bootstrap_key: SigningKey::generate().public_key(),
            controller_key: SigningKey::generate().public_key(),
            owner_invitation_key: SigningKey::generate().public_key(),
        },
    )
    .await
}

/// A setup package row for `device_id`. Its manifest is never read by the counts.
async fn setup_package(
    db: &DatabaseConnection,
    device_id: &str,
    owner: &str,
    status: &str,
    expires_at: i64,
    created_at: i64,
) {
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceEnrollment" (id,"deviceId","ownerId","jwtId",manifest,status,"expiresAt","createdAt") VALUES ($1,$2,$3,$4,'{}',$5,$6,$7)"#,
        [
            format!("package-{device_id}").into(),
            device_id.into(),
            owner.into(),
            format!("jwt-{device_id}").into(),
            status.into(),
            expires_at.into(),
            created_at.into(),
        ],
    ))
    .await
    .unwrap();
}

async fn account_backup(
    db: &DatabaseConnection,
    user: &str,
    key_id: &str,
    key: &SigningKey,
    revision: i64,
    updated_at: i64,
) {
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceControllerVault" ("userId","keyId","publicKey",ciphertext,revision,"updatedAt") VALUES ($1,$2,$3,$4,$5,$6)"#,
        [
            user.into(),
            key_id.into(),
            serde_json::to_string(&key.public_key()).unwrap().into(),
            BACKUP_CIPHERTEXT.into(),
            revision.into(),
            updated_at.into(),
        ],
    ))
    .await
    .unwrap();
}

async fn usage_of(state: &DeviceContext<'_>, user: &str) -> DeviceUsageView {
    view::usage(state, user).await.unwrap()
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn usage_reports_exactly_what_setup_enforces() {
    backend_jwt::init_for_tests();
    let database = TestDatabase::create(&["owner", "other"]).await;
    let db = &database.db;
    let config = StandaloneConfig {
        max_devices_per_user: 6,
        max_pending_enrollments_per_user: 2,
        ..hub(true)
    };
    let state = device_context(db, &config);
    let disabled_config = hub(false);
    assert_status(
        view::usage(&device_context(db, &disabled_config), "owner").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    let now = chrono::Utc::now().timestamp();
    for (device, owner) in [
        ("mine-1", "owner"),
        ("mine-2", "owner"),
        ("mine-revoked", "owner"),
        ("theirs", "other"),
        ("theirs-revoked", "other"),
    ] {
        register(db, device, owner, now).await;
    }
    for (owner, device) in [("owner", "mine-revoked"), ("other", "theirs-revoked")] {
        repository(&state).revoke(owner, device).await.unwrap();
    }

    let start = usage_of(&state, "owner").await;
    assert!((now..=now + 60).contains(&start.server_time));
    assert_eq!(
        start.limits,
        DeviceLimits {
            max_devices: 6,
            max_pending_enrollments: 2,
            enrollment_ttl_seconds: 86_400,
            max_enrollments_per_day: 16,
            max_account_backups: 256,
        }
    );
    assert_eq!(
        start.usage,
        DeviceUsage {
            active_devices: 2,
            revoked_devices: 1,
            pending_enrollments: 0,
            enrollments_last_24h: 0,
            account_backups: 0,
        }
    );

    // A new setup is refused exactly when the waiting ones reach their limit.
    let first = request_setup(&state, "owner").await.unwrap();
    let second = request_setup(&state, "owner").await.unwrap();
    request_setup(&state, "other").await.unwrap();
    let waiting = usage_of(&state, "owner").await.usage;
    assert_eq!(
        (waiting.pending_enrollments, waiting.enrollments_last_24h),
        (2, 2)
    );
    assert_status(
        request_setup(&state, "owner").await,
        StatusCode::TOO_MANY_REQUESTS,
    );

    // A cancelled or lapsed package frees its place and stays counted for the day.
    repository(&state)
        .cancel_enrollment("owner", &first.manifest.enrollment_id)
        .await
        .unwrap();
    db.execute_raw(sql(
        r#"UPDATE "DeviceEnrollment" SET "expiresAt" = $1 WHERE id = $2"#,
        [
            (now - 1).into(),
            second.manifest.enrollment_id.clone().into(),
        ],
    ))
    .await
    .unwrap();
    let freed = usage_of(&state, "owner").await.usage;
    assert_eq!(
        (freed.pending_enrollments, freed.enrollments_last_24h),
        (0, 2)
    );
    request_setup(&state, "owner").await.unwrap();

    // Devices and waiting setups share the device limit.
    for device in ["mine-3", "mine-4", "mine-5"] {
        register(db, device, "owner", now).await;
    }
    let crowded = usage_of(&state, "owner").await;
    assert_eq!(
        crowded.usage.active_devices + crowded.usage.pending_enrollments,
        u64::from(crowded.limits.max_devices)
    );
    assert!(crowded.usage.pending_enrollments < u64::from(crowded.limits.max_pending_enrollments));
    assert_status(
        request_setup(&state, "owner").await,
        StatusCode::TOO_MANY_REQUESTS,
    );
    repository(&state).revoke("owner", "mine-5").await.unwrap();
    request_setup(&state, "owner").await.unwrap();
    let roomy = usage_of(&state, "owner").await.usage;
    assert_eq!(
        roomy,
        DeviceUsage {
            active_devices: 4,
            revoked_devices: 2,
            pending_enrollments: 2,
            enrollments_last_24h: 4,
            account_backups: 0,
        }
    );

    // The daily allowance counts every package of the last 24 hours, whatever became
    // of it, and the next one is refused exactly when it is spent.
    db.execute_raw(sql(
        r#"UPDATE "DeviceEnrollment" SET status = 'cancelled' WHERE "ownerId" = 'owner' AND status = 'pending'"#,
        [],
    ))
    .await
    .unwrap();
    let per_day = start.limits.max_enrollments_per_day;
    for index in roomy.enrollments_last_24h..per_day - 1 {
        setup_package(
            db,
            &format!("churn-{index}"),
            "owner",
            "cancelled",
            now + 600,
            now,
        )
        .await;
    }
    setup_package(
        db,
        "yesterday",
        "owner",
        "cancelled",
        now + 600,
        now - 86_400 - 60,
    )
    .await;
    assert_eq!(
        usage_of(&state, "owner").await.usage.enrollments_last_24h,
        per_day - 1
    );
    let last = request_setup(&state, "owner").await.unwrap();
    repository(&state)
        .cancel_enrollment("owner", &last.manifest.enrollment_id)
        .await
        .unwrap();
    let spent = usage_of(&state, "owner").await.usage;
    assert_eq!(
        (spent.pending_enrollments, spent.enrollments_last_24h),
        (0, per_day)
    );
    let refused = request_setup(&state, "owner")
        .await
        .err()
        .expect("the daily allowance is spent");
    assert_eq!(refused.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        refused
            .public_message()
            .is_some_and(|message| message.contains(&format!("limited to {per_day} per day")))
    );

    // Another account's devices and packages never count.
    assert_eq!(
        usage_of(&state, "other").await.usage,
        DeviceUsage {
            active_devices: 1,
            revoked_devices: 1,
            pending_enrollments: 1,
            enrollments_last_24h: 1,
            account_backups: 0,
        }
    );

    database.discard().await;
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn account_backups_are_listed_without_keys_and_counted_by_slot() {
    backend_jwt::init_for_tests();
    let database = TestDatabase::create(&["owner", "other", "hoarder", "stranger"]).await;
    let db = &database.db;
    let config = hub(true);
    let state = device_context(db, &config);
    let disabled_config = hub(false);
    assert_status(
        recovery::backups(&device_context(db, &disabled_config), "owner").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    let now = chrono::Utc::now().timestamp();
    register(db, "active", "owner", now).await;
    register(db, "revoked", "owner", now).await;
    repository(&state).revoke("owner", "revoked").await.unwrap();
    register(db, "shared", "other", now).await;
    setup_package(db, "waiting", "owner", "pending", now + 600, now).await;
    setup_package(db, "lapsed", "owner", "pending", now - 1, now).await;
    setup_package(db, "cancelled", "owner", "cancelled", now + 600, now).await;
    let (mine, theirs) = (SigningKey::generate(), SigningKey::generate());
    let stored = [
        ("active", 3, now - 10),
        ("revoked", 1, now - 20),
        ("waiting", 1, now - 30),
        ("lapsed", 1, now - 40),
        ("cancelled", 2, now - 50),
        ("shared", 1, now - 60),
    ];
    for (key_id, revision, updated_at) in stored {
        account_backup(db, "owner", key_id, &mine, revision, updated_at).await;
    }
    account_backup(db, "other", "shared", &theirs, 5, now - 5).await;

    let listed = recovery::backups(&state, "owner").await.unwrap();
    assert_eq!(
        listed
            .vaults
            .iter()
            .map(|vault| (
                vault.key_id.as_str(),
                vault.revision as i64,
                vault.updated_at
            ))
            .collect::<Vec<_>>(),
        stored,
        "every stored backup is listed, newest first"
    );
    let thumbprint = mine.public_key().thumbprint().unwrap();
    assert!(
        listed
            .vaults
            .iter()
            .all(|vault| vault.public_key_thumbprint == thumbprint)
    );
    // A slot is held for an active device and for a package that can still be started;
    // the device may belong to someone who shares it.
    assert_eq!((listed.used, listed.max), (3, 256));
    assert_eq!(
        usage_of(&state, "owner").await.usage.account_backups,
        listed.used
    );

    let wire = serde_json::to_value(&listed).unwrap();
    assert_eq!(fields(&wire), BTreeSet::from(["vaults", "used", "max"]));
    for vault in wire["vaults"].as_array().unwrap() {
        assert_eq!(
            fields(vault),
            BTreeSet::from(["key_id", "revision", "updated_at", "public_key_thumbprint"])
        );
    }
    assert!(!wire.to_string().contains(BACKUP_CIPHERTEXT));

    let others = recovery::backups(&state, "other").await.unwrap();
    assert_eq!(
        others,
        recovery::AccountBackupList {
            vaults: vec![recovery::AccountBackupView {
                key_id: "shared".into(),
                revision: 5,
                updated_at: now - 5,
                public_key_thumbprint: theirs.public_key().thumbprint().unwrap(),
            }],
            used: 1,
            max: 256,
        }
    );
    let none = recovery::backups(&state, "stranger").await.unwrap();
    assert!(none.vaults.is_empty());
    assert_eq!(none.used, 0);

    // The list is bounded and keeps the newest backups.
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceControllerVault" ("userId","keyId","publicKey",ciphertext,revision,"updatedAt")
        SELECT 'hoarder', 'bulk-' || sequence, $1, $2, 1, $3 + sequence FROM generate_series(1,300) AS sequence"#,
        [
            serde_json::to_string(&mine.public_key()).unwrap().into(),
            BACKUP_CIPHERTEXT.into(),
            now.into(),
        ],
    ))
    .await
    .unwrap();
    let bulk = recovery::backups(&state, "hoarder").await.unwrap();
    assert_eq!(bulk.vaults.len(), 256);
    assert_eq!(bulk.vaults[0].key_id, "bulk-300");
    assert_eq!(bulk.vaults[255].key_id, "bulk-45");
    assert_eq!(bulk.used, 0);

    // A suspended account is refused before this read; the read itself gives it nothing.
    db.execute_unprepared(r#"UPDATE "User" SET status = 'SUSPENDED' WHERE id = 'other'"#)
        .await
        .unwrap();
    let suspended = recovery::backups(&state, "other").await.unwrap();
    assert!(suspended.vaults.is_empty());
    assert_eq!(suspended.used, 0);

    database.discard().await;
}

/// The consumed setup package of a registered device. Its manifest names the key that
/// signs the device's access rules.
async fn enrolled(
    db: &DatabaseConnection,
    device_id: &str,
    owner: &str,
    invitation: &SigningKey,
    now: i64,
) {
    let manifest = OnboardingManifest {
        version: PROTOCOL_VERSION,
        enrollment_id: format!("enrollment-{device_id}"),
        device_id: device_id.into(),
        owner_id: owner.into(),
        name: setup_name(device_id),
        api_base_url: API_BASE.into(),
        bootstrap_key: SigningKey::generate().public_key(),
        controller_key: SigningKey::generate().public_key(),
        owner_invitation_key: invitation.public_key(),
        issued_at: now,
        expires_at: now + 600,
    };
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceEnrollment" (id,"deviceId","ownerId","jwtId",manifest,status,"expiresAt","createdAt","consumedAt") VALUES ($1,$2,$3,$4,$5,'consumed',$6,$7,$7)"#,
        [
            manifest.enrollment_id.clone().into(),
            device_id.into(),
            owner.into(),
            format!("jwt-{device_id}").into(),
            serde_json::to_string(&manifest).unwrap().into(),
            manifest.expires_at.into(),
            now.into(),
        ],
    ))
    .await
    .unwrap();
}

fn access_grant(
    user: &str,
    key: &SigningKey,
    scope: ManagementScope,
    capabilities: &[ManagementCapability],
    expires_at: i64,
) -> ManagementGrant {
    ManagementGrant {
        grant_id: format!("grant-{user}"),
        user_id: user.into(),
        controller_key: key.public_key(),
        scope,
        capabilities: capabilities.to_vec(),
        expires_at,
        group_id: None,
        group_version: None,
    }
}

/// Every refusal of `own_access` reads the same, so a missing device is not told
/// apart from access that ended.
fn assert_no_access<T>(result: Result<T, ApiError>) {
    let refusal = result.err().expect("access must be refused");
    assert_eq!(refusal.status(), StatusCode::FORBIDDEN);
    assert_eq!(refusal.public_message(), None);
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn own_access_is_read_from_the_owners_current_rules_only() {
    backend_jwt::init_for_tests();
    let database = TestDatabase::create(&["owner", "reader", "other", "outsider"]).await;
    let db = &database.db;
    let config = hub(true);
    let state = device_context(db, &config);
    let now = chrono::Utc::now().timestamp();
    let invitation = SigningKey::generate();
    let (reader, other) = (SigningKey::generate(), SigningKey::generate());
    register(db, "device", "owner", now).await;
    enrolled(db, "device", "owner", &invitation, now).await;
    let access =
        |user: &'static str, device: &'static str| management::own_access(&state, user, device);
    let publish = |compact: String| {
        management::persist_policy(
            db,
            crate::db::DbDialect::Postgres,
            "device".into(),
            1,
            "owner".into(),
            invitation.public_key(),
            compact,
        )
    };

    let disabled_config = hub(false);
    assert_status(
        management::own_access(&device_context(db, &disabled_config), "owner", "device").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );

    // Before any rules exist the owner needs none and nobody else has access.
    assert_eq!(
        serde_json::to_value(access("owner", "device").await.unwrap()).unwrap(),
        serde_json::json!({
            "device_id": "device",
            "role": "owner",
            "owner_id": "owner",
            "policy_version": 0,
            "policy_expires_at": null,
            "applied_version": 0,
            "applied": true,
            "grants": [],
        })
    );
    assert_no_access(access("reader", "device").await);

    let mut rules = ManagementPolicy {
        version: 1,
        device_id: "device".into(),
        policy_version: 1,
        previous_policy_digest: None,
        grants: vec![
            access_grant(
                "reader",
                &reader,
                ManagementScope::Device,
                &[ManagementCapability::Status, ManagementCapability::Logs],
                now + 300,
            ),
            access_grant(
                "other",
                &other,
                ManagementScope::Project {
                    project_id: "invoice-ai".into(),
                },
                &[ManagementCapability::Deploy],
                now + 200,
            ),
        ],
        issued_at: now - 100,
        expires_at: now + 600,
    };
    let first = sign_management_policy(&rules, &invitation).unwrap();
    publish(first.clone()).await.unwrap();

    let shared = serde_json::to_value(access("reader", "device").await.unwrap()).unwrap();
    assert_eq!(
        shared,
        serde_json::json!({
            "device_id": "device",
            "role": "grantee",
            "owner_id": "owner",
            "policy_version": 1,
            "policy_expires_at": now + 600,
            "applied_version": 0,
            "applied": false,
            "grants": [{
                "grant_id": "grant-reader",
                "scope": {"kind": "device"},
                "capabilities": ["status", "logs"],
                "expires_at": now + 300,
                "controller_key_thumbprint": reader.public_key().thumbprint().unwrap(),
                "group_id": null,
            }],
        }),
        "a recipient sees their own grants and nothing of anyone else's"
    );
    assert!(!shared.to_string().contains(&first));
    let owned = access("owner", "device").await.unwrap();
    assert_eq!(
        (
            owned.role,
            owned.policy_version,
            owned.policy_expires_at,
            owned.applied,
        ),
        (AccessRole::Owner, 1, Some(now + 600), false)
    );
    assert!(owned.grants.is_empty());
    let theirs = access("other", "device").await.unwrap();
    assert_eq!(theirs.role, AccessRole::Grantee);
    assert_eq!(
        serde_json::to_value(&theirs.grants).unwrap(),
        serde_json::json!([{
            "grant_id": "grant-other",
            "scope": {"kind": "project", "project_id": "invoice-ai"},
            "capabilities": ["deploy"],
            "expires_at": now + 200,
            "controller_key_thumbprint": other.public_key().thumbprint().unwrap(),
            "group_id": null,
        }])
    );

    // Someone the rules never named learns nothing, not even whether the device exists.
    assert_no_access(access("outsider", "device").await);
    assert_no_access(access("outsider", "missing").await);
    assert_no_access(access("owner", "missing").await);

    db.execute_raw(sql(
        r#"INSERT INTO "DeviceManagementApplied" ("deviceId",version,digest,"appliedAt") VALUES ('device',1,$1,$2)"#,
        [compact_digest(&first).into(), now.into()],
    ))
    .await
    .unwrap();
    let confirmed = access("reader", "device").await.unwrap();
    assert_eq!((confirmed.applied_version, confirmed.applied), (1, true));

    // The next rules leave the reader only a grant that has already ended.
    rules.policy_version = 2;
    rules.previous_policy_digest = Some(compact_digest(&first));
    rules.grants[0].expires_at = now - 10;
    let second = sign_management_policy(&rules, &invitation).unwrap();
    publish(second).await.unwrap();
    assert_no_access(access("reader", "device").await);
    let waiting = access("other", "device").await.unwrap();
    assert_eq!(
        (
            waiting.policy_version,
            waiting.applied_version,
            waiting.applied
        ),
        (2, 1, false)
    );

    // Access also stops while the owner's account is suspended.
    db.execute_unprepared(r#"UPDATE "User" SET status = 'SUSPENDED' WHERE id = 'owner'"#)
        .await
        .unwrap();
    assert_no_access(access("other", "device").await);
    db.execute_unprepared(r#"UPDATE "User" SET status = 'ACTIVE' WHERE id = 'owner'"#)
        .await
        .unwrap();

    // Rules that lapsed while the request ran give nothing, whatever the recipient
    // row still says.
    rules.policy_version = 3;
    rules.issued_at = now - 1000;
    rules.expires_at = now - 1;
    rules.grants = vec![access_grant(
        "reader",
        &reader,
        ManagementScope::Device,
        &[ManagementCapability::Status],
        now - 1,
    )];
    let lapsed = sign_management_policy(&rules, &invitation).unwrap();
    stored_rules(
        db,
        "device",
        3,
        &lapsed,
        now + 600,
        &[("reader", now + 300)],
    )
    .await;
    assert_no_access(access("reader", "device").await);

    // Rules the owner's key did not sign are an error, never a statement about access.
    access_rules(db, "device", 4, now + 600, &[("reader", now + 300)]).await;
    assert_status(
        access("reader", "device").await,
        StatusCode::INTERNAL_SERVER_ERROR,
    );
    assert_eq!(
        access("owner", "device").await.unwrap().policy_version,
        4,
        "the owner's answer never depends on the stored signature"
    );

    repository(&state).revoke("owner", "device").await.unwrap();
    assert_no_access(access("owner", "device").await);

    database.discard().await;
}
