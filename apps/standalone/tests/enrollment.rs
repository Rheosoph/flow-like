use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::post,
};
use flow_like_device_protocol::*;
use flow_like_standalone::{
    enrollment::{self, DeviceSession},
    state::StateStore,
    supervisor, vault,
};
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Default)]
struct Registry {
    base: String,
    manifest: Option<OnboardingManifest>,
    challenge: Option<EnrollmentChallenge>,
    receipt: Option<DeviceReceipt>,
    token_requests: usize,
    heartbeats: usize,
    replay: HashSet<String>,
    reject_next_heartbeat: bool,
    revoked: bool,
}
type Shared = Arc<Mutex<Registry>>;

async fn create(
    State(state): State<Shared>,
    headers: HeaderMap,
    Json(body): Json<CreateEnrollmentRequest>,
) -> Json<CreateEnrollmentResponse> {
    assert_eq!(
        headers["authorization"],
        "Bearer enrollment-test-user-token"
    );
    let mut state = state.lock().await;
    assert_eq!(body.api_base_url, state.base);
    let now = enrollment::unix_time().unwrap();
    let manifest = OnboardingManifest {
        version: 1,
        enrollment_id: Uuid::new_v4().to_string(),
        device_id: Uuid::new_v4().to_string(),
        owner_id: "owner".into(),
        name: body.name,
        api_base_url: body.api_base_url,
        bootstrap_key: body.bootstrap_key,
        controller_key: body.controller_key,
        owner_invitation_key: body.owner_invitation_key,
        issued_at: now,
        expires_at: now + 86400,
    };
    validate_manifest(&manifest).unwrap();
    state.manifest = Some(manifest.clone());
    Json(CreateEnrollmentResponse {
        enrollment_token: "one-use-bootstrap-token".into(),
        manifest,
    })
}

async fn challenge(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<ChallengeRequest>,
) -> Json<EnrollmentChallenge> {
    assert_eq!(body.enrollment_token, "one-use-bootstrap-token");
    let mut state = state.lock().await;
    assert_eq!(id, state.manifest.as_ref().unwrap().enrollment_id);
    let challenge = EnrollmentChallenge {
        challenge_id: Uuid::new_v4().to_string(),
        nonce: "VhF2QDp2QgpCIh2hsyIeDE_lWL4nGCPEyzutUfRkoes".into(),
        expires_at: enrollment::unix_time().unwrap() + 60,
    };
    state.challenge = Some(challenge.clone());
    Json(challenge)
}

async fn redeem(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<RedeemEnrollmentRequest>,
) -> StatusCode {
    let mut state = state.lock().await;
    assert!(state.receipt.is_none());
    let now = enrollment::unix_time().unwrap();
    let manifest = state.manifest.as_ref().unwrap();
    assert_eq!(body.enrollment_token, "one-use-bootstrap-token");
    assert_eq!(id, manifest.enrollment_id);
    assert_eq!(
        &verify_manifest(&body.manifest_jws, &manifest.controller_key, now).unwrap(),
        manifest
    );
    let binding = verify_binding(&body.binding_jws, &manifest.bootstrap_key, now).unwrap();
    let challenge = state.challenge.as_ref().unwrap();
    let endpoint = endpoint_url(&state.base, &format!("/devices/enrollments/{id}/redeem")).unwrap();
    verify_enrollment_proof(
        &body.proof_jws,
        &binding.identity.auth_key,
        &EnrollmentProofContext {
            device_id: &manifest.device_id,
            endpoint: &endpoint,
            challenge_id: &challenge.challenge_id,
            challenge_nonce: &challenge.nonce,
            binding_digest: &compact_digest(&body.binding_jws),
            manifest_digest: &compact_digest(&body.manifest_jws),
            now,
        },
    )
    .unwrap();
    state.receipt = Some(DeviceReceipt {
        enrollment_id: id,
        device_id: manifest.device_id.clone(),
        owner_id: manifest.owner_id.clone(),
        name: manifest.name.clone(),
        identity: binding.identity,
        manifest_jws: body.manifest_jws,
        binding_jws: body.binding_jws,
        registered_at: now,
        auth_epoch: 1,
    });
    // The server committed, but the caller did not receive the receipt.
    StatusCode::INTERNAL_SERVER_ERROR
}

