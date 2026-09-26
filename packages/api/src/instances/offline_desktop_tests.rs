use super::*;
use flow_like::hub::StandaloneConfig;
use sea_orm::{ConnectOptions, Database, DatabaseConnection};

const INSTALLATION: &str = "71bfc449-a2f1-4fa1-aed8-a3819d0f32d5";

fn desktop(sub: &str, app_id: &str, installation_id: &str) -> ReplayPrincipal<'static> {
    ReplayPrincipal::Desktop {
        principal: DesktopReplayPrincipal {
            sub: sub.into(),
            app_id: app_id.into(),
            installation_id: installation_id.into(),
        },
        authority: DesktopAuthority::Fixed(RolePermissions::ExecuteEvents),
    }
}

fn with_permissions(permissions: RolePermissions) -> ReplayPrincipal<'static> {
    ReplayPrincipal::Desktop {
        principal: DesktopReplayPrincipal {
            sub: "owner".into(),
            app_id: "project".into(),
            installation_id: INSTALLATION.into(),
        },
        authority: DesktopAuthority::Fixed(permissions),
    }
}

fn put_request(purpose: StoragePurpose, bytes: &[u8]) -> OfflineReplayRequest {
    OfflineReplayRequest {
        operation_id: uuid::Uuid::new_v4().to_string(),
        resource: OfflineResource::File {
            purpose,
            path: "report.txt".into(),
        },
        expected: OfflineExpected::FileAbsent,
        mutation: OfflineMutation::FilePut {
            data_base64: STANDARD.encode(bytes),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        },
    }
}

fn table_request(mutation: OfflineMutation) -> OfflineReplayRequest {
    OfflineReplayRequest {
        operation_id: uuid::Uuid::new_v4().to_string(),
        resource: OfflineResource::Table {
            purpose: StoragePurpose::User,
            database: "db".into(),
            table: "rows".into(),
        },
        expected: OfflineExpected::TableVersion {
            version: 1,
            fingerprint: Some(format!("blake3:{}", "0".repeat(64))),
        },
        mutation,
    }
}

fn key_for(principal: &ReplayPrincipal<'_>, request: &OfflineReplayRequest) -> ReceiptKey {
    principal
        .receipt_key(request, &request.digest().unwrap())
        .unwrap()
}

#[test]
fn desktop_receipt_keys_never_collide_with_instance_grants() {
    let request = put_request(StoragePurpose::User, b"payload");
    let key = key_for(&desktop("owner", "project", INSTALLATION), &request);
    assert!(key.grant_id.starts_with("desktop:"));
    assert_eq!(key.grant_id.len(), "desktop:".len() + 64);
    assert_eq!(key.authz_version, 0);
    assert!(
        validate_instance_identifier(&key.grant_id).is_err(),
        "instance grant IDs are identifiers and can never equal a desktop key"
    );
    assert_eq!(
        key.grant_id,
        key_for(&desktop("owner", "project", INSTALLATION), &request).grant_id,
        "keys are stable per account, app and installation"
    );
    for other in [
        desktop("other", "project", INSTALLATION),
        desktop("owner", "other", INSTALLATION),
        desktop("owner", "project", "0a7c52b4-3e2f-4c1d-9f1e-2b8d6c5a4e3f"),
    ] {
        assert_ne!(key_for(&other, &request).grant_id, key.grant_id);
    }
    let owner = desktop("owner", "project", INSTALLATION).owner();
    assert_eq!(owner.instance_id, INSTALLATION);
    assert_eq!(owner.project_id, "project");
    assert_eq!(owner.sub, "owner");
    assert!(
        desktop("owner", "project", INSTALLATION)
            .claim_lock()
            .is_none()
    );
}

#[test]
fn digest_reuse_is_409_offline_digest_reused_on_both_paths() {
    let request = put_request(StoragePurpose::User, b"payload");
    let key = key_for(&desktop("owner", "project", INSTALLATION), &request);
    ensure_same_digest(&key.digest, &key).unwrap();
    let error = ensure_same_digest("another-digest", &key).unwrap_err();
    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.public_code(), OFFLINE_ERROR_DIGEST_REUSED);
    assert_eq!(
        error.public_message(),
        Some("Offline operation ID was already used with another payload or precondition")
    );
}

