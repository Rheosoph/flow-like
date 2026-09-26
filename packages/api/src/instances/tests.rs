use super::*;
use axum::http::{HeaderValue, StatusCode};
use flow_like::hub::{StandaloneConfig, UserTiers};
use sea_orm::{ConnectOptions, Database, DatabaseConnection, TransactionTrait};

const API: &str = "https://instance-test.example/api/v1";
const MODEL_PATH: &str = "/instances/chat/completions";

fn assert_status<T>(result: Result<T, ApiError>, expected: StatusCode) {
    assert_eq!(result.err().expect("request must fail").status(), expected);
}
fn assertion(id: &str, path: &str, key: &SigningKey) -> String {
    let current = now();
    sign_workload_assertion(
        &ClientAssertion {
            iss: id.into(),
            sub: id.into(),
            aud: endpoint_url(API, path).unwrap(),
            iat: current,
            nbf: current,
            exp: current + 60,
            jti: uuid::Uuid::new_v4().to_string(),
        },
        key,
    )
    .unwrap()
}
fn token_request(id: &str, key: &SigningKey) -> InstanceTokenRequest {
    InstanceTokenRequest {
        client_assertion: assertion(id, &format!("/instances/{id}/token"), key),
    }
}
fn headers(session: &InstanceTokenResponse, key: &SigningKey, nonce: Option<&str>) -> HeaderMap {
    proof_headers(session, key, nonce, "POST", MODEL_PATH)
}
fn proof_headers(
    session: &InstanceTokenResponse,
    key: &SigningKey,
    nonce: Option<&str>,
    method: &str,
    path: &str,
) -> HeaderMap {
    let proof = sign_dpop(
        &DpopProof {
            jti: uuid::Uuid::new_v4().to_string(),
            htm: method.into(),
            htu: endpoint_url(API, path).unwrap(),
            iat: now(),
            ath: Some(access_token_hash(&session.access_token)),
            nonce: nonce.map(str::to_owned),
        },
        key,
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_str(&format!("DPoP {}", session.access_token)).unwrap(),
    );
    headers.insert("dpop", HeaderValue::from_str(&proof).unwrap());
    headers
}
fn registration(
    grant: &ResourceGrantResponse,
    billing: &BillingGrantResponse,
    id: &str,
    device: &SigningKey,
    workload: &SigningKey,
) -> InstanceRegistrationRequest {
    registration_optional(grant, Some(billing), id, device, workload)
}
fn registration_optional(
    grant: &ResourceGrantResponse,
    billing: Option<&BillingGrantResponse>,
    id: &str,
    device: &SigningKey,
    workload: &SigningKey,
) -> InstanceRegistrationRequest {
    registration_for_purpose(
        grant,
        billing,
        id,
        device,
        workload,
        InstancePurpose::Workload,
    )
}
fn registration_for_purpose(
    grant: &ResourceGrantResponse,
    billing: Option<&BillingGrantResponse>,
    id: &str,
    device: &SigningKey,
    workload: &SigningKey,
    purpose: InstancePurpose,
) -> InstanceRegistrationRequest {
    let current = now();
    let binding = InstanceRegistration {
        version: PROTOCOL_VERSION,
        purpose,
        device_id: grant.device_id.clone(),
        device_auth_epoch: 1,
        instance_id: id.into(),
        placement_id: grant.placement_id.clone(),
        deployment_id: grant.deployment_id.clone(),
        project_id: grant.project_id.clone(),
        grant_id: grant.grant_id.clone(),
        authz_version: grant.authz_version,
        billing_grant_id: billing.map(|billing| billing.billing_grant_id.clone()),
        billing_authz_version: billing.map(|billing| billing.authz_version),
        workload_key: workload.public_key(),
        aud: endpoint_url(API, &format!("/devices/{}/instances", grant.device_id)).unwrap(),
        iat: current,
        nbf: current,
        exp: current + 60,
        jti: uuid::Uuid::new_v4().to_string(),
    };
    let signed = sign_instance_registration(&binding, device).unwrap();
    let proof = InstancePossession {
        iss: id.into(),
        sub: id.into(),
        aud: binding.aud,
        iat: current,
        nbf: current,
        exp: current + 60,
        jti: uuid::Uuid::new_v4().to_string(),
        registration_digest: compact_digest(&signed),
    };
    InstanceRegistrationRequest {
        registration_jws: signed,
        possession_jws: sign_instance_possession(&proof, workload).unwrap(),
    }
}
fn request(placement: &str) -> CreateResourceGrantRequest {
    CreateResourceGrantRequest {
        placement_id: placement.into(),
        deployment_id: "deployment".into(),
        project_id: "offline-project".into(),
        app_id: None,
        online_access: None,
        model_ids: vec!["model".into()],
        max_instances: 1,
        expires_at: now() + 3600,
    }
}
fn tiers() -> UserTiers {
    serde_json::from_value(serde_json::json!({"FREE":{"max_non_visible_projects":10,"max_remote_executions":100,"execution_tier":"FREE","max_total_size":1000000,"max_llm_cost":1000000,"max_llm_calls":100,"llm_tiers":["FREE"],"product_id":null}})).unwrap()
}

