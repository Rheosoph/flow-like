use super::*;
use axum::http::{HeaderValue, StatusCode};
use sea_orm::{ConnectOptions, ConnectionTrait, Database};

const API_BASE: &str = "https://device-test.example/api/v1";

fn assert_status<T>(result: Result<T, ApiError>, expected: StatusCode) {
    assert_eq!(result.err().expect("request must fail").status(), expected);
}

fn assertion(device_id: &str, path: &str, key: &SigningKey) -> String {
    let now = chrono::Utc::now().timestamp();
    sign_client_assertion(
        &ClientAssertion {
            iss: device_id.into(),
            sub: device_id.into(),
            aud: endpoint_url(API_BASE, path).unwrap(),
            iat: now,
            nbf: now,
            exp: now + 60,
            jti: uuid::Uuid::new_v4().to_string(),
        },
        key,
    )
    .unwrap()
}

fn request_headers(token: &str, key: &SigningKey, path: &str) -> HeaderMap {
    let proof = sign_dpop(
        &DpopProof {
            jti: uuid::Uuid::new_v4().to_string(),
            htm: "POST".into(),
            htu: endpoint_url(API_BASE, path).unwrap(),
            iat: chrono::Utc::now().timestamp(),
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

/// Exercise the production service and signed wire protocol against the actual
/// registry migration. No cloud storage, runtime dispatchers or identity provider
/// are initialized; human authentication is covered at its separate boundary.
#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to a disposable PostgreSQL server"]
async fn signed_enrollment_session_presence_and_revocation() {
    backend_jwt::init_for_tests();
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
    for statement in
        include_str!("../../prisma/migrations/20260921120000_standalone_devices/migration.sql")
            .split(';')
            .filter(|statement| !statement.trim().is_empty())
    {
        db.execute_unprepared(statement).await.unwrap();
    }
    db.execute_unprepared(r#"CREATE TABLE "User" (id TEXT PRIMARY KEY, status TEXT NOT NULL, "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#).await.unwrap();
    db.execute_unprepared(r#"INSERT INTO "User" (id,status) VALUES ('owner','ACTIVE')"#)
        .await
        .unwrap();

    let policy = StandaloneConfig {
        enabled: true,
        api_base_url: Some(API_BASE.into()),
        ..StandaloneConfig::default()
    };
    let state = DeviceContext {
        db: &db,
        dialect: crate::db::DbDialect::Postgres,
        config: &policy,
        domain: "unused.example",
        secure: true,
    };
    let disabled_policy = StandaloneConfig {
        enabled: false,
        ..policy.clone()
    };
    let disabled = DeviceContext {
        config: &disabled_policy,
        ..state
    };
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
    assert_status(
        device_principal(
            &state,
            device_id,
            &wrong_headers,
            "POST",
            &heartbeat_path,
            true,
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
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
    assert_status(
        device_principal(&state, device_id, &headers, "POST", &heartbeat_path, true).await,
        StatusCode::UNAUTHORIZED,
    );

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
    assert_status(
        device_principal(&state, device_id, &headers, "POST", &heartbeat_path, true).await,
        StatusCode::UNAUTHORIZED,
    );
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
    assert_eq!(
        repository(&state).list("owner").await.unwrap()[0].status,
        DeviceRegistrationStatus::Revoked
    );

    drop(state);
    drop(disabled);
    db.close().await.unwrap();
    admin
        .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    admin.close().await.unwrap();
}