#[test]
fn desktop_4xx_responses_never_follow_a_claim() {
    let principal = desktop("owner", "project", INSTALLATION);
    let mut traversal = put_request(StoragePurpose::User, b"x");
    traversal.resource = OfflineResource::File {
        purpose: StoragePurpose::User,
        path: "../other/report.txt".into(),
    };
    let mut temporary = put_request(StoragePurpose::Temporary, b"x");
    temporary.resource = OfflineResource::File {
        purpose: StoragePurpose::Temporary,
        path: "scratch.txt".into(),
    };
    let volatile = table_request(OfflineMutation::TableUpdate {
        filter: "id = 1".into(),
        updates: std::collections::BTreeMap::from([("seen".into(), "now()".into())]),
    });
    let empty = table_request(OfflineMutation::TableInsert { rows: vec![] });
    for request in [traversal, volatile, empty] {
        request.digest().unwrap();
        principal
            .receipt_key(&request, &request.digest().unwrap())
            .unwrap();
        let error = replay_target(&principal, &request)
            .err()
            .expect("rejected before any receipt is read");
        assert!(
            matches!(error.status().as_u16(), 400 | 403),
            "{:?}",
            request.resource
        );
    }
    assert!(replay_target(&principal, &temporary).is_ok());
    assert!(
        temporary.validate_desktop(&DESKTOP_OFFLINE_LIMITS).is_err(),
        "the handler rejects temporary paths before replay"
    );
    for request in [
        put_request(StoragePurpose::User, b"ok"),
        table_request(OfflineMutation::TableDelete {
            filter: "id = 1".into(),
        }),
    ] {
        assert!(replay_target(&principal, &request).is_ok());
    }
}

#[flow_like_types::tokio::test]
async fn desktop_recheck_applies_the_permission_matrix() {
    let db = DatabaseConnection::default();
    let config = StandaloneConfig::default();
    let context = DeviceContext {
        db: &db,
        dialect: crate::db::DbDialect::Postgres,
        config: &config,
        domain: "unused.example",
        secure: true,
    };
    let user_file = put_request(StoragePurpose::User, b"x").resource;
    let project_file = put_request(StoragePurpose::Files, b"x").resource;
    let runner = with_permissions(RolePermissions::ExecuteEvents);
    runner.recheck(&context, &user_file).await.unwrap();
    let error = runner.recheck(&context, &project_file).await.unwrap_err();
    assert_eq!(error.status(), StatusCode::FORBIDDEN);
    assert_eq!(error.public_code(), OFFLINE_ERROR_FORBIDDEN);
    with_permissions(RolePermissions::ExecuteEvents | RolePermissions::WriteFiles)
        .recheck(&context, &project_file)
        .await
        .unwrap();
    assert!(
        with_permissions(RolePermissions::empty())
            .recheck(&context, &user_file)
            .await
            .is_err(),
        "a revoked role stops a claimed replay before dispatch"
    );
    let deadline = runner.upload_deadline(&context).await.unwrap();
    assert!((now() + 299..=now() + 301).contains(&deadline));
}

#[flow_like_types::tokio::test]
async fn same_content_create_is_applied_and_different_content_conflicts() {
    let store = object_store::memory::InMemory::new();
    let path = Path::from("users/owner/apps/project/report.txt");
    let request = put_request(StoragePurpose::User, b"device bytes");
    let digest = request.digest().unwrap();
    let existing = store
        .put(&path, b"device bytes".to_vec().into())
        .await
        .unwrap();
    let applied = put_file(&store, &path, &request, &digest).await;
    assert_eq!(applied.status, OfflineReplayStatus::Applied);
    assert_eq!(
        applied.result,
        Some(OfflineExpected::FileRevision {
            e_tag: existing.e_tag,
            version: existing.version,
        })
    );

    let longer = Path::from("users/owner/apps/project/longer.txt");
    store
        .put(&longer, b"device bytes and more".to_vec().into())
        .await
        .unwrap();
    assert_eq!(
        put_file(&store, &longer, &request, &digest).await.status,
        OfflineReplayStatus::Conflict
    );
    let same_length = Path::from("users/owner/apps/project/same-length.txt");
    store
        .put(&same_length, b"other  bytes".to_vec().into())
        .await
        .unwrap();
    assert_eq!(
        put_file(&store, &same_length, &request, &digest)
            .await
            .status,
        OfflineReplayStatus::Conflict
    );
    assert_eq!(
        store
            .get(&same_length)
            .await
            .unwrap()
            .bytes()
            .await
            .unwrap()
            .as_ref(),
        b"other  bytes",
        "a conflicting create never replaces cloud content"
    );

    let empty = put_request(StoragePurpose::User, b"");
    let empty_path = Path::from("users/owner/apps/project/empty.txt");
    store.put(&empty_path, Vec::new().into()).await.unwrap();
    assert_eq!(
        put_file(&store, &empty_path, &empty, &empty.digest().unwrap())
            .await
            .status,
        OfflineReplayStatus::Applied
    );
}

