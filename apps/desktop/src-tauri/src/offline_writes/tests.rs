//! Tests of desktop offline writes (design §7 C).

mod fixture;

use super::{
    OfflineWrites, api_base, commands,
    data_studio::{self, DataStudioTarget, Route},
    engine_dir, hub_key, installation_id, pending_count, scope_id, texts, valid_scope,
};
use super::{
    host::{CloudAccess, LeaseCloud, Services, TransportStatus, map_answer, sidecar},
    object_index::{ContentRoots, Known, ObjectIndex},
    run_storage::{
        RunStorage, RunStorageMode, RunStorageRequest, disconnected_subject, hosted_config,
        hosted_subject,
    },
    scope::{
        AppOfflineScope, Coalescer, ConfiguredTable, OfflineLimitsDto, OfflineTablePurpose,
        OfflineTableStatus, RunMode, ScopeDescriptor, ScopeKey, keep_both_name, read_descriptor,
        read_descriptors, sync_state, write_descriptor,
    },
    stores::{
        CacheMiss, DesktopCloudView, DesktopFileStore, Failure, FileLayer, LocalCacheView, Refusal,
        ScopeConnectivity, UnavailableStore, UnreadyTables, failure, offline_observation,
    },
};
use fixture::{
    ALICE, APP, BOB, DirectoryCloud, Fixture, TempDir, bulky_rows, connect, eventually, jwt, rows,
    selection,
};
use flow_like::{
    app::AppVisibility,
    credentials::{
        SharedCredentials, StoreType,
        aws_credentials::AwsSharedCredentials,
        renewable::{RenewableSharedCredentials, SharedCredentialRefresh},
    },
    flow_like_storage::{
        Path,
        databases::vector::{
            VectorStore,
            lancedb::{DatabaseSelector, LanceDBVectorStore},
        },
        files::store::{FlowLikeStore, local_store::LocalObjectStore},
        lance::session::Session,
        lancedb::Connection,
        object_store::{
            self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
            ObjectStore, ObjectStoreExt, PutMode, PutMultipartOptions, PutOptions, PutPayload,
            PutResult, memory::InMemory, path::Path as ObjectPath,
        },
    },
    state::FlowLikeConfig,
};
use flow_like_device_protocol::{
    OfflineExpected, OfflineMutation, OfflineReplayRequest, OfflineReplayStatus, OfflineResource,
    StoragePurpose, validate_installation_id,
};
use flow_like_offline_writes::{
    Connectivity, Observation, OfflineHost, ReplayErrorKind, TableActivation, fs::directory_bytes,
};
use flow_like_types::{
    authorization::AuthorizationError,
    base64::{Engine, engine::general_purpose::STANDARD},
};
use futures::{StreamExt, TryStreamExt, stream::BoxStream};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant, SystemTime},
};

const MIB: u64 = 1024 * 1024;
const INSTALLATION: &str = "3f0e0c55-0a8f-4d5c-9a51-6f1f3f7b2c10";

fn key(path: &str) -> ObjectPath {
    ObjectPath::parse(path).unwrap()
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn project_db() -> Path {
    Path::from("apps").join(APP).join("storage").join("db")
}

async fn local_connection() -> (TempDir, Connection) {
    let directory = TempDir::new("local-db");
    let connection = connect(directory.path()).await.unwrap();
    (directory, connection)
}

fn opened(connection: &Connection, table: &str) -> LanceDBVectorStore {
    LanceDBVectorStore::from_connection_for_overlay(
        connection.clone(),
        table.to_owned(),
        DatabaseSelector::default(),
    )
    .unwrap()
}

fn request<'a>(token: &'a str, hub: &'a str) -> RunStorageRequest<'a> {
    RunStorageRequest {
        visibility: &AppVisibility::Private,
        app_id: APP,
        hub,
        secure: false,
        token: Some(token),
        session_id: None,
        webview: None,
        event_id: None,
        device_credentials: None,
    }
}

fn unreachable() -> flow_like_types::Error {
    flow_like_types::Error::new(AuthorizationError::Unavailable).context("presign failed")
}

struct Refresh(SharedCredentials);

#[async_trait::async_trait]
impl SharedCredentialRefresh for Refresh {
    async fn refresh(&self) -> Result<SharedCredentials, AuthorizationError> {
        Ok(self.0.clone())
    }
}

fn aws(user_prefix: &str) -> SharedCredentials {
    SharedCredentials::Aws(AwsSharedCredentials {
        access_key_id: Some("access".into()),
        secret_access_key: Some("secret".into()),
        session_token: Some("session".into()),
        meta_bucket: "meta".into(),
        content_bucket: "content".into(),
        logs_bucket: String::new(),
        meta_config: None,
        content_config: None,
        logs_config: None,
        region: "eu-central-1".into(),
        expiration: Some((SystemTime::now() + Duration::from_secs(3600)).into()),
        content_path_prefix: Some(format!("apps/{APP}")),
        user_content_path_prefix: Some(user_prefix.to_owned()),
    })
}

async fn lease(subject: &str) -> Arc<RenewableSharedCredentials> {
    let initial = aws(&format!("users/{subject}/apps/{APP}"));
    RenewableSharedCredentials::new(initial.clone(), APP.into(), Arc::new(Refresh(initial)))
        .await
        .unwrap()
}

fn cache_view(dir: &TempDir, subject: &str) -> LocalCacheView {
    let roots = ContentRoots::new(APP, Some(subject));
    for folder in ["project", "user", "index"] {
        std::fs::create_dir_all(dir.path().join(folder)).unwrap();
    }
    LocalCacheView {
        index: Some(Arc::new(
            ObjectIndex::open(&dir.path().join("index"), roots.clone()).unwrap(),
        )),
        project: Arc::new(LocalObjectStore::new(dir.path().join("project")).unwrap()),
        user: Arc::new(LocalObjectStore::new(dir.path().join("user")).unwrap()),
        roots,
    }
}

fn write_local(root: &std::path::Path, key: &str, bytes: &[u8]) {
    let path = root.join(key);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn meta(path: &str, size: u64, e_tag: Option<&str>, version: Option<&str>) -> ObjectMeta {
    ObjectMeta {
        location: key(path),
        last_modified: chrono::Utc::now(),
        size,
        e_tag: e_tag.map(str::to_owned),
        version: version.map(str::to_owned),
    }
}

async fn read(store: &dyn ObjectStore, path: &str) -> object_store::Result<Vec<u8>> {
    Ok(store.get(&key(path)).await?.bytes().await?.to_vec())
}

fn queued_request(scope: &AppOfflineScope, operation_id: &str) -> OfflineReplayRequest {
    let manager = scope.open_manager().unwrap();
    let operation = manager.queue().operation(operation_id).unwrap().unwrap();
    serde_json::from_value(operation.payload).unwrap()
}

fn offline_id(result: &PutResult) -> String {
    result
        .e_tag
        .as_deref()
        .and_then(|tag| tag.strip_prefix("offline-"))
        .expect("a queued write")
        .to_owned()
}

/// A cloud content store whose failures tests switch on.
#[derive(Debug, Default)]
struct TestCloud {
    inner: InMemory,
    fail_puts: AtomicBool,
    slow_gets: Mutex<Duration>,
    forbid_missing: AtomicBool,
    forbidden: Mutex<HashSet<String>>,
}

impl std::fmt::Display for TestCloud {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TestCloud")
    }
}

#[async_trait::async_trait]
impl ObjectStore for TestCloud {
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        if self.fail_puts.load(Ordering::SeqCst) {
            return Err(object_store::Error::Generic {
                store: "TestCloud",
                source: Box::new(std::io::Error::new(
                    std::io::ErrorKind::ConnectionRefused,
                    "connection refused",
                )),
            });
        }
        self.inner.put_opts(path, payload, options).await
    }

    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.inner.put_multipart_opts(path, options).await
    }

    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        let delay = *self.slow_gets.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        let forbidden = || object_store::Error::PermissionDenied {
            path: path.to_string(),
            source: "Access Denied".into(),
        };
        if self.forbidden.lock().unwrap().contains(path.as_ref()) {
            return Err(forbidden());
        }
        match self.inner.get_opts(path, options).await {
            Err(object_store::Error::NotFound { .. })
                if self.forbid_missing.load(Ordering::SeqCst) =>
            {
                Err(forbidden())
            }
            other => other,
        }
    }

    fn delete_stream(
        &self,
        paths: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        self.inner.delete_stream(paths)
    }

    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.inner.list(prefix)
    }

    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        self.inner.list_with_delimiter(prefix).await
    }

    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.inner.copy_opts(from, to, options).await
    }
}

/// A scope whose engine is open and whose hub keeps every change queued.
async fn queuing_scope(fixture: &Fixture, subject: &str) -> Arc<AppOfflineScope> {
    let scope = fixture.supported(subject).await;
    scope.manager().await.unwrap();
    fixture.hub.force(
        503,
        json!({"error": {"code": "ERROR", "message": "maintenance"}}),
    );
    scope
}

async fn managed_store(scope: &AppOfflineScope, table: &str) -> LanceDBVectorStore {
    let manager = scope.manager().await.unwrap();
    manager
        .managed_store(
            &scope.database_path(OfflineTablePurpose::Storage),
            table,
            DatabaseSelector::default(),
        )
        .await
        .unwrap()
        .expect("a registered table")
}

async fn insert_row(store: &mut LanceDBVectorStore, row: i64) -> String {
    store
        .insert(vec![json!({"id": row, "value": row, "label": "x"})])
        .await
        .unwrap();
    store.last_write_receipt().unwrap().operation_id
}

/// The error text of a decoration that must fail.
fn refused(result: flow_like_types::Result<LanceDBVectorStore>) -> String {
    match result {
        Ok(_) => panic!("the table must not open"),
        Err(error) => error.to_string(),
    }
}

async fn table_state_of(
    manager: &flow_like_offline_writes::WriteManager,
    table: &str,
) -> flow_like_offline_writes::TableState {
    manager
        .table_states()
        .await
        .unwrap()
        .into_iter()
        .find(|state| state.table.table == table)
        .expect("a registered table")
}

