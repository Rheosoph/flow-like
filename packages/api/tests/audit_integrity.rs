//! Audit regression against a disposable PostgreSQL database.
//! Run with AUDIT_TEST_DATABASE_URL and `cargo test -p flow-like-api --test
//! audit_integrity -- --ignored`. The test creates its own initially absent
//! AuditEntry, MutationLock and ExecutionRun tables; it refuses existing fixtures.

use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like_api::{
    audit::{
        chain::{EntryHashFields, compute_entry_hash_v2},
        service::{AuditEntryInput, AuditFilter, AuditService},
        sign,
    },
    db::DbDialect,
    entity::{audit_entry, sea_orm_active_enums::AuditActorType},
};
use p256::{
    ecdsa::{Signature, SigningKey, signature::Signer},
    pkcs8::{EncodePrivateKey, EncodePublicKey, LineEnding},
};
use sea_orm::{
    ActiveEnum, ColumnTrait, ConnectionTrait, Database, DatabaseBackend, EntityTrait, QueryFilter,
    Statement,
};

fn input(chain_id: Option<&str>, action: &str, resource_id: &str) -> AuditEntryInput {
    AuditEntryInput {
        actor_id: "audit-test-user".into(),
        actor_type: AuditActorType::User,
        actor_ip: Some("192.0.2.10".into()),
        action: action.into(),
        resource_type: "AuditTest".into(),
        resource_id: resource_id.into(),
        chain_id: chain_id.map(str::to_owned),
        summary: "Audit regression fixture".into(),
        details: None,
    }
}

fn filter(chain_id: Option<&str>, action: Option<&str>) -> AuditFilter {
    AuditFilter {
        chain_id: chain_id.map(str::to_owned),
        action: action.map(str::to_owned),
        actor_id: None,
        resource_type: None,
        resource_id: None,
        limit: Some(200),
        offset: None,
    }
}