#[flow_like_types::tokio::test]
async fn attempting_file_put_reconciles_when_bytes_match_and_stays_unknown_otherwise() {
    let store = object_store::memory::InMemory::new();
    let path = Path::from("apps/project/upload/report.txt");
    let request = put_request(StoragePurpose::Files, b"queued bytes");
    let digest = request.digest().unwrap();
    assert!(
        reconcile_put(&store, &path, &request, &digest)
            .await
            .is_none(),
        "an absent object proves nothing"
    );
    store
        .put(&path, b"other writer".to_vec().into())
        .await
        .unwrap();
    assert!(
        reconcile_put(&store, &path, &request, &digest)
            .await
            .is_none()
    );
    store
        .put(&path, b"queued bytes, then more".to_vec().into())
        .await
        .unwrap();
    assert!(
        reconcile_put(&store, &path, &request, &digest)
            .await
            .is_none()
    );
    let written = store
        .put(&path, b"queued bytes".to_vec().into())
        .await
        .unwrap();
    let reconciled = reconcile_put(&store, &path, &request, &digest)
        .await
        .expect("the object holds exactly the queued bytes");
    assert_eq!(reconciled.status, OfflineReplayStatus::Applied);
    assert_eq!(reconciled.operation_id, request.operation_id);
    assert_eq!(reconciled.digest, digest);
    assert_eq!(
        reconciled.result,
        Some(OfflineExpected::FileRevision {
            e_tag: written.e_tag,
            version: written.version,
        })
    );
    let mut delete = request.clone();
    delete.mutation = OfflineMutation::FileDelete;
    assert!(
        reconcile_put(&store, &path, &delete, &digest)
            .await
            .is_none(),
        "absence or presence never proves which delete ran"
    );
}

#[test]
fn desktop_audit_names_the_replayed_resource() {
    let (action, resource_type, resource_id, details) =
        desktop_audit(&put_request(StoragePurpose::User, b"x"));
    assert_eq!(
        (action, resource_type, resource_id.as_str()),
        ("storage.files.offline_replay", "StorageFile", "report.txt")
    );
    assert_eq!(details["kind"], "file_put");
    assert_eq!(details["user_scoped"], true);
    let request = table_request(OfflineMutation::TableDelete {
        filter: "id = 1".into(),
    });
    let (action, resource_type, resource_id, details) = desktop_audit(&request);
    assert_eq!(
        (action, resource_type, resource_id.as_str()),
        ("database.rows.offline_replay", "DatabaseTable", "rows")
    );
    assert_eq!(details["operation_id"], request.operation_id.as_str());
    assert_eq!(details["kind"], "table_delete");
    for action in [
        "database.rows.offline_replay",
        "storage.files.offline_replay",
    ] {
        assert_eq!(
            crate::audit::level::required_level(action),
            crate::audit::level::AuditLevel::Verbose
        );
    }
}

async fn receipt_database(url: &str) -> DatabaseConnection {
    let admin = Database::connect(url).await.unwrap();
    let schema = format!("desktop_offline_{}", uuid::Uuid::new_v4().simple());
    admin
        .execute_unprepared(&format!("CREATE SCHEMA {schema}"))
        .await
        .unwrap();
    let mut options = ConnectOptions::new(url.to_owned());
    options
        .set_schema_search_path(&schema)
        .max_connections(8)
        .min_connections(1);
    let db = Database::connect(options).await.unwrap();
    for statement in
        include_str!("../../prisma/migrations/20260923150000_instance_offline_replay/migration.sql")
            .split(';')
            .filter(|statement| !statement.trim().is_empty())
    {
        db.execute_unprepared(statement).await.unwrap();
    }
    db
}

