use super::*;
use crate::devices::view::DeviceRelationship;
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
        instances(state, &grant.delegating_user_id, &grant.device_id)
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

/// A private schema with the device tables and the few platform tables the instance
/// routes read, holding the accounts `owner` and `other` and the hosted model `model`.
struct TestSchema {
    admin: DatabaseConnection,
    db: DatabaseConnection,
    name: String,
}

impl TestSchema {
    async fn create() -> Self {
        backend_jwt::init_for_tests();
        let url =
            std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").expect("Use disposable PostgreSQL");
        let admin = Database::connect(&url).await.unwrap();
        let name = format!("instance_test_{}", uuid::Uuid::new_v4().simple());
        admin
            .execute_unprepared(&format!("CREATE SCHEMA {name}"))
            .await
            .unwrap();
        let mut options = ConnectOptions::new(url);
        options
            .set_schema_search_path(&name)
            .max_connections(8)
            .min_connections(1);
        let db = Database::connect(options).await.unwrap();
        migrate(&db).await;
        insert_model(&db, "model", "FREE").await;
        Self { admin, db, name }
    }

    async fn drop(self) {
        self.db.close().await.unwrap();
        self.admin
            .execute_unprepared(&format!("DROP SCHEMA {} CASCADE", self.name))
            .await
            .unwrap();
        self.admin.close().await.unwrap();
    }
}

fn standalone() -> StandaloneConfig {
    StandaloneConfig {
        enabled: true,
        api_base_url: Some(API.into()),
        ..Default::default()
    }
}

fn context<'a>(db: &'a DatabaseConnection, config: &'a StandaloneConfig) -> DeviceContext<'a> {
    DeviceContext {
        db,
        dialect: crate::db::DbDialect::Postgres,
        config,
        domain: "unused.example",
        secure: true,
    }
}

async fn insert_model(db: &DatabaseConnection, id: &str, tier: &str) {
    let parameters = serde_json::json!({"context_length":32768,"model_classification":flow_like::bit::BitModelClassification::default(),"provider":{"provider_name":"hosted:openrouter","model_id":"upstream-model","params":{"tier":tier}}});
    db.execute_raw(sql(
        r#"INSERT INTO "Bit" (id,type,parameters) VALUES ($1,'LLM',$2)"#,
        [id.into(), parameters.into()],
    ))
    .await
    .unwrap();
}