#[tokio::test]
#[ignore = "requires AUDIT_TEST_DATABASE_URL pointing to a disposable PostgreSQL database"]
async fn concurrent_appends_roundtrip_verify_and_detect_tampering() {
    let url = std::env::var("AUDIT_TEST_DATABASE_URL")
        .expect("set AUDIT_TEST_DATABASE_URL to a disposable PostgreSQL database");
    let db = Database::connect(url).await.unwrap();
    db.execute_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"
        CREATE TABLE "AuditEntry" (
            "id" TEXT PRIMARY KEY, "sequence" BIGINT NOT NULL,
            "timestamp" TIMESTAMPTZ(3) NOT NULL, "actorId" TEXT NOT NULL,
            "actorType" TEXT NOT NULL, "actorIp" TEXT, "action" TEXT NOT NULL,
            "resourceType" TEXT NOT NULL, "resourceId" TEXT NOT NULL, "chainId" TEXT,
            "summary" TEXT NOT NULL, "details" JSONB, "entryHash" TEXT NOT NULL,
            "prevHash" TEXT NOT NULL, "signature" TEXT, "kid" TEXT,
            UNIQUE ("chainId", "sequence")
        )"#,
    ))
    .await
    .unwrap();
    db.execute_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"
        CREATE TABLE "MutationLock" (
            "id" BIGINT PRIMARY KEY, "owner" TEXT, "expiresAt" TIMESTAMPTZ,
            "updatedAt" TIMESTAMPTZ NOT NULL DEFAULT now()
        )"#,
    ))
    .await
    .unwrap();
    let key = SigningKey::from_slice(&[19; 32]).unwrap();
    let pem = key.to_pkcs8_pem(LineEnding::LF).unwrap();
    sign::init(
        Some(&STANDARD.encode(pem.as_bytes())),
        Some("audit-test-key".into()),
    );

    // A null-scoped unique index does not protect this empty-root append race.
    let mut writers = tokio::task::JoinSet::new();
    for index in 0..24 {
        let db = db.clone();
        writers.spawn(async move {
            AuditService::record(
                &db,
                DbDialect::Postgres,
                input(None, "app.create", &format!("root-{index}")),
            )
            .await
            .unwrap()
        });
    }
    let mut sequences = Vec::new();
    while let Some(result) = writers.join_next().await {
        sequences.push(result.unwrap().sequence);
    }
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=24).collect::<Vec<_>>());
    let result = AuditService::verify_chain(&db, DbDialect::Postgres, None, None, None)
        .await
        .unwrap();
    assert!(result.valid && result.fully_authenticated, "{result:?}");
    assert_eq!(result.entries_checked, 24);

    let mut writers = tokio::task::JoinSet::new();
    for index in 0..24 {
        let db = db.clone();
        writers.spawn(async move {
            AuditService::record(
                &db,
                DbDialect::Postgres,
                input(
                    Some("test-branch"),
                    "app.update",
                    &format!("branch-{index}"),
                ),
            )
            .await
            .unwrap()
        });
    }
    let mut sequences = Vec::new();
    while let Some(result) = writers.join_next().await {
        sequences.push(result.unwrap().sequence);
    }
    sequences.sort_unstable();
    assert_eq!(sequences, (1..=24).collect::<Vec<_>>());
    let result =
        AuditService::verify_chain(&db, DbDialect::Postgres, Some("test-branch"), None, None)
            .await
            .unwrap();
    assert!(result.valid && result.fully_authenticated, "{result:?}");
    assert!(
        AuditService::verify_chain(
            &db,
            DbDialect::Postgres,
            Some("test-branch"),
            Some(3),
            Some(9)
        )
        .await
        .unwrap()
        .fully_authenticated
    );

    // A repeated terminal callback commits one transition even across replicas.
    let mut writers = tokio::task::JoinSet::new();
    for _ in 0..12 {
        let db = db.clone();
        writers.spawn(async move {
            AuditService::record_once(
                &db,
                DbDialect::Postgres,
                input(Some("test-branch"), "execution.completed", "same-run"),
            )
            .await
            .unwrap()
        });
    }
    let mut ids = std::collections::HashSet::new();
    while let Some(result) = writers.join_next().await {
        ids.insert(result.unwrap().id);
    }
    assert_eq!(ids.len(), 1);

    // ExecutionRun stores milliseconds. The sweep must reselect the same
    // persisted completion timestamp before it can append timeout evidence.
    db.execute_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"CREATE TABLE "ExecutionRun" (
            "id" TEXT PRIMARY KEY, "boardId" TEXT NOT NULL,
            "version" TEXT, "eventId" TEXT, "nodeId" TEXT,
            "logLevel" INTEGER NOT NULL DEFAULT 0,
            "inputPayloadLen" BIGINT NOT NULL DEFAULT 0, "inputPayloadKey" TEXT,
            "outputPayloadLen" BIGINT NOT NULL DEFAULT 0, "errorMessage" TEXT,
            "progress" INTEGER NOT NULL DEFAULT 0, "currentStep" TEXT,
            "startedAt" TIMESTAMPTZ(3), "completedAt" TIMESTAMPTZ(3),
            "expiresAt" TIMESTAMPTZ(3), "userId" TEXT, "appId" TEXT NOT NULL,
            "createdAt" TIMESTAMPTZ(3) NOT NULL, "updatedAt" TIMESTAMPTZ(3) NOT NULL,
            "technicalUserId" TEXT, "correlationKeys" JSONB, "parentRunId" TEXT,
            "traceId" TEXT, "regressionRunId" TEXT, "shadowOfRunId" TEXT,
            "variantName" TEXT, "status" TEXT NOT NULL, "mode" TEXT NOT NULL,
            "callerAppChain" JSONB, "runVariant" TEXT NOT NULL DEFAULT 'PRIMARY'
        )"#,
    ))
    .await
    .unwrap();
    db.execute_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"INSERT INTO "ExecutionRun"
            ("id", "boardId", "eventId", "appId", "createdAt", "updatedAt", "status", "mode")
            VALUES
            ('stale-run', 'board-1', 'event-1', 'sweep-app', now() - interval '2 hours', now() - interval '2 hours', 'PENDING', 'HTTP'),
            ('local-run', 'board-1', 'event-1', 'sweep-app', now() - interval '2 hours', now() - interval '2 hours', 'PENDING', 'LOCAL'),
            ('finished-run', 'board-1', 'event-1', 'sweep-app', now() - interval '2 hours', now() - interval '2 hours', 'COMPLETED', 'HTTP')"#,
    )).await.unwrap();
    let sweep_context = flow_like_api::audit::ExecutionAuditContext {
        db: std::sync::Arc::new(db.clone()),
        dialect: DbDialect::Postgres,
        enabled: true,
    };
    assert_eq!(
        flow_like_api::execution::run_sweeper::sweep_once(
            &sweep_context,
            std::time::Duration::from_secs(3600),
            10,
        )
        .await
        .unwrap(),
        1
    );
    let timed_out = flow_like_api::entity::execution_run::Entity::find_by_id("stale-run")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        timed_out.status,
        flow_like_api::entity::sea_orm_active_enums::RunStatus::Timeout
    );
    let completed_at = timed_out.completed_at.unwrap();
    assert_eq!(completed_at.timestamp_subsec_nanos() % 1_000_000, 0);
    let timeouts = AuditService::query(
        &db,
        filter(Some("sweep-app"), Some("execution.event.timeout")),
    )
    .await
    .unwrap();
    assert_eq!(
        timeouts.len(),
        1,
        "the timeout must produce its audit entry"
    );
    let recorded_at = chrono::DateTime::parse_from_rfc3339(
        timeouts[0].details.as_ref().unwrap()["completed_at"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(recorded_at, completed_at);
    assert_eq!(timeouts[0].resource_id, "stale-run");
    assert_eq!(
        flow_like_api::execution::run_sweeper::sweep_once(
            &sweep_context,
            std::time::Duration::from_secs(3600),
            10,
        )
        .await
        .unwrap(),
        0
    );
    assert!(
        AuditService::verify_chain(&db, DbDialect::Postgres, Some("sweep-app"), None, None)
            .await
            .unwrap()
            .fully_authenticated
    );

    // PostgreSQL JSONB normalizes decimal spelling, exponent notation and signed zero.
    for (index, details) in [
        serde_json::json!({"v": -0.0}),
        serde_json::json!({"v": 1e18}),
        serde_json::json!({"v": 1e-7}),
        serde_json::json!({"v": 1.2345678901234567e30}),
        serde_json::json!({"v": f64::MIN_POSITIVE}),
        serde_json::json!({"v": f64::MAX}),
        serde_json::json!({"v": 9_007_199_254_740_993_u64}),
        serde_json::json!({"v": [1.0, null, {"z": 2, "a": true}]}),
        serde_json::Value::Null,
    ]
    .into_iter()
    .enumerate()
    {
        let mut entry = input(Some("numeric"), "app.update", &format!("numeric-{index}"));
        entry.details = Some(details);
        AuditService::record(&db, DbDialect::Postgres, entry)
            .await
            .unwrap();
    }
    let result = AuditService::verify_chain(&db, DbDialect::Postgres, Some("numeric"), None, None)
        .await
        .unwrap();
    assert!(result.valid && result.fully_authenticated, "{result:?}");

    AuditService::record(
        &db,
        DbDialect::Postgres,
        input(None, "application.update", "prefix-trap"),
    )
    .await
    .unwrap();
    let root = AuditService::query(&db, filter(None, None)).await.unwrap();
    assert!(root.iter().all(|entry| entry.chain_id.is_none()));
    let matching = AuditService::query(&db, filter(None, Some("app.*")))
        .await
        .unwrap();
    assert_eq!(matching.len(), 24);

    let mut historical = AuditService::record(
        &db,
        DbDialect::Postgres,
        input(None, "audit.rotation.fixture", "historical-root"),
    )
    .await
    .unwrap();
    let predecessor = audit_entry::Entity::find()
        .filter(audit_entry::Column::ChainId.is_null())
        .filter(audit_entry::Column::Sequence.eq(historical.sequence - 1))
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    historical.kid = Some("unavailable-historical-key".into());
    historical.entry_hash = compute_entry_hash_v2(&EntryHashFields {
        id: &historical.id,
        sequence: historical.sequence,
        timestamp: &historical.timestamp,
        actor_id: &historical.actor_id,
        actor_type: &historical.actor_type.to_value(),
        actor_ip: historical.actor_ip.as_deref(),
        action: &historical.action,
        resource_type: &historical.resource_type,
        resource_id: &historical.resource_id,
        chain_id: None,
        summary: &historical.summary,
        details: historical.details.as_ref(),
        prev_hash: &historical.prev_hash,
        prev_signature: predecessor.signature.as_deref(),
        kid: historical.kid.as_deref(),
    });
    let historical_key = SigningKey::from_slice(&[23; 32]).unwrap();
    let historical_signature: Signature = historical_key.sign(historical.entry_hash.as_bytes());
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"UPDATE "AuditEntry" SET "entryHash" = $2, signature = $3, kid = $4 WHERE id = $1"#,
        [
            historical.id.into(),
            historical.entry_hash.into(),
            STANDARD.encode(historical_signature.to_der()).into(),
            historical.kid.into(),
        ],
    ))
    .await
    .unwrap();
    AuditService::record(
        &db,
        DbDialect::Postgres,
        input(Some("historical-anchor"), "app.create", "historical-branch"),
    )
    .await
    .unwrap();
    let result = AuditService::verify_chain(
        &db,
        DbDialect::Postgres,
        Some("historical-anchor"),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(!result.valid && !result.fully_authenticated, "{result:?}");
    assert_eq!(result.unverifiable_signatures, 1);
    assert_eq!(result.first_broken_at, None);
    let public_key = historical_key
        .verifying_key()
        .to_public_key_pem(LineEnding::LF)
        .unwrap();
    let registry = serde_json::json!({"unavailable-historical-key": public_key}).to_string();
    sign::init_verifying_keys(Some(&registry)).unwrap();
    let result = AuditService::verify_chain(
        &db,
        DbDialect::Postgres,
        Some("historical-anchor"),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(result.valid && result.fully_authenticated, "{result:?}");
    assert_eq!(result.signatures_verified, 2);

    let tail = AuditService::query(&db, filter(Some("test-branch"), None))
        .await
        .unwrap()
        .remove(0);
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"UPDATE "AuditEntry" SET signature = 'tampered' WHERE id = $1"#,
        [tail.id.clone().into()],
    ))
    .await
    .unwrap();
    let result =
        AuditService::verify_chain(&db, DbDialect::Postgres, Some("test-branch"), None, None)
            .await
            .unwrap();
    assert!(!result.valid);
    assert_eq!(result.first_broken_at, Some(tail.sequence));
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"UPDATE "AuditEntry" SET signature = $2, summary = 'tampered' WHERE id = $1"#,
        [tail.id.clone().into(), tail.signature.clone().into()],
    ))
    .await
    .unwrap();
    assert!(
        !AuditService::verify_chain(&db, DbDialect::Postgres, Some("test-branch"), None, None)
            .await
            .unwrap()
            .valid
    );

    audit_entry::Entity::delete_many()
        .filter(audit_entry::Column::ChainId.eq("numeric"))
        .filter(audit_entry::Column::Sequence.eq(3))
        .exec(&db)
        .await
        .unwrap();
    let result = AuditService::verify_chain(&db, DbDialect::Postgres, Some("numeric"), None, None)
        .await
        .unwrap();
    assert!(!result.valid);
    assert_eq!(result.first_broken_at, Some(3));
    assert!(
        !AuditService::verify_chain(&db, DbDialect::Postgres, None, Some(1000), Some(1001))
            .await
            .unwrap()
            .valid
    );
    assert!(
        AuditService::verify_chain(&db, DbDialect::Postgres, None, Some(0), None)
            .await
            .is_err()
    );
}