async fn receipt(
    State(state): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<ReceiptRequest>,
) -> std::result::Result<Json<DeviceReceipt>, StatusCode> {
    let state = state.lock().await;
    let receipt = state.receipt.as_ref().ok_or(StatusCode::NOT_FOUND)?;
    let endpoint = endpoint_url(&state.base, &format!("/devices/{id}/receipt")).unwrap();
    verify_client_assertion(
        &body.client_assertion,
        &receipt.identity.auth_key,
        &id,
        &endpoint,
        enrollment::unix_time().unwrap(),
    )
    .unwrap();
    Ok(Json(receipt.clone()))
}

async fn token(
    State(state): State<Shared>,
    Json(body): Json<DeviceTokenRequest>,
) -> std::result::Result<Json<DeviceTokenResponse>, StatusCode> {
    let mut state = state.lock().await;
    if state.revoked {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let receipt = state.receipt.as_ref().unwrap();
    let endpoint = endpoint_url(&state.base, "/devices/token").unwrap();
    let assertion = verify_client_assertion(
        &body.client_assertion,
        &receipt.identity.auth_key,
        &receipt.device_id,
        &endpoint,
        enrollment::unix_time().unwrap(),
    )
    .unwrap();
    assert!(state.replay.insert(assertion.jti));
    state.token_requests += 1;
    Ok(Json(DeviceTokenResponse {
        access_token: format!("test-session-{}", state.token_requests),
        token_type: "DPoP".into(),
        expires_in: 600,
        expires_at: enrollment::unix_time().unwrap() + 600,
    }))
}

async fn heartbeat(
    State(state): State<Shared>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<DeviceHeartbeat>,
) -> std::result::Result<Json<DeviceStatus>, StatusCode> {
    let mut state = state.lock().await;
    if state.revoked {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let receipt = state.receipt.as_ref().unwrap();
    assert_eq!(body.version, 1);
    assert_eq!(receipt.device_id, id);
    let token = headers["authorization"]
        .to_str()
        .unwrap()
        .strip_prefix("DPoP ")
        .unwrap();
    let endpoint = endpoint_url(&state.base, &format!("/devices/{id}/heartbeat")).unwrap();
    let claims = verify_dpop(
        headers["dpop"].to_str().unwrap(),
        &receipt.identity.auth_key,
        &DpopContext {
            method: "POST",
            url: &endpoint,
            access_token: Some(token),
            nonce: None,
            key_thumbprint: &receipt.identity.auth_key.thumbprint().unwrap(),
            now: enrollment::unix_time().unwrap(),
        },
    )
    .unwrap();
    assert!(state.replay.insert(claims.jti));
    if std::mem::take(&mut state.reject_next_heartbeat) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    state.heartbeats += 1;
    let receipt = state.receipt.as_ref().unwrap();
    Ok(Json(DeviceStatus {
        device_id: receipt.device_id.clone(),
        owner_id: receipt.owner_id.clone(),
        name: receipt.name.clone(),
        identity: receipt.identity.clone(),
        status: DeviceRegistrationStatus::Active,
        registered_at: receipt.registered_at,
        last_seen_at: Some(enrollment::unix_time().unwrap()),
        auth_epoch: receipt.auth_epoch,
    }))
}

#[tokio::test]
#[ignore = "requires binding a loopback listener"]
async fn package_recovery_session_renewal_and_revocation() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/v1", listener.local_addr().unwrap());
    let registry = Arc::new(Mutex::new(Registry {
        base: base.clone(),
        ..Registry::default()
    }));
    let router = Router::new()
        .route("/devices/enrollments", post(create))
        .route("/devices/enrollments/{id}/challenge", post(challenge))
        .route("/devices/enrollments/{id}/redeem", post(redeem))
        .route("/devices/{id}/receipt", post(receipt))
        .route("/devices/token", post(token))
        .route("/devices/{id}/heartbeat", post(heartbeat))
        .with_state(registry.clone());
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().nest("/api/v1", router))
            .await
            .unwrap()
    });
    let temp = tempfile::tempdir().unwrap();
    let controller = temp.path().join("controller");
    let package = temp.path().join("package");
    let target = supervisor::prepare_state_dir(&temp.path().join("target")).unwrap();
    let binary = temp.path().join("binary");
    std::fs::write(&binary, b"test executable").unwrap();
    let password = b"correct password kept by owner";
    let manifest = enrollment::create_package(
        &controller,
        &base,
        "edge-api",
        &package,
        "enrollment-test-user-token",
        password,
        &binary,
    )
    .await
    .unwrap();
    let controller_vault =
        vault::read_private(&controller.join(format!("{}.vault", manifest.device_id))).unwrap();
    let invitation_vault =
        vault::read_private(&controller.join(format!("{}.invitation.vault", manifest.device_id)))
            .unwrap();
    let controller_context = vault::controller_context(&manifest.device_id);
    let invitation_context = vault::invitation_context(&manifest.device_id);
    let controller_seed = vault::open(password, &controller_context, &controller_vault).unwrap();
    assert_eq!(controller_seed.len(), 32);
    assert_eq!(
        SigningKey::from_bytes(controller_seed.as_slice().try_into().unwrap()).public_key(),
        manifest.controller_key
    );
    drop(controller_seed);
    let invitation_seed = vault::open(password, &invitation_context, &invitation_vault).unwrap();
    assert_eq!(invitation_seed.len(), 32);
    assert_eq!(
        SigningKey::from_bytes(invitation_seed.as_slice().try_into().unwrap()).public_key(),
        manifest.owner_invitation_key
    );
    drop(invitation_seed);
    assert!(vault::open(password, &invitation_context, &controller_vault).is_err());
    assert!(vault::open(password, &controller_context, &invitation_vault).is_err());
    let package_json = vault::read_private(&package.join("onboarding.json")).unwrap();
    let package_value: Value = serde_json::from_slice(&package_json).unwrap();
    let mut package_fields: Vec<_> = package_value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    package_fields.sort_unstable();
    assert_eq!(
        package_fields,
        [
            "bootstrap_secret",
            "enrollment_token",
            "manifest",
            "manifest_jws"
        ]
    );
    assert!(
        !package
            .join(format!("{}.vault", manifest.device_id))
            .exists()
    );
    assert!(
        !package
            .join(format!("{}.invitation.vault", manifest.device_id))
            .exists()
    );
    assert!(
        !std::str::from_utf8(&package_json)
            .unwrap()
            .contains("enrollment-test-user-token")
    );
    assert!(enrollment::enroll(&target, &package).await.is_err());
    let pending = StateStore::open(&target.join("management.sqlite"))
        .unwrap()
        .registration()
        .unwrap()
        .unwrap();
    assert!(pending.receipt.is_none());
    assert!(pending.binding_jws.is_some());
    let receipt = enrollment::enroll(&target, &package).await.unwrap();
    assert_eq!(receipt.device_id, manifest.device_id);
    assert!(!package.join("onboarding.json").exists());
    assert_eq!(
        enrollment::recover_registration(&target).await.unwrap(),
        receipt
    );
    let session = Arc::new(DeviceSession::load(&target).unwrap().unwrap());
    let heartbeat = DeviceHeartbeat {
        version: 1,
        boot_id: Uuid::new_v4().to_string(),
        agent_version: "test".into(),
        uptime_seconds: 1,
    };
    let mut joins = Vec::new();
    for _ in 0..8 {
        let session = session.clone();
        let heartbeat = heartbeat.clone();
        joins.push(tokio::spawn(async move {
            session.heartbeat(&heartbeat).await.unwrap();
        }));
    }
    for join in joins {
        join.await.unwrap();
    }
    assert_eq!(registry.lock().await.token_requests, 1);
    assert_eq!(registry.lock().await.heartbeats, 8);
    registry.lock().await.reject_next_heartbeat = true;
    assert!(session.heartbeat(&heartbeat).await.is_err());
    session.heartbeat(&heartbeat).await.unwrap();
    assert_eq!(registry.lock().await.token_requests, 2);
    registry.lock().await.revoked = true;
    assert!(session.heartbeat(&heartbeat).await.is_err());
    assert!(session.heartbeat(&heartbeat).await.is_err());
    assert_eq!(registry.lock().await.heartbeats, 9);
    server.abort();
}