async fn migrate(db: &DatabaseConnection) {
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
        include_str!("../../prisma/migrations/20261001120000_device_console/migration.sql"),
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
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn signed_instances_enforce_consent_scope_leases_and_revocation() {
    let schema = TestSchema::create().await;
    let db = schema.db.clone();
    let policy = standalone();
    let state = context(&db, &policy);
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
        ResourceGrantResponse {
            effective_expires_at: Some(grant.expires_at),
            effective_limit: Some(EffectiveLimit::Approval),
            ..grant.clone()
        }
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
    let replayed = registration(
        &grant,
        &billing,
        "replayed-replica",
        &device_key,
        &SigningKey::generate(),
    );
    let replayed_binding = verify_instance_registration(
        &replayed.registration_jws,
        &device_key.public_key(),
        &device_id,
        &endpoint_url(API, &format!("/devices/{device_id}/instances")).unwrap(),
        now(),
    )
    .unwrap();
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceProofReplay" ("deviceId","authEpoch","proofId","expiresAt") VALUES ($1,1,$2,$3)"#,
        [
            device_id.clone().into(),
            replayed_binding.jti.into(),
            (now() + 60).into(),
        ],
    ))
    .await
    .unwrap();
    let replay_error = register(&state, &device_id, replayed).await.unwrap_err();
    assert_eq!(replay_error.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(replay_error.public_code(), INSTANCE_PROOF_INVALID);
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
            client_assertion: retire_assertion.clone(),
        },
    )
    .await
    .unwrap();
    let replayed_retire = retire(
        &state,
        &device_id,
        "replica-two",
        ReceiptRequest {
            client_assertion: retire_assertion,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(replayed_retire.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(replayed_retire.public_code(), INSTANCE_PROOF_INVALID);
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

    schema.drop().await;
}

/// How `user` finds the device in their device list, if at all.
async fn relationship(
    state: &DeviceContext<'_>,
    user: &str,
    device_id: &str,
) -> Option<DeviceRelationship> {
    devices::view::visible_devices(state, user)
        .await
        .unwrap()
        .into_iter()
        .find(|device| device.device_id == device_id)
        .map(|device| device.relationship)
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
        vec![ResourceGrantResponse {
            effective_expires_at: Some(deployment_deadline),
            effective_limit: Some(EffectiveLimit::SharingGrant),
            ..grant.clone()
        }]
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
    assert_eq!(
        relationship(state, "other", &device_id).await,
        Some(DeviceRelationship::CloudApproval),
        "consent stays reachable after sharing is withdrawn so the delegator can revoke it"
    );
    revoke_billing(state, "owner", &device_id, &billing.billing_grant_id)
        .await
        .unwrap();
    let revoked = revoke_grant(state, "other", &device_id, &grant.grant_id)
        .await
        .unwrap();
    assert_eq!(revoked.status, "revoked");
    assert!(revoked.authz_version > grant.authz_version);
    assert_eq!(
        relationship(state, "other", &device_id).await,
        None,
        "a former delegate with nothing left to revoke no longer sees the device"
    );
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
    revoke_grant(state, "other", &device_id, &grant.grant_id)
        .await
        .unwrap();
    let device = device_id.as_str();
    let sees_device = move || async move { relationship(state, "other", device).await.is_some() };
    assert!(
        sees_device().await,
        "a payer whose sponsorship is still active keeps seeing the device"
    );
    let set_billing_expiry = |expires_at: i64| {
        db.execute_raw(sql(
            r#"UPDATE "PlacementBillingGrant" SET "expiresAt"=$2 WHERE id=$1"#,
            [billing.billing_grant_id.clone().into(), expires_at.into()],
        ))
    };
    set_billing_expiry(now() - 1).await.unwrap();
    assert!(
        !sees_device().await,
        "an expired sponsorship gives the payer no view"
    );
    set_billing_expiry(billing.expires_at).await.unwrap();
    assert!(sees_device().await);
    revoke_billing(state, "other", &device_id, &billing.billing_grant_id)
        .await
        .unwrap();
    assert!(
        !sees_device().await,
        "a revoked sponsorship gives the payer no view"
    );
}

fn approval(delegator: &str, expires_at: i64) -> ResourceGrantResponse {
    ResourceGrantResponse {
        grant_id: "grant".into(),
        device_id: "device".into(),
        placement_id: "placement".into(),
        deployment_id: "deployment".into(),
        project_id: "project".into(),
        app_id: None,
        delegating_user_id: delegator.into(),
        authz_version: 1,
        model_ids: vec!["model".into()],
        max_instances: 1,
        expires_at,
        status: "active".into(),
        online_access: None,
        effective_expires_at: None,
        effective_limit: None,
        online_write_blocked: None,
        approved_by_user_id: Some(delegator.into()),
        created_at: Some(1),
    }
}

fn permission(
    id: &str,
    user: &str,
    scope: ManagementScope,
    capability: ManagementCapability,
    expires_at: i64,
) -> ManagementGrant {
    ManagementGrant {
        grant_id: id.into(),
        user_id: user.into(),
        controller_key: SigningKey::generate().public_key(),
        scope,
        capabilities: vec![capability],
        expires_at,
        group_id: None,
        group_version: None,
    }
}

fn deploy(id: &str, user: &str, scope: ManagementScope, expires_at: i64) -> ManagementGrant {
    permission(id, user, scope, ManagementCapability::Deploy, expires_at)
}

fn placement_scope(project: &str, placement: &str) -> ManagementScope {
    ManagementScope::Placement {
        project_id: project.into(),
        placement_id: placement.into(),
    }
}

fn rules_for(
    device_id: &str,
    issued_at: i64,
    expires_at: i64,
    grants: Vec<ManagementGrant>,
) -> ManagementPolicy {
    ManagementPolicy {
        version: 1,
        device_id: device_id.into(),
        policy_version: 1,
        previous_policy_digest: None,
        grants,
        issued_at,
        expires_at,
    }
}

#[test]
fn a_delegated_approval_ends_with_whichever_limit_comes_first() {
    const NOW: i64 = 1_800_000_000;
    let rules = |grants| rules_for("device", NOW - 100, NOW + 3_000, grants);
    let here = || placement_scope("project", "placement");
    let everywhere = || ManagementScope::Project {
        project_id: "project".into(),
    };
    let long = approval("other", NOW + 5_000);
    let end = |rules: &ManagementPolicy, grant: &ResourceGrantResponse| {
        delegated_deadline(Some(rules), grant, NOW)
    };

    assert_eq!(
        end(&rules(vec![deploy("a", "other", here(), NOW + 900)]), &long),
        (NOW + 900, EffectiveLimit::SharingGrant)
    );
    // The widest permission that covers the placement counts, as it does at use.
    assert_eq!(
        end(
            &rules(vec![
                deploy("a", "other", here(), NOW + 900),
                deploy("b", "other", everywhere(), NOW + 1_100),
                deploy("c", "other", ManagementScope::Device, NOW + 1_000),
            ]),
            &long
        ),
        (NOW + 1_100, EffectiveLimit::SharingGrant)
    );
    // A permission that lasts as long as the rules is limited by the rules.
    let to_the_end = rules(vec![deploy("a", "other", here(), NOW + 3_000)]);
    assert_eq!(
        end(&to_the_end, &long),
        (NOW + 3_000, EffectiveLimit::AccessRules)
    );
    assert_eq!(
        end(&to_the_end, &approval("other", NOW + 500)),
        (NOW + 500, EffectiveLimit::Approval)
    );
    assert_eq!(
        end(&to_the_end, &approval("other", NOW + 3_000)),
        (NOW + 3_000, EffectiveLimit::Approval),
        "the approval wins a tie"
    );
    // Rules that do not let the approver deploy this placement took that away when issued.
    for grants in [
        Vec::new(),
        vec![deploy("a", "someone-else", here(), NOW + 900)],
        vec![deploy(
            "a",
            "other",
            placement_scope("project", "another-placement"),
            NOW + 900,
        )],
        vec![deploy(
            "a",
            "other",
            placement_scope("another-project", "placement"),
            NOW + 900,
        )],
        vec![permission(
            "a",
            "other",
            here(),
            ManagementCapability::Status,
            NOW + 900,
        )],
    ] {
        assert_eq!(
            end(&rules(grants), &long),
            (NOW - 100, EffectiveLimit::SharingGrant)
        );
    }
    assert_eq!(
        end(&rules(vec![deploy("a", "other", here(), NOW - 10)]), &long),
        (NOW - 10, EffectiveLimit::SharingGrant),
        "an ended permission keeps its end"
    );
    assert_eq!(
        delegated_deadline(
            Some(&rules_for("device", NOW + 60, NOW + 3_000, Vec::new())),
            &long,
            NOW
        ),
        (NOW, EffectiveLimit::SharingGrant),
        "a loss is never reported in the future"
    );
    assert_eq!(
        end(
            &rules_for(
                "device",
                NOW - 7_200,
                NOW - 3_600,
                vec![deploy("a", "other", here(), NOW - 3_600)]
            ),
            &long
        ),
        (NOW - 3_600, EffectiveLimit::AccessRules)
    );
    assert_eq!(
        delegated_deadline(None, &long, NOW),
        (NOW, EffectiveLimit::AccessRules)
    );
    assert_eq!(
        delegated_deadline(None, &approval("other", NOW - 5), NOW),
        (NOW - 5, EffectiveLimit::Approval)
    );
}

#[test]
fn bound_value_lists_are_numbered_from_their_first_position() {
    assert_eq!(placeholders(1, 3), "$1,$2,$3");
    assert_eq!(placeholders(2, 1), "$2");
}

async fn publish_rules(
    db: &DatabaseConnection,
    signer: &SigningKey,
    previous: Option<&str>,
    mut rules: ManagementPolicy,
) -> String {
    rules.policy_version = db
        .query_one_raw(sql(
            r#"SELECT COUNT(*) AS count FROM "DeviceManagementPolicy" WHERE "deviceId"=$1"#,
            [rules.device_id.clone().into()],
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "count")
        .unwrap() as u64
        + 1;
    rules.previous_policy_digest = previous.map(str::to_owned);
    let signed = sign_management_policy(&rules, signer).unwrap();
    let digest = compact_digest(&signed);
    db.execute_raw(sql(r#"INSERT INTO "DeviceManagementPolicy"("deviceId",version,digest,"policyJws","expiresAt","createdAt") VALUES($1,$2,$3,$4,$5,$6)"#, [rules.device_id.clone().into(),(rules.policy_version as i64).into(),digest.clone().into(),signed.into(),rules.expires_at.into(),now().into()])).await.unwrap();
    digest
}

fn rule_reads() -> usize {
    ACCESS_RULE_READS.with(|reads| reads.replace(0))
}

fn effective(
    grants: &[ResourceGrantResponse],
    placement: &str,
) -> (Option<i64>, Option<EffectiveLimit>) {
    let grant = grants
        .iter()
        .find(|grant| grant.placement_id == placement)
        .unwrap_or_else(|| panic!("approval for {placement} is listed"));
    (grant.effective_expires_at, grant.effective_limit)
}

async fn approvals_report_when_they_really_end(state: &DeviceContext<'_>, db: &DatabaseConnection) {
    let (device_id, _, invitation) = seed_device(db, &SigningKey::generate()).await;
    let long = |placement: &str| CreateResourceGrantRequest {
        expires_at: now() + 7_200,
        ..request(placement)
    };
    let owned = create_grant(state, "owner", &device_id, long("owned"))
        .await
        .unwrap();
    assert_eq!(owned.approved_by_user_id.as_deref(), Some("owner"));
    assert!((now() - 5..=now()).contains(&owned.created_at.unwrap()));
    assert_eq!(
        (owned.effective_expires_at, owned.effective_limit),
        (None, None)
    );
    rule_reads();
    let own_end = (Some(owned.expires_at), Some(EffectiveLimit::Approval));
    assert_eq!(
        effective(&grants(state, "owner", &device_id).await.unwrap(), "owned"),
        own_end
    );
    assert_eq!(rule_reads(), 0, "the owner's approvals depend on no rules");

    let issued = now();
    let everywhere = ManagementScope::Project {
        project_id: "offline-project".into(),
    };
    let first = publish_rules(
        db,
        &invitation,
        None,
        rules_for(
            &device_id,
            issued,
            issued + 3_600,
            vec![
                deploy("project", "other", everywhere.clone(), issued + 900),
                deploy(
                    "b",
                    "other",
                    placement_scope("offline-project", "b"),
                    issued + 1_100,
                ),
                deploy(
                    "c",
                    "other",
                    placement_scope("offline-project", "c"),
                    issued + 3_600,
                ),
            ],
        ),
    )
    .await;
    for placement in ["a", "b", "c"] {
        let created = create_grant(state, "other", &device_id, long(placement))
            .await
            .unwrap();
        assert_eq!(created.approved_by_user_id.as_deref(), Some("other"));
    }
    rule_reads();
    let listed = grants(state, "owner", &device_id).await.unwrap();
    assert_eq!(rule_reads(), 1, "three approvals, one verification");
    assert_eq!(listed.len(), 4);
    let sharing = Some(EffectiveLimit::SharingGrant);
    assert_eq!(effective(&listed, "owned"), own_end);
    assert_eq!(effective(&listed, "a"), (Some(issued + 900), sharing));
    assert_eq!(effective(&listed, "b"), (Some(issued + 1_100), sharing));
    assert_eq!(
        effective(&listed, "c"),
        (Some(issued + 3_600), Some(EffectiveLimit::AccessRules))
    );
    let theirs = grants(state, "other", &device_id).await.unwrap();
    assert_eq!(theirs.len(), 3);
    assert!(
        theirs
            .iter()
            .all(|grant| grant.delegating_user_id == "other")
    );
    assert_eq!(
        theirs,
        listed
            .iter()
            .filter(|grant| grant.delegating_user_id == "other")
            .cloned()
            .collect::<Vec<_>>()
    );
    assert!(grants(state, "third", &device_id).await.unwrap().is_empty());
    let a = listed
        .iter()
        .find(|grant| grant.placement_id == "a")
        .unwrap();
    rule_reads();
    assert_eq!(
        &get_grant(state, "other", &device_id, &a.grant_id)
            .await
            .unwrap(),
        a
    );
    assert_eq!(rule_reads(), 1);
    assert_status(
        get_grant(state, "third", &device_id, &a.grant_id).await,
        StatusCode::NOT_FOUND,
    );

    // Newer rules without the approver: every approval of theirs ended when those were issued.
    let reissued = now();
    let second = publish_rules(
        db,
        &invitation,
        Some(&first),
        rules_for(&device_id, reissued, reissued + 3_600, Vec::new()),
    )
    .await;
    let listed = grants(state, "owner", &device_id).await.unwrap();
    for placement in ["a", "b", "c"] {
        assert_eq!(effective(&listed, placement), (Some(reissued), sharing));
    }
    assert_eq!(effective(&listed, "owned"), own_end);
    assert_status(
        approve_billing(
            state,
            "other",
            &device_id,
            &a.grant_id,
            ApproveBillingGrantRequest {
                limit_micros: 100,
                expires_at: now() + 600,
            },
        )
        .await,
        StatusCode::FORBIDDEN,
    );

    // Ended rules keep their end time instead of failing the read.
    let third = publish_rules(
        db,
        &invitation,
        Some(&second),
        rules_for(
            &device_id,
            reissued - 7_200,
            reissued - 3_600,
            vec![deploy("project", "other", everywhere, reissued - 3_600)],
        ),
    )
    .await;
    let listed = grants(state, "other", &device_id).await.unwrap();
    for placement in ["a", "b", "c"] {
        assert_eq!(
            effective(&listed, placement),
            (Some(reissued - 3_600), Some(EffectiveLimit::AccessRules))
        );
    }

    // Rules nobody can verify end the approvals now; the read still answers.
    publish_rules(
        db,
        &SigningKey::generate(),
        Some(&third),
        rules_for(&device_id, reissued, reissued + 3_600, Vec::new()),
    )
    .await;
    let before = now();
    let listed = grants(state, "owner", &device_id).await.unwrap();
    for placement in ["a", "b", "c"] {
        let (at, limit) = effective(&listed, placement);
        assert!((before..=now()).contains(&at.unwrap()));
        assert_eq!(limit, Some(EffectiveLimit::AccessRules));
    }
    assert_eq!(effective(&listed, "owned"), own_end);

    revoke_grant(state, "other", &device_id, &a.grant_id)
        .await
        .unwrap();
    let listed = grants(state, "other", &device_id).await.unwrap();
    assert_eq!(effective(&listed, "a"), (None, None));
    assert_eq!(
        listed.last().map(|grant| grant.status.as_str()),
        Some("revoked")
    );
}

fn storage_full() -> ApiError {
    ApiError::coded(
        StatusCode::PAYMENT_REQUIRED,
        "PLAN_LIMIT_EXCEEDED",
        "Storage is full",
    )
}

async fn blocked_writes_are_shown_only_to_the_approver_and_project_readers(
    state: &DeviceContext<'_>,
    db: &DatabaseConnection,
) -> (String, ResourceGrantResponse) {
    let (device_id, _, invitation) = seed_device(db, &SigningKey::generate()).await;
    db.execute_unprepared(r#"INSERT INTO "App" VALUES ('quota-project','ACTIVE','quota-owner-role'); INSERT INTO "Role" VALUES ('quota-owner-role','quota-project',1),('quota-team-role','quota-project',4); INSERT INTO "Membership" VALUES ('other','quota-project','quota-owner-role')"#).await.unwrap();
    let issued = now();
    publish_rules(
        db,
        &invitation,
        None,
        rules_for(
            &device_id,
            issued,
            issued + 3_600,
            vec![deploy(
                "device",
                "other",
                ManagementScope::Device,
                issued + 3_600,
            )],
        ),
    )
    .await;
    let online = |placement: &str, access| CreateResourceGrantRequest {
        placement_id: placement.into(),
        deployment_id: "deployment".into(),
        project_id: "quota-project".into(),
        app_id: Some("quota-project".into()),
        online_access: Some(access),
        model_ids: Vec::new(),
        max_instances: 1,
        expires_at: now() + 1_800,
    };
    for (placement, access) in [
        ("write-one", OnlineProjectAccess::ReadWrite),
        ("write-two", OnlineProjectAccess::ReadWrite),
        ("read", OnlineProjectAccess::ReadOnly),
    ] {
        create_grant(state, "other", &device_id, online(placement, access))
            .await
            .unwrap();
    }
    create_grant(state, "owner", &device_id, request("offline"))
        .await
        .unwrap();

    let asked = std::cell::RefCell::new(Vec::new());
    let full = |project: String, payer: String| {
        asked.borrow_mut().push((project, payer));
        async { Err(storage_full()) }
    };
    let blocked = |grants: &[ResourceGrantResponse]| {
        grants
            .iter()
            .filter(|grant| grant.online_write_blocked == Some(OnlineWriteBlock::StorageFull))
            .map(|grant| grant.placement_id.clone())
            .collect::<std::collections::BTreeSet<_>>()
    };
    let both = ["write-one", "write-two"]
        .map(str::to_owned)
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();

    let mut theirs = grants(state, "other", &device_id).await.unwrap();
    project::mark_blocked_writes_with(db, "other", &mut theirs, full)
        .await
        .unwrap();
    assert_eq!(blocked(&theirs), both);
    assert_eq!(
        asked.take(),
        vec![("quota-project".to_owned(), "other".to_owned())],
        "one quota read per project"
    );

    // The device owner sees the approvals but is no member of the project.
    let mut all = grants(state, "owner", &device_id).await.unwrap();
    assert_eq!(all.len(), 4);
    project::mark_blocked_writes_with(db, "owner", &mut all, full)
        .await
        .unwrap();
    assert!(blocked(&all).is_empty());
    assert!(asked.take().is_empty(), "the quota is not even read");

    db.execute_unprepared(
        r#"INSERT INTO "Membership" VALUES ('owner','quota-project','quota-team-role')"#,
    )
    .await
    .unwrap();
    project::mark_blocked_writes_with(db, "owner", &mut all, full)
        .await
        .unwrap();
    assert!(
        blocked(&all).is_empty(),
        "a member who cannot read the project learns nothing either"
    );
    assert!(asked.take().is_empty());
    db.execute_unprepared(r#"UPDATE "Role" SET permissions=256 WHERE id='quota-team-role'"#)
        .await
        .unwrap();
    project::mark_blocked_writes_with(db, "owner", &mut all, full)
        .await
        .unwrap();
    assert_eq!(blocked(&all), both);
    assert_eq!(asked.take().len(), 1);

    for quota in [Ok(()), Err(ApiError::internal("capacity lookup failed"))] {
        let mut theirs = grants(state, "other", &device_id).await.unwrap();
        let quota = std::cell::RefCell::new(Some(quota));
        project::mark_blocked_writes_with(db, "other", &mut theirs, |_, _| {
            let answer = quota.borrow_mut().take().expect("asked once");
            async move { answer }
        })
        .await
        .unwrap();
        assert!(
            blocked(&theirs).is_empty(),
            "room or an unknown quota is no block"
        );
    }

    let write_two = theirs
        .iter()
        .find(|grant| grant.placement_id == "write-two")
        .unwrap();
    revoke_grant(state, "other", &device_id, &write_two.grant_id)
        .await
        .unwrap();
    db.execute_raw(sql(
        r#"UPDATE "PlacementResourceGrant" SET "expiresAt"=$2 WHERE "deviceId"=$1 AND "placementId"='write-one'"#,
        [device_id.clone().into(), (now() - 1).into()],
    ))
    .await
    .unwrap();
    let mut theirs = grants(state, "other", &device_id).await.unwrap();
    project::mark_blocked_writes_with(db, "other", &mut theirs, full)
        .await
        .unwrap();
    assert!(blocked(&theirs).is_empty());
    assert!(asked.take().is_empty(), "ended approvals issue no leases");

    let read = theirs
        .into_iter()
        .find(|grant| grant.placement_id == "read")
        .unwrap();
    (device_id, read)
}

async fn spend_is_listed_per_instance_for_those_who_see_the_limit(
    state: &DeviceContext<'_>,
    db: &DatabaseConnection,
) {
    let (device_id, _, invitation) = seed_device(db, &SigningKey::generate()).await;
    let (elsewhere, _, _) = seed_device(db, &SigningKey::generate()).await;
    let issued = now();
    publish_rules(
        db,
        &invitation,
        None,
        rules_for(
            &device_id,
            issued,
            issued + 3_600,
            vec![deploy(
                "device",
                "other",
                ManagementScope::Device,
                issued + 3_600,
            )],
        ),
    )
    .await;
    let limit = |limit_micros| ApproveBillingGrantRequest {
        limit_micros,
        expires_at: now() + 1_800,
    };
    let owned = create_grant(state, "owner", &device_id, request("owned"))
        .await
        .unwrap();
    let paid = approve_billing(state, "owner", &device_id, &owned.grant_id, limit(1_000))
        .await
        .unwrap();
    assert_eq!(paid.approved_by_user_id.as_deref(), Some("owner"));
    assert!((now() - 5..=now()).contains(&paid.created_at.unwrap()));
    let delegated = create_grant(state, "other", &device_id, request("delegated"))
        .await
        .unwrap();
    let theirs = approve_billing(state, "other", &device_id, &delegated.grant_id, limit(500))
        .await
        .unwrap();
    assert_eq!(
        billing_grant(state, "owner", &device_id, &paid.billing_grant_id)
            .await
            .unwrap(),
        paid
    );

    let ids = |billing: Vec<BillingGrantResponse>| {
        billing
            .into_iter()
            .map(|billing| billing.billing_grant_id)
            .collect::<std::collections::BTreeSet<_>>()
    };
    assert_eq!(
        ids(billing_grants(state, "owner", &device_id).await.unwrap()),
        [&paid, &theirs]
            .map(|billing| billing.billing_grant_id.clone())
            .into_iter()
            .collect()
    );
    assert_eq!(
        billing_grants(state, "other", &device_id).await.unwrap(),
        vec![theirs.clone()]
    );
    assert!(
        billing_grants(state, "third", &device_id)
            .await
            .unwrap()
            .is_empty()
    );

    let admit = |operation: &str,
                 billing: &str,
                 instance: &str,
                 used: i64,
                 reserved: i64,
                 at: i64| {
        db.execute_raw(sql(
            r#"INSERT INTO "InstanceUsageAdmission" ("operationId","billingGrantId","instanceId","payerId",authority,"requiredModelTier","ceilingMicros","usedMicros","reservedMicros",status,"createdAt") VALUES ($1,$2,$3,'owner','{}','FREE',100,$4,$5,'settled',$6)"#,
            [
                operation.into(),
                billing.into(),
                instance.into(),
                used.into(),
                reserved.into(),
                at.into(),
            ],
        ))
    };
    let paid_id = paid.billing_grant_id.as_str();
    admit("one", paid_id, "replica-a", 10, 0, 1_000)
        .await
        .unwrap();
    admit("two", paid_id, "replica-a", 15, 5, 2_000)
        .await
        .unwrap();
    admit("three", paid_id, "replica-b", 7, 0, 1_500)
        .await
        .unwrap();
    admit(
        "other",
        &theirs.billing_grant_id,
        "replica-a",
        400,
        40,
        9_000,
    )
    .await
    .unwrap();
    let usage = billing_usage(state, "owner", &device_id, paid_id)
        .await
        .unwrap();
    assert_eq!(
        usage,
        BillingGrantUsage {
            billing_grant_id: paid_id.into(),
            totals: BillingGrantUsageTotals {
                used_micros: 32,
                reserved_micros: 5,
                operations: 3,
            },
            instances: vec![
                BillingGrantInstanceUsage {
                    instance_id: "replica-a".into(),
                    used_micros: 25,
                    reserved_micros: 5,
                    operations: 2,
                    first_at: 1_000,
                    last_at: 2_000,
                },
                BillingGrantInstanceUsage {
                    instance_id: "replica-b".into(),
                    used_micros: 7,
                    reserved_micros: 0,
                    operations: 1,
                    first_at: 1_500,
                    last_at: 1_500,
                },
            ],
        }
    );
    assert_eq!(
        billing_usage(state, "other", &device_id, &theirs.billing_grant_id)
            .await
            .unwrap()
            .totals,
        BillingGrantUsageTotals {
            used_micros: 400,
            reserved_micros: 40,
            operations: 1,
        }
    );
    // Someone else's spending, an unknown limit and a limit of another device all read
    // as missing.
    for (viewer, device, billing) in [
        ("other", device_id.as_str(), paid_id),
        ("third", device_id.as_str(), paid_id),
        (
            "third",
            device_id.as_str(),
            theirs.billing_grant_id.as_str(),
        ),
        ("owner", device_id.as_str(), "no-such-limit"),
        ("owner", elsewhere.as_str(), paid_id),
        ("owner", "no-such-device", paid_id),
    ] {
        assert_status(
            billing_usage(state, viewer, device, billing).await,
            StatusCode::NOT_FOUND,
        );
    }

    db.execute_raw(sql(
        r#"INSERT INTO "InstanceUsageAdmission" ("operationId","billingGrantId","instanceId","payerId",authority,"requiredModelTier","ceilingMicros","usedMicros","reservedMicros",status,"createdAt") SELECT 'bulk-'||n,$1,'bulk-'||n,'owner','{}','FREE',1,1,0,'settled',3000+n FROM generate_series(1,120) n"#,
        [paid_id.into()],
    ))
    .await
    .unwrap();
    let usage = billing_usage(state, "owner", &device_id, paid_id)
        .await
        .unwrap();
    assert_eq!(usage.instances.len(), 100);
    assert_eq!(usage.instances[0].instance_id, "bulk-120");
    assert_eq!(
        usage.totals,
        BillingGrantUsageTotals {
            used_micros: 152,
            reserved_micros: 5,
            operations: 123,
        },
        "totals cover the instances the list leaves out"
    );
}

async fn eligibility_judges_every_model_against_the_callers_plan(
    state: &DeviceContext<'_>,
    db: &DatabaseConnection,
    storage_only: (String, ResourceGrantResponse),
) {
    let (device_id, _, _) = seed_device(db, &SigningKey::generate()).await;
    insert_model(db, "premium-model", "PREMIUM").await;
    insert_model(db, "gone-model", "FREE").await;
    db.execute_raw(sql(
        r#"INSERT INTO "Bit" (id,type,parameters) VALUES ('embedding-model','EMBEDDING',$1)"#,
        [serde_json::json!({"languages":["en"],"vector_length":8,"input_length":8,"prefix":{"query":"","paragraph":""},"pooling":"Mean","provider":{"provider_name":"hosted:internal","model_id":"upstream-embedding"}}).into()],
    ))
    .await
    .unwrap();
    let models = |placement: &str, model_ids: &[&str]| CreateResourceGrantRequest {
        model_ids: model_ids.iter().map(|id| (*id).to_owned()).collect(),
        ..request(placement)
    };
    let mixed = create_grant(
        state,
        "owner",
        &device_id,
        models(
            "mixed",
            &["model", "premium-model", "gone-model", "embedding-model"],
        ),
    )
    .await
    .unwrap();
    db.execute_unprepared(r#"DELETE FROM "Bit" WHERE id='gone-model'"#)
        .await
        .unwrap();
    let model = |model_id: &str, tier: Option<&str>, allowed| budget::ModelEligibility {
        model_id: model_id.into(),
        tier: tier.map(str::to_owned),
        allowed,
    };
    let tiers = tiers();
    assert_eq!(
        billing_eligibility(state, &tiers, "owner", &device_id, &mixed.grant_id)
            .await
            .unwrap(),
        BillingEligibility {
            payer_id: "owner".into(),
            plan: "FREE".into(),
            eligible: false,
            models: vec![
                model("embedding-model", None, true),
                model("gone-model", None, false),
                model("model", Some("FREE"), true),
                model("premium-model", Some("PREMIUM"), false),
            ],
        }
    );
    let included = create_grant(
        state,
        "owner",
        &device_id,
        models("included", &["model", "embedding-model"]),
    )
    .await
    .unwrap();
    let covered = billing_eligibility(state, &tiers, "owner", &device_id, &included.grant_id)
        .await
        .unwrap();
    assert!(covered.eligible);
    assert_eq!(covered.models.len(), 2);
    // Reading eligibility approves nothing.
    assert_status(
        get_billing(state, "owner", &device_id, &included.grant_id).await,
        StatusCode::NOT_FOUND,
    );
    assert_status(
        billing_eligibility(state, &tiers, "third", &device_id, &included.grant_id).await,
        StatusCode::NOT_FOUND,
    );
    assert_status(
        billing_eligibility(state, &tiers, "owner", &device_id, "no-such-approval").await,
        StatusCode::NOT_FOUND,
    );
    let (storage_device, storage_grant) = storage_only;
    let nothing_to_pay = billing_eligibility(
        state,
        &tiers,
        "other",
        &storage_device,
        &storage_grant.grant_id,
    )
    .await
    .unwrap();
    assert_eq!(nothing_to_pay.payer_id, "other");
    assert!(!nothing_to_pay.eligible && nothing_to_pay.models.is_empty());
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn approval_reads_report_limits_spend_and_plan_coverage() {
    let schema = TestSchema::create().await;
    let db = schema.db.clone();
    let policy = standalone();
    let state = context(&db, &policy);
    db.execute_unprepared(r#"INSERT INTO "User" (id,status) VALUES ('third','ACTIVE')"#)
        .await
        .unwrap();
    approvals_report_when_they_really_end(&state, &db).await;
    let storage_only =
        blocked_writes_are_shown_only_to_the_approver_and_project_readers(&state, &db).await;
    spend_is_listed_per_instance_for_those_who_see_the_limit(&state, &db).await;
    eligibility_judges_every_model_against_the_callers_plan(&state, &db, storage_only).await;
    let disabled = StandaloneConfig::default();
    assert_status(
        billing_usage(&context(&db, &disabled), "owner", "device", "billing").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    assert_status(
        billing_eligibility(
            &context(&db, &disabled),
            &tiers(),
            "owner",
            "device",
            "grant",
        )
        .await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    schema.drop().await;
}

#[test]
fn fleet_reads_are_bounded_and_bind_exactly_what_they_reference() {
    for (app, limit, bound) in [(None, 1000, 2), (Some("app"), 500, 4)] {
        let statement = fleet_statement("viewer", app, 1_800_000_000);
        let sql = statement.sql.as_str();
        assert_eq!(
            statement.values.as_ref().map_or(0, |values| values.0.len()),
            bound,
            "{sql}"
        );
        for position in 1..=bound {
            assert!(sql.contains(&format!("${position}")), "{sql}");
        }
        assert!(!sql.contains(&format!("${}", bound + 1)), "{sql}");
        assert!(
            sql.contains(&format!(" LIMIT {limit}) g LEFT JOIN ")),
            "{sql}"
        );
        // Every source of visible approvals is narrowed to the app, or none is.
        assert_eq!(
            sql.matches(r#" AND v."appId"=$3"#).count(),
            if app.is_some() { 3 } else { 0 },
            "{sql}"
        );
        assert_eq!(sql.contains("WorkloadInstance"), app.is_some());
    }
}

/// A device registered by `owner`, and the key its access rules are signed with.
async fn seed_device_of(db: &DatabaseConnection, owner: &str) -> (String, SigningKey) {
    let (device_id, _, invitation) = seed_device(db, &SigningKey::generate()).await;
    db.execute_raw(sql(
        r#"UPDATE "ManagedDevice" SET "ownerId"=$2 WHERE id=$1"#,
        [device_id.clone().into(), owner.into()],
    ))
    .await
    .unwrap();
    (device_id, invitation)
}

/// Access rules that let `other` deploy anywhere on the device for 900 seconds.
async fn let_other_deploy(db: &DatabaseConnection, device_id: &str, signer: &SigningKey) -> String {
    let issued = now();
    publish_rules(
        db,
        signer,
        None,
        rules_for(
            device_id,
            issued,
            issued + 3_600,
            vec![deploy(
                "device",
                "other",
                ManagementScope::Device,
                issued + 900,
            )],
        ),
    )
    .await
}

/// A lease as a registration leaves it.
async fn hold_lease(
    db: &DatabaseConnection,
    grant: &ResourceGrantResponse,
    status: &str,
    purpose: Option<&str>,
    lease_expires_at: i64,
) {
    let key = serde_json::to_string(&SigningKey::generate().public_key()).unwrap();
    db.execute_raw(sql(
        r#"INSERT INTO "WorkloadInstance" (id,purpose,"deviceId","grantId","workloadKey","workloadKeyThumbprint","deviceAuthEpoch","grantAuthzVersion",status,"registeredAt","leaseExpiresAt","registrationJws") VALUES ($1,$2,$3,$4,$5,$1,1,1,$6,$7,$8,'registration')"#,
        [
            uuid::Uuid::new_v4().to_string().into(),
            purpose.map(str::to_owned).into(),
            grant.device_id.clone().into(),
            grant.grant_id.clone().into(),
            key.into(),
            status.into(),
            now().into(),
            lease_expires_at.into(),
        ],
    ))
    .await
    .unwrap();
}

fn fleet_app(
    placement: &str,
    access: OnlineProjectAccess,
    models: &[&str],
) -> CreateResourceGrantRequest {
    CreateResourceGrantRequest {
        placement_id: placement.into(),
        deployment_id: format!("deployment-{placement}"),
        project_id: "fleet-app".into(),
        app_id: Some("fleet-app".into()),
        online_access: Some(access),
        model_ids: models.iter().map(|model| (*model).to_owned()).collect(),
        max_instances: 3,
        expires_at: now() + 7_200,
    }
}

fn spending(limit_micros: i64) -> ApproveBillingGrantRequest {
    ApproveBillingGrantRequest {
        limit_micros,
        expires_at: now() + 1_800,
    }
}

type Services = std::collections::BTreeSet<(String, String)>;

fn services<'a>(pairs: impl IntoIterator<Item = (&'a String, &'a str)>) -> Services {
    pairs
        .into_iter()
        .map(|(device, placement)| (device.clone(), placement.to_owned()))
        .collect()
}

async fn summary_of(state: &DeviceContext<'_>, viewer: &str) -> resource_summary::ResourceSummary {
    let (server_time, found) = resource_summary::visible_approvals(state, viewer)
        .await
        .unwrap();
    resource_summary::summarize(server_time, found, viewer)
}

fn summarized(summary: &resource_summary::ResourceSummary) -> Services {
    summary
        .devices
        .iter()
        .flat_map(|device| {
            device
                .approvals
                .iter()
                .map(|approval| (device.device_id.clone(), approval.placement_id.clone()))
        })
        .collect()
}

fn summarized_approval<'a>(
    summary: &'a resource_summary::ResourceSummary,
    device_id: &str,
    placement: &str,
) -> &'a resource_summary::ApprovalSummary {
    summary
        .devices
        .iter()
        .filter(|device| device.device_id == device_id)
        .flat_map(|device| &device.approvals)
        .find(|approval| approval.placement_id == placement)
        .unwrap_or_else(|| panic!("the approval of {placement} is summarized"))
}

/// The limits listed for a device as (limit, whether the viewer pays).
fn summarized_limits(
    summary: &resource_summary::ResourceSummary,
    device_id: &str,
) -> Vec<(i64, bool)> {
    summary
        .devices
        .iter()
        .filter(|device| device.device_id == device_id)
        .flat_map(|device| &device.billing)
        .map(|limit| (limit.limit_micros, limit.payer_is_me))
        .collect()
}

struct Fleet {
    /// The owner's, shared with nobody.
    edge: String,
    /// The owner's; `other` is a current recipient of its access rules.
    lab: String,
    lab_signer: SigningKey,
    lab_rules: String,
    /// The owner's; its rules let `other` deploy, who never accepted the share.
    bench: String,
    /// Registered by `other`.
    theirs: String,
    shared_until: i64,
    lab_bot: ResourceGrantResponse,
    lab_reports: ResourceGrantResponse,
    bench_bot: ResourceGrantResponse,
}

async fn seed_fleet(state: &DeviceContext<'_>, db: &DatabaseConnection) -> Fleet {
    db.execute_unprepared(r#"INSERT INTO "User" (id,status) VALUES ('third','ACTIVE'); INSERT INTO "App" VALUES ('fleet-app','ACTIVE','fleet-owner-role'); INSERT INTO "Role" VALUES ('fleet-owner-role','fleet-app',1); INSERT INTO "Membership" VALUES ('other','fleet-app','fleet-owner-role')"#).await.unwrap();
    let (edge, _) = seed_device_of(db, "owner").await;
    let (lab, lab_signer) = seed_device_of(db, "owner").await;
    let (bench, bench_signer) = seed_device_of(db, "owner").await;
    let (theirs, _) = seed_device_of(db, "other").await;
    let shared_until = now() + 900;
    let lab_rules = let_other_deploy(db, &lab, &lab_signer).await;
    let_other_deploy(db, &bench, &bench_signer).await;
    db.execute_raw(sql(
        r#"INSERT INTO "DeviceManagementRecipient"("deviceId",version,"grantId","userId","expiresAt") VALUES($1,1,'device','other',$2)"#,
        [lab.clone().into(), shared_until.into()],
    ))
    .await
    .unwrap();

    create_grant(state, "owner", &edge, request("own"))
        .await
        .unwrap();
    create_grant(state, "owner", &lab, request("local"))
        .await
        .unwrap();
    let read_write = fleet_app("bot", OnlineProjectAccess::ReadWrite, &["model"]);
    let lab_bot = create_grant(state, "other", &lab, read_write)
        .await
        .unwrap();
    let read_only = |placement| fleet_app(placement, OnlineProjectAccess::ReadOnly, &[]);
    let lab_reports = create_grant(state, "other", &lab, read_only("reports"))
        .await
        .unwrap();
    let bench_bot = create_grant(state, "other", &bench, read_only("bot"))
        .await
        .unwrap();
    create_grant(state, "other", &theirs, read_only("bot"))
        .await
        .unwrap();
    Fleet {
        edge,
        lab,
        lab_signer,
        lab_rules,
        bench,
        theirs,
        shared_until,
        lab_bot,
        lab_reports,
        bench_bot,
    }
}

async fn the_summary_lists_each_viewers_approvals_and_limits(
    state: &DeviceContext<'_>,
    db: &DatabaseConnection,
    fleet: &Fleet,
) {
    let Fleet {
        edge,
        lab,
        bench,
        theirs,
        ..
    } = fleet;
    let own = grants(state, "owner", edge).await.unwrap().remove(0);
    let owners_limit = approve_billing(state, "owner", edge, &own.grant_id, spending(1_000))
        .await
        .unwrap();
    let bot_limit = approve_billing(state, "other", lab, &fleet.lab_bot.grant_id, spending(500))
        .await
        .unwrap();

    // The owner sees every approval on their devices and nothing on anyone else's.
    rule_reads();
    let before = now();
    let summary = summary_of(state, "owner").await;
    assert_eq!(
        rule_reads(),
        2,
        "two devices carry approvals by someone else; each one's rules are verified once"
    );
    assert!((before..=now()).contains(&summary.server_time));
    assert_eq!(
        summarized(&summary),
        services([
            (edge, "own"),
            (lab, "local"),
            (lab, "bot"),
            (lab, "reports"),
            (bench, "bot"),
        ])
    );
    let own_approval = summarized_approval(&summary, edge, "own");
    assert_eq!(
        (
            own_approval.effective_expires_at,
            own_approval.effective_limit,
            own_approval.approver_is_me,
            own_approval.payer_is_me,
        ),
        (
            Some(own.expires_at),
            Some(EffectiveLimit::Approval),
            true,
            true
        )
    );
    for (device, placement) in [(lab, "bot"), (lab, "reports"), (bench, "bot")] {
        let approval = summarized_approval(&summary, device, placement);
        assert_eq!(approval.app_id.as_deref(), Some("fleet-app"));
        assert_eq!(approval.status, "active");
        assert!(!approval.approver_is_me && !approval.payer_is_me);
        assert!(
            (fleet.shared_until..=fleet.shared_until + 5)
                .contains(&approval.effective_expires_at.unwrap()),
            "it ends with the approver's Deploy permission"
        );
        assert_eq!(approval.effective_limit, Some(EffectiveLimit::SharingGrant));
    }
    assert_eq!(summarized_limits(&summary, edge), vec![(1_000, true)]);
    assert_eq!(summarized_limits(&summary, lab), vec![(500, false)]);

    // Everyone else sees only what they gave, wherever it is.
    rule_reads();
    let (server_time, mut found) = resource_summary::visible_approvals(state, "other")
        .await
        .unwrap();
    assert_eq!(rule_reads(), 2, "their own device needs no rules");
    let asked = std::cell::RefCell::new(Vec::new());
    let full = |project: String, payer: String| {
        asked.borrow_mut().push((project, payer));
        async { Err(storage_full()) }
    };
    project::mark_blocked_writes_with(db, "other", &mut found.grants, full)
        .await
        .unwrap();
    assert_eq!(
        asked.take(),
        vec![("fleet-app".to_owned(), "other".to_owned())],
        "one quota read for the whole fleet"
    );
    let summary = resource_summary::summarize(server_time, found, "other");
    assert_eq!(
        summarized(&summary),
        services([
            (lab, "bot"),
            (lab, "reports"),
            (bench, "bot"),
            (theirs, "bot"),
        ])
    );
    let blocked: Services = summary
        .devices
        .iter()
        .flat_map(|device| {
            device
                .approvals
                .iter()
                .filter(|approval| {
                    approval.online_write_blocked == Some(OnlineWriteBlock::StorageFull)
                })
                .map(|approval| (device.device_id.clone(), approval.placement_id.clone()))
        })
        .collect();
    assert_eq!(blocked, services([(lab, "bot")]));
    let bot = summarized_approval(&summary, lab, "bot");
    assert!(bot.approver_is_me && bot.payer_is_me);
    assert_eq!(bot.online_access, Some(OnlineProjectAccess::ReadWrite));
    let on_their_own = summarized_approval(&summary, theirs, "bot");
    assert_eq!(
        (
            on_their_own.effective_expires_at,
            on_their_own.effective_limit
        ),
        (
            Some(on_their_own.expires_at),
            Some(EffectiveLimit::Approval)
        )
    );
    assert_eq!(summarized_limits(&summary, lab), vec![(500, true)]);

    // The device owner is no member of the project: its storage state is not theirs to see.
    let (_, mut found) = resource_summary::visible_approvals(state, "owner")
        .await
        .unwrap();
    project::mark_blocked_writes_with(db, "owner", &mut found.grants, full)
        .await
        .unwrap();
    assert!(asked.take().is_empty());
    assert!(
        found
            .grants
            .iter()
            .all(|grant| grant.online_write_blocked.is_none())
    );

    rule_reads();
    let nothing = summary_of(state, "third").await;
    assert!(nothing.devices.is_empty());
    assert_eq!(rule_reads(), 0);

    // Paying for an approval shows it, and only it, until the limit is withdrawn.
    let pay = |status: &'static str| {
        sql(
            r#"INSERT INTO "PlacementBillingGrant" (id,"grantId","payerId","approvedByUserId",status,"authzVersion","limitMicros","usedMicros","reservedMicros","expiresAt","createdAt") VALUES ('sponsored',$1,'third','third',$2,1,300,0,0,$3,$4) ON CONFLICT (id) DO UPDATE SET status=EXCLUDED.status"#,
            [
                fleet.bench_bot.grant_id.clone().into(),
                status.into(),
                (now() + 600).into(),
                now().into(),
            ],
        )
    };
    db.execute_raw(pay("active")).await.unwrap();
    let sponsored = summary_of(state, "third").await;
    assert_eq!(summarized(&sponsored), services([(bench, "bot")]));
    let approval = summarized_approval(&sponsored, bench, "bot");
    assert!(approval.payer_is_me && !approval.approver_is_me);
    assert_eq!(summarized_limits(&sponsored, bench), vec![(300, true)]);
    db.execute_raw(pay("revoked")).await.unwrap();
    assert!(summary_of(state, "third").await.devices.is_empty());

    // A withdrawn limit is not listed; one that ran out is, until a new one replaces it.
    revoke_billing(state, "owner", edge, &owners_limit.billing_grant_id)
        .await
        .unwrap();
    db.execute_raw(sql(
        r#"UPDATE "PlacementBillingGrant" SET "createdAt"=$2,"expiresAt"=$3 WHERE id=$1"#,
        [
            bot_limit.billing_grant_id.clone().into(),
            (now() - 100).into(),
            (now() - 10).into(),
        ],
    ))
    .await
    .unwrap();
    let summary = summary_of(state, "owner").await;
    assert!(summarized_limits(&summary, edge).is_empty());
    assert!(!summarized_approval(&summary, edge, "own").payer_is_me);
    let ran_out = &summary
        .devices
        .iter()
        .find(|device| &device.device_id == lab)
        .unwrap()
        .billing;
    assert_eq!(ran_out.len(), 1);
    assert_eq!(ran_out[0].billing_grant_id, bot_limit.billing_grant_id);
    assert!(ran_out[0].expires_at < summary.server_time);
    approve_billing(state, "other", lab, &fleet.lab_bot.grant_id, spending(700))
        .await
        .unwrap();
    assert_eq!(
        summarized_limits(&summary_of(state, "other").await, lab),
        vec![(700, true)]
    );
}

/// Services by (device, placement) and how the viewer relates to the device.
type Placed = std::collections::BTreeMap<(String, String), DeviceRelationship>;

fn placed_on<'a>(
    entries: impl IntoIterator<Item = (&'a String, &'a str, DeviceRelationship)>,
) -> Placed {
    entries
        .into_iter()
        .map(|(device, placement, relationship)| {
            ((device.clone(), placement.to_owned()), relationship)
        })
        .collect()
}

async fn placed(state: &DeviceContext<'_>, viewer: &str) -> app_placements::AppDevicePlacements {
    app_placements::placements(state, viewer, "fleet-app")
        .await
        .unwrap()
}

fn relationships(found: &app_placements::AppDevicePlacements) -> Placed {
    let listed: Placed = found
        .placements
        .iter()
        .map(|placement| {
            (
                (placement.device_id.clone(), placement.placement_id.clone()),
                placement.relationship,
            )
        })
        .collect();
    assert_eq!(
        listed.len(),
        found.placements.len(),
        "one entry per service"
    );
    listed
}

fn placement<'a>(
    found: &'a app_placements::AppDevicePlacements,
    device_id: &str,
    placement: &str,
) -> &'a app_placements::AppDevicePlacement {
    found
        .placements
        .iter()
        .find(|entry| entry.device_id == device_id && entry.placement_id == placement)
        .unwrap_or_else(|| panic!("{placement} is listed"))
}

async fn an_apps_placements_are_listed_once_per_service(
    state: &DeviceContext<'_>,
    db: &DatabaseConnection,
    fleet: &Fleet,
) {
    let Fleet {
        lab, bench, theirs, ..
    } = fleet;
    let leased = now();
    for (status, purpose, lease) in [
        ("active", None, leased + 300),
        ("active", Some("workload"), leased + 500),
        ("validating", Some("rollout_validation"), leased + 100),
        ("active", None, leased - 5),
        ("retired", None, leased + 900),
    ] {
        hold_lease(db, &fleet.lab_bot, status, purpose, lease).await;
    }

    // The approver sees their approvals wherever the device is still in their list.
    let theirs_view = placed(state, "other").await;
    assert!((leased..=now()).contains(&theirs_view.server_time));
    assert_eq!(
        relationships(&theirs_view),
        placed_on([
            (lab, "bot", DeviceRelationship::Shared),
            (lab, "reports", DeviceRelationship::Shared),
            (bench, "bot", DeviceRelationship::CloudApproval),
            (theirs, "bot", DeviceRelationship::Owner),
        ])
    );
    let bot = placement(&theirs_view, lab, "bot");
    assert_eq!(bot.deployment_id, "deployment-bot");
    assert_eq!(bot.grant.grant_id, fleet.lab_bot.grant_id);
    assert_eq!(bot.grant.status, "active");
    assert_eq!(bot.grant.model_ids, vec!["model".to_owned()]);
    assert_eq!(bot.grant.max_instances, 3);
    assert_eq!(
        bot.grant.online_access,
        Some(OnlineProjectAccess::ReadWrite)
    );
    assert_eq!(bot.grant.approved_by_user_id.as_deref(), Some("other"));
    assert_eq!(bot.grant.created_at, fleet.lab_bot.created_at);
    assert_eq!(
        bot.grant.effective_limit,
        Some(EffectiveLimit::SharingGrant)
    );
    assert!(
        (fleet.shared_until..=fleet.shared_until + 5)
            .contains(&bot.grant.effective_expires_at.unwrap())
    );
    let limit = bot.billing.as_ref().expect("the limit in force");
    assert_eq!((limit.limit_micros, limit.payer_is_me), (700, true));
    let shown = instances(state, "other", lab)
        .await
        .unwrap()
        .into_iter()
        .filter(|lease| lease.grant_id == fleet.lab_bot.grant_id)
        .count();
    assert_eq!(
        (bot.instances.active, shown),
        (3, 3),
        "the leases the device's instance list shows"
    );
    assert_eq!(bot.instances.newest_lease_expires_at, Some(leased + 500));
    let idle = placement(&theirs_view, lab, "reports");
    assert!(idle.billing.is_none());
    assert_eq!(
        (
            idle.instances.active,
            idle.instances.newest_lease_expires_at
        ),
        (0, None)
    );

    // A device's owner sees every approval on it, and nothing on other people's devices.
    let owners_view = placed(state, "owner").await;
    assert_eq!(
        relationships(&owners_view),
        placed_on([
            (lab, "bot", DeviceRelationship::Owner),
            (lab, "reports", DeviceRelationship::Owner),
            (bench, "bot", DeviceRelationship::Owner),
        ])
    );
    let limit = placement(&owners_view, lab, "bot")
        .billing
        .as_ref()
        .unwrap();
    assert_eq!((limit.limit_micros, limit.payer_is_me), (700, false));
    assert!(placed(state, "third").await.placements.is_empty());
    assert!(
        app_placements::placements(state, "other", "another-app")
            .await
            .unwrap()
            .placements
            .is_empty()
    );

    // A withdrawn limit leaves the service without one.
    let in_force = get_billing(state, "other", lab, &fleet.lab_bot.grant_id)
        .await
        .unwrap();
    revoke_billing(state, "other", lab, &in_force.billing_grant_id)
        .await
        .unwrap();
    assert!(
        placement(&placed(state, "other").await, lab, "bot")
            .billing
            .is_none()
    );

    // A replaced approval is listed once, as the one that runs.
    revoke_grant(state, "other", lab, &fleet.lab_reports.grant_id)
        .await
        .unwrap();
    let withdrawn = placed(state, "other").await;
    let reports = placement(&withdrawn, lab, "reports");
    assert_eq!(reports.grant.status, "revoked");
    assert_eq!(
        (
            reports.grant.effective_expires_at,
            reports.grant.effective_limit
        ),
        (None, None)
    );
    let replacement = create_grant(
        state,
        "other",
        lab,
        fleet_app("reports", OnlineProjectAccess::ReadOnly, &[]),
    )
    .await
    .unwrap();
    let replaced = placed(state, "other").await;
    assert_eq!(replaced.placements.len(), 4);
    let reports = placement(&replaced, lab, "reports");
    assert_eq!(
        (
            reports.grant.grant_id.as_str(),
            reports.grant.status.as_str()
        ),
        (replacement.grant_id.as_str(), "active")
    );

    // Sharing ended: the approvals stay reachable for as long as they are not withdrawn.
    let reissued = now();
    publish_rules(
        db,
        &fleet.lab_signer,
        Some(&fleet.lab_rules),
        rules_for(lab, reissued, reissued + 3_600, Vec::new()),
    )
    .await;
    let unshared = placed(state, "other").await;
    assert_eq!(
        relationships(&unshared),
        placed_on([
            (lab, "bot", DeviceRelationship::CloudApproval),
            (lab, "reports", DeviceRelationship::CloudApproval),
            (bench, "bot", DeviceRelationship::CloudApproval),
            (theirs, "bot", DeviceRelationship::Owner),
        ])
    );
    let ended = &placement(&unshared, lab, "bot").grant;
    assert_eq!(
        (ended.effective_expires_at, ended.effective_limit),
        (Some(reissued), Some(EffectiveLimit::SharingGrant))
    );
    for grant_id in [&fleet.lab_bot.grant_id, &replacement.grant_id] {
        revoke_grant(state, "other", lab, grant_id).await.unwrap();
    }
    assert_eq!(
        relationships(&placed(state, "other").await),
        placed_on([
            (bench, "bot", DeviceRelationship::CloudApproval),
            (theirs, "bot", DeviceRelationship::Owner),
        ]),
        "a device that left their list shows nothing of what they once approved"
    );
    let history = placed(state, "owner").await;
    assert_eq!(history.placements.len(), 3);
    for service in ["bot", "reports"] {
        assert_eq!(placement(&history, lab, service).grant.status, "revoked");
    }
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn fleet_reads_list_only_what_the_caller_may_see() {
    let schema = TestSchema::create().await;
    let db = schema.db.clone();
    let policy = standalone();
    let state = context(&db, &policy);
    let fleet = seed_fleet(&state, &db).await;
    the_summary_lists_each_viewers_approvals_and_limits(&state, &db, &fleet).await;
    an_apps_placements_are_listed_once_per_service(&state, &db, &fleet).await;
    let disabled = StandaloneConfig::default();
    assert_status(
        resource_summary::visible_approvals(&context(&db, &disabled), "owner").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    assert_status(
        app_placements::placements(&context(&db, &disabled), "owner", "fleet-app").await,
        StatusCode::SERVICE_UNAVAILABLE,
    );
    schema.drop().await;
}