/// Waits until background downloads of `table` stopped growing its cached bytes.
async fn settle_cache(manager: &flow_like_offline_writes::WriteManager, table: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut last = u64::MAX;
    let mut stable = 0;
    while stable < 5 {
        assert!(
            Instant::now() < deadline,
            "the cache of '{table}' never settled"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        let cached = table_state_of(manager, table).await.cached_bytes;
        stable = if cached == last && cached > 0 {
            stable + 1
        } else {
            0
        };
        last = cached;
    }
}

async fn cloud_bytes_of(fixture: &Fixture, table: &str) -> u64 {
    let _ = fixture.cloud_table(ALICE, table).await;
    directory_bytes(
        &fixture
            .cloud_dir
            .path()
            .join(format!("apps/{APP}/storage/db/{table}.lance")),
    )
    .unwrap()
}

fn status_of(scope: &AppOfflineScope, table: &str) -> OfflineTableStatus {
    scope
        .configured(OfflineTablePurpose::Storage, table)
        .map(|table| table.status)
        .expect("a configured table")
}

#[test]
fn scope_id_is_stable_and_binds_hub_subject_app_and_installation() {
    let base = scope_id("https://hub.test", ALICE, APP, INSTALLATION);
    assert_eq!(base, scope_id("https://hub.test", ALICE, APP, INSTALLATION));
    assert!(valid_scope(&base));
    let expected = blake3::hash(
        format!(r#"{{"app":"{APP}","hub":"https://hub.test","installation":"{INSTALLATION}","subject":"{ALICE}","v":1}}"#)
            .as_bytes(),
    )
    .to_hex()
    .to_string();
    assert_eq!(base, expected);
    for other in [
        scope_id("https://other.test", ALICE, APP, INSTALLATION),
        scope_id("https://hub.test/prefix", ALICE, APP, INSTALLATION),
        scope_id("https://hub.test", BOB, APP, INSTALLATION),
        scope_id("https://hub.test", ALICE, "app2", INSTALLATION),
        scope_id(
            "https://hub.test",
            ALICE,
            APP,
            "0b7c2b52-9c4e-4a6f-8d11-2f7d0f6a1b90",
        ),
    ] {
        assert_ne!(base, other);
    }
}

#[test]
fn hub_key_keeps_path_prefixes_and_lowercases_scheme_and_host() {
    for (hub, secure, expected) in [
        (
            "api.example.test/prefix",
            true,
            Some("https://api.example.test/prefix"),
        ),
        (
            "api.example.test/prefix/",
            true,
            Some("https://api.example.test/prefix"),
        ),
        ("API.Example.TEST", true, Some("https://api.example.test")),
        (
            "HTTPS://Api.Example.Test/Prefix/",
            false,
            Some("https://api.example.test/Prefix"),
        ),
        ("localhost:8080", false, Some("http://localhost:8080")),
        (
            "https://api.example.test/api/v1",
            true,
            Some("https://api.example.test/api/v1"),
        ),
        ("", true, None),
        ("https://user:secret@api.example.test", true, None),
        ("ftp://api.example.test", true, None),
    ] {
        assert_eq!(hub_key(hub, secure).as_deref(), expected, "hub {hub:?}");
    }
    assert_eq!(
        api_base("https://api.example.test/prefix"),
        "https://api.example.test/prefix/api/v1"
    );
    assert_eq!(
        api_base("https://api.example.test/api/v1"),
        "https://api.example.test/api/v1"
    );
}

#[test]
fn installation_id_is_created_once_and_private() {
    let dir = TempDir::new("installation");
    let root = dir.path().join("offline-writes");
    let first = installation_id(&root).unwrap();
    validate_installation_id(&first).unwrap();
    assert_eq!(installation_id(&root).unwrap(), first);
    assert_eq!(
        OfflineWrites::at(root.clone()).installation_id().unwrap(),
        first
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root.join("installation-id"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        let directory = std::fs::metadata(&root).unwrap().permissions().mode();
        assert_eq!(directory & 0o077, 0);
    }
    std::fs::write(root.join("installation-id"), "not-a-uuid").unwrap();
    let replaced = installation_id(&root).unwrap();
    validate_installation_id(&replaced).unwrap();
    assert_ne!(replaced, first);
}

#[test]
fn descriptor_roundtrip_and_atomic_write() {
    let dir = TempDir::new("descriptor");
    let root = dir.path().join("offline-writes");
    let key = ScopeKey {
        hub: "https://hub.test".into(),
        subject: "auth0|123".into(),
        app_id: APP.into(),
    };
    let id = scope_id(&key.hub, &key.subject, &key.app_id, INSTALLATION);
    let mut descriptor = ScopeDescriptor::new(&id, &key);
    descriptor.hub_support = Some(true);
    descriptor.provider = Some("az".into());
    descriptor.tables.push(ConfiguredTable {
        purpose: OfflineTablePurpose::Storage,
        database: "db".into(),
        table: "orders".into(),
        primary_key: "id".into(),
        prefetch: true,
        status: OfflineTableStatus::Settling,
        error: None,
        enabled_at: 1_727_200_000,
    });
    write_descriptor(&root, &descriptor).unwrap();
    let read = read_descriptor(&root, &id).unwrap().unwrap();
    assert_eq!(
        serde_json::to_value(&read).unwrap(),
        serde_json::to_value(&descriptor).unwrap()
    );
    assert_eq!(read.key(), key);

    let raw: Value = serde_json::from_slice(
        &std::fs::read(root.join("scopes").join(format!("{id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(raw["appId"], APP);
    assert_eq!(raw["subject"], "auth0|123");
    assert_eq!(raw["hubSupport"], true);
    assert_eq!(raw["limits"]["maxMirrorBytes"], 1024 * 1024 * 1024);
    assert_eq!(raw["tables"][0]["primaryKey"], "id");
    assert_eq!(raw["tables"][0]["status"], "settling");
    let leftovers: Vec<String> = std::fs::read_dir(root.join("scopes"))
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".tmp"))
        .collect();
    assert!(
        leftovers.is_empty(),
        "no temporary files remain: {leftovers:?}"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(root.join("scopes").join(format!("{id}.json")))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    assert_eq!(read_descriptors(&root).len(), 1);

    let other = scope_id("https://hub.test", BOB, APP, INSTALLATION);
    std::fs::copy(
        root.join("scopes").join(format!("{id}.json")),
        root.join("scopes").join(format!("{other}.json")),
    )
    .unwrap();
    assert!(
        read_descriptor(&root, &other).is_err(),
        "a descriptor names its own scope"
    );
}

#[test]
fn object_index_records_revisions_per_full_key() {
    let dir = TempDir::new("index");
    let index = ObjectIndex::open(dir.path(), ContentRoots::new(APP, Some(ALICE))).unwrap();
    let text_file = format!("apps/{APP}/upload/a.txt");
    let json_file = format!("apps/{APP}/upload/a.json");
    index
        .observe_present(&meta(&text_file, 3, Some("etag-txt"), None))
        .unwrap();
    index
        .observe_present(&meta(&json_file, 4, Some("etag-json"), Some("7")))
        .unwrap();
    let Known::Present(found) = index.lookup(&key(&text_file)).unwrap() else {
        panic!("a.txt is known present");
    };
    assert_eq!(found.e_tag.as_deref(), Some("etag-txt"));
    assert_eq!(found.size, 3);
    let Known::Present(found) = index.lookup(&key(&json_file)).unwrap() else {
        panic!("a.json is known present");
    };
    assert_eq!(
        (found.e_tag.as_deref(), found.version.as_deref()),
        (Some("etag-json"), Some("7"))
    );

    index.observe_absent(&key(&text_file)).unwrap();
    assert_eq!(index.lookup(&key(&text_file)).unwrap(), Known::Absent);
    assert!(matches!(
        index.lookup(&key(&json_file)).unwrap(),
        Known::Present(_)
    ));

    for outside in [
        "apps/other/upload/a.txt".to_owned(),
        format!("apps/{APP}/storage/db/t.lance/data/x.lance"),
        format!("users/{BOB}/apps/{APP}/a.txt"),
        "tmp/a.txt".to_owned(),
    ] {
        index
            .observe_present(&meta(&outside, 1, Some("x"), None))
            .unwrap();
        assert_eq!(
            index.lookup(&key(&outside)).unwrap(),
            Known::Unknown,
            "{outside}"
        );
    }
    let user_file = format!("users/{ALICE}/apps/{APP}/notes/a.txt");
    index
        .observe_present(&meta(&user_file, 2, None, Some("12")))
        .unwrap();
    assert!(matches!(
        index.lookup(&key(&user_file)).unwrap(),
        Known::Present(_)
    ));
}

#[test]
fn object_index_listing_snapshots_follow_own_writes() {
    let dir = TempDir::new("listing");
    let index = ObjectIndex::open(dir.path(), ContentRoots::new(APP, Some(ALICE))).unwrap();
    let upload = key(&format!("apps/{APP}/upload"));
    let x = format!("apps/{APP}/upload/x.txt");
    let y = format!("apps/{APP}/upload/dir/y.txt");
    let z = format!("apps/{APP}/upload/dir/z.txt");
    index
        .observe_present(&meta(
            &format!("apps/{APP}/upload/stale.txt"),
            1,
            None,
            None,
        ))
        .unwrap();
    index
        .record_listing(
            &upload,
            &[meta(&x, 1, Some("1"), None), meta(&y, 2, Some("2"), None)],
        )
        .unwrap();
    let names = |entries: Vec<ObjectMeta>| {
        let mut names: Vec<String> = entries
            .into_iter()
            .map(|meta| meta.location.to_string())
            .collect();
        names.sort();
        names
    };
    assert_eq!(
        names(index.listing(&upload).unwrap().unwrap()),
        vec![y.clone(), x.clone()]
    );
    assert_eq!(
        names(
            index
                .listing(&key(&format!("apps/{APP}/upload/dir")))
                .unwrap()
                .unwrap()
        ),
        vec![y.clone()]
    );
    assert_eq!(
        index.lookup(&key(&z)).unwrap(),
        Known::Absent,
        "a complete listing proves absence"
    );
    assert_eq!(
        index
            .lookup(&key(&format!("apps/{APP}/upload/stale.txt")))
            .unwrap(),
        Known::Absent,
        "rows the listing lacks are absent"
    );

    index
        .observe_present(&meta(&z, 3, Some("3"), None))
        .unwrap();
    index.observe_absent(&key(&x)).unwrap();
    assert_eq!(
        names(index.listing(&upload).unwrap().unwrap()),
        vec![y.clone(), z.clone()]
    );

    assert!(
        index
            .listing(&key(&format!("apps/{APP}/storage/reports")))
            .unwrap()
            .is_none()
    );
    assert_eq!(
        index
            .lookup(&key(&format!("apps/{APP}/storage/reports/q.csv")))
            .unwrap(),
        Known::Unknown
    );

    let fresh = format!("apps/{APP}/upload/dir/new.txt");
    index.mark_queued(&key(&fresh), "op-1").unwrap();
    assert!(!index.forget_queued(&key(&fresh), "op-2").unwrap());
    assert!(index.forget_queued(&key(&fresh), "op-1").unwrap());
    assert!(!index.forget_queued(&key(&fresh), "op-1").unwrap());

    index.forget(&key(&y)).unwrap();
    assert!(
        index.listing(&upload).unwrap().is_none(),
        "a copy target drops covering listings"
    );
    assert_eq!(index.lookup(&key(&y)).unwrap(), Known::Unknown);
}

fn listed_names(entries: Option<Vec<ObjectMeta>>) -> Vec<String> {
    let mut names: Vec<String> = entries
        .expect("a recorded listing")
        .into_iter()
        .map(|meta| meta.location.to_string())
        .collect();
    names.sort();
    names
}

#[test]
fn object_index_newer_listings_reconcile_nested_snapshots() {
    let dir = TempDir::new("nested-listings");
    let index = ObjectIndex::open(dir.path(), ContentRoots::new(APP, Some(ALICE))).unwrap();
    let upload = key(&format!("apps/{APP}/upload"));
    let docs = key(&format!("apps/{APP}/upload/docs"));
    let a = format!("apps/{APP}/upload/docs/a.txt");
    let b = format!("apps/{APP}/upload/docs/b.txt");
    let c = format!("apps/{APP}/upload/docs/c.txt");
    let x = format!("apps/{APP}/upload/x.txt");

    index
        .record_listing(&docs, &[meta(&a, 1, Some("a"), None)])
        .unwrap();
    index
        .record_listing(
            &upload,
            &[meta(&b, 1, Some("b"), None), meta(&x, 1, Some("x"), None)],
        )
        .unwrap();
    assert_eq!(
        listed_names(index.listing(&docs).unwrap()),
        vec![b.clone()],
        "a newer listing above drops the older snapshots below it"
    );
    assert_eq!(index.lookup(&key(&a)).unwrap(), Known::Absent);

    index
        .record_listing(&docs, &[meta(&c, 1, Some("c"), None)])
        .unwrap();
    assert_eq!(
        listed_names(index.listing(&upload).unwrap()),
        vec![c.clone(), x.clone()],
        "a newer listing below updates the snapshots above it"
    );
    assert_eq!(index.lookup(&key(&b)).unwrap(), Known::Absent);
    assert!(matches!(index.lookup(&key(&c)).unwrap(), Known::Present(_)));
}

#[test]
fn object_index_listing_sweep_is_exact_about_case_and_wildcards() {
    let dir = TempDir::new("listing-sweep");
    let index = ObjectIndex::open(dir.path(), ContentRoots::new(APP, Some(ALICE))).unwrap();
    let capitalized = format!("apps/{APP}/upload/Docs/a.txt");
    let wildcard = format!("apps/{APP}/upload/axb/f.txt");
    for path in [&capitalized, &wildcard] {
        index
            .observe_present(&meta(path, 1, Some("1"), None))
            .unwrap();
    }
    for folder in ["docs", "a_b"] {
        index
            .record_listing(&key(&format!("apps/{APP}/upload/{folder}")), &[])
            .unwrap();
    }
    for path in [&capitalized, &wildcard] {
        assert!(
            matches!(index.lookup(&key(path)).unwrap(), Known::Present(_)),
            "{path} is outside the listed folders"
        );
    }
}

#[test]
fn object_index_folder_listing_says_nothing_about_a_file_of_its_name() {
    let dir = TempDir::new("folder-name");
    let index = ObjectIndex::open(dir.path(), ContentRoots::new(APP, Some(ALICE))).unwrap();
    let report = format!("apps/{APP}/upload/report");
    index
        .record_listing(
            &key(&report),
            &[meta(&format!("{report}/part-1"), 1, None, None)],
        )
        .unwrap();
    assert_eq!(index.lookup(&key(&report)).unwrap(), Known::Unknown);
    assert_eq!(
        index.lookup(&key(&format!("{report}/part-2"))).unwrap(),
        Known::Absent
    );
}

#[test]
fn object_index_row_bound_trims_objects_and_keeps_listings() {
    let dir = TempDir::new("row-bound");
    let index = ObjectIndex::open(dir.path(), ContentRoots::new(APP, Some(ALICE)))
        .unwrap()
        .with_max_objects(2);
    let upload = key(&format!("apps/{APP}/upload"));
    let files: Vec<String> = (0..4)
        .map(|file| format!("apps/{APP}/upload/{file}.txt"))
        .collect();
    let entries: Vec<ObjectMeta> = files
        .iter()
        .map(|path| meta(path, 1, Some("1"), None))
        .collect();
    index.record_listing(&upload, &entries).unwrap();
    assert_eq!(listed_names(index.listing(&upload).unwrap()), files);
    let rows: i64 = rusqlite::Connection::open(dir.path().join("objects.sqlite"))
        .unwrap()
        .query_row("SELECT COUNT(*) FROM objects", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 2);
    for path in &files {
        assert!(
            matches!(index.lookup(&key(path)).unwrap(), Known::Present(_)),
            "{path}"
        );
    }
}

#[test]
fn object_index_records_one_listing_level_in_one_batch() {
    let dir = TempDir::new("listing-level");
    let index = ObjectIndex::open(dir.path(), ContentRoots::new(APP, Some(ALICE))).unwrap();
    let upload = key(&format!("apps/{APP}/upload"));
    let same = format!("apps/{APP}/upload/same.txt");
    let changed = format!("apps/{APP}/upload/changed.txt");
    let fresh = format!("apps/{APP}/upload/fresh.txt");
    let foreign = "apps/other/upload/x.txt";
    let unchanged = meta(&same, 1, Some("1"), None);
    index
        .record_listing(
            &upload,
            &[unchanged.clone(), meta(&changed, 1, Some("1"), None)],
        )
        .unwrap();
    index
        .observe_present_all(&[
            unchanged,
            meta(&changed, 2, Some("2"), None),
            meta(&fresh, 3, Some("3"), None),
            meta(foreign, 4, Some("4"), None),
        ])
        .unwrap();
    let mut sizes: Vec<(String, u64)> = index
        .listing(&upload)
        .unwrap()
        .unwrap()
        .into_iter()
        .map(|meta| (meta.location.to_string(), meta.size))
        .collect();
    sizes.sort();
    assert_eq!(sizes, vec![(changed.clone(), 2), (fresh, 3), (same, 1)]);
    let Known::Present(recorded) = index.lookup(&key(&changed)).unwrap() else {
        panic!("{changed} is known present");
    };
    assert_eq!((recorded.size, recorded.e_tag.as_deref()), (2, Some("2")));
    assert_eq!(index.lookup(&key(foreign)).unwrap(), Known::Unknown);
}

#[tokio::test]
async fn local_cache_view_known_absent_present_unknown() {
    let dir = TempDir::new("cache-view");
    let view = cache_view(&dir, ALICE);
    let index = view.index.clone().unwrap();
    let project = dir.path().join("project");
    let present = format!("apps/{APP}/upload/present.txt");
    let stale = format!("apps/{APP}/upload/stale.txt");
    let uncached = format!("apps/{APP}/upload/uncached.txt");
    let unknown = format!("apps/{APP}/upload/unknown.txt");
    let absent = format!("apps/{APP}/upload/absent.txt");
    let user = format!("users/{ALICE}/apps/{APP}/notes.txt");

    write_local(&project, &present, b"cached");
    index
        .observe_present(&meta(&present, 6, Some("etag-1"), None))
        .unwrap();
    assert_eq!(read(&view, &present).await.unwrap(), b"cached");
    let head = view.head(&key(&present)).await.unwrap();
    assert_eq!(head.e_tag.as_deref(), Some("etag-1"));

    write_local(&project, &stale, b"older bytes");
    index
        .observe_present(&meta(&stale, 3, Some("etag-2"), None))
        .unwrap();
    assert_eq!(view.head(&key(&stale)).await.unwrap().size, 3);
    let error = read(&view, &stale).await.unwrap_err();
    assert!(text(&error).contains(&texts::not_cached(&stale)), "{error}");

    index
        .observe_present(&meta(&uncached, 4, Some("etag-3"), None))
        .unwrap();
    assert_eq!(
        view.head(&key(&uncached)).await.unwrap().e_tag.as_deref(),
        Some("etag-3")
    );
    let error = read(&view, &uncached).await.unwrap_err();
    assert_eq!(failure(&error), Failure::Connect);

    write_local(&project, &unknown, b"legacy");
    let head = view.head(&key(&unknown)).await.unwrap();
    assert_eq!((head.size, head.e_tag, head.version), (6, None, None));
    assert_eq!(read(&view, &unknown).await.unwrap(), b"legacy");

    index.observe_absent(&key(&absent)).unwrap();
    write_local(&project, &absent, b"deleted in the cloud");
    assert!(matches!(
        view.head(&key(&absent)).await,
        Err(object_store::Error::NotFound { .. })
    ));

    let error = view
        .head(&key(&format!("apps/{APP}/upload/nowhere.txt")))
        .await
        .unwrap_err();
    assert!(
        text(&error).contains("is not cached on this device"),
        "{error}"
    );

    write_local(&dir.path().join("user"), &user, b"mine");
    assert_eq!(read(&view, &user).await.unwrap(), b"mine");
}

#[tokio::test]
async fn local_cache_view_denies_other_apps_and_accounts_and_passes_metadata_through() {
    let dir = TempDir::new("cache-scope");
    let view = cache_view(&dir, ALICE);
    let project = dir.path().join("project");
    for denied in [
        "apps/other/upload/x.txt".to_owned(),
        "apps/other/storage/x.txt".to_owned(),
        format!("users/{BOB}/apps/{APP}/x.txt"),
        format!("users/{ALICE}/apps/other/x.txt"),
        "tmp/global/apps/app1/x".to_owned(),
    ] {
        write_local(&project, &denied, b"secret");
        let error = read(&view, &denied).await.unwrap_err();
        assert!(
            matches!(error, object_store::Error::PermissionDenied { .. }),
            "{denied}: {error}"
        );
        assert!(
            text(&error).contains(&texts::other_scope(&denied)),
            "{error}"
        );
        let listed: Vec<_> = view.list(Some(&key(&denied))).collect().await;
        assert!(
            listed.iter().all(Result::is_err),
            "{denied} cannot be listed"
        );
    }
    for metadata in [
        format!("apps/{APP}/metadata/app.json"),
        "apps/other/boards/b.json".to_owned(),
    ] {
        write_local(&project, &metadata, b"{}");
        assert_eq!(read(&view, &metadata).await.unwrap(), b"{}", "{metadata}");
    }
    let database = format!("apps/{APP}/storage/db/orders.lance/_versions/1.manifest");
    let error = read(&view, &database).await.unwrap_err();
    assert!(
        text(&error).contains(&texts::database_unavailable(false)),
        "{error}"
    );
    let error = view
        .put(&key(&format!("apps/{APP}/other/x.txt")), "x".into())
        .await
        .unwrap_err();
    assert!(text(&error).contains(&texts::write_needs_hub(&format!("apps/{APP}/other/x.txt"))));
}

fn disconnected_file_store(view: LocalCacheView) -> FlowLikeStore {
    FlowLikeStore::Signed(Arc::new(DesktopFileStore {
        layer: FileLayer::Refused(Refusal::HubUnsupported),
        cloud: None,
        cache: view,
    }))
}

fn is_denied(error: &object_store::Error) -> bool {
    matches!(error, object_store::Error::PermissionDenied { .. })
}

#[tokio::test]
async fn metadata_passthrough_denies_case_variants_and_content_ancestors() {
    let dir = TempDir::new("cache-ancestors");
    let view = cache_view(&dir, ALICE);
    let project = dir.path().join("project");
    for variant in [
        "apps/other/Upload/x.txt".to_owned(),
        "apps/other/STORAGE/x.txt".to_owned(),
        "apps/other/upload./x.txt".to_owned(),
        "apps/other/\u{17f}torage/x.txt".to_owned(),
        format!("apps/{APP}/Upload/x.txt"),
        format!("apps/{APP}/sTorage/x.txt"),
    ] {
        write_local(&project, &variant, b"secret");
        let error = read(&view, &variant).await.unwrap_err();
        assert!(
            text(&error).contains(&texts::other_scope(&variant)),
            "{variant}: {error}"
        );
        assert!(
            is_denied(&view.head(&key(&variant)).await.unwrap_err()),
            "{variant}"
        );
    }
    for ancestor in [
        "apps".to_owned(),
        "apps/other".to_owned(),
        format!("apps/{APP}"),
    ] {
        let listed: Vec<_> = view.list(Some(&key(&ancestor))).collect().await;
        assert!(
            !listed.is_empty()
                && listed
                    .iter()
                    .all(|entry| entry.as_ref().is_err_and(is_denied)),
            "{ancestor} cannot be listed"
        );
        let error = view
            .list_with_delimiter(Some(&key(&ancestor)))
            .await
            .unwrap_err();
        assert!(
            text(&error).contains(&texts::other_scope(&ancestor)),
            "{ancestor}: {error}"
        );
        assert!(
            is_denied(&view.head(&key(&ancestor)).await.unwrap_err()),
            "{ancestor}"
        );
    }
    assert!(
        view.list(None)
            .collect::<Vec<_>>()
            .await
            .iter()
            .all(Result::is_err)
    );
    assert!(view.list_with_delimiter(None).await.is_err());

    let own_metadata = format!("apps/{APP}/metadata/app.json");
    let own_board = format!("apps/{APP}/boards/b.json");
    let other_board = "apps/other/boards/b.json".to_owned();
    for metadata in [&own_metadata, &own_board, &other_board] {
        write_local(&project, metadata, b"{}");
        assert_eq!(read(&view, metadata).await.unwrap(), b"{}", "{metadata}");
    }
    let listed = view
        .list_with_delimiter(Some(&key(&format!("apps/{APP}/metadata"))))
        .await
        .unwrap();
    assert_eq!(
        listed
            .objects
            .iter()
            .map(|meta| meta.location.to_string())
            .collect::<Vec<_>>(),
        vec![own_metadata]
    );

    let store = disconnected_file_store(view);
    for denied in [
        "apps".to_owned(),
        "apps/other".to_owned(),
        "apps/other/Upload/x.txt".to_owned(),
    ] {
        let error = store
            .sign("GET", &key(&denied), Duration::from_secs(60))
            .await
            .unwrap_err();
        assert!(
            text(&error).contains(&texts::other_scope(&denied)),
            "{denied}: {error}"
        );
    }
    assert!(
        store
            .sign("GET", &key(&own_board), Duration::from_secs(60))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn disconnected_links_need_the_whole_file_on_this_device() {
    let dir = TempDir::new("disconnected-links");
    let view = cache_view(&dir, ALICE);
    let index = view.index.clone().unwrap();
    let store = disconnected_file_store(view);
    let large = format!("apps/{APP}/upload/large.bin");
    let size = 9 * MIB;
    index
        .observe_present(&meta(&large, size, Some("etag"), None))
        .unwrap();
    let error = store
        .sign("GET", &key(&large), Duration::from_secs(60))
        .await
        .unwrap_err();
    assert!(text(&error).contains(&texts::not_cached(&large)), "{error}");

    let local = dir.path().join("project").join(&large);
    write_local(&dir.path().join("project"), &large, b"partial");
    let error = store
        .sign("GET", &key(&large), Duration::from_secs(60))
        .await
        .unwrap_err();
    assert!(text(&error).contains(&texts::not_cached(&large)), "{error}");

    std::fs::File::options()
        .write(true)
        .open(&local)
        .unwrap()
        .set_len(size)
        .unwrap();
    assert!(
        store
            .sign("GET", &key(&large), Duration::from_secs(60))
            .await
            .is_ok()
    );
}

fn hosted_view(dir: &TempDir, cloud: Arc<TestCloud>) -> DesktopCloudView {
    DesktopCloudView::new(
        Some(cloud),
        cache_view(dir, ALICE),
        ScopeConnectivity::new("http://127.0.0.1:9"),
    )
}

#[tokio::test]
async fn versioned_miss_does_not_record_the_key_absent() {
    let dir = TempDir::new("versioned-miss");
    let view = hosted_view(&dir, Arc::new(TestCloud::default()));
    let index = view.cache.index.clone().unwrap();
    let path = format!("apps/{APP}/upload/versioned.txt");
    let versioned = GetOptions {
        version: Some("7".into()),
        ..GetOptions::default()
    };
    assert!(matches!(
        view.get_opts(&key(&path), versioned).await,
        Err(object_store::Error::NotFound { .. })
    ));
    assert_eq!(index.lookup(&key(&path)).unwrap(), Known::Unknown);
    assert!(matches!(
        view.get_opts(&key(&path), GetOptions::default()).await,
        Err(object_store::Error::NotFound { .. })
    ));
    assert_eq!(index.lookup(&key(&path)).unwrap(), Known::Absent);
}

#[tokio::test]
async fn online_multipart_upload_is_recorded_when_it_completes() {
    let dir = TempDir::new("multipart");
    let cloud = Arc::new(TestCloud::default());
    let view = hosted_view(&dir, cloud.clone());
    let index = view.cache.index.clone().unwrap();
    let upload = key(&format!("apps/{APP}/upload"));
    let existing = format!("apps/{APP}/upload/existing.txt");
    index
        .record_listing(&upload, &[meta(&existing, 1, Some("1"), None)])
        .unwrap();

    let big = format!("apps/{APP}/upload/big.bin");
    let mut writer = view.put_multipart(&key(&big)).await.unwrap();
    assert!(
        index.listing(&upload).unwrap().is_some(),
        "starting an upload keeps the folder listing"
    );
    assert_eq!(index.lookup(&key(&big)).unwrap(), Known::Absent);
    writer.put_part("abc".into()).await.unwrap();
    writer.put_part("defg".into()).await.unwrap();
    let result = writer.complete().await.unwrap();
    assert_eq!(read(&cloud.inner, &big).await.unwrap(), b"abcdefg");
    let Known::Present(recorded) = index.lookup(&key(&big)).unwrap() else {
        panic!("a completed upload is recorded");
    };
    assert_eq!((recorded.size, recorded.e_tag), (7, result.e_tag));
    assert_eq!(
        listed_names(index.listing(&upload).unwrap()),
        vec![big.clone(), existing.clone()]
    );

    let aborted = format!("apps/{APP}/upload/aborted.bin");
    let mut writer = view.put_multipart(&key(&aborted)).await.unwrap();
    writer.put_part("x".into()).await.unwrap();
    writer.abort().await.unwrap();
    assert_eq!(index.lookup(&key(&aborted)).unwrap(), Known::Absent);
    assert_eq!(
        listed_names(index.listing(&upload).unwrap()),
        vec![big, existing]
    );
}

#[test]
fn cache_miss_is_classified_offline() {
    let miss = object_store::Error::Generic {
        store: "DesktopOfflineCache",
        source: Box::new(CacheMiss(texts::not_cached("apps/app1/upload/a.txt"))),
    };
    assert_eq!(offline_observation(&miss), Some(Observation::ConnectFailed));
    assert_eq!(failure(&miss), Failure::Connect);
    let timeout = object_store::Error::Generic {
        store: "Test",
        source: Box::new(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out",
        )),
    };
    assert_eq!(offline_observation(&timeout), Some(Observation::TimedOut));
    let refused = object_store::Error::Generic {
        store: "Test",
        source: Box::new(std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "refused",
        )),
    };
    assert_eq!(
        offline_observation(&refused),
        Some(Observation::ConnectFailed)
    );
    let lease = object_store::Error::Generic {
        store: "Test",
        source: Box::new(AuthorizationError::Unavailable),
    };
    assert_eq!(
        offline_observation(&lease),
        Some(Observation::ConnectFailed)
    );
    let expired = object_store::Error::Generic {
        store: "Test",
        source: Box::new(AuthorizationError::Expired),
    };
    assert_eq!(
        failure(&expired),
        Failure::Other,
        "the hub answered: the lease expired or it returned expired credentials"
    );
    for online in [
        expired,
        object_store::Error::NotFound {
            path: "a".into(),
            source: "missing".into(),
        },
        object_store::Error::PermissionDenied {
            path: "a".into(),
            source: "denied".into(),
        },
        object_store::Error::Generic {
            store: "Test",
            source: Box::new(AuthorizationError::Denied),
        },
    ] {
        assert_eq!(offline_observation(&online), None, "{online}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_store_rejects_writes_outside_roots_with_e5() {
    let fixture = Fixture::new().await;
    let scope = queuing_scope(&fixture, ALICE).await;
    let store = scope.disconnected_store().unwrap();
    let outside = format!("apps/{APP}/other/x.txt");
    let error = store.put(&key(&outside), "x".into()).await.unwrap_err();
    assert!(
        text(&error).contains(&texts::write_needs_hub(&outside)),
        "{error}"
    );
    for denied in ["apps/other/upload/x.txt", "tmp/x.txt"] {
        let error = store.put(&key(denied), "x".into()).await.unwrap_err();
        assert!(
            matches!(error, object_store::Error::PermissionDenied { .. }),
            "{denied}: {error}"
        );
    }
    let database = format!("apps/{APP}/storage/db/orders.lance/data/x.lance");
    let error = store.put(&key(&database), "x".into()).await.unwrap_err();
    assert!(
        text(&error).contains("Lance objects cannot be buffered"),
        "{error}"
    );
    assert_eq!(
        scope
            .open_manager()
            .unwrap()
            .queue()
            .status()
            .unwrap()
            .pending_count,
        0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_new_file_is_queued_and_readable_from_pending_bytes() {
    let fixture = Fixture::new().await;
    let scope = queuing_scope(&fixture, ALICE).await;
    let store = Arc::new(scope.disconnected_store().unwrap());
    let path = format!("apps/{APP}/upload/new.txt");
    let result = store.put(&key(&path), "hello".into()).await.unwrap();
    let operation = offline_id(&result);
    let request = queued_request(&scope, &operation);
    assert_eq!(request.expected, OfflineExpected::FileAbsent);
    assert_eq!(read(store.as_ref(), &path).await.unwrap(), b"hello");
    assert_eq!(store.head(&key(&path)).await.unwrap().size, 5);

    let signed = FlowLikeStore::Signed(store.clone());
    let link = signed
        .sign("GET", &key(&path), Duration::from_secs(60))
        .await
        .unwrap();
    assert!(link.as_str().starts_with("data:"), "{link}");
    let error = signed
        .sign("PUT", &key(&path), Duration::from_secs(60))
        .await
        .unwrap_err();
    assert!(
        text(&error).contains(&texts::waiting_to_upload(&path)),
        "{error}"
    );
    assert!(
        scope
            .index()
            .unwrap()
            .forget_queued(&key(&path), &operation)
            .unwrap()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn offline_listing_without_snapshot_fails_with_e20() {
    let fixture = Fixture::new().await;
    let scope = queuing_scope(&fixture, ALICE).await;
    let store = scope.disconnected_store().unwrap();
    let reports = key(&format!("apps/{APP}/upload/reports"));
    let listed: Result<Vec<ObjectMeta>, _> = store.list(Some(&reports)).try_collect().await;
    let error = listed.unwrap_err();
    assert!(
        text(&error).contains(&texts::listing_unavailable(&format!(
            "apps/{APP}/upload/reports"
        ))),
        "{error}"
    );
    assert!(store.list_with_delimiter(Some(&reports)).await.is_err());

    scope
        .index()
        .unwrap()
        .record_listing(
            &key(&format!("apps/{APP}/upload")),
            &[meta(
                &format!("apps/{APP}/upload/reports/a.csv"),
                1,
                Some("1"),
                None,
            )],
        )
        .unwrap();
    store
        .put(
            &key(&format!("apps/{APP}/upload/reports/b.csv")),
            "b".into(),
        )
        .await
        .unwrap();
    let mut names: Vec<String> = store
        .list(Some(&reports))
        .map_ok(|meta| meta.location.to_string())
        .try_collect()
        .await
        .unwrap();
    names.sort();
    assert_eq!(
        names,
        vec![
            format!("apps/{APP}/upload/reports/a.csv"),
            format!("apps/{APP}/upload/reports/b.csv")
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn explicit_create_of_unknown_path_fails_with_e21() {
    let fixture = Fixture::new().await;
    let scope = queuing_scope(&fixture, ALICE).await;
    let store = scope.disconnected_store().unwrap();
    let path = format!("apps/{APP}/upload/maybe.txt");
    let create = PutOptions {
        mode: PutMode::Create,
        ..PutOptions::default()
    };
    let error = store
        .put_opts(&key(&path), "x".into(), create)
        .await
        .unwrap_err();
    assert!(
        text(&error).contains(&format!(
            "'{path}' may already exist in the cloud, and this device cannot check while offline."
        )),
        "{error}"
    );
    let queued = store.put(&key(&path), "x".into()).await.unwrap();
    assert_eq!(
        queued_request(&scope, &offline_id(&queued)).expected,
        OfflineExpected::FileAbsent
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn delta_log_writes_fail_offline_with_e22_and_pass_online() {
    let fixture = Fixture::new().await;
    let scope = queuing_scope(&fixture, ALICE).await;
    let commit = format!("apps/{APP}/upload/events/_delta_log/00000000000000000000.json");
    let error = scope
        .disconnected_store()
        .unwrap()
        .put(&key(&commit), "{}".into())
        .await
        .unwrap_err();
    assert!(
        text(&error).contains(&format!(
            "'{commit}' belongs to a table format commit log (Delta, Iceberg or Hudi)."
        )),
        "{error}"
    );

    let cloud = Arc::new(TestCloud::default());
    let hosted = scope.decorate_content(FlowLikeStore::Other(cloud.clone()));
    assert!(matches!(hosted, FlowLikeStore::Signed(_)));
    let written = hosted
        .as_generic()
        .put(&key(&commit), "{}".into())
        .await
        .unwrap();
    assert!(written.e_tag.is_none_or(|tag| !tag.starts_with("offline-")));
    assert_eq!(read(&cloud.inner, &commit).await.unwrap(), b"{}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn s3_existing_file_overwrite_fails_offline() {
    let fixture = Fixture::new().await;
    let scope = queuing_scope(&fixture, ALICE).await;
    assert_eq!(scope.provider(), "s3");
    let path = format!("apps/{APP}/upload/report.csv");
    scope
        .index()
        .unwrap()
        .observe_present(&meta(&path, 3, Some("\"s3-etag\""), None))
        .unwrap();
    let store = scope.disconnected_store().unwrap();
    let unreplayable = "This cloud provider cannot replay conditional file overwrites or deletes";
    let error = store.put(&key(&path), "new".into()).await.unwrap_err();
    assert!(text(&error).contains(unreplayable), "{error}");
    let error = store.delete(&key(&path)).await.unwrap_err();
    assert!(text(&error).contains(unreplayable), "{error}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn azure_and_gcs_queue_file_revision_from_the_index() {
    let fixture = Fixture::new().await;
    for (subject, provider, e_tag, version) in [
        (ALICE, "az", Some("0x8DC1"), None),
        (BOB, "gs", None, Some("1712000000000")),
    ] {
        let scope = queuing_scope(&fixture, subject).await;
        scope
            .update_descriptor(|descriptor| descriptor.provider = Some(provider.into()))
            .unwrap();
        let path = format!("apps/{APP}/upload/{provider}.txt");
        scope
            .index()
            .unwrap()
            .observe_present(&meta(&path, 3, e_tag, version))
            .unwrap();
        let store = scope.disconnected_store().unwrap();
        let queued = store.put(&key(&path), "new".into()).await.unwrap();
        assert_eq!(
            queued_request(&scope, &offline_id(&queued)).expected,
            OfflineExpected::FileRevision {
                e_tag: e_tag.map(str::to_owned),
                version: version.map(str::to_owned),
            },
            "{provider}"
        );

        let legacy = format!("apps/{APP}/upload/{provider}-legacy.txt");
        write_local(&fixture.dirs().project, &legacy, b"old");
        let error = store.put(&key(&legacy), "new".into()).await.unwrap_err();
        assert!(text(&error).contains(&format!(
            "This device does not know the cloud revision of '{legacy}', so it cannot be replaced or deleted offline."
        )), "{provider}: {error}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hub_without_support_blocks_offline_file_writes_with_e26() {
    let fixture = Fixture::new().await;
    let scope = fixture.scope(ALICE);
    let path = format!("apps/{APP}/upload/a.txt");
    for support in [None, Some(false)] {
        scope
            .update_descriptor(|descriptor| descriptor.hub_support = support)
            .unwrap();
        let error = scope
            .disconnected_store()
            .unwrap()
            .put(&key(&path), "x".into())
            .await
            .unwrap_err();
        assert!(
            text(&error).contains(&texts::hub_rejects_files(&path)),
            "{support:?}: {error}"
        );
    }
    let outside = format!("apps/{APP}/other/a.txt");
    let error = scope
        .disconnected_store()
        .unwrap()
        .put(&key(&outside), "x".into())
        .await
        .unwrap_err();
    assert!(
        text(&error).contains(&texts::write_needs_hub(&outside)),
        "{error}"
    );
    assert!(
        FlowLikeStore::Other(Arc::new(InMemory::new()))
            .as_generic()
            .to_string()
            == scope
                .decorate_content(FlowLikeStore::Other(Arc::new(InMemory::new())))
                .as_generic()
                .to_string(),
        "Hosted runs keep their cloud store while the hub lacks support"
    );
}

#[tokio::test]
async fn unavailable_store_names_the_table_in_its_error() {
    let tables = UnreadyTables {
        project: HashSet::from(["orders".to_owned()]),
        user: HashSet::from(["notes".to_owned()]),
    };
    let project = format!("apps/{APP}/storage/db");
    let user = format!("users/{ALICE}/apps/{APP}/db");
    assert_eq!(
        tables.text(&format!("{project}/orders.lance/_versions/1.manifest")),
        texts::table_not_ready("orders")
    );
    assert_eq!(
        tables.text(&format!("{project}/customers.lance/_versions/1.manifest")),
        texts::table_unavailable("customers")
    );
    assert_eq!(
        tables.text(&format!("{user}/notes.lance/data/a.lance")),
        texts::table_not_ready("notes")
    );
    assert_eq!(
        tables.text(&format!("{user}/orders.lance/data/a.lance")),
        texts::table_unavailable("orders")
    );
    assert_eq!(tables.text(&project), texts::database_unavailable(false));
    assert_eq!(tables.text(&user), texts::database_unavailable(true));

    let store = UnavailableStore::new(Arc::new(tables));
    let error = store
        .get(&key(&format!("{project}/customers.lance/_latest.manifest")))
        .await
        .unwrap_err();
    assert!(
        text(&error).contains(&texts::table_unavailable("customers")),
        "{error}"
    );
    let error = store
        .put(&key(&format!("{project}/orders.lance/x")), "x".into())
        .await
        .unwrap_err();
    assert!(
        text(&error).contains(&texts::table_not_ready("orders")),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_direct_write_falls_back_to_queue_on_connect_error() {
    let fixture = Fixture::new().await;
    let scope = queuing_scope(&fixture, ALICE).await;
    let cloud = Arc::new(TestCloud::default());
    let hosted = scope
        .decorate_content(FlowLikeStore::Other(cloud.clone()))
        .as_generic();
    assert_eq!(hosted.to_string(), "DesktopFileStore");

    let direct = format!("apps/{APP}/upload/direct.txt");
    let written = hosted.put(&key(&direct), "cloud".into()).await.unwrap();
    assert!(written.e_tag.is_none_or(|tag| !tag.starts_with("offline-")));
    assert_eq!(read(&cloud.inner, &direct).await.unwrap(), b"cloud");
    let Known::Present(recorded) = scope.index().unwrap().lookup(&key(&direct)).unwrap() else {
        panic!("a direct write is recorded");
    };
    assert_eq!(recorded.size, 5);

    cloud.fail_puts.store(true, Ordering::SeqCst);
    let queued_path = format!("apps/{APP}/upload/queued.txt");
    let queued = hosted
        .put(&key(&queued_path), "later".into())
        .await
        .unwrap();
    let operation = offline_id(&queued);
    assert_eq!(
        queued_request(&scope, &operation).expected,
        OfflineExpected::FileAbsent
    );
    assert!(read(&cloud.inner, &queued_path).await.is_err());
    assert_eq!(read(hosted.as_ref(), &queued_path).await.unwrap(), b"later");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !scope.connectivity().is_offline(),
        "a reachable hub keeps the breaker closed"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_probe_timeout_serves_cache_hits_and_keeps_waiting_on_misses() {
    let fixture = Fixture::new().await;
    let scope = fixture.supported(ALICE).await;
    let cloud = Arc::new(TestCloud::default());
    let cached = format!("apps/{APP}/upload/cached.txt");
    let uncached = format!("apps/{APP}/upload/uncached.txt");
    cloud
        .inner
        .put(&key(&cached), "cloud copy".into())
        .await
        .unwrap();
    cloud
        .inner
        .put(&key(&uncached), "cloud only".into())
        .await
        .unwrap();
    write_local(&fixture.dirs().project, &cached, b"local copy");
    *cloud.slow_gets.lock().unwrap() = Duration::from_millis(800);
    let mut view = DesktopCloudView::new(
        Some(cloud.clone()),
        scope.cache_view().unwrap(),
        scope.connectivity(),
    );
    view.probe = Duration::from_millis(50);

    let started = Instant::now();
    assert_eq!(read(&view, &cached).await.unwrap(), b"local copy");
    assert!(
        started.elapsed() < Duration::from_millis(700),
        "a cache hit is served at the probe"
    );

    let started = Instant::now();
    assert_eq!(read(&view, &uncached).await.unwrap(), b"cloud only");
    assert!(
        started.elapsed() >= Duration::from_millis(800),
        "a miss waits for the cloud"
    );
    assert!(matches!(
        scope.index().unwrap().lookup(&key(&uncached)).unwrap(),
        Known::Present(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn breaker_opens_only_after_a_failed_reachability_check() {
    let fixture = Fixture::new().await;
    let reachable = ScopeConnectivity::new(&fixture.hub_key());
    reachable.observe(Observation::ConnectFailed);
    reachable.observe(Observation::TimedOut);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !reachable.is_offline(),
        "a hub that answers keeps the breaker closed"
    );

    let unreachable = ScopeConnectivity::new("http://127.0.0.1:9");
    let reconnected = Arc::new(AtomicUsize::new(0));
    let counter = reconnected.clone();
    unreachable.on_reconnect(Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    }));
    assert!(!unreachable.is_offline());
    unreachable.observe(Observation::TimedOut);
    eventually("the breaker to open", || async { unreachable.is_offline() }).await;
    unreachable.observe(Observation::Succeeded);
    assert!(!unreachable.is_offline());
    assert_eq!(reconnected.load(Ordering::SeqCst), 1);
}

#[test]
fn status_sync_state_derivation() {
    use commands::OfflineSyncState::*;
    assert_eq!(sync_state(1, TransportStatus::Idle, 3), Blocked);
    assert_eq!(sync_state(2, TransportStatus::WaitingForSignIn, 3), Blocked);
    assert_eq!(
        sync_state(0, TransportStatus::WaitingForSignIn, 3),
        WaitingForSignIn
    );
    assert_eq!(
        sync_state(0, TransportStatus::WaitingForConnection, 3),
        WaitingForConnection
    );
    assert_eq!(sync_state(0, TransportStatus::HubError, 3), HubError);
    assert_eq!(sync_state(0, TransportStatus::Idle, 3), Syncing);
    assert_eq!(sync_state(0, TransportStatus::HubError, 0), Idle);
    assert_eq!(sync_state(0, TransportStatus::Idle, 0), Idle);
}

fn probe_request() -> OfflineReplayRequest {
    let bytes = b"x";
    OfflineReplayRequest {
        operation_id: uuid::Uuid::new_v4().to_string(),
        resource: OfflineResource::File {
            purpose: StoragePurpose::Files,
            path: "a.txt".into(),
        },
        expected: OfflineExpected::FileAbsent,
        mutation: OfflineMutation::FilePut {
            data_base64: STANDARD.encode(bytes),
            sha256: {
                use sha2::Digest;
                format!("{:x}", sha2::Sha256::digest(bytes))
            },
        },
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_status_mapping_matches_contract() {
    let fixture = Fixture::new().await;
    let scope = fixture.supported(ALICE).await;
    let host = scope.host().unwrap();
    let request = probe_request();
    let envelope = |code: &str, message: &str| json!({"error": {"code": code, "message": message}});
    let applied = json!({
        "operation_id": request.operation_id,
        "digest": request.digest().unwrap(),
        "status": "applied",
        "result": {"kind": "file_revision", "e_tag": "e", "version": null},
        "message": null
    });
    fixture.hub.force(200, applied);
    let response = host.replay(&request).await.unwrap();
    assert_eq!(response.status, OfflineReplayStatus::Applied);
    assert_eq!(scope.transport(), TransportStatus::Idle);
    let received = fixture.hub.received().pop().unwrap();
    assert_eq!(
        received.authorization.as_deref(),
        Some(format!("Bearer {}", jwt(ALICE)).as_str())
    );
    assert_eq!(received.subject.as_deref(), Some(ALICE));
    assert_eq!(
        received.installation,
        Some(fixture.registry.installation_id().unwrap())
    );
    assert_eq!(received.request.as_ref(), Some(&request));

    let long = "m".repeat(3000);
    let cases: Vec<(
        u16,
        Value,
        ReplayErrorKind,
        Option<&str>,
        String,
        TransportStatus,
    )> = vec![
        (
            400,
            envelope(
                "OFFLINE_INVALID",
                "Offline installation identifier is missing or invalid",
            ),
            ReplayErrorKind::NotClaimed,
            Some("invalid"),
            "Offline installation identifier is missing or invalid".into(),
            TransportStatus::Idle,
        ),
        (
            422,
            envelope("BAD", &long),
            ReplayErrorKind::NotClaimed,
            Some("invalid"),
            "m".repeat(2048),
            TransportStatus::Idle,
        ),
        (
            401,
            envelope("UNAUTHORIZED", "expired"),
            ReplayErrorKind::Unavailable,
            None,
            texts::sign_in_to_sync(ALICE),
            TransportStatus::WaitingForSignIn,
        ),
        (
            403,
            envelope("OFFLINE_FORBIDDEN", "no"),
            ReplayErrorKind::NotClaimed,
            Some("forbidden"),
            texts::FORBIDDEN.into(),
            TransportStatus::Idle,
        ),
        (
            404,
            json!({}),
            ReplayErrorKind::NotClaimed,
            Some("endpoint_missing"),
            texts::ENDPOINT_MISSING.into(),
            TransportStatus::Idle,
        ),
        (
            405,
            json!({}),
            ReplayErrorKind::NotClaimed,
            Some("endpoint_missing"),
            texts::ENDPOINT_MISSING.into(),
            TransportStatus::Idle,
        ),
        (
            409,
            envelope(
                "OFFLINE_SUBJECT_MISMATCH",
                "These changes were queued for another account",
            ),
            ReplayErrorKind::NotClaimed,
            Some("subject_mismatch"),
            texts::subject_mismatch(ALICE),
            TransportStatus::Idle,
        ),
        (
            409,
            envelope("OFFLINE_DIGEST_REUSED", "Operation ID reused"),
            ReplayErrorKind::Rejected,
            Some("digest_reused"),
            "Operation ID reused".into(),
            TransportStatus::Idle,
        ),
        (
            409,
            envelope("DATABASE_CONFLICT", "retry"),
            ReplayErrorKind::Unavailable,
            None,
            "retry".into(),
            TransportStatus::HubError,
        ),
        (
            413,
            envelope("OFFLINE_LIMIT_EXCEEDED", "too big"),
            ReplayErrorKind::NotClaimed,
            Some("hub_limit"),
            texts::hub_limit("5.0 MB"),
            TransportStatus::Idle,
        ),
        (
            423,
            envelope("LOCKED", "locked"),
            ReplayErrorKind::Unavailable,
            None,
            "locked".into(),
            TransportStatus::HubError,
        ),
        (
            429,
            envelope("RATE", "slow down"),
            ReplayErrorKind::Unavailable,
            None,
            "slow down".into(),
            TransportStatus::HubError,
        ),
        (
            418,
            envelope("TEAPOT", "I refuse"),
            ReplayErrorKind::Rejected,
            None,
            "I refuse".into(),
            TransportStatus::Idle,
        ),
        (
            500,
            envelope("DATABASE_COMMIT_UNKNOWN", "unknown"),
            ReplayErrorKind::Unavailable,
            None,
            "unknown".into(),
            TransportStatus::HubError,
        ),
        (
            503,
            json!({}),
            ReplayErrorKind::Unavailable,
            None,
            String::new(),
            TransportStatus::HubError,
        ),
    ];
    for (status, body, kind, code, message, transport) in cases {
        fixture.hub.force(status, body);
        let error = host.replay(&request).await.unwrap_err();
        assert_eq!(error.kind, kind, "HTTP {status}");
        assert_eq!(error.code.as_deref(), code, "HTTP {status}");
        if !message.is_empty() {
            assert_eq!(error.message, message, "HTTP {status}");
        }
        assert_eq!(scope.transport(), transport, "HTTP {status}");
    }

    let (answer, _) = map_answer(413, b"", ALICE, None);
    assert_eq!(answer.unwrap_err().message, texts::hub_limit("its limit"));

    *fixture.tokens.session.lock().unwrap() = None;
    let error = host.replay(&request).await.unwrap_err();
    assert_eq!(error.kind, ReplayErrorKind::Unavailable);
    assert_eq!(error.message, texts::sign_in_to_sync(ALICE));
    assert_eq!(scope.transport(), TransportStatus::WaitingForSignIn);

    *fixture.tokens.session.lock().unwrap() = Some(jwt(ALICE));
    let offline = fixture
        .registry
        .obtain(
            None,
            fixture.dirs(),
            ScopeKey {
                hub: "http://127.0.0.1:9".into(),
                subject: ALICE.into(),
                app_id: APP.into(),
            },
        )
        .unwrap();
    let error = offline.host().unwrap().replay(&request).await.unwrap_err();
    assert_eq!(error.kind, ReplayErrorKind::Unavailable);
    assert_eq!(offline.transport(), TransportStatus::WaitingForConnection);
}

#[tokio::test]
async fn token_resolution_prefers_session_then_bound_sink_pat_with_matching_digest() {
    let fixture = Fixture::new().await;
    *fixture.tokens.session.lock().unwrap() = None;
    let scope = fixture.scope(ALICE);
    let host = scope.host().unwrap();
    assert_eq!(host.token(), None);
    fixture
        .tokens
        .sinks
        .lock()
        .unwrap()
        .insert("evt-1".into(), "pat_one".into());
    assert_eq!(
        host.token(),
        None,
        "a sink must be bound to the scope first"
    );
    scope.record_sink("evt-1", "pat_one").unwrap();
    assert_eq!(host.token().as_deref(), Some("pat_one"));
    fixture
        .tokens
        .sinks
        .lock()
        .unwrap()
        .insert("evt-1".into(), "pat_rotated".into());
    assert_eq!(
        host.token(),
        None,
        "a sink token whose digest changed is not used"
    );
    *fixture.tokens.session.lock().unwrap() = Some(jwt(ALICE));
    assert_eq!(host.token(), Some(jwt(ALICE)));

    for event in 0..40 {
        scope
            .record_sink(&format!("evt-{event}"), "pat_many")
            .unwrap();
    }
    let sinks = scope.descriptor().sinks;
    assert_eq!(sinks.len(), 32);
    assert_eq!(sinks.last().unwrap().event_id, "evt-39");
    assert_eq!(sinks.first().unwrap().event_id, "evt-8");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pat_subject_comes_from_the_confirmed_authority() {
    assert_eq!(
        disconnected_subject("pat_x", |_| Some("owner".into())).as_deref(),
        Some("owner")
    );
    assert_eq!(disconnected_subject("pat_x", |_| None), None);
    assert_eq!(
        disconnected_subject(&jwt(ALICE), |_| panic!("JWTs name their subject")).as_deref(),
        Some(ALICE)
    );
    let encoded = aws(&format!("users/auth0%7C123/apps/{APP}"));
    assert_eq!(
        hosted_subject("pat_x", &encoded, APP).as_deref(),
        Some("auth0|123")
    );
    assert_eq!(hosted_subject("pat_x", &encoded, "other"), None);
    assert_eq!(
        hosted_subject(&jwt(BOB), &encoded, APP).as_deref(),
        Some(BOB)
    );

    let fixture = Fixture::new().await;
    let unbound = RunStorage::from_prepared(
        Some(&fixture.registry),
        None,
        fixture.dirs(),
        request("pat_unknown", &fixture.hub.url),
        Err(unreachable()),
        |_| None,
    )
    .await
    .unwrap();
    assert_eq!(unbound.mode(), RunStorageMode::Disconnected);
    assert!(unbound.scope().is_none());
    assert_eq!(unbound.unbound_reason(), Some(texts::UNATTRIBUTED));
    let config = unbound
        .disconnected_config(FlowLikeConfig::default())
        .unwrap();
    let decorate = config.callbacks.decorate_database.clone().unwrap();
    let (_db_dir, connection) = local_connection().await;
    let error = refused(decorate(project_db(), opened(&connection, "orders")).await);
    assert_eq!(error, texts::UNATTRIBUTED);
    let files = config
        .stores
        .app_storage_store
        .clone()
        .unwrap()
        .as_generic();
    let error = files
        .put(&key(&format!("apps/{APP}/upload/a.txt")), "x".into())
        .await
        .unwrap_err();
    assert!(text(&error).contains(texts::UNATTRIBUTED), "{error}");

    let bound = RunStorage::from_prepared(
        Some(&fixture.registry),
        None,
        fixture.dirs(),
        request("pat_known", &fixture.hub.url),
        Err(unreachable()),
        |_| Some(ALICE.into()),
    )
    .await
    .unwrap();
    assert_eq!(bound.scope().unwrap().subject(), ALICE);
}

#[tokio::test]
async fn lease_is_held_weakly() {
    let fixture = Fixture::new().await;
    let scope = fixture.scope(ALICE);
    let live = lease(ALICE).await;
    scope.offer_lease(Arc::downgrade(&live));
    assert!(scope.lease().is_some_and(|held| Arc::ptr_eq(&held, &live)));
    assert_eq!(scope.descriptor().provider.as_deref(), Some("s3"));
    drop(live);
    assert!(
        scope.lease().is_none(),
        "the scope never keeps a lease alive"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_tickets_delay_activation_until_older_runs_end() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    let earlier = scope.hosted_run_started();
    let configured = scope
        .begin_table(&selection("orders", false))
        .await
        .unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(configured.clone()));
    eventually("the table to settle", || async {
        status_of(&scope, "orders") == OfflineTableStatus::Settling
    })
    .await;
    let overview = commands::overview(&fixture.registry, &scope).await.unwrap();
    assert_eq!(overview.tables[0].waiting_for_runs, 1);

    let later = scope.hosted_run_started();
    let (_db_dir, connection) = local_connection().await;
    let path = scope.database_path(OfflineTablePurpose::Storage);
    let error = refused(
        scope
            .decorate(&path, opened(&connection, "orders"), RunMode::Hosted)
            .await,
    );
    assert!(
        error.contains("is being prepared for offline use on this device"),
        "{error}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!setup.is_finished(), "activation waits for the older run");
    assert_eq!(status_of(&scope, "orders"), OfflineTableStatus::Settling);

    drop(earlier);
    setup.await.unwrap();
    assert_eq!(status_of(&scope, "orders"), OfflineTableStatus::Ready);
    let manager = scope.open_manager().unwrap();
    assert_eq!(
        manager.managed_table_names(&path),
        vec!["orders".to_owned()]
    );
    let store = scope
        .decorate(&path, opened(&connection, "orders"), RunMode::Hosted)
        .await
        .unwrap();
    assert!(store.is_durably_managed());
    drop(later);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn manager_failure_fails_configured_tables_with_e7_in_hosted_runs() {
    let fixture = Fixture::new().await;
    let scope = fixture.scope(ALICE);
    scope
        .update_descriptor(|descriptor| {
            descriptor.hub_support = Some(true);
            descriptor.tables.push(ConfiguredTable {
                purpose: OfflineTablePurpose::Storage,
                database: "db".into(),
                table: "orders".into(),
                primary_key: "id".into(),
                prefetch: false,
                status: OfflineTableStatus::Ready,
                error: None,
                enabled_at: 0,
            });
        })
        .unwrap();
    std::fs::create_dir_all(scope.engine_root().parent().unwrap()).unwrap();
    std::fs::write(scope.engine_root(), b"not a directory").unwrap();
    assert!(scope.manager().await.is_err());
    let failure = scope.manager_error().unwrap();

    let path = scope.database_path(OfflineTablePurpose::Storage);
    assert!(scope.is_managed(&path, "orders"));
    assert!(!scope.is_managed(&path, "customers"));
    let (_db_dir, connection) = local_connection().await;
    for mode in [RunMode::Hosted, RunMode::Disconnected] {
        let error = refused(
            scope
                .decorate(&path, opened(&connection, "orders"), mode)
                .await,
        );
        assert_eq!(error, texts::unavailable(&failure));
    }
    let unmanaged = scope
        .decorate(&path, opened(&connection, "customers"), RunMode::Hosted)
        .await
        .unwrap();
    assert!(
        !unmanaged.is_durably_managed(),
        "unconfigured tables keep the cloud in Hosted runs"
    );
    let error = scope
        .disconnected_store()
        .unwrap()
        .put(&key(&format!("apps/{APP}/upload/a.txt")), "x".into())
        .await
        .unwrap_err();
    assert!(
        text(&error).contains("Offline changes are unavailable on this device"),
        "{error}"
    );
    let hosted = scope.decorate_content(FlowLikeStore::Memory(Arc::new(InMemory::new())));
    assert!(
        matches!(hosted, FlowLikeStore::Memory(_)),
        "Hosted files stay unbuffered"
    );
    let Err(error) = data_studio::managed(&scope, "orders", false, None).await else {
        panic!("Data Studio refuses a table whose offline changes cannot open");
    };
    assert!(
        text(&error).starts_with("Offline changes are unavailable on this device"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_studio_routes_offline_apps_to_device_and_online_tables_to_managed_or_errors() {
    assert!(matches!(
        data_studio::route(&AppVisibility::Offline, Some("orders"), None).unwrap(),
        Route::Device
    ));
    for token in [None, Some("not-a-jwt")] {
        let Err(error) = data_studio::route(&AppVisibility::Private, Some("orders"), token) else {
            panic!("an online app needs a signed-in account");
        };
        assert_eq!(text(error), texts::SIGN_IN_DATA);
    }
    let Err(error) = data_studio::route(&AppVisibility::Public, None, Some(&jwt(ALICE))) else {
        panic!("table-less commands of online apps use the hub");
    };
    assert_eq!(text(error), texts::HUB_ONLY_TABLES);

    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "notes", rows(0..2))
        .await;
    let scope = fixture.supported(ALICE).await;
    let expect_error = |target: flow_like_types::Result<DataStudioTarget>| match target {
        Ok(_) => panic!("Data Studio must refuse this table"),
        Err(error) => text(error),
    };
    assert_eq!(
        expect_error(data_studio::managed(&scope, "orders", false, None).await),
        texts::not_configured("orders")
    );

    scope
        .begin_table(&selection("drafts", false))
        .await
        .unwrap();
    assert_eq!(
        expect_error(data_studio::managed(&scope, "drafts", false, None).await),
        texts::table_not_ready("drafts")
    );

    let ticket = scope.hosted_run_started();
    let notes = scope.begin_table(&selection("notes", false)).await.unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(notes));
    eventually("notes to settle", || async {
        status_of(&scope, "notes") == OfflineTableStatus::Settling
    })
    .await;
    assert!(
        expect_error(data_studio::managed(&scope, "notes", false, None).await)
            .contains("is being prepared for offline use")
    );
    drop(ticket);
    setup.await.unwrap();

    fixture.enable(&scope, "orders", false).await;
    let DataStudioTarget::Managed(store) = data_studio::managed(&scope, "orders", false, None)
        .await
        .unwrap()
    else {
        panic!("an active table is managed");
    };
    assert!(store.is_durably_managed());
    assert_eq!(store.count(None).await.unwrap(), 3);
    assert_eq!(
        expect_error(data_studio::managed(&scope, "orders", true, None).await),
        texts::not_configured("orders")
    );
    let structural = store.optimize(true).await.unwrap_err();
    assert!(text(structural).contains("Offline-buffered tables do not support"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn data_studio_uses_the_token_subject_not_the_registry_subject() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let alice = fixture.supported(ALICE).await;
    fixture.enable(&alice, "orders", false).await;

    let Route::Account { subject, table } =
        data_studio::route(&AppVisibility::Private, Some("orders"), Some(&jwt(BOB))).unwrap()
    else {
        panic!("an online app routes by account");
    };
    assert_eq!((subject.as_str(), table), (BOB, "orders"));
    let bob = fixture.scope(&subject);
    let Err(error) = data_studio::managed(&bob, table, false, None).await else {
        panic!("another account's configuration never applies");
    };
    assert_eq!(text(error), texts::not_configured("orders"));
    assert!(matches!(
        data_studio::managed(&alice, "orders", false, None)
            .await
            .unwrap(),
        DataStudioTarget::Managed(_)
    ));
    assert_eq!(
        commands::table_route(&alice, "orders", false),
        commands::OfflineTableRoute::Device
    );
    assert_eq!(
        commands::table_route(&bob, "orders", false),
        commands::OfflineTableRoute::Hub
    );
    assert_eq!(
        commands::table_route(&alice, "orders", true),
        commands::OfflineTableRoute::Hub
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn file_discarded_removes_queued_cache_copies_only() {
    let fixture = Fixture::new().await;
    let scope = fixture.supported(ALICE).await;
    let host = scope.host().unwrap();
    let index = scope.index().unwrap();
    let project = fixture.dirs().project;
    let queued = format!("apps/{APP}/upload/a.txt");
    let tagged = format!("apps/{APP}/upload/b.txt");
    write_local(&project, &queued, b"queued");
    write_local(
        &project,
        sidecar(&key(&queued)).unwrap().as_ref(),
        b"offline-op-1",
    );
    write_local(&project, &tagged, b"cloud");
    write_local(
        &project,
        sidecar(&key(&tagged)).unwrap().as_ref(),
        b"real-etag",
    );
    index.mark_queued(&key(&queued), "op-1").unwrap();
    index.mark_queued(&key(&tagged), "op-3").unwrap();

    host.file_discarded(&key(&queued), "op-2").unwrap();
    assert!(
        project.join(&queued).exists(),
        "another operation's copy stays"
    );
    host.file_discarded(&key(&queued), "op-1").unwrap();
    assert!(!project.join(&queued).exists());
    assert!(
        !project
            .join(sidecar(&key(&queued)).unwrap().as_ref())
            .exists()
    );
    assert_eq!(index.lookup(&key(&queued)).unwrap(), Known::Unknown);

    host.file_discarded(&key(&tagged), "op-3").unwrap();
    assert!(!project.join(&tagged).exists());
    assert!(
        project
            .join(sidecar(&key(&tagged)).unwrap().as_ref())
            .exists(),
        "a real ETag stays"
    );

    let acknowledged = format!("apps/{APP}/upload/c.txt");
    index.mark_queued(&key(&acknowledged), "op-4").unwrap();
    host.file_acknowledged(
        &key(&acknowledged),
        "op-4",
        b"synced",
        &OfflineExpected::FileRevision {
            e_tag: Some("etag-4".into()),
            version: None,
        },
    )
    .unwrap();
    assert_eq!(
        std::fs::read(project.join(&acknowledged)).unwrap(),
        b"synced"
    );
    assert_eq!(
        std::fs::read_to_string(project.join(sidecar(&key(&acknowledged)).unwrap().as_ref()))
            .unwrap(),
        "etag-4"
    );
    let Known::Present(recorded) = index.lookup(&key(&acknowledged)).unwrap() else {
        panic!("an acknowledged file is present");
    };
    assert_eq!(recorded.e_tag.as_deref(), Some("etag-4"));
    assert!(
        !index.forget_queued(&key(&acknowledged), "op-4").unwrap(),
        "the mark is cleared"
    );
    host.file_deleted(&key(&acknowledged), "op-5").unwrap();
    assert!(!project.join(&acknowledged).exists());
    assert_eq!(index.lookup(&key(&acknowledged)).unwrap(), Known::Absent);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_both_uploads_a_copy_and_skips_the_conflict() {
    let fixture = Fixture::new().await;
    let scope = fixture.supported(ALICE).await;
    let manager = scope.manager().await.unwrap();
    let path = format!("apps/{APP}/upload/report.txt");
    write_local(fixture.cloud_dir.path(), &path, b"cloud version");
    let queued = scope
        .disconnected_store()
        .unwrap()
        .put(&key(&path), "device version".into())
        .await
        .unwrap();
    let operation = offline_id(&queued);
    eventually("the conflict", || async {
        manager
            .queue()
            .operation_state(&operation)
            .unwrap()
            .unwrap()
            .state
            == "conflict"
    })
    .await;
    let new_path = scope.keep_both(&operation).await.unwrap();
    assert!(
        new_path.starts_with("report (offline copy ") && new_path.ends_with(").txt"),
        "{new_path}"
    );
    let cloud = fixture
        .cloud
        .objects(&scope.host().unwrap())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        read(cloud.as_ref(), &format!("apps/{APP}/upload/{new_path}"))
            .await
            .unwrap(),
        b"device version"
    );
    eventually("the skip", || async {
        manager
            .queue()
            .operation_state(&operation)
            .unwrap()
            .unwrap()
            .state
            == "skipped"
    })
    .await;
    assert_eq!(read(cloud.as_ref(), &path).await.unwrap(), b"cloud version");
    let reason = manager
        .queue()
        .operation_state(&operation)
        .unwrap()
        .unwrap()
        .error
        .unwrap();
    assert_eq!(reason, format!("desktop:{ALICE}: kept as {new_path}"));

    let now = chrono::Utc::now();
    let first = keep_both_name("dir/a.tar.gz", now, |_| false);
    assert!(first.starts_with("dir/a.tar (offline copy ") && first.ends_with(").gz"));
    let taken = first.clone();
    let second = keep_both_name("dir/a.tar.gz", now, |candidate| candidate == taken);
    assert!(second.ends_with(" 2).gz"), "{second}");
    assert!(keep_both_name("README", now, |_| false).starts_with("README (offline copy "));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forget_app_closes_and_removes_scopes_with_tombstone_fallback() {
    let fixture = Fixture::new().await;
    let alice = queuing_scope(&fixture, ALICE).await;
    let bob = queuing_scope(&fixture, BOB).await;
    let root = fixture.registry.root().to_path_buf();
    let alice_engine = alice.engine_root();
    let bob_engine = bob.engine_root();
    assert!(alice_engine.exists() && bob_engine.exists());

    fixture
        .registry
        .forget(APP, Some(&fixture.key(ALICE)), false)
        .await
        .unwrap();
    assert!(!alice_engine.exists());
    assert!(
        !root
            .join("scopes")
            .join(format!("{}.json", alice.id()))
            .exists()
    );
    assert!(!root.join("scopes").join(alice.id()).exists());
    assert!(bob_engine.exists());
    assert!(
        alice.manager().await.is_err(),
        "a forgotten scope is closed"
    );
    assert!(fixture.registry.existing(&fixture.key(ALICE)).is_none());

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let app_engines = bob_engine.parent().unwrap().to_path_buf();
        std::fs::set_permissions(&app_engines, std::fs::Permissions::from_mode(0o500)).unwrap();
        fixture.registry.forget(APP, None, true).await.unwrap();
        std::fs::set_permissions(&app_engines, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(bob_engine.exists(), "the locked engine directory survives");
        assert!(
            root.join("scopes")
                .join(bob.id())
                .join("forgotten")
                .exists()
        );
        assert!(
            read_descriptors(&root).is_empty(),
            "tombstoned scopes are never listed"
        );
        fixture.registry.delete_tombstoned();
        assert!(!bob_engine.exists());
        assert!(!root.join("scopes").join(bob.id()).exists());
    }
}

fn file_snapshot(directory: &std::path::Path) -> Vec<(String, u64, SystemTime)> {
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .flatten()
        .map(|entry| {
            let metadata = entry.metadata().unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                metadata.len(),
                metadata.modified().unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn other_accounts_pending_is_read_only() {
    let fixture = Fixture::new().await;
    let alice = queuing_scope(&fixture, ALICE).await;
    let files = alice.disconnected_store().unwrap();
    for name in ["a.txt", "b.txt"] {
        files
            .put(&key(&format!("apps/{APP}/upload/{name}")), "x".into())
            .await
            .unwrap();
    }
    alice.close().await;
    let bob = fixture.supported(BOB).await;
    let before = file_snapshot(&alice.engine_root());
    assert_eq!(fixture.registry.other_accounts_pending(APP, bob.id()), 2);
    assert_eq!(
        file_snapshot(&alice.engine_root()),
        before,
        "reading never writes"
    );
    assert_eq!(pending_count(&bob.engine_root()), 0);
    assert!(!bob.engine_root().exists(), "reading creates nothing");
    assert_eq!(fixture.registry.other_accounts_pending(APP, alice.id()), 0);
    assert_eq!(fixture.registry.other_accounts_pending("app2", bob.id()), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn remote_objects_is_the_undecorated_content_store() {
    let fixture = Fixture::new().await;
    let registry = OfflineWrites::with_services(
        fixture.root.path().join("lease-scopes"),
        Services {
            cloud: Arc::new(LeaseCloud),
            ..fixture.services()
        },
    );
    let scope = registry
        .obtain(None, fixture.dirs(), fixture.key(ALICE))
        .unwrap();
    scope
        .update_descriptor(|descriptor| descriptor.hub_support = Some(true))
        .unwrap();
    scope.manager().await.unwrap();
    *fixture.tokens.session.lock().unwrap() = None;
    let orders = ConfiguredTable::from_selection(&selection("orders", false))
        .unwrap()
        .selection();
    let Err(error) = scope.host().unwrap().remote_table(&orders).await else {
        panic!("without a lease and a token the cloud table is out of reach");
    };
    assert_eq!(text(error), texts::NO_TABLE_ACCESS);
    let Err(error) = scope.host().unwrap().remote_objects().await else {
        panic!("without a lease and a token the cloud files are out of reach");
    };
    assert_eq!(text(error), texts::NO_DATA_ACCESS);

    let live = lease(ALICE).await;
    scope.offer_lease(Arc::downgrade(&live));
    assert!(live.install_content_decorator(scope.content_decorator()));
    let decorated = live.to_store_type(StoreType::Content).await.unwrap();
    assert_eq!(decorated.as_generic().to_string(), "DesktopFileStore");
    let objects = scope.host().unwrap().remote_objects().await.unwrap();
    assert_ne!(objects.to_string(), "DesktopFileStore");
    assert!(objects.to_string().contains("AmazonS3"), "{objects}");
}

async fn prefetch_limit(fixture: &Fixture, table: &str) -> u64 {
    64 * MIB + cloud_bytes_of(fixture, table).await * 3 / 2
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn set_prefetch_command_persists_the_choice_and_keeps_false_on_e31() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", bulky_rows(3000))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    let limit = prefetch_limit(&fixture, "orders").await;
    scope
        .set_limits(OfflineLimitsDto {
            max_mirror_bytes: limit,
            ..OfflineLimitsDto::default()
        })
        .await
        .unwrap();
    let refused = scope
        .set_prefetch(OfflineTablePurpose::Storage, "orders", true)
        .await;
    let Err(refused) = refused else {
        panic!("Download everything of a table larger than the limit allows is refused with E31");
    };
    assert!(
        text(&refused).starts_with("Downloading all of table 'orders' needs"),
        "{refused}"
    );
    assert!(
        !scope
            .configured(OfflineTablePurpose::Storage, "orders")
            .unwrap()
            .prefetch
    );
    let manager = scope.open_manager().unwrap();
    assert!(
        manager
            .table_states()
            .await
            .unwrap()
            .iter()
            .all(|state| !state.prefetch)
    );

    scope
        .set_limits(OfflineLimitsDto {
            max_mirror_bytes: 4 * 1024 * MIB,
            ..OfflineLimitsDto::default()
        })
        .await
        .unwrap();
    let state = scope
        .set_prefetch(OfflineTablePurpose::Storage, "orders", true)
        .await
        .unwrap();
    assert!(state.prefetch);
    assert!(
        scope
            .configured(OfflineTablePurpose::Storage, "orders")
            .unwrap()
            .prefetch
    );
    assert!(
        manager
            .table_states()
            .await
            .unwrap()
            .iter()
            .all(|state| state.prefetch)
    );

    scope
        .begin_table(&selection("drafts", false))
        .await
        .unwrap();
    let pending = scope
        .set_prefetch(OfflineTablePurpose::Storage, "drafts", true)
        .await
        .unwrap();
    assert!(pending.prefetch);
    assert_eq!(pending.status, OfflineTableStatus::Preparing);
    assert!(fixture.event_count(super::TABLES_EVENT) > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_prefetch_flag_repairs_the_descriptor_at_startup() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    scope
        .set_prefetch(OfflineTablePurpose::Storage, "orders", true)
        .await
        .unwrap();
    scope
        .update_descriptor(|descriptor| descriptor.tables[0].prefetch = false)
        .unwrap();
    scope.close().await;

    let restarted = fixture.new_registry();
    let reopened = restarted
        .obtain(None, fixture.dirs(), fixture.key(ALICE))
        .unwrap();
    assert!(
        !reopened
            .configured(OfflineTablePurpose::Storage, "orders")
            .unwrap()
            .prefetch
    );
    reopened.manager().await.unwrap();
    let table = reopened
        .configured(OfflineTablePurpose::Storage, "orders")
        .unwrap();
    assert!(table.prefetch, "the engine's flag is authoritative");
    assert_eq!(table.status, OfflineTableStatus::Ready);
    let manager = reopened.open_manager().unwrap();
    assert_eq!(
        manager.managed_table_names(&reopened.database_path(OfflineTablePurpose::Storage)),
        vec!["orders".to_owned()]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn enable_with_prefetch_over_the_limit_ends_in_error_without_registration() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", bulky_rows(3000))
        .await;
    let scope = fixture.supported(ALICE).await;
    let limit = prefetch_limit(&fixture, "orders").await;
    scope
        .set_limits(OfflineLimitsDto {
            max_mirror_bytes: limit,
            ..OfflineLimitsDto::default()
        })
        .await
        .unwrap();
    let table = fixture.enable(&scope, "orders", true).await;
    assert_eq!(table.status, OfflineTableStatus::Error, "{table:?}");
    assert!(
        table
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("Downloading all of table 'orders' needs")),
        "{table:?}"
    );
    let manager = scope.open_manager().unwrap();
    assert!(
        !manager.table_is_managed(&scope.database_path(OfflineTablePurpose::Storage), "orders")
    );
    assert!(
        manager
            .queue()
            .resource_revision(&table.resource())
            .unwrap()
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_runs_open_the_cloud_table_while_its_key_is_validated() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    scope.manager().await.unwrap();
    let paused = fixture.cloud.gate.write().await;
    let configured = scope
        .begin_table(&selection("orders", false))
        .await
        .unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(configured));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(status_of(&scope, "orders"), OfflineTableStatus::Preparing);

    let (_db_dir, connection) = local_connection().await;
    let path = scope.database_path(OfflineTablePurpose::Storage);
    assert!(!scope.is_managed(&path, "orders"));
    let hosted = scope
        .decorate(&path, opened(&connection, "orders"), RunMode::Hosted)
        .await
        .unwrap();
    assert!(
        !hosted.is_durably_managed(),
        "Hosted runs keep the cloud table during validation"
    );
    let error = refused(
        scope
            .decorate(&path, opened(&connection, "orders"), RunMode::Disconnected)
            .await,
    );
    assert_eq!(error, texts::table_not_ready("orders"));

    drop(paused);
    setup.await.unwrap();
    assert_eq!(status_of(&scope, "orders"), OfflineTableStatus::Ready);
    let managed = scope
        .decorate(&path, opened(&connection, "orders"), RunMode::Hosted)
        .await
        .unwrap();
    assert!(managed.is_durably_managed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_activation_stays_settling_with_a_table_mirror_error_and_retries() {
    let fixture = Fixture::with_retry(Duration::ZERO).await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    let ticket = scope.hosted_run_started();
    let configured = scope
        .begin_table(&selection("orders", false))
        .await
        .unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(configured));
    eventually("the table to settle", || async {
        status_of(&scope, "orders") == OfflineTableStatus::Settling
    })
    .await;
    fixture.cloud.set_online(false);
    drop(ticket);
    setup.await.unwrap();
    assert_eq!(status_of(&scope, "orders"), OfflineTableStatus::Settling);
    let overview = commands::overview(&fixture.registry, &scope).await.unwrap();
    let row = &overview.tables[0];
    assert!(
        row.mirror_error
            .as_deref()
            .is_some_and(|error| error.starts_with("The cloud table could not be read:")),
        "{row:?}"
    );
    assert_eq!(row.waiting_for_runs, 0);

    fixture.cloud.set_online(true);
    scope.retry_activations();
    eventually("the retried activation", || async {
        status_of(&scope, "orders") == OfflineTableStatus::Ready
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overview_reports_mirror_fields_errors_per_table_and_usage() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let cloud = fixture
        .seed(ALICE, StoragePurpose::Storage, "customers", rows(0..2))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    fixture.enable(&scope, "customers", false).await;
    let missing = fixture.enable(&scope, "missing", false).await;
    assert_eq!(
        missing.error.as_deref(),
        Some(texts::missing_in_cloud("missing").as_str())
    );

    cloud.drop_table("customers", &[]).await.unwrap();
    let manager = scope.open_manager().unwrap();
    let customers = scope
        .configured(OfflineTablePurpose::Storage, "customers")
        .unwrap();
    manager.refresh_table(&customers.selection()).await.unwrap();

    let overview = commands::overview(&fixture.registry, &scope).await.unwrap();
    assert!(overview.available);
    assert_eq!(overview.subject.as_deref(), Some(ALICE));
    assert_eq!(overview.provider.as_deref(), Some("s3"));
    assert_eq!(overview.hub_support, Some(true));
    assert_eq!(
        overview.hub_limits.unwrap().max_request_bytes,
        Some(5_000_000)
    );
    let engine = manager.table_states().await.unwrap();
    let row = |name: &str| {
        overview
            .tables
            .iter()
            .find(|row| row.table == name)
            .unwrap()
            .clone()
    };

    let orders = row("orders");
    let state = engine
        .iter()
        .find(|state| state.table.table == "orders")
        .unwrap();
    assert_eq!(orders.status, OfflineTableStatus::Ready);
    assert!(orders.snapshot_version.is_some() && orders.refreshed_at.is_some());
    assert_eq!(
        (
            orders.cached_bytes,
            orders.total_bytes,
            orders.local_bytes,
            orders.offline_complete,
            orders.downloading,
            orders.key_indexed
        ),
        (
            state.cached_bytes,
            state.total_bytes,
            state.local_bytes,
            state.offline_complete,
            state.downloading,
            state.key_indexed
        )
    );
    assert!(orders.local_bytes > 0);
    assert_eq!(orders.mirror_error, None);
    assert!(!orders.remote_missing);

    let customers = row("customers");
    assert!(customers.remote_missing);
    assert_eq!(customers.status, OfflineTableStatus::Error);
    assert_eq!(
        customers.error.as_deref(),
        Some(texts::deleted_in_cloud("customers").as_str())
    );
    assert_eq!(
        customers.mirror_error.as_deref(),
        Some(texts::deleted_in_cloud("customers").as_str())
    );
    assert_eq!(
        scope
            .configured(OfflineTablePurpose::Storage, "customers")
            .unwrap()
            .status,
        OfflineTableStatus::Error
    );

    let missing = row("missing");
    assert_eq!(missing.status, OfflineTableStatus::Error);
    assert_eq!(
        (
            missing.cached_bytes,
            missing.local_bytes,
            missing.mirror_error.clone()
        ),
        (0, 0, None)
    );

    let usage = manager.mirror_usage().unwrap();
    assert_eq!(overview.usage.required_bytes, usage.required);
    assert_eq!(overview.usage.pinned_bytes, usage.pinned_bytes);
    assert_eq!(overview.usage.cache_bytes, usage.cache_bytes);
    assert_eq!(
        overview.usage.max_download_bytes_per_day,
        Some(2 * overview.limits.max_mirror_bytes)
    );
    assert_eq!(overview.queue.sync_state, commands::OfflineSyncState::Idle);
    assert_eq!(overview.other_accounts_pending, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mirror_events_are_coalesced_per_scope_with_a_trailing_edge() {
    let emitted = Arc::new(AtomicUsize::new(0));
    let counter = emitted.clone();
    let coalescer = Coalescer::new(Duration::from_millis(500), move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    coalescer.fire();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        emitted.load(Ordering::SeqCst),
        1,
        "the first change is emitted at once"
    );
    for _ in 0..10 {
        coalescer.fire();
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        emitted.load(Ordering::SeqCst),
        1,
        "changes within the interval wait"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        emitted.load(Ordering::SeqCst),
        2,
        "the last change gets a trailing emission"
    );
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(emitted.load(Ordering::SeqCst), 2);
    coalescer.fire();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(emitted.load(Ordering::SeqCst), 3);

    let fixture = Fixture::new().await;
    let first = fixture.scope(ALICE);
    let second = fixture
        .registry
        .obtain(
            None,
            fixture.dirs(),
            ScopeKey {
                app_id: "app2".into(),
                ..fixture.key(ALICE)
            },
        )
        .unwrap();
    first.host().unwrap().mirror_changed();
    second.host().unwrap().mirror_changed();
    tokio::time::sleep(Duration::from_millis(150)).await;
    for _ in 0..5 {
        first.host().unwrap().mirror_changed();
    }
    tokio::time::sleep(Duration::from_millis(800)).await;
    let apps: Vec<String> = fixture
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|(event, _)| event == super::MIRROR_EVENT)
        .map(|(_, payload)| payload["appId"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(apps.iter().filter(|app| *app == APP).count(), 2);
    assert_eq!(apps.iter().filter(|app| *app == "app2").count(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sync_now_returns_before_refreshes_finish() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    let before = fixture.event_count(super::MIRROR_EVENT);
    *fixture.cloud.delay.lock().unwrap() = Duration::from_secs(2);
    let started = Instant::now();
    scope.sync_now().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "Sync now never waits for refreshes"
    );
    eventually("the background refresh", || async {
        fixture.event_count(super::MIRROR_EVENT) > before
    })
    .await;
    assert!(started.elapsed() >= Duration::from_secs(2));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn limits_below_required_are_refused() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    let error = scope
        .set_limits(OfflineLimitsDto {
            max_mirror_bytes: 2 * MIB,
            ..OfflineLimitsDto::default()
        })
        .await
        .unwrap_err();
    assert!(
        text(&error).starts_with(
            "Tables that download everything and tables with queued changes need at least"
        ),
        "{error}"
    );
    assert_eq!(scope.descriptor().limits, OfflineLimitsDto::default());
    let error = scope
        .set_limits(OfflineLimitsDto {
            max_operations: 0,
            ..OfflineLimitsDto::default()
        })
        .await
        .unwrap_err();
    assert!(text(&error).contains("operation limit"), "{error}");
    let accepted = OfflineLimitsDto {
        max_queue_bytes: 64 * MIB,
        max_operations: 500,
        max_age_seconds: 3600,
        max_mirror_bytes: 2048 * MIB,
    };
    assert_eq!(scope.set_limits(accepted).await.unwrap(), accepted);
    assert_eq!(scope.descriptor().limits, accepted);
    assert_eq!(
        scope
            .open_manager()
            .unwrap()
            .mirror_usage()
            .unwrap()
            .maximum,
        2048 * MIB
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forbidden_head_of_a_missing_file_is_confirmed_absent_by_listing() {
    let fixture = Fixture::new().await;
    let scope = fixture.supported(ALICE).await;
    let cloud = Arc::new(TestCloud::default());
    cloud.forbid_missing.store(true, Ordering::SeqCst);
    let present = format!("apps/{APP}/upload/present.txt");
    let missing = format!("apps/{APP}/upload/missing.txt");
    cloud.inner.put(&key(&present), "x".into()).await.unwrap();
    cloud.forbidden.lock().unwrap().insert(present.clone());
    let view = DesktopCloudView::new(
        Some(cloud.clone()),
        scope.cache_view().unwrap(),
        scope.connectivity(),
    );

    assert!(matches!(
        view.head(&key(&missing)).await,
        Err(object_store::Error::NotFound { .. })
    ));
    assert_eq!(
        scope.index().unwrap().lookup(&key(&missing)).unwrap(),
        Known::Absent
    );
    assert!(matches!(
        view.head(&key(&present)).await,
        Err(object_store::Error::PermissionDenied { .. })
    ));
    assert_eq!(
        scope.index().unwrap().lookup(&key(&present)).unwrap(),
        Known::Unknown
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_run_config_decorates_active_tables_and_rejects_others_with_e1_e3_e23() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "notes", rows(0..2))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    let ticket = scope.hosted_run_started();
    let notes = scope.begin_table(&selection("notes", false)).await.unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(notes));
    eventually("notes to settle", || async {
        status_of(&scope, "notes") == OfflineTableStatus::Settling
    })
    .await;
    scope
        .begin_table(&selection("drafts", false))
        .await
        .unwrap();

    let token = jwt(ALICE);
    let storage = RunStorage::from_prepared(
        Some(&fixture.registry),
        None,
        fixture.dirs(),
        request(&token, &fixture.hub.url),
        Err(unreachable()),
        |_| None,
    )
    .await
    .unwrap();
    assert_eq!(storage.mode(), RunStorageMode::Disconnected);
    assert!(Arc::ptr_eq(storage.scope().unwrap(), &scope));
    let config = storage
        .disconnected_config(FlowLikeConfig::default())
        .unwrap();
    let callbacks = config.callbacks.clone();
    let decorate = callbacks.decorate_database.clone().unwrap();
    let (_db_dir, connection) = local_connection().await;
    let path = project_db();
    let orders = decorate(path.clone(), opened(&connection, "orders"))
        .await
        .unwrap();
    assert!(orders.is_durably_managed());
    let failure = |table: &'static str| {
        let decorate = decorate.clone();
        let store = opened(&connection, table);
        let path = path.clone();
        async move { refused(decorate(path, store).await) }
    };
    assert_eq!(
        failure("customers").await,
        texts::table_unavailable("customers")
    );
    assert_eq!(failure("drafts").await, texts::table_not_ready("drafts"));
    assert!(
        failure("notes")
            .await
            .contains("is being prepared for offline use on this device")
    );

    let managed = callbacks.database_table_is_managed.clone().unwrap();
    assert!(managed(&path, "orders") && managed(&path, "notes"));
    assert!(!managed(&path, "drafts") && !managed(&path, "customers"));
    let names = callbacks.database_table_names.clone().unwrap();
    assert_eq!(
        names(path.clone()).await.unwrap(),
        vec!["orders".to_owned()]
    );
    assert!(callbacks.database_table_notice.is_none());
    assert!(matches!(
        config.stores.app_storage_store,
        Some(FlowLikeStore::Signed(_))
    ));
    assert!(matches!(
        config.stores.user_store,
        Some(FlowLikeStore::Signed(_))
    ));

    let session = Arc::new(Session::new(0, 0, storage.disconnected_registry()));
    let build = callbacks.build_project_database.clone().unwrap();
    let database = build(path.clone())
        .session(session.clone())
        .execute()
        .await
        .unwrap();
    let error = database.open_table("drafts").execute().await.unwrap_err();
    assert!(
        text(&error).contains(&texts::table_not_ready("drafts")),
        "{error}"
    );
    let error = database
        .open_table("customers")
        .execute()
        .await
        .unwrap_err();
    assert!(
        text(&error).contains(&texts::table_unavailable("customers")),
        "{error}"
    );
    let user = callbacks.build_user_database.clone().unwrap()(
        Path::from("users")
            .join(ALICE)
            .join("apps")
            .join(APP)
            .join("db"),
    )
    .session(session.clone())
    .execute()
    .await
    .unwrap();
    let error = user.open_table("notes").execute().await.unwrap_err();
    assert!(
        text(&error).contains(&texts::table_unavailable("notes")),
        "{error}"
    );
    let agents = build(Path::from("apps").join(APP).join("storage").join(".agents"))
        .session(session)
        .execute()
        .await
        .unwrap();
    assert!(agents.table_names().execute().await.unwrap().is_empty());
    assert!(
        fixture
            .dirs()
            .project
            .join(format!("apps/{APP}/storage/.agents"))
            .exists()
    );

    drop(ticket);
    setup.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_lance_cache_does_not_require_offline_configuration() {
    let directory = TempDir::new("ordinary-lance-cache");
    let live = lease(ALICE).await;
    let token = jwt(ALICE);
    let storage = RunStorage::from_prepared(
        None,
        None,
        super::scope::CacheDirs {
            project: directory.path().to_owned(),
            user: directory.path().join("user"),
        },
        request(&token, "localhost:7777"),
        Ok(SharedCredentials::Renewable(live.clone())),
        |_| None,
    )
    .await
    .unwrap();
    assert_eq!(storage.mode(), RunStorageMode::Hosted);
    assert!(storage.scope().is_none());
    let registry = live.lance_registry().unwrap();
    let store = registry
        .get_store(
            format!("s3://content/apps/{APP}/storage/db/orders.lance")
                .parse()
                .unwrap(),
            &flow_like::flow_like_storage::lance_io::object_store::ObjectStoreParams::default(),
        )
        .await
        .unwrap();
    assert!(store.inner.to_string().starts_with("CachedLanceStore("));
    assert!(directory.path().join(".lance-read-cache").is_dir());
    assert!(matches!(
        live.to_store_type(StoreType::Content).await.unwrap(),
        FlowLikeStore::AWS(_)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_run_config_decorates_every_online_app() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    let live = lease(ALICE).await;
    let token = jwt(ALICE);
    let storage = RunStorage::from_prepared(
        Some(&fixture.registry),
        None,
        fixture.dirs(),
        request(&token, &fixture.hub.url),
        Ok(SharedCredentials::Renewable(live.clone())),
        |_| None,
    )
    .await
    .unwrap();
    assert_eq!(storage.mode(), RunStorageMode::Hosted);
    assert!(Arc::ptr_eq(storage.scope().unwrap(), &scope));
    assert!(scope.lease().is_some_and(|held| Arc::ptr_eq(&held, &live)));
    let decorated = live.to_store_type(StoreType::Content).await.unwrap();
    assert_eq!(decorated.as_generic().to_string(), "DesktopFileStore");

    let config = hosted_config(FlowLikeConfig::default(), storage.scope().unwrap());
    let callbacks = config.callbacks;
    let decorate = callbacks.decorate_database.clone().unwrap();
    let (_db_dir, connection) = local_connection().await;
    let path = project_db();
    assert!(
        decorate(path.clone(), opened(&connection, "orders"))
            .await
            .unwrap()
            .is_durably_managed()
    );
    assert!(
        !decorate(path.clone(), opened(&connection, "customers"))
            .await
            .unwrap()
            .is_durably_managed()
    );
    assert!(callbacks.database_table_is_managed.clone().unwrap()(
        &path, "orders"
    ));
    assert!(callbacks.database_table_names.is_some());
    let notice = callbacks.database_table_notice.clone().unwrap();
    assert_eq!(notice(&path, "orders"), None, "a fresh copy logs nothing");
    let resource = scope
        .configured(OfflineTablePurpose::Storage, "orders")
        .unwrap()
        .resource();
    let manager = scope.open_manager().unwrap();
    let stale = chrono::Utc::now().timestamp() - 16 * 60;
    manager.queue().record_refresh(&resource, stale).unwrap();
    let warning = notice(&path, "orders").expect("a stale copy is noticed");
    assert!(
        warning.starts_with("Table 'orders' is read from this device's offline copy from "),
        "{warning}"
    );
    assert!(
        warning
            .ends_with("newer cloud changes are not visible yet: the next refresh has not run yet"),
        "{warning}"
    );
    assert_eq!(notice(&path, "customers"), None);
    scope.connectivity().set_offline(true);
    assert_eq!(
        notice(&path, "orders"),
        None,
        "offline staleness is expected"
    );
    scope.connectivity().set_offline(false);

    let other = fixture
        .registry
        .obtain(
            None,
            fixture.dirs(),
            ScopeKey {
                app_id: "app2".into(),
                ..fixture.key(ALICE)
            },
        )
        .unwrap();
    let bare = hosted_config(FlowLikeConfig::default(), &other).callbacks;
    let decorate = bare.decorate_database.unwrap();
    let unchanged = decorate(
        Path::from("apps/app2/storage/db"),
        opened(&connection, "orders"),
    )
    .await
    .unwrap();
    assert!(!unchanged.is_durably_managed());
    assert!(
        bare.database_table_names.is_none(),
        "without an engine the cloud lists tables"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_run_reads_cached_rows_and_fails_uncached_reads_with_e36() {
    let fixture = Fixture::new().await;
    let cloud = fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    cloud
        .open_table("orders")
        .execute()
        .await
        .unwrap()
        .add(
            flow_like::flow_like_storage::arrow_utils::value_to_batch_reader(rows(100..103))
                .unwrap(),
        )
        .execute()
        .await
        .unwrap();
    cloud
        .open_table("orders")
        .execute()
        .await
        .unwrap()
        .create_index(
            &["id"],
            flow_like::flow_like_storage::lancedb::index::Index::BTree(Default::default()),
        )
        .execute()
        .await
        .unwrap();
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    let manager = scope.open_manager().unwrap();
    let hosted = managed_store(&scope, "orders").await;
    assert_eq!(hosted.filter("id = 1", None, 10, 0).await.unwrap().len(), 1);
    settle_cache(&manager, "orders").await;
    let state = table_state_of(&manager, "orders").await;
    assert_eq!(state.key_indexed, Some(true));
    assert!(
        !state.offline_complete,
        "only the files of the lookup are on this device"
    );

    fixture.hub.stop();
    tokio::time::sleep(Duration::from_millis(200)).await;
    fixture.cloud.set_online(false);
    scope.connectivity().set_offline(true);
    let token = jwt(ALICE);
    let storage = RunStorage::from_prepared(
        Some(&fixture.registry),
        None,
        fixture.dirs(),
        request(&token, &fixture.hub.url),
        Err(unreachable()),
        |_| None,
    )
    .await
    .unwrap();
    let config = storage
        .disconnected_config(FlowLikeConfig::default())
        .unwrap();
    let (_db_dir, connection) = local_connection().await;
    let store = config.callbacks.decorate_database.clone().unwrap()(
        project_db(),
        opened(&connection, "orders"),
    )
    .await
    .unwrap();
    assert_eq!(
        store.filter("id = 1", None, 10, 0).await.unwrap().len(),
        1,
        "cached rows stay readable"
    );
    let error = match store.filter("id = 100", None, 10, 0).await {
        Ok(rows) => panic!(
            "a read of data this device never fetched fails with E36 instead of returning rows: {rows:?}"
        ),
        Err(error) => error,
    };
    assert!(
        text(&error).contains("Table 'orders' needs data that is not on this device yet"),
        "{error}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disconnected_writes_are_read_back_from_the_table_and_file_overlays() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", true).await;
    let manager = scope.open_manager().unwrap();
    eventually("the table to be fully available offline", || async {
        table_state_of(&manager, "orders").await.offline_complete
    })
    .await;
    fixture.hub.force(
        503,
        json!({"error": {"code": "ERROR", "message": "maintenance"}}),
    );
    fixture.cloud.set_online(false);
    scope.connectivity().set_offline(true);
    let token = jwt(ALICE);
    let storage = RunStorage::from_prepared(
        Some(&fixture.registry),
        None,
        fixture.dirs(),
        request(&token, &fixture.hub.url),
        Err(unreachable()),
        |_| None,
    )
    .await
    .unwrap();
    let config = storage
        .disconnected_config(FlowLikeConfig::default())
        .unwrap();
    let (_db_dir, connection) = local_connection().await;
    let mut store = config.callbacks.decorate_database.clone().unwrap()(
        project_db(),
        opened(&connection, "orders"),
    )
    .await
    .unwrap();
    store
        .insert(vec![json!({"id": 7, "value": 70, "label": "offline"})])
        .await
        .unwrap();
    let written = store.filter("id = 7", None, 10, 0).await.unwrap();
    assert_eq!(written.len(), 1);
    assert_eq!(written[0]["label"], "offline");
    assert_eq!(store.count(None).await.unwrap(), 4);

    let files = config
        .stores
        .app_storage_store
        .clone()
        .unwrap()
        .as_generic();
    let path = format!("apps/{APP}/upload/notes/new.txt");
    files.put(&key(&path), "draft".into()).await.unwrap();
    assert_eq!(read(files.as_ref(), &path).await.unwrap(), b"draft");
    assert_eq!(files.head(&key(&path)).await.unwrap().size, 5);

    let manager = scope.open_manager().unwrap();
    assert_eq!(manager.queue().status().unwrap().pending_count, 2);
    fixture.cloud.set_online(true);
    assert_eq!(
        fixture
            .cloud_table(ALICE, "orders")
            .await
            .count_rows(None)
            .await
            .unwrap(),
        3
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn subjects_and_hubs_get_separate_scopes_and_queues() {
    let fixture = Fixture::new().await;
    let alice = queuing_scope(&fixture, ALICE).await;
    let bob = fixture.scope(BOB);
    let elsewhere = fixture
        .registry
        .obtain(
            None,
            fixture.dirs(),
            ScopeKey {
                hub: "https://other.example.test".into(),
                ..fixture.key(ALICE)
            },
        )
        .unwrap();
    let ids: HashSet<&str> = [alice.id(), bob.id(), elsewhere.id()].into_iter().collect();
    assert_eq!(ids.len(), 3);
    assert_ne!(alice.engine_root(), bob.engine_root());
    assert_ne!(alice.engine_root(), elsewhere.engine_root());
    assert_eq!(
        alice.engine_root(),
        engine_dir(fixture.registry.root(), APP, alice.id())
    );

    alice
        .disconnected_store()
        .unwrap()
        .put(&key(&format!("apps/{APP}/upload/a.txt")), "x".into())
        .await
        .unwrap();
    assert_eq!(pending_count(&alice.engine_root()), 1);
    assert_eq!(pending_count(&bob.engine_root()), 0);
    assert_eq!(pending_count(&elsewhere.engine_root()), 0);
    assert_eq!(
        alice.location_prefix(StoragePurpose::User).as_deref(),
        Some("users/alice/apps/app1/")
    );
    assert_eq!(
        bob.location_prefix(StoragePurpose::User).as_deref(),
        Some("users/bob/apps/app1/")
    );
    let descriptors = read_descriptors(fixture.registry.root());
    let recorded = descriptors
        .iter()
        .find(|descriptor| descriptor.scope == alice.id())
        .unwrap();
    assert_eq!(
        (recorded.hub.as_str(), recorded.subject.as_str()),
        (fixture.hub_key().as_str(), ALICE)
    );
    assert_eq!(fixture.registry.other_accounts_pending(APP, bob.id()), 1);
    assert_eq!(
        fixture
            .registry
            .obtain(None, fixture.dirs(), fixture.key(ALICE))
            .unwrap()
            .id(),
        alice.id(),
        "the registry keeps one scope per key"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn blocked_file_lane_does_not_stop_table_replay_through_the_hub() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    fixture.hub.force_files(
        403,
        json!({"error": {"code": "OFFLINE_FORBIDDEN", "message": "no"}}),
    );
    let file = scope
        .disconnected_store()
        .unwrap()
        .put(&key(&format!("apps/{APP}/upload/a.txt")), "x".into())
        .await
        .unwrap();
    let file_operation = offline_id(&file);
    let mut store = managed_store(&scope, "orders").await;
    store
        .insert(vec![json!({"id": 9, "value": 90, "label": "queued"})])
        .await
        .unwrap();
    let table_operation = store.last_write_receipt().unwrap().operation_id;
    let manager = scope.open_manager().unwrap();
    eventually("the table replay", || async {
        manager
            .queue()
            .operation_state(&table_operation)
            .unwrap()
            .unwrap()
            .state
            == "applied"
    })
    .await;
    eventually("the blocked file", || async {
        manager
            .queue()
            .operation_state(&file_operation)
            .unwrap()
            .unwrap()
            .state
            == "blocked"
    })
    .await;
    assert_eq!(
        fixture
            .cloud_table(ALICE, "orders")
            .await
            .count_rows(None)
            .await
            .unwrap(),
        4
    );

    let overview = commands::overview(&fixture.registry, &scope).await.unwrap();
    assert_eq!(
        overview.queue.sync_state,
        commands::OfflineSyncState::Blocked
    );
    assert_eq!(overview.queue.blocked_heads.len(), 1);
    let head = &overview.queue.blocked_heads[0];
    assert_eq!(head.operation_id, file_operation);
    assert_eq!(head.kind, commands::OfflineOperationKind::FileWrite);
    assert_eq!(head.error_code.as_deref(), Some("forbidden"));
    assert_eq!(head.error.as_deref(), Some(texts::FORBIDDEN));
    assert_eq!(
        head.resource,
        commands::OfflineResourceLabel::File {
            purpose: "files".into(),
            path: "a.txt".into(),
        }
    );
    let listed = commands::operations(&manager, None, None).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        commands::lookup(manager.queue().operation_state(&table_operation).unwrap()).state,
        commands::OfflineOperationLookupState::Applied
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn skip_command_requires_acknowledgement_after_an_attempt() {
    let fixture = Fixture::new().await;
    let scope = fixture.supported(ALICE).await;
    let manager = scope.manager().await.unwrap();
    scope
        .update_descriptor(|descriptor| descriptor.provider = Some("az".into()))
        .unwrap();
    fixture.hub.force_files(
        418,
        json!({"error": {"code": "TEAPOT", "message": "I refuse"}}),
    );
    let files = scope.disconnected_store().unwrap();
    let first = offline_id(
        &files
            .put(&key(&format!("apps/{APP}/upload/a.txt")), "1".into())
            .await
            .unwrap(),
    );
    eventually("the rejected change", || async {
        manager
            .queue()
            .operation_state(&first)
            .unwrap()
            .unwrap()
            .state
            == "blocked"
    })
    .await;
    let second = offline_id(
        &files
            .put(&key(&format!("apps/{APP}/upload/a.txt")), "2".into())
            .await
            .unwrap(),
    );
    let head = commands::operations(&manager, None, None).unwrap();
    assert_eq!(head[0].attempts, 1);
    let error = scope.skip(&second, "later change", true).await.unwrap_err();
    assert!(
        text(&error).contains("Only the oldest queued change of a resource may be skipped"),
        "{error}"
    );
    let error = scope.skip(&first, "not needed", false).await.unwrap_err();
    assert!(
        text(&error).contains("may already have reached the cloud"),
        "{error}"
    );
    scope.skip(&first, "not needed", true).await.unwrap();
    eventually("the skip", || async {
        manager
            .queue()
            .operation_state(&first)
            .unwrap()
            .unwrap()
            .state
            == "skipped"
    })
    .await;
    let lookup = manager.queue().operation_state(&first).unwrap().unwrap();
    assert_eq!(
        lookup.error.as_deref(),
        Some(format!("desktop:{ALICE}: not needed").as_str())
    );

    fixture.hub.release();
    fixture.hub.force_files(
        403,
        json!({"error": {"code": "OFFLINE_FORBIDDEN", "message": "no"}}),
    );
    let unclaimed = offline_id(
        &files
            .put(&key(&format!("apps/{APP}/upload/b.txt")), "3".into())
            .await
            .unwrap(),
    );
    eventually("the forbidden change", || async {
        manager
            .queue()
            .operation_state(&unclaimed)
            .unwrap()
            .unwrap()
            .state
            == "blocked"
    })
    .await;
    scope.skip(&unclaimed, "no access", false).await.unwrap();
    eventually("the unclaimed skip", || async {
        manager
            .queue()
            .operation_state(&unclaimed)
            .unwrap()
            .unwrap()
            .state
            == "skipped"
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operation_state_command_maps_every_lookup_state() {
    use commands::OfflineOperationLookupState as State;
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    let manager = scope.open_manager().unwrap();
    let state = |id: &str| commands::lookup(manager.queue().operation_state(id).unwrap());
    assert_eq!(
        state("00000000-0000-4000-8000-000000000000").state,
        State::Unknown
    );

    *fixture.tokens.session.lock().unwrap() = None;
    let mut store = managed_store(&scope, "orders").await;
    let first = insert_row(&mut store, 10).await;
    eventually("the first attempt", || async {
        state(&first).state == State::Attempting
    })
    .await;
    let second = insert_row(&mut store, 11).await;
    let third = insert_row(&mut store, 12).await;
    assert_eq!(state(&second).state, State::Pending);
    assert_eq!(state(&third).state, State::Pending);

    *fixture.tokens.session.lock().unwrap() = Some(jwt(ALICE));
    scope.wake();
    eventually("the merged replay", || async {
        state(&second).state == State::Applied
    })
    .await;
    assert_eq!(state(&first).state, State::Applied);
    let merged = state(&third);
    assert_eq!(merged.state, State::Superseded);
    assert_eq!(merged.superseded_by.as_deref(), Some(second.as_str()));
    assert_eq!(
        fixture
            .cloud_table(ALICE, "orders")
            .await
            .count_rows(None)
            .await
            .unwrap(),
        6
    );

    fixture.hub.force_files(
        403,
        json!({"error": {"code": "OFFLINE_FORBIDDEN", "message": "no"}}),
    );
    let file = offline_id(
        &scope
            .disconnected_store()
            .unwrap()
            .put(&key(&format!("apps/{APP}/upload/a.txt")), "x".into())
            .await
            .unwrap(),
    );
    eventually("the blocked file", || async {
        state(&file).state == State::Blocked
    })
    .await;
    assert_eq!(state(&file).error_code.as_deref(), Some("forbidden"));
    scope.skip(&file, "no access", false).await.unwrap();
    eventually("the skipped file", || async {
        state(&file).state == State::Skipped
    })
    .await;

    for (raw, expected) in [
        ("conflict", State::Conflict),
        ("outcome_unknown", State::OutcomeUnknown),
        ("blocked", State::Blocked),
    ] {
        let mapped = commands::lookup(Some(flow_like_offline_writes::OperationLookup {
            state: raw.into(),
            superseded_by: None,
            error: None,
            error_code: Some("hub_limit".into()),
        }));
        assert_eq!(mapped.state, expected);
        assert_eq!(mapped.error_code.as_deref(), Some("hub_limit"));
    }
    let _ = (
        TableActivation::Active,
        DirectoryCloud::new(fixture.cloud_dir.path()),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replaced_setups_never_activate_a_newer_setup_of_the_same_table() {
    let fixture = Fixture::new().await;
    for table in ["orders", "notes"] {
        fixture
            .seed(ALICE, StoragePurpose::Storage, table, rows(0..3))
            .await;
    }
    let scope = fixture.supported(ALICE).await;
    let by_value = commands::OfflineTableSelection {
        primary_key: "value".into(),
        ..selection("notes", false)
    };
    for (table, replacement) in [("orders", selection("orders", false)), ("notes", by_value)] {
        let older_run = scope.hosted_run_started();
        let first = scope.begin_table(&selection(table, false)).await.unwrap();
        let first_setup = tokio::spawn(scope.clone().configure_table(first.clone()));
        eventually("the first setup to settle", || async {
            status_of(&scope, table) == OfflineTableStatus::Settling
        })
        .await;
        assert!(
            !scope.setup_pending(
                &scope
                    .configured(OfflineTablePurpose::Storage, table)
                    .unwrap()
            )
        );
        tokio::time::timeout(Duration::from_secs(5), scope.clone().configure_table(first))
            .await
            .expect("a second setup of a running one returns at once");

        if replacement.primary_key == "id" {
            scope
                .remove_table(OfflineTablePurpose::Storage, table)
                .await
                .unwrap();
        }
        let direct_run = scope.hosted_run_started();
        let second = scope.begin_table(&replacement).await.unwrap();
        assert!(scope.setup_pending(&second));
        let second_setup = tokio::spawn(scope.clone().configure_table(second));
        eventually("the replacing setup to settle", || async {
            status_of(&scope, table) == OfflineTableStatus::Settling
        })
        .await;

        drop(older_run);
        first_setup.await.unwrap();
        let manager = scope.open_manager().unwrap();
        let state = table_state_of(&manager, table).await;
        assert_eq!(
            state.activation,
            Some(TableActivation::Held),
            "{table}: a replaced setup activated the newer registration while a newer run writes directly"
        );
        assert_eq!(state.table.primary_key, replacement.primary_key);
        assert_eq!(status_of(&scope, table), OfflineTableStatus::Settling);
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(
            !second_setup.is_finished(),
            "{table}: the newer setup waits for the direct run"
        );

        drop(direct_run);
        second_setup.await.unwrap();
        assert_eq!(status_of(&scope, table), OfflineTableStatus::Ready);
        assert_eq!(
            table_state_of(&manager, table).await.activation,
            Some(TableActivation::Active)
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn forgotten_scope_never_writes_its_descriptor_again() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    let run = scope.hosted_run_started();
    let configured = scope
        .begin_table(&selection("orders", false))
        .await
        .unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(configured));
    eventually("the table to settle", || async {
        status_of(&scope, "orders") == OfflineTableStatus::Settling
    })
    .await;
    let scopes = fixture.registry.root().join("scopes");
    let descriptor = scopes.join(format!("{}.json", scope.id()));
    let directory = scopes.join(scope.id());
    fixture
        .registry
        .forget(APP, Some(&fixture.key(ALICE)), false)
        .await
        .unwrap();
    assert!(!descriptor.exists() && !directory.exists());

    drop(run);
    setup.await.unwrap();
    scope
        .clone()
        .configure_table(ConfiguredTable::from_selection(&selection("notes", false)).unwrap())
        .await;
    let Err(error) = scope.record_sink("evt-1", "pat_one") else {
        panic!("a forgotten scope records no sinks");
    };
    assert_eq!(text(error), texts::FORGOTTEN);
    let Err(error) = scope.update_descriptor(|descriptor| descriptor.sinks.clear()) else {
        panic!("a forgotten scope writes no descriptor");
    };
    assert_eq!(text(error), texts::FORGOTTEN);
    assert!(scope.refresh_capabilities(&jwt(ALICE)).await.is_err());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !descriptor.exists(),
        "a forgotten scope never resurrects its descriptor"
    );
    assert!(!directory.exists());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn download_everything_chosen_while_preparing_is_applied_after_registration() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", bulky_rows(3000))
        .await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "notes", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    let manager = scope.manager().await.unwrap();
    let limit = prefetch_limit(&fixture, "orders").await;
    scope
        .set_limits(OfflineLimitsDto {
            max_mirror_bytes: limit,
            ..OfflineLimitsDto::default()
        })
        .await
        .unwrap();

    let paused = fixture.cloud.gate.write().await;
    let orders = scope
        .begin_table(&selection("orders", false))
        .await
        .unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(orders));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(status_of(&scope, "orders"), OfflineTableStatus::Preparing);
    let chosen = scope
        .set_prefetch(OfflineTablePurpose::Storage, "orders", true)
        .await
        .unwrap();
    assert!(chosen.prefetch);
    drop(paused);
    setup.await.unwrap();
    let table = scope
        .configured(OfflineTablePurpose::Storage, "orders")
        .unwrap();
    assert_eq!(table.status, OfflineTableStatus::Ready, "{table:?}");
    assert!(!table.prefetch, "{table:?}");
    assert!(
        table
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("Downloading all of table 'orders' needs")),
        "{table:?}"
    );
    assert!(!table_state_of(&manager, "orders").await.prefetch);

    scope
        .set_limits(OfflineLimitsDto {
            max_mirror_bytes: 4 * 1024 * MIB,
            ..OfflineLimitsDto::default()
        })
        .await
        .unwrap();
    let paused = fixture.cloud.gate.write().await;
    let notes = scope.begin_table(&selection("notes", false)).await.unwrap();
    let setup = tokio::spawn(scope.clone().configure_table(notes));
    tokio::time::sleep(Duration::from_millis(300)).await;
    scope
        .set_prefetch(OfflineTablePurpose::Storage, "notes", true)
        .await
        .unwrap();
    drop(paused);
    setup.await.unwrap();
    let table = scope
        .configured(OfflineTablePurpose::Storage, "notes")
        .unwrap();
    assert_eq!(table.status, OfflineTableStatus::Ready, "{table:?}");
    assert!(table.prefetch && table.error.is_none(), "{table:?}");
    assert!(table_state_of(&manager, "notes").await.prefetch);

    let state = scope
        .set_prefetch(OfflineTablePurpose::Storage, "orders", true)
        .await
        .unwrap();
    assert!(state.prefetch);
    assert_eq!(
        state.error, None,
        "an accepted choice clears the old refusal"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hosted_runs_without_a_subject_fail_with_e6_once_tables_are_configured() {
    let encoded = aws(&format!("users/auth0%7C123/apps/{APP}"));
    assert_eq!(
        hosted_subject("Bearer pat_x", &encoded, APP).as_deref(),
        Some("auth0|123")
    );
    assert_eq!(
        hosted_subject(&format!("Bearer {}", jwt(BOB)), &encoded, APP).as_deref(),
        Some(BOB)
    );
    assert_eq!(
        disconnected_subject("Bearer pat_x", |token| Some(format!("owner of {token}"))).as_deref(),
        Some("owner of Bearer pat_x")
    );

    let fixture = Fixture::new().await;
    let anonymous = || async {
        let initial = aws("");
        let live = RenewableSharedCredentials::new(
            initial.clone(),
            APP.into(),
            Arc::new(Refresh(initial)),
        )
        .await
        .unwrap();
        RunStorage::from_prepared(
            Some(&fixture.registry),
            None,
            fixture.dirs(),
            request("pat_x", &fixture.hub.url),
            Ok(SharedCredentials::Renewable(live)),
            |_| None,
        )
        .await
    };
    let hosted = anonymous().await.unwrap();
    assert_eq!(hosted.mode(), RunStorageMode::Hosted);
    assert!(hosted.scope().is_none());

    fixture
        .supported(ALICE)
        .await
        .begin_table(&selection("orders", false))
        .await
        .unwrap();
    let Err(error) = anonymous().await else {
        panic!("an unattributed Hosted run must not open configured tables unfenced");
    };
    assert_eq!(text(error), texts::UNATTRIBUTED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transport_changes_emit_a_status_event() {
    let fixture = Fixture::new().await;
    let scope = fixture.supported(ALICE).await;
    scope.manager().await.unwrap();
    let host = scope.host().unwrap();
    tokio::time::sleep(Duration::from_millis(700)).await;
    let before = fixture.event_count(super::STATUS_EVENT);
    *fixture.tokens.session.lock().unwrap() = None;
    host.replay(&probe_request()).await.unwrap_err();
    assert_eq!(scope.transport(), TransportStatus::WaitingForSignIn);
    eventually("a status event for the new transport state", || async {
        fixture.event_count(super::STATUS_EVENT) > before
    })
    .await;
    tokio::time::sleep(Duration::from_millis(700)).await;
    let settled = fixture.event_count(super::STATUS_EVENT);
    host.replay(&probe_request()).await.unwrap_err();
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        fixture.event_count(super::STATUS_EVENT),
        settled,
        "an unchanged transport state emits nothing"
    );
}

#[test]
fn keep_both_reason_fits_the_engine_limit() {
    let name = "report (offline copy 2026-09-30 120000).txt";
    assert_eq!(
        super::scope::kept_as_reason(ALICE, name).unwrap(),
        format!("desktop:{ALICE}: kept as {name}")
    );
    let long = format!("{}/{name}", "ä".repeat(600));
    let reason = super::scope::kept_as_reason(ALICE, &long).unwrap();
    assert!((1020..=1024).contains(&reason.len()), "{}", reason.len());
    assert!(reason.starts_with(&format!("desktop:{ALICE}: kept as …")));
    assert!(reason.ends_with(&format!("/{name}")));
    assert!(super::scope::kept_as_reason(&"s".repeat(1100), name).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn keep_both_checks_the_skip_reason_before_uploading() {
    let fixture = Fixture::new().await;
    let subject = "s".repeat(1030);
    let scope = fixture.supported(&subject).await;
    let manager = scope.manager().await.unwrap();
    let path = format!("apps/{APP}/upload/report.txt");
    write_local(fixture.cloud_dir.path(), &path, b"cloud version");
    let queued = scope
        .disconnected_store()
        .unwrap()
        .put(&key(&path), "device version".into())
        .await
        .unwrap();
    let operation = offline_id(&queued);
    let state = || {
        manager
            .queue()
            .operation_state(&operation)
            .unwrap()
            .unwrap()
            .state
    };
    eventually("the conflict", || async { state() == "conflict" }).await;
    let error = scope.keep_both(&operation).await.unwrap_err();
    assert!(
        format!("{error:#}").contains("longer than 1024 bytes"),
        "{error:#}"
    );
    let uploads: Vec<_> =
        std::fs::read_dir(fixture.cloud_dir.path().join(format!("apps/{APP}/upload")))
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name())
            .collect();
    assert_eq!(uploads.len(), 1, "no copy was uploaded: {uploads:?}");
    assert_eq!(state(), "conflict");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_changing_commands_and_data_studio_need_a_signed_in_session() {
    let fixture = Fixture::new().await;
    fixture
        .seed(ALICE, StoragePurpose::Storage, "orders", rows(0..3))
        .await;
    let scope = fixture.supported(ALICE).await;
    fixture.enable(&scope, "orders", false).await;
    assert!(commands::signed_in(scope.clone()).is_ok());

    *fixture.tokens.session.lock().unwrap() = None;
    assert!(!scope.signed_in());
    let Err(error) = commands::signed_in(scope.clone()) else {
        panic!("a JWT without a signed-in session manages nothing");
    };
    assert_eq!(text(error), texts::SIGN_IN_MANAGE);
    let Err(error) = data_studio::managed(&scope, "orders", false, None).await else {
        panic!("Data Studio needs a signed-in session");
    };
    assert_eq!(text(error), texts::SIGN_IN_DATA);

    *fixture.tokens.session.lock().unwrap() = Some(jwt(ALICE));
    assert!(matches!(
        data_studio::managed(&scope, "orders", false, None)
            .await
            .unwrap(),
        DataStudioTarget::Managed(_)
    ));
}