#[flow_like_types::tokio::test]
#[ignore = "requires FLOW_LIKE_DEVICE_TEST_DATABASE_URL pointing to disposable PostgreSQL"]
async fn desktop_receipt_lifecycle() {
    let url =
        std::env::var("FLOW_LIKE_DEVICE_TEST_DATABASE_URL").expect("Use disposable PostgreSQL");
    let db = receipt_database(&url).await;
    let config = StandaloneConfig::default();
    let context = DeviceContext {
        db: &db,
        dialect: crate::db::DbDialect::Postgres,
        config: &config,
        domain: "unused.example",
        secure: true,
    };
    let principal = desktop("owner", "project", INSTALLATION);
    let request = put_request(StoragePurpose::User, b"one");
    let digest = request.digest().unwrap();
    let key = key_for(&principal, &request);

    let (first, second) = flow_like_types::tokio::join!(
        claim(&context, &principal, &key, None),
        claim(&context, &principal, &key, None)
    );
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "ON CONFLICT admits exactly one dispatcher"
    );
    assert_eq!(receipt(&db, &key).await.unwrap(), Some(None));
    let row = db
        .query_one_raw(sql(
            r#"SELECT "instanceId","projectId","delegatingUserId" FROM "InstanceOfflineReceipt" WHERE "grantId"=$1 AND "authzVersion"=$2 AND "operationId"=$3"#,
            key.values(),
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.try_get::<String>("", "instanceId").unwrap(),
        INSTALLATION
    );
    assert_eq!(row.try_get::<String>("", "projectId").unwrap(), "project");
    assert_eq!(
        row.try_get::<String>("", "delegatingUserId").unwrap(),
        "owner"
    );
    assert!(
        !claim(&context, &principal, &key, None).await.unwrap(),
        "a retained attempt is never redispatched"
    );

    let mut changed = key.clone();
    changed.digest = "different-payload".into();
    let error = claim(&context, &principal, &changed, None)
        .await
        .unwrap_err();
    assert_eq!(error.status(), StatusCode::CONFLICT);
    assert_eq!(error.public_code(), OFFLINE_ERROR_DIGEST_REUSED);

    let other_installation = desktop("owner", "project", "0a7c52b4-3e2f-4c1d-9f1e-2b8d6c5a4e3f");
    let other_key = key_for(&other_installation, &request);
    assert!(receipt(&db, &other_key).await.unwrap().is_none());
    assert!(
        claim(&context, &other_installation, &other_key, None)
            .await
            .unwrap(),
        "receipts are scoped per installation"
    );

    let applied = response(
        &request,
        &digest,
        OfflineReplayStatus::Applied,
        Some(OfflineExpected::FileRevision {
            e_tag: Some("etag".into()),
            version: None,
        }),
        None,
    );
    assert_eq!(
        retain(&context, &key, applied.clone()).await.unwrap(),
        (applied.clone(), true),
        "the write that retains Applied is the one that audits it"
    );
    assert_eq!(
        retain(
            &context,
            &key,
            response(&request, &digest, OfflineReplayStatus::Conflict, None, None)
        )
        .await
        .unwrap(),
        (applied, false),
        "a retained result is never replaced or audited again"
    );

    let blocked_request = put_request(StoragePurpose::User, b"retry");
    let blocked_digest = blocked_request.digest().unwrap();
    let blocked_key = key_for(&principal, &blocked_request);
    assert!(
        claim(&context, &principal, &blocked_key, None)
            .await
            .unwrap()
    );
    let blocked = response(
        &blocked_request,
        &blocked_digest,
        OfflineReplayStatus::Blocked,
        None,
        Some("Your role in this project does not allow this offline change"),
    );
    finish(&context, &blocked_key, blocked.clone())
        .await
        .unwrap();
    assert_eq!(
        receipt(&db, &blocked_key).await.unwrap(),
        Some(Some(blocked))
    );
    let (first, second) = flow_like_types::tokio::join!(
        claim(&context, &principal, &blocked_key, None),
        claim(&context, &principal, &blocked_key, None)
    );
    assert_ne!(
        first.unwrap(),
        second.unwrap(),
        "a blocked retry is re-admitted exactly once"
    );
    assert_eq!(receipt(&db, &blocked_key).await.unwrap(), Some(None));
}