async fn validation_instances_are_metadata_only_and_bounded(
    state: &DeviceContext<'_>,
    db: &DatabaseConnection,
    grant: &ResourceGrantResponse,
    device: &SigningKey,
) {
    let first = SigningKey::generate();
    let second = SigningKey::generate();
    let third = SigningKey::generate();
    let register_validation = |id, key: &SigningKey| {
        registration_for_purpose(
            grant,
            None,
            id,
            device,
            key,
            InstancePurpose::RolloutValidation,
        )
    };
    // The live worker already occupies max_instances=1. Validation has a
    // separate device bound and cannot create extra serving capacity.
    let registered = register(
        state,
        &grant.device_id,
        register_validation("validation-one", &first),
    )
    .await
    .unwrap();
    assert_eq!(registered.purpose, InstancePurpose::RolloutValidation);
    assert_eq!(
        read_instance(db, "validation-one").await.unwrap().status,
        "validating"
    );
    // This is the authorization predicate used by API replicas before validation
    // existed. Such replicas must never admit a new validation identity.
    assert_eq!(
        db.execute_raw(sql(
            r#"UPDATE "WorkloadInstance" SET "keyEpoch"="keyEpoch" WHERE id=$1 AND status='active'"#,
            ["validation-one".into()],
        ))
        .await
        .unwrap()
        .rows_affected(),
        0
    );
    register(
        state,
        &grant.device_id,
        register_validation("validation-two", &second),
    )
    .await
    .unwrap();
    let other_grant = create_grant(
        state,
        &grant.delegating_user_id,
        &grant.device_id,
        CreateResourceGrantRequest {
            placement_id: "validation-other-placement".into(),
            deployment_id: grant.deployment_id.clone(),
            project_id: grant.project_id.clone(),
            app_id: grant.app_id.clone(),
            online_access: grant.online_access,
            model_ids: Vec::new(),
            max_instances: 1,
            expires_at: grant.expires_at,
        },
    )
    .await
    .unwrap();
    let fourth = SigningKey::generate();
    let other_request = || {
        registration_for_purpose(
            &other_grant,
            None,
            "validation-four",
            device,
            &fourth,
            InstancePurpose::RolloutValidation,
        )
    };
    assert_status(
        register(state, &grant.device_id, other_request()).await,
        StatusCode::TOO_MANY_REQUESTS,
    );
    assert_status(
        register(
            state,
            &grant.device_id,
            register_validation("validation-three", &third),
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
    );
    assert_status(
        register(
            state,
            &grant.device_id,
            registration_optional(grant, None, "extra-worker", device, &third),
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
    );
    assert_status(
        token(
            state,
            "validation-one",
            token_request("validation-one", &first),
        )
        .await,
        StatusCode::FORBIDDEN,
    );
    let issue = || InstanceTokenRequest {
        client_assertion: assertion(
            "validation-one",
            "/instances/validation-one/project-token",
            &first,
        ),
    };
    for (status, purpose) in [
        ("active", Some("rollout_validation")),
        ("validating", Some("workload")),
        ("validating", None),
    ] {
        db.execute_raw(sql(
            r#"UPDATE "WorkloadInstance" SET status=$2,purpose=$3 WHERE id=$1"#,
            [
                "validation-one".into(),
                status.into(),
                purpose.map(str::to_owned).into(),
            ],
        ))
        .await
        .unwrap();
        assert_status(
            project::token(state, "validation-one", issue()).await,
            StatusCode::UNAUTHORIZED,
        );
    }
    db.execute_raw(sql(
        r#"UPDATE "WorkloadInstance" SET status='validating',purpose='rollout_validation' WHERE id=$1"#,
        ["validation-one".into()],
    ))
    .await
    .unwrap();
    let session = project::token(state, "validation-one", issue())
        .await
        .unwrap();
    assert_eq!(session.lease_expires_at, registered.lease_expires_at);
    for path in [
        "/instances/project/app",
        "/instances/project/events/event/versions/1/0/0",
        "/instances/project/boards/board/versions/1/0/0/pages/page",
    ] {
        let headers = proof_headers(&session, &first, Some(&session.dpop_nonce), "GET", path);
        let authorized = project::authenticate(state, &headers, "GET", path)
            .await
            .unwrap();
        project::recheck(state, &authorized).await.unwrap();
    }
    for path in [
        "/instances/project/storage",
        "/instances/project/storage/storage",
        "/instances/project/storage/metadata",
        OFFLINE_REPLAY_PATH,
    ] {
        assert_status(
            project::authenticate(
                state,
                &proof_headers(&session, &first, Some(&session.dpop_nonce), "POST", path),
                "POST",
                path,
            )
            .await,
            StatusCode::FORBIDDEN,
        );
    }
    // A caller cannot relabel a correctly signed validation token as a worker.
    let mut claims: serde_json::Value = backend_jwt::verify_typed(
        &session.access_token,
        TokenType::InstanceProject,
        "flow-like-instance-project+jwt",
    )
    .unwrap();
    claims["purpose"] = "workload".into();
    claims["access"] = "read_write".into();
    claims["scope"] = INSTANCE_PROJECT_WRITE_SCOPE.into();
    let forged = InstanceTokenResponse {
        access_token: backend_jwt::sign_typed(&claims, "flow-like-instance-project+jwt").unwrap(),
        token_type: session.token_type.clone(),
        expires_in: session.expires_in,
        expires_at: session.expires_at,
        dpop_nonce: session.dpop_nonce.clone(),
        lease_expires_at: session.lease_expires_at,
    };
    assert_status(
        project::authenticate(
            state,
            &proof_headers(
                &forged,
                &first,
                Some(&forged.dpop_nonce),
                "GET",
                "/instances/project/app",
            ),
            "GET",
            "/instances/project/app",
        )
        .await,
        StatusCode::FORBIDDEN,
    );
    // Even a future lease cannot revive the original validation key after its
    // fixed registration window. Expiry frees its separate capacity slot.
    db.execute_raw(sql(
        r#"UPDATE "WorkloadInstance" SET "registeredAt"=$2,"leaseExpiresAt"=$3 WHERE id=$1"#,
        [
            "validation-one".into(),
            (now() - INSTANCE_VALIDATION_SECONDS - 1).into(),
            (now() + 600).into(),
        ],
    ))
    .await
    .unwrap();
    assert_status(
        project::token(state, "validation-one", issue()).await,
        StatusCode::UNAUTHORIZED,
    );
    assert!(
        list_instances(db, &grant.device_id)
            .await
            .unwrap()
            .iter()
            .all(|instance| instance.instance_id != "validation-one")
    );
    register(
        state,
        &grant.device_id,
        register_validation("validation-three", &third),
    )
    .await
    .unwrap();
    let current = now();
    let retire_proof = sign_client_assertion(
        &ClientAssertion {
            iss: grant.device_id.clone(),
            sub: grant.device_id.clone(),
            aud: endpoint_url(
                API,
                &format!("/devices/{}/instances/validation-two", grant.device_id),
            )
            .unwrap(),
            iat: current,
            nbf: current,
            exp: current + 60,
            jti: uuid::Uuid::new_v4().to_string(),
        },
        device,
    )
    .unwrap();
    retire(
        state,
        &grant.device_id,
        "validation-two",
        ReceiptRequest {
            client_assertion: retire_proof,
        },
    )
    .await
    .unwrap();
    assert_status(
        project::token(
            state,
            "validation-two",
            InstanceTokenRequest {
                client_assertion: assertion(
                    "validation-two",
                    "/instances/validation-two/project-token",
                    &second,
                ),
            },
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    register(state, &grant.device_id, other_request())
        .await
        .unwrap();
    let fourth_session = project::token(
        state,
        "validation-four",
        InstanceTokenRequest {
            client_assertion: assertion(
                "validation-four",
                "/instances/validation-four/project-token",
                &fourth,
            ),
        },
    )
    .await
    .unwrap();
    let auth = project::authenticate(
        state,
        &proof_headers(
            &fourth_session,
            &fourth,
            Some(&fourth_session.dpop_nonce),
            "GET",
            "/instances/project/app",
        ),
        "GET",
        "/instances/project/app",
    )
    .await
    .unwrap();
    revoke_grant(
        state,
        &grant.delegating_user_id,
        &grant.device_id,
        &other_grant.grant_id,
    )
    .await
    .unwrap();
    assert_status(
        project::recheck(state, &auth).await,
        StatusCode::UNAUTHORIZED,
    );
}
async fn seed_device(
    db: &DatabaseConnection,
    key: &SigningKey,
) -> (String, SigningKey, SigningKey) {
    let device_id = uuid::Uuid::new_v4().to_string();
    let bootstrap = SigningKey::generate();
    let controller = SigningKey::generate();
    let invitation = SigningKey::generate();
    let current = now();
    let manifest = OnboardingManifest {
        version: PROTOCOL_VERSION,
        enrollment_id: uuid::Uuid::new_v4().to_string(),
        device_id: device_id.clone(),
        owner_id: "owner".into(),
        name: "Instance test device".into(),
        api_base_url: API.into(),
        bootstrap_key: bootstrap.public_key(),
        controller_key: controller.public_key(),
        owner_invitation_key: invitation.public_key(),
        issued_at: current,
        expires_at: current + 3600,
    };
    let identity = DeviceIdentity {
        auth_key: key.public_key(),
        management_key: [42; 32],
        telemetry_key: SigningKey::generate().public_key(),
    };
    let manifest_jws = sign_manifest(&manifest, &controller).unwrap();
    let binding = EnrollmentBinding {
        version: PROTOCOL_VERSION,
        enrollment_id: manifest.enrollment_id.clone(),
        device_id: device_id.clone(),
        identity: identity.clone(),
        manifest_digest: compact_digest(&manifest_jws),
        challenge_id: "challenge".into(),
        challenge_nonce: "random-test-challenge-1234".into(),
        issued_at: current,
        expires_at: current + 60,
    };
    let receipt = DeviceReceipt {
        enrollment_id: manifest.enrollment_id.clone(),
        device_id: device_id.clone(),
        owner_id: "owner".into(),
        name: manifest.name.clone(),
        identity: identity.clone(),
        manifest_jws,
        binding_jws: sign_binding(&binding, &bootstrap).unwrap(),
        registered_at: current,
        auth_epoch: 1,
    };
    db.execute_raw(sql(r#"INSERT INTO "ManagedDevice" (id,"ownerId",name,status,"authEpoch",identity,receipt,"registeredAt") VALUES ($1,'owner','Device','active',1,$2,$3,$4)"#,[device_id.clone().into(),serde_json::to_string(&identity).unwrap().into(),serde_json::to_string(&receipt).unwrap().into(),current.into()])).await.unwrap();
    db.execute_raw(sql(r#"INSERT INTO "DeviceEnrollment" (id,"deviceId","ownerId","jwtId",manifest,status,"expiresAt","createdAt","consumedAt") VALUES ($1,$2,'owner',$1,$3,'consumed',$4,$5,$5)"#,[manifest.enrollment_id.clone().into(),device_id.clone().into(),serde_json::to_string(&manifest).unwrap().into(),manifest.expires_at.into(),current.into()])).await.unwrap();
    (device_id, controller, invitation)
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn signed_instances_enforce_consent_scope_leases_and_revocation() {
    backend_jwt::init_for_tests();
    let url =
        std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").expect("Use disposable PostgreSQL");
    let admin = Database::connect(&url).await.unwrap();
    let schema = format!("instance_test_{}", uuid::Uuid::new_v4().simple());
    admin
        .execute_unprepared(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();
    let mut options = ConnectOptions::new(url);
    options
        .set_schema_search_path(&schema)
        .max_connections(8)
        .min_connections(1);
    let db = Database::connect(options).await.unwrap();
    for migration in [
        include_str!("../../prisma/migrations/20260921120000_standalone_devices/migration.sql"),
        include_str!("../../prisma/migrations/20260921140000_instance_resources/migration.sql"),
        include_str!("../../prisma/migrations/20260922120000_device_management/migration.sql"),
        include_str!(
            "../../prisma/migrations/20260922010000_instance_online_resources/migration.sql"
        ),
        include_str!("../../prisma/migrations/20260923120000_instance_validation/migration.sql"),
        include_str!(
            "../../prisma/migrations/20260923150000_instance_offline_replay/migration.sql"
        ),
    ] {
        for statement in migration.split(';').filter(|s| !s.trim().is_empty()) {
            db.execute_unprepared(statement).await.unwrap();
        }
    }
    for statement in [
        r#"CREATE TABLE "User" (id TEXT PRIMARY KEY,status TEXT NOT NULL,tier TEXT NOT NULL DEFAULT 'FREE',"updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#,
        r#"INSERT INTO "User" (id,status) VALUES ('owner','ACTIVE'),('other','ACTIVE')"#,
        r#"CREATE TABLE "Bit" (id TEXT PRIMARY KEY,type TEXT NOT NULL,parameters JSONB NOT NULL,"updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#,
        r#"CREATE TABLE "App" (id TEXT PRIMARY KEY,status TEXT NOT NULL,"ownerRoleId" TEXT)"#,
        r#"CREATE TABLE "Membership" ("userId" TEXT,"appId" TEXT,"roleId" TEXT, PRIMARY KEY ("userId","appId"))"#,
        r#"CREATE TABLE "Role" (id TEXT PRIMARY KEY,"appId" TEXT,permissions BIGINT NOT NULL)"#,
        r#"CREATE TABLE "ProjectCapacity" ("appId" TEXT PRIMARY KEY,"payerId" TEXT)"#,
        r#"CREATE TABLE "MutationLock" (id BIGINT PRIMARY KEY,owner TEXT,"expiresAt" TIMESTAMPTZ,"updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#,
        r#"CREATE TABLE "AccountCapacity" ("payerId" TEXT PRIMARY KEY,"storageBytes" BIGINT NOT NULL DEFAULT 0,"reservedStorageBytes" BIGINT NOT NULL DEFAULT 0,"projectCount" BIGINT NOT NULL DEFAULT 0,initialized BOOLEAN NOT NULL DEFAULT false,"baselineAppId" TEXT NOT NULL DEFAULT '',"updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#,
        r#"CREATE TABLE "StorageUploadGrant" (id TEXT PRIMARY KEY,"payerId" TEXT NOT NULL,"appId" TEXT NOT NULL,bucket TEXT NOT NULL,"objectKey" TEXT NOT NULL,"maxBytes" BIGINT NOT NULL,"reservedBytes" BIGINT NOT NULL,"expiresAt" TIMESTAMPTZ NOT NULL,"updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now())"#,
        r#"CREATE TABLE "FileAccountingObject" (id TEXT PRIMARY KEY,size BIGINT NOT NULL)"#,
    ] {
        db.execute_unprepared(statement).await.unwrap();
    }
    let bit_type = "LLM";
    let parameters = serde_json::json!({"context_length":32768,"model_classification":flow_like::bit::BitModelClassification::default(),"provider":{"provider_name":"hosted:openrouter","model_id":"upstream-model","params":{"tier":"FREE"}}});
    db.execute_raw(sql(
        r#"INSERT INTO "Bit" (id,type,parameters) VALUES ('model',$1,$2)"#,
        [bit_type.into(), parameters.into()],
    ))
    .await
    .unwrap();
    let policy = StandaloneConfig {
        enabled: true,
        api_base_url: Some(API.into()),
        ..Default::default()
    };
    let state = DeviceContext {
        db: &db,
        dialect: crate::db::DbDialect::Postgres,
        config: &policy,
        domain: "unused.example",
        secure: true,
    };
    let device_key = SigningKey::generate();
    let (device_id, controller_key, _) = seed_device(&db, &device_key).await;
    assert_status(
        create_grant(&state, "other", &device_id, request("placement")).await,
        StatusCode::NOT_FOUND,
    );
    let mut invalid_limit = request("placement");
    invalid_limit.max_instances = 0;
    assert_status(
        create_grant(&state, "owner", &device_id, invalid_limit).await,
        StatusCode::BAD_REQUEST,
    );
    let grant = create_grant(&state, "owner", &device_id, request("placement"))
        .await
        .unwrap();
    assert_eq!(
        get_grant(&state, "owner", &device_id, &grant.grant_id)
            .await
            .unwrap(),
        grant
    );
    assert_status(
        create_grant(&state, "owner", &device_id, request("placement")).await,
        StatusCode::CONFLICT,
    );
    assert_status(
        approve_billing(
            &state,
            "other",
            &device_id,
            &grant.grant_id,
            ApproveBillingGrantRequest {
                limit_micros: 100,
                expires_at: now() + 1800,
            },
        )
        .await,
        StatusCode::NOT_FOUND,
    );
    let billing = approve_billing(
        &state,
        "owner",
        &device_id,
        &grant.grant_id,
        ApproveBillingGrantRequest {
            limit_micros: 100,
            expires_at: now() + 1800,
        },
    )
    .await
    .unwrap();
    assert_eq!(billing.payer_id, "owner");
    assert_eq!(
        get_billing(&state, "owner", &device_id, &grant.grant_id)
            .await
            .unwrap(),
        billing
    );
    assert_status(
        approve_billing(
            &state,
            "owner",
            &device_id,
            &grant.grant_id,
            ApproveBillingGrantRequest {
                limit_micros: 100,
                expires_at: now() + 1800,
            },
        )
        .await,
        StatusCode::CONFLICT,
    );

    assert_status(
        register(
            &state,
            &device_id,
            registration(
                &grant,
                &billing,
                "controller-key-replica",
                &device_key,
                &controller_key,
            ),
        )
        .await,
        StatusCode::FORBIDDEN,
    );
    let workload = SigningKey::generate();
    let id = "replica-one";
    let valid_registration = registration(&grant, &billing, id, &device_key, &workload);
    let bad_registration = InstanceRegistrationRequest {
        registration_jws: valid_registration.registration_jws.clone(),
        possession_jws: "invalid".into(),
    };
    assert_status(
        register(&state, &device_id, bad_registration).await,
        StatusCode::UNAUTHORIZED,
    );
    let registered = register(&state, &device_id, valid_registration)
        .await
        .unwrap();
    assert_eq!(registered.workload_key, workload.public_key());
    // Rows created before the purpose migration remain ordinary workloads.
    db.execute_raw(sql(
        r#"UPDATE "WorkloadInstance" SET purpose=NULL WHERE id=$1"#,
        [id.into()],
    ))
    .await
    .unwrap();
    assert_eq!(
        read_instance(&db, id).await.unwrap().receipt.purpose,
        InstancePurpose::Workload
    );
    assert_status(
        register(
            &state,
            &device_id,
            registration(
                &grant,
                &billing,
                "reused-key-replica",
                &device_key,
                &workload,
            ),
        )
        .await,
        StatusCode::CONFLICT,
    );
    let competitor = SigningKey::generate();
    assert_status(
        register(
            &state,
            &device_id,
            registration(&grant, &billing, "replica-two", &device_key, &competitor),
        )
        .await,
        StatusCode::TOO_MANY_REQUESTS,
    );
    let issued_request = token_request(id, &workload);
    let replay = issued_request.client_assertion.clone();
    let session = token(&state, id, issued_request).await.unwrap();
    assert_eq!(session.token_type, "DPoP");
    assert!((1..=300).contains(&session.expires_in));
    assert_status(
        token(
            &state,
            id,
            InstanceTokenRequest {
                client_assertion: replay,
            },
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        authenticate_model_request_with_context(
            &state,
            &headers(&session, &workload, None),
            "POST",
            MODEL_PATH,
            "model",
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        authenticate_model_request_with_context(
            &state,
            &headers(&session, &workload, Some(&session.dpop_nonce)),
            "POST",
            MODEL_PATH,
            "other-model",
        )
        .await,
        StatusCode::FORBIDDEN,
    );
    let request_headers = headers(&session, &workload, Some(&session.dpop_nonce));
    let usage = authenticate_model_request_with_context(
        &state,
        &request_headers,
        "POST",
        MODEL_PATH,
        "model",
    )
    .await
    .unwrap();
    assert_eq!(usage.delegated_user_id, "owner");
    assert_eq!(usage.payer_id, "owner");
    assert_status(
        authenticate_model_request_with_context(
            &state,
            &request_headers,
            "POST",
            MODEL_PATH,
            "model",
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    let recovered = receipt(
        &state,
        id,
        ReceiptRequest {
            client_assertion: assertion(id, &format!("/instances/{id}/receipt"), &workload),
        },
    )
    .await
    .unwrap();
    assert_eq!(recovered.registration_jws, registered.registration_jws);

    // Model requests reserve one aggregate budget, even across different tokens.
    let tiers = tiers();
    let tx = db.begin().await.unwrap();
    reserve_budget(&tx, &usage, "first", 60, "FREE", &tiers)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let tx = db.begin().await.unwrap();
    assert_status(
        reserve_budget(&tx, &usage, "second", 41, "FREE", &tiers).await,
        StatusCode::TOO_MANY_REQUESTS,
    );
    tx.rollback().await.unwrap();
    let tx = db.begin().await.unwrap();
    authorize_start(&tx, "first", &tiers).await.unwrap();
    settle_budget(&tx, "first", 25, true).await.unwrap();
    tx.commit().await.unwrap();
    let billed = get_billing(&state, "owner", &device_id, &grant.grant_id)
        .await
        .unwrap();
    assert_eq!((billed.used_micros, billed.reserved_micros), (25, 0));

    // Expired live-process leases reacquire a slot for the same key. They cannot
    // oversubscribe a slot already held by a replacement process.
    db.execute_raw(sql(
        r#"UPDATE "WorkloadInstance" SET "leaseExpiresAt"=$2 WHERE id=$1"#,
        [id.into(), (now() - 1).into()],
    ))
    .await
    .unwrap();
    register(
        &state,
        &device_id,
        registration(&grant, &billing, "replica-two", &device_key, &competitor),
    )
    .await
    .unwrap();
    assert_status(
        token(&state, id, token_request(id, &workload)).await,
        StatusCode::TOO_MANY_REQUESTS,
    );
    let current = now();
    let retire_path = format!("/devices/{device_id}/instances/replica-two");
    let retire_assertion = sign_client_assertion(
        &ClientAssertion {
            iss: device_id.clone(),
            sub: device_id.clone(),
            aud: endpoint_url(API, &retire_path).unwrap(),
            iat: current,
            nbf: current,
            exp: current + 60,
            jti: uuid::Uuid::new_v4().to_string(),
        },
        &device_key,
    )
    .unwrap();
    retire(
        &state,
        &device_id,
        "replica-two",
        ReceiptRequest {
            client_assertion: retire_assertion,
        },
    )
    .await
    .unwrap();
    let renewed = token(&state, id, token_request(id, &workload))
        .await
        .unwrap();
    assert_status(
        token(
            &state,
            "replica-two",
            token_request("replica-two", &competitor),
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );

    // Current consent is checked even while an already-issued JWT is unexpired.
    revoke_billing(&state, "owner", &device_id, &billing.billing_grant_id)
        .await
        .unwrap();
    assert_status(
        authenticate_model_request_with_context(
            &state,
            &headers(&renewed, &workload, Some(&renewed.dpop_nonce)),
            "POST",
            MODEL_PATH,
            "model",
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        token(&state, id, token_request(id, &workload)).await,
        StatusCode::UNAUTHORIZED,
    );
    let replacement = approve_billing(
        &state,
        "owner",
        &device_id,
        &grant.grant_id,
        ApproveBillingGrantRequest {
            limit_micros: 100,
            expires_at: now() + 1000,
        },
    )
    .await
    .unwrap();
    assert_ne!(replacement.billing_grant_id, billing.billing_grant_id);
    assert_status(
        token(&state, id, token_request(id, &workload)).await,
        StatusCode::UNAUTHORIZED,
    );
    let replacement_key = SigningKey::generate();
    register(
        &state,
        &device_id,
        registration(
            &grant,
            &replacement,
            "replica-three",
            &device_key,
            &replacement_key,
        ),
    )
    .await
    .unwrap();
    // Old registrations cannot acquire an updated authority epoch on renewal.
    db.execute_raw(sql(
        r#"UPDATE "PlacementResourceGrant" SET "authzVersion"=2 WHERE id=$1"#,
        [grant.grant_id.clone().into()],
    ))
    .await
    .unwrap();
    assert_status(
        token(
            &state,
            "replica-three",
            token_request("replica-three", &replacement_key),
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    revoke_grant(&state, "owner", &device_id, &grant.grant_id)
        .await
        .unwrap();
    let new_grant = create_grant(&state, "owner", &device_id, request("placement"))
        .await
        .unwrap();
    assert_ne!(new_grant.grant_id, grant.grant_id);

    // Online grants are fenced against membership removal before final admission.
    db.execute_unprepared(r#"INSERT INTO "App" VALUES ('online','ACTIVE','owner-role'); INSERT INTO "Role" VALUES ('owner-role','online',1); INSERT INTO "Membership" VALUES ('owner','online','owner-role')"#).await.unwrap();
    let mut online = request("online-placement");
    online.project_id = "online".into();
    online.app_id = Some("online".into());
    let online_grant = create_grant(&state, "owner", &device_id, online)
        .await
        .unwrap();
    let online_billing = approve_billing(
        &state,
        "owner",
        &device_id,
        &online_grant.grant_id,
        ApproveBillingGrantRequest {
            limit_micros: 100,
            expires_at: now() + 1000,
        },
    )
    .await
    .unwrap();
    let online_key = SigningKey::generate();
    register(
        &state,
        &device_id,
        registration(
            &online_grant,
            &online_billing,
            "online-instance",
            &device_key,
            &online_key,
        ),
    )
    .await
    .unwrap();
    let online_session = token(
        &state,
        "online-instance",
        token_request("online-instance", &online_key),
    )
    .await
    .unwrap();
    let online_usage = authenticate_model_request_with_context(
        &state,
        &headers(
            &online_session,
            &online_key,
            Some(&online_session.dpop_nonce),
        ),
        "POST",
        MODEL_PATH,
        "model",
    )
    .await
    .unwrap();
    let remove = db.begin().await.unwrap();
    remove
        .execute_unprepared(
            r#"DELETE FROM "Membership" WHERE "userId"='owner' AND "appId"='online'"#,
        )
        .await
        .unwrap();
    let (admission, _) = flow_like_types::tokio::join!(
        async {
            let tx = db.begin().await.unwrap();
            let result =
                reserve_budget(&tx, &online_usage, "removed-member", 1, "FREE", &tiers).await;
            tx.rollback().await.unwrap();
            result
        },
        async {
            flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            remove.commit().await.unwrap();
        }
    );
    assert_status(admission, StatusCode::FORBIDDEN);
    assert_eq!(
        get_billing(&state, "owner", &device_id, &online_grant.grant_id)
            .await
            .unwrap()
            .reserved_micros,
        0
    );

    // Storage-only placements have no personal AI allowance. Their project
    // token cannot enter a model route or reach another project's API paths.
    db.execute_unprepared(r#"INSERT INTO "App" VALUES ('storage-project','ACTIVE','storage-owner-role'); INSERT INTO "Role" VALUES ('storage-owner-role','storage-project',1); INSERT INTO "Membership" VALUES ('owner','storage-project','storage-owner-role')"#).await.unwrap();
    let mut storage_request = request("storage-placement");
    storage_request.app_id = Some("storage-project".into());
    storage_request.project_id = "storage-project".into();
    storage_request.online_access = Some(OnlineProjectAccess::ReadWrite);
    storage_request.model_ids.clear();
    let storage_grant = create_grant(&state, "owner", &device_id, storage_request.clone())
        .await
        .unwrap();
    assert_status(
        approve_billing(
            &state,
            "owner",
            &device_id,
            &storage_grant.grant_id,
            ApproveBillingGrantRequest {
                limit_micros: 100,
                expires_at: now() + 1000,
            },
        )
        .await,
        StatusCode::BAD_REQUEST,
    );
    let storage_key = SigningKey::generate();
    let storage_id = "storage-instance";
    let registered = register(
        &state,
        &device_id,
        registration_optional(&storage_grant, None, storage_id, &device_key, &storage_key),
    )
    .await
    .unwrap();
    assert!(registered.billing_grant_id.is_none());
    validation_instances_are_metadata_only_and_bounded(&state, &db, &storage_grant, &device_key)
        .await;
    assert_status(
        token(&state, storage_id, token_request(storage_id, &storage_key)).await,
        StatusCode::FORBIDDEN,
    );
    let issue_project = || InstanceTokenRequest {
        client_assertion: assertion(
            storage_id,
            &format!("/instances/{storage_id}/project-token"),
            &storage_key,
        ),
    };
    // Cloud credentials can remain valid while an idle worker's short registry
    // lease expires. Its same workload key renews registration on the next refresh.
    db.execute_raw(sql(
        r#"UPDATE "WorkloadInstance" SET "leaseExpiresAt"=$2 WHERE id=$1"#,
        [storage_id.into(), (now() - 1).into()],
    ))
    .await
    .unwrap();
    let project_session = project::token(&state, storage_id, issue_project())
        .await
        .unwrap();
    assert!(project_session.lease_expires_at > now());
    assert_eq!(
        read_instance(&db, storage_id)
            .await
            .unwrap()
            .receipt
            .lease_expires_at,
        project_session.lease_expires_at
    );
    let project_claims: serde_json::Value = backend_jwt::verify_typed(
        &project_session.access_token,
        crate::backend_jwt::TokenType::InstanceProject,
        "flow-like-instance-project+jwt",
    )
    .unwrap();
    assert_eq!(project_claims["sub"], storage_grant.delegating_user_id);
    assert!(project_session.expires_at <= now() + MAX_INSTANCE_PROJECT_TOKEN_SECONDS);

    assert!(crate::devices::jwt::is_device_credential(
        &project_session.access_token
    ));
    assert_status(
        authenticate_model_request_with_context(
            &state,
            &headers(
                &project_session,
                &storage_key,
                Some(&project_session.dpop_nonce),
            ),
            "POST",
            MODEL_PATH,
            "model",
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    let path = "/instances/project/storage";
    let project_headers = proof_headers(
        &project_session,
        &storage_key,
        Some(&project_session.dpop_nonce),
        "POST",
        path,
    );
    let authorized = project::authenticate(&state, &project_headers, "POST", path)
        .await
        .unwrap();
    assert_status(
        project::authenticate(&state, &project_headers, "POST", path).await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        project::authenticate(
            &state,
            &proof_headers(&project_session, &storage_key, None, "POST", path),
            "POST",
            path,
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        project::authenticate(
            &state,
            &proof_headers(
                &project_session,
                &storage_key,
                Some(&project_session.dpop_nonce),
                "POST",
                path,
            ),
            "POST",
            "/instances/project/storage/storage",
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    assert!(project::recheck(&state, &authorized).await.unwrap() <= project_session.expires_at);
    let storage_deadline = project::recheck_storage(&state, &authorized).await.unwrap();
    assert_eq!(storage_deadline, storage_grant.expires_at);
    assert!(storage_deadline > project_session.lease_expires_at);
    assert!(storage_deadline > project_session.expires_at);
    let offline_auth = offline::authorize(
        &state,
        &proof_headers(
            &project_session,
            &storage_key,
            Some(&project_session.dpop_nonce),
            "POST",
            OFFLINE_REPLAY_PATH,
        ),
    )
    .await
    .unwrap();
    offline::assert_receipt_lifecycle(&state, &offline_auth).await;
    #[cfg(feature = "aws")]
    offline::assert_file_http_lifecycle(&state, &project_session, &storage_key).await;
    let mut read_only_request = storage_request.clone();
    read_only_request.placement_id = "read-only-storage".into();
    read_only_request.online_access = Some(OnlineProjectAccess::ReadOnly);
    let read_only_grant = create_grant(&state, "owner", &device_id, read_only_request)
        .await
        .unwrap();
    let read_only_key = SigningKey::generate();
    register(
        &state,
        &device_id,
        registration_optional(
            &read_only_grant,
            None,
            "read-only-instance",
            &device_key,
            &read_only_key,
        ),
    )
    .await
    .unwrap();
    let read_only_session = project::token(
        &state,
        "read-only-instance",
        InstanceTokenRequest {
            client_assertion: assertion(
                "read-only-instance",
                "/instances/read-only-instance/project-token",
                &read_only_key,
            ),
        },
    )
    .await
    .unwrap();
    assert_status(
        offline::authorize(
            &state,
            &proof_headers(
                &read_only_session,
                &read_only_key,
                Some(&read_only_session.dpop_nonce),
                "POST",
                OFFLINE_REPLAY_PATH,
            ),
        )
        .await,
        StatusCode::FORBIDDEN,
    );

    // A provider response produced between these transactions must be discarded.
    revoke_grant(&state, "owner", &device_id, &storage_grant.grant_id)
        .await
        .unwrap();
    offline::assert_revoked_attempt(&state, &offline_auth).await;
    assert_status(
        project::recheck(&state, &authorized).await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        project::recheck_storage(&state, &authorized).await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        project::token(&state, storage_id, issue_project()).await,
        StatusCode::UNAUTHORIZED,
    );

    // An independently approved AI allowance can expire without stopping the
    // online project. The workload keeps its original billing binding forever.
    storage_request.placement_id = "combined-placement".into();
    storage_request.model_ids = vec!["model".into()];
    let combined = create_grant(&state, "owner", &device_id, storage_request.clone())
        .await
        .unwrap();
    let combined_billing = approve_billing(
        &state,
        "owner",
        &device_id,
        &combined.grant_id,
        ApproveBillingGrantRequest {
            limit_micros: 100,
            expires_at: now() + 1000,
        },
    )
    .await
    .unwrap();
    let combined_key = SigningKey::generate();
    let combined_id = "combined-instance";
    register(
        &state,
        &device_id,
        registration(
            &combined,
            &combined_billing,
            combined_id,
            &device_key,
            &combined_key,
        ),
    )
    .await
    .unwrap();
    token(
        &state,
        combined_id,
        token_request(combined_id, &combined_key),
    )
    .await
    .unwrap();
    revoke_billing(
        &state,
        "owner",
        &device_id,
        &combined_billing.billing_grant_id,
    )
    .await
    .unwrap();
    assert_status(
        token(
            &state,
            combined_id,
            token_request(combined_id, &combined_key),
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    let issue_combined = || InstanceTokenRequest {
        client_assertion: assertion(
            combined_id,
            &format!("/instances/{combined_id}/project-token"),
            &combined_key,
        ),
    };
    let combined_session = project::token(&state, combined_id, issue_combined())
        .await
        .unwrap();
    let combined_auth = project::authenticate(
        &state,
        &proof_headers(
            &combined_session,
            &combined_key,
            Some(&combined_session.dpop_nonce),
            "POST",
            path,
        ),
        "POST",
        path,
    )
    .await
    .unwrap();
    let transfer = db.begin().await.unwrap();
    transfer.execute_unprepared(r#"UPDATE "Membership" SET "userId"='other' WHERE "appId"='storage-project' AND "userId"='owner'"#).await.unwrap();
    let (after_transfer, _) =
        flow_like_types::tokio::join!(project::recheck(&state, &combined_auth), async {
            flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(40)).await;
            transfer.commit().await.unwrap();
        });
    assert_status(after_transfer, StatusCode::FORBIDDEN);
    storage_request.placement_id = "foreign-owner-placement".into();
    assert_status(
        create_grant(&state, "owner", &device_id, storage_request).await,
        StatusCode::FORBIDDEN,
    );

    let epoch_grant = create_grant(&state, "owner", &device_id, request("epoch-placement"))
        .await
        .unwrap();
    let epoch_billing = approve_billing(
        &state,
        "owner",
        &device_id,
        &epoch_grant.grant_id,
        ApproveBillingGrantRequest {
            limit_micros: 100,
            expires_at: now() + 1000,
        },
    )
    .await
    .unwrap();
    let epoch_key = SigningKey::generate();
    register(
        &state,
        &device_id,
        registration(
            &epoch_grant,
            &epoch_billing,
            "epoch-instance",
            &device_key,
            &epoch_key,
        ),
    )
    .await
    .unwrap();
    let epoch_token = token(
        &state,
        "epoch-instance",
        token_request("epoch-instance", &epoch_key),
    )
    .await
    .unwrap();
    db.execute_raw(sql(
        r#"UPDATE "ManagedDevice" SET "authEpoch"=2 WHERE id=$1"#,
        [device_id.clone().into()],
    ))
    .await
    .unwrap();
    assert_status(
        token(
            &state,
            "epoch-instance",
            token_request("epoch-instance", &epoch_key),
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );
    assert_status(
        authenticate_model_request_with_context(
            &state,
            &headers(&epoch_token, &epoch_key, Some(&epoch_token.dpop_nonce)),
            "POST",
            MODEL_PATH,
            "model",
        )
        .await,
        StatusCode::UNAUTHORIZED,
    );

    distinct_host_project_and_payer_consent(&state, &db).await;

    db.close().await.unwrap();
    admin
        .execute_unprepared(&format!("DROP SCHEMA {schema} CASCADE"))
        .await
        .unwrap();
    admin.close().await.unwrap();
}

async fn distinct_host_project_and_payer_consent(
    state: &DeviceContext<'_>,
    db: &DatabaseConnection,
) {
    let device_key = SigningKey::generate();
    let (device_id, _, invitation) = seed_device(db, &device_key).await;
    db.execute_unprepared(r#"INSERT INTO "App" VALUES ('shared-project','ACTIVE','shared-owner-role'); INSERT INTO "Role" VALUES ('shared-owner-role','shared-project',1); INSERT INTO "Membership" VALUES ('other','shared-project','shared-owner-role')"#).await.unwrap();
    let request = CreateResourceGrantRequest {
        placement_id: "shared-placement".into(),
        deployment_id: "shared-deployment".into(),
        project_id: "shared-project".into(),
        app_id: Some("shared-project".into()),
        online_access: Some(OnlineProjectAccess::ReadWrite),
        model_ids: vec!["model".into()],
        max_instances: 1,
        expires_at: now() + 1800,
    };
    assert_status(
        create_grant(state, "other", &device_id, request.clone()).await,
        StatusCode::NOT_FOUND,
    );
    let mut policy = ManagementPolicy {
        version: 1,
        device_id: device_id.clone(),
        policy_version: 1,
        previous_policy_digest: None,
        issued_at: now(),
        expires_at: now() + 3600,
        grants: vec![ManagementGrant {
            grant_id: "shared-deployment".into(),
            user_id: "other".into(),
            controller_key: SigningKey::generate().public_key(),
            scope: ManagementScope::Project {
                project_id: "shared-project".into(),
            },
            capabilities: vec![ManagementCapability::Deploy],
            expires_at: now() + 900,
            group_id: None,
            group_version: None,
        }],
    };
    let deployment_deadline = now() + 1100;
    let mut matching = policy.grants[0].clone();
    matching.grant_id = "placement-deployment".into();
    matching.scope = ManagementScope::Placement {
        project_id: "shared-project".into(),
        placement_id: "shared-placement".into(),
    };
    matching.expires_at = deployment_deadline;
    let mut unrelated = matching.clone();
    unrelated.grant_id = "another-placement-deployment".into();
    unrelated.scope = ManagementScope::Placement {
        project_id: "shared-project".into(),
        placement_id: "another-placement".into(),
    };
    unrelated.expires_at = now() + 1700;
    policy.grants.extend([matching, unrelated]);
    let signed = sign_management_policy(&policy, &invitation).unwrap();
    db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,1,$2,$3,$4,$5)"#, [device_id.clone().into(),compact_digest(&signed).into(),signed.clone().into(),policy.expires_at.into(),now().into()])).await.unwrap();
    let mut outside = request.clone();
    outside.project_id = "elsewhere".into();
    outside.app_id = Some("elsewhere".into());
    assert_status(
        create_grant(state, "other", &device_id, outside).await,
        StatusCode::FORBIDDEN,
    );
    let grant = create_grant(state, "other", &device_id, request.clone())
        .await
        .unwrap();
    assert_eq!(grant.delegating_user_id, "other");
    assert_eq!(
        grants(state, "other", &device_id).await.unwrap(),
        vec![grant.clone()]
    );
    let billing = approve_billing(
        state,
        "owner",
        &device_id,
        &grant.grant_id,
        ApproveBillingGrantRequest {
            limit_micros: 100,
            expires_at: now() + 1200,
        },
    )
    .await
    .unwrap();
    assert_eq!(billing.payer_id, "owner");
    assert_status(
        revoke_billing(state, "other", &device_id, &billing.billing_grant_id).await,
        StatusCode::NOT_FOUND,
    );
    let workload = SigningKey::generate();
    register(
        state,
        &device_id,
        registration(&grant, &billing, "shared-instance", &device_key, &workload),
    )
    .await
    .unwrap();
    let session = token(
        state,
        "shared-instance",
        token_request("shared-instance", &workload),
    )
    .await
    .unwrap();
    assert_eq!(jwt::verify(&session.access_token).unwrap().sub, "other");
    let usage = authenticate_model_request_with_context(
        state,
        &headers(&session, &workload, Some(&session.dpop_nonce)),
        "POST",
        MODEL_PATH,
        "model",
    )
    .await
    .unwrap();
    assert_eq!(usage.delegated_user_id, "other");
    assert_eq!(usage.payer_id, "owner");
    let session = project::token(
        state,
        "shared-instance",
        InstanceTokenRequest {
            client_assertion: assertion(
                "shared-instance",
                "/instances/shared-instance/project-token",
                &workload,
            ),
        },
    )
    .await
    .unwrap();
    let auth = project::authenticate(
        state,
        &proof_headers(
            &session,
            &workload,
            Some(&session.dpop_nonce),
            "GET",
            "/instances/project/app",
        ),
        "GET",
        "/instances/project/app",
    )
    .await
    .unwrap();
    assert_eq!(auth.claims.sub, "other");
    let storage_deadline = project::recheck_storage(state, &auth).await.unwrap();
    assert_eq!(storage_deadline, deployment_deadline);
    assert!(storage_deadline < grant.expires_at);
    assert!(storage_deadline > session.lease_expires_at);
    assert!(storage_deadline > session.expires_at);
    db.execute_unprepared(
        r#"DELETE FROM "Membership" WHERE "userId"='other' AND "appId"='shared-project'"#,
    )
    .await
    .unwrap();
    assert_status(project::recheck(state, &auth).await, StatusCode::FORBIDDEN);
    assert_status(
        project::recheck_storage(state, &auth).await,
        StatusCode::FORBIDDEN,
    );
    db.execute_unprepared(
        r#"INSERT INTO "Membership" VALUES ('other','shared-project','shared-owner-role')"#,
    )
    .await
    .unwrap();
    policy.policy_version = 2;
    policy.previous_policy_digest = Some(compact_digest(&signed));
    policy.grants.clear();
    let signed = sign_management_policy(&policy, &invitation).unwrap();
    db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,2,$2,$3,$4,$5)"#, [device_id.clone().into(),compact_digest(&signed).into(),signed.clone().into(),policy.expires_at.into(),now().into()])).await.unwrap();
    assert_status(project::recheck(state, &auth).await, StatusCode::FORBIDDEN);
    assert!(
        consent_devices(state, "other")
            .await
            .unwrap()
            .iter()
            .any(|device| device.device_id == device_id)
    );
    revoke_billing(state, "owner", &device_id, &billing.billing_grant_id)
        .await
        .unwrap();
    revoke_grant(state, "other", &device_id, &grant.grant_id)
        .await
        .unwrap();
    policy.policy_version = 3;
    policy.previous_policy_digest = Some(compact_digest(&signed));
    policy.grants.push(ManagementGrant {
        grant_id: "shared-again".into(),
        user_id: "other".into(),
        controller_key: SigningKey::generate().public_key(),
        scope: ManagementScope::Device,
        capabilities: vec![ManagementCapability::Deploy],
        expires_at: now() + 1800,
        group_id: None,
        group_version: None,
    });
    let signed = sign_management_policy(&policy, &invitation).unwrap();
    db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,3,$2,$3,$4,$5)"#, [device_id.clone().into(),compact_digest(&signed).into(),signed.into(),policy.expires_at.into(),now().into()])).await.unwrap();
    let grant = create_grant(state, "other", &device_id, request)
        .await
        .unwrap();
    let billing = approve_billing(
        state,
        "other",
        &device_id,
        &grant.grant_id,
        ApproveBillingGrantRequest {
            limit_micros: 100,
            expires_at: now() + 1200,
        },
    )
    .await
    .unwrap();
    db.execute_raw(sql(
        r#"UPDATE "ManagedDevice" SET status='revoked',"authEpoch"=2 WHERE id=$1"#,
        [device_id.clone().into()],
    ))
    .await
    .unwrap();
    revoke_billing(state, "other", &device_id, &billing.billing_grant_id)
        .await
        .unwrap();
    revoke_grant(state, "other", &device_id, &grant.grant_id)
        .await
        .unwrap();
}
