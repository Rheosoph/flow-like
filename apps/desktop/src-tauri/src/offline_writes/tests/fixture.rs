//! Test fixtures: temporary directories, a directory-backed cloud, a local hub that applies
//! replays to that cloud, and scopes wired to both.

use crate::offline_writes::{
    OfflineWrites,
    commands::OfflineTableSelection,
    host::{CloudAccess, DesktopHost, Services, TokenSource},
    hub_key,
    scope::{
        AppOfflineScope, CacheDirs, ConfiguredTable, OfflineTablePurpose, ScopeKey, location_prefix,
    },
};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{Path as UrlPath, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use flow_like::flow_like_storage::{
    arrow_utils::value_to_batch_reader,
    databases::vector::offline_replay::{self, ReplayMarker, ReplayMutation, ReplayOutcome},
    lance::session::Session,
    lance_io::object_store::{
        ObjectStore as LanceObjectStore, ObjectStoreParams, ObjectStoreProvider,
        ObjectStoreRegistry,
    },
    lancedb::{self, Connection},
    object_store::{ObjectStore, local::LocalFileSystem},
};
use flow_like_device_protocol::{
    OFFLINE_INSTALLATION_HEADER, OFFLINE_SUBJECT_HEADER, OfflineExpected, OfflineMutation,
    OfflineReplayRequest, OfflineReplayResponse, OfflineReplayStatus, OfflineResource,
    StoragePurpose,
};
use flow_like_offline_writes::OfflineHost;
use flow_like_types::{
    authorization::AuthorizationError,
    base64::{
        Engine,
        engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    },
    reqwest::Url,
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub(crate) const APP: &str = "app1";
pub(crate) const ALICE: &str = "alice";
pub(crate) const BOB: &str = "bob";

pub(crate) struct TempDir(PathBuf);

impl TempDir {
    pub(crate) fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "flow-like-offline-{label}-{}",
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir_all(&path).expect("a temporary test directory");
        Self(path)
    }

    pub(crate) fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An unsigned JWT whose payload names `subject`.
pub(crate) fn jwt(subject: &str) -> String {
    format!(
        "e30.{}.signature",
        URL_SAFE_NO_PAD.encode(json!({"sub": subject, "exp": 4_102_444_800u64}).to_string())
    )
}

#[derive(Default)]
pub(crate) struct StubTokens {
    pub(crate) session: Mutex<Option<String>>,
    pub(crate) sinks: Mutex<HashMap<String, String>>,
}

impl TokenSource for StubTokens {
    fn session_token(&self, _hub: &str, _subject: &str) -> Option<String> {
        self.session.lock().unwrap().clone()
    }

    fn sink_token(&self, _app_id: &str, event_id: &str) -> Option<String> {
        self.sinks.lock().unwrap().get(event_id).cloned()
    }
}

/// The cloud bucket as a directory; offline answers like an unreachable hub.
pub(crate) struct DirectoryCloud {
    pub(crate) root: PathBuf,
    pub(crate) online: AtomicBool,
    pub(crate) delay: Mutex<Duration>,
    /// Held for writing by a test to pause every database connection.
    pub(crate) gate: tokio::sync::RwLock<()>,
    files: Arc<LocalFileSystem>,
    session: Arc<Session>,
}

/// Lance's view of the test bucket: `cloudsim://bucket/{key}` reads `{root}/{key}`.
#[derive(Debug)]
struct CloudProvider(Arc<LocalFileSystem>);

#[async_trait::async_trait]
impl ObjectStoreProvider for CloudProvider {
    async fn new_store(
        &self,
        url: Url,
        params: &ObjectStoreParams,
    ) -> flow_like::flow_like_storage::lance::Result<LanceObjectStore> {
        Ok(LanceObjectStore::new(
            self.0.clone(),
            url,
            params.block_size,
            None,
            false,
            false,
            8,
            0,
            None,
        ))
    }
}

impl DirectoryCloud {
    pub(crate) fn new(root: &Path) -> Arc<Self> {
        let files = Arc::new(LocalFileSystem::new_with_prefix(root).expect("a test bucket"));
        let registry = Arc::new(ObjectStoreRegistry::default());
        registry.insert("cloudsim", Arc::new(CloudProvider(files.clone())));
        Arc::new(Self {
            root: root.to_path_buf(),
            online: AtomicBool::new(true),
            delay: Mutex::new(Duration::ZERO),
            gate: tokio::sync::RwLock::new(()),
            files,
            session: Arc::new(Session::new(16 << 20, 16 << 20, registry)),
        })
    }

    pub(crate) fn set_online(&self, online: bool) {
        self.online.store(online, Ordering::SeqCst);
    }

    fn reachable(&self) -> Result<()> {
        if self.online.load(Ordering::SeqCst) {
            Ok(())
        } else {
            Err(anyhow::Error::new(AuthorizationError::Unavailable)
                .context("the test cloud is offline"))
        }
    }
}

#[async_trait::async_trait]
impl CloudAccess for DirectoryCloud {
    async fn database(
        &self,
        host: &DesktopHost,
        purpose: StoragePurpose,
    ) -> Result<Option<Connection>> {
        let _open = self.gate.read().await;
        let delay = *self.delay.lock().unwrap();
        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
        self.reachable()?;
        let prefix = host
            .location_prefix(purpose)
            .context("the purpose has no database")?;
        std::fs::create_dir_all(self.root.join(&prefix).join("db"))?;
        Ok(Some(
            lancedb::connect(&format!("cloudsim://bucket/{prefix}db"))
                .session(self.session.clone())
                .execute()
                .await?,
        ))
    }

    async fn objects(&self, _host: &DesktopHost) -> Result<Option<Arc<dyn ObjectStore>>> {
        self.reachable()?;
        Ok(Some(self.files.clone()))
    }
}

pub(crate) async fn connect(directory: &Path) -> Result<Connection> {
    std::fs::create_dir_all(directory)?;
    Ok(
        lancedb::connect(directory.to_str().context("a UTF-8 test path")?)
            .execute()
            .await?,
    )
}

/// Records every replay request the hub received.
#[derive(Clone, Debug)]
pub(crate) struct ReceivedReplay {
    pub(crate) authorization: Option<String>,
    pub(crate) installation: Option<String>,
    pub(crate) subject: Option<String>,
    pub(crate) request: Option<OfflineReplayRequest>,
}

pub(crate) struct HubState {
    pub(crate) cloud: PathBuf,
    pub(crate) capabilities: Mutex<u16>,
    /// Every replay answers with this status and body.
    pub(crate) forced: Mutex<Option<(u16, String)>>,
    /// File replays answer with this status and body.
    pub(crate) file_forced: Mutex<Option<(u16, String)>>,
    pub(crate) received: Mutex<Vec<ReceivedReplay>>,
}

pub(crate) struct Hub {
    pub(crate) url: String,
    pub(crate) state: Arc<HubState>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Hub {
    pub(crate) async fn start(cloud: &Path) -> Self {
        let state = Arc::new(HubState {
            cloud: cloud.to_path_buf(),
            capabilities: Mutex::new(200),
            forced: Mutex::new(None),
            file_forced: Mutex::new(None),
            received: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .route("/api/v1/health", get(|| async { "ok" }))
            .route(
                "/api/v1/apps/{app}/invoke/offline/capabilities",
                get(capabilities),
            )
            .route("/api/v1/apps/{app}/invoke/offline/replay", post(replay))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a local port for the test hub");
        let url = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Self { url, state, task }
    }

    /// The hub becomes unreachable, so the scope's breaker stays open.
    pub(crate) fn stop(&self) {
        self.task.abort();
    }

    pub(crate) fn force(&self, status: u16, body: Value) {
        *self.state.forced.lock().unwrap() = Some((status, body.to_string()));
    }

    pub(crate) fn force_files(&self, status: u16, body: Value) {
        *self.state.file_forced.lock().unwrap() = Some((status, body.to_string()));
    }

    pub(crate) fn release(&self) {
        *self.state.forced.lock().unwrap() = None;
        *self.state.file_forced.lock().unwrap() = None;
    }

    pub(crate) fn received(&self) -> Vec<ReceivedReplay> {
        self.state.received.lock().unwrap().clone()
    }
}

async fn capabilities(State(state): State<Arc<HubState>>) -> Response {
    let status = *state.capabilities.lock().unwrap();
    if status != 200 {
        return StatusCode::from_u16(status).unwrap().into_response();
    }
    Json(json!({
        "version": 1,
        "limits": {"maxOperationBytes": 4_194_304, "maxFileBytes": 3_145_728, "maxRequestBytes": 5_000_000},
        "provider": "s3",
        "futureField": true
    }))
    .into_response()
}

fn answer(status: u16, body: &str) -> Response {
    (
        StatusCode::from_u16(status).unwrap(),
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body.to_owned(),
    )
        .into_response()
}

async fn replay(
    State(state): State<Arc<HubState>>,
    UrlPath(app): UrlPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    };
    let request: Option<OfflineReplayRequest> = serde_json::from_slice(&body).ok();
    let subject = header(OFFLINE_SUBJECT_HEADER).unwrap_or_default();
    state.received.lock().unwrap().push(ReceivedReplay {
        authorization: header("authorization"),
        installation: header(OFFLINE_INSTALLATION_HEADER),
        subject: Some(subject.clone()),
        request: request.clone(),
    });
    if let Some((status, body)) = state.forced.lock().unwrap().clone() {
        return answer(status, &body);
    }
    let Some(request) = request else {
        return answer(
            400,
            r#"{"error":{"code":"OFFLINE_INVALID","message":"Unreadable replay"}}"#,
        );
    };
    let is_file = matches!(request.resource, OfflineResource::File { .. });
    if is_file && let Some((status, body)) = state.file_forced.lock().unwrap().clone() {
        return answer(status, &body);
    }
    let key = ScopeKey {
        hub: String::new(),
        subject,
        app_id: app,
    };
    let applied = if is_file {
        apply_file(&state.cloud, &key, &request)
    } else {
        apply_table(&state.cloud, &key, &request).await
    };
    match applied {
        Ok(response) => Json(response).into_response(),
        Err(error) => answer(
            500,
            &json!({"error": {"code": "ERROR", "message": error.to_string()}}).to_string(),
        ),
    }
}

fn respond(
    request: &OfflineReplayRequest,
    status: OfflineReplayStatus,
    result: Option<OfflineExpected>,
) -> Result<OfflineReplayResponse> {
    Ok(OfflineReplayResponse {
        operation_id: request.operation_id.clone(),
        digest: request.digest()?,
        status,
        result,
        message: None,
    })
}

fn replay_mutation(mutation: OfflineMutation) -> Result<ReplayMutation> {
    Ok(match mutation {
        OfflineMutation::TableInsert { rows } => ReplayMutation::Insert { items: rows },
        OfflineMutation::TableUpsert { id_field, rows } => ReplayMutation::Upsert {
            items: rows,
            id_field,
        },
        OfflineMutation::TableUpdate { filter, updates } => ReplayMutation::Update {
            filter,
            updates: updates.into_iter().collect(),
        },
        OfflineMutation::TableDelete { filter } => ReplayMutation::Delete { filter },
        _ => anyhow::bail!("a file mutation reached the table replay"),
    })
}

async fn apply_table(
    cloud: &Path,
    key: &ScopeKey,
    request: &OfflineReplayRequest,
) -> Result<OfflineReplayResponse> {
    let OfflineResource::Table { purpose, table, .. } = &request.resource else {
        anyhow::bail!("not a table replay");
    };
    let OfflineExpected::TableVersion {
        version,
        fingerprint,
    } = &request.expected
    else {
        anyhow::bail!("a table replay without a table version");
    };
    let prefix = location_prefix(key, *purpose).context("unknown purpose")?;
    let db = connect(&cloud.join(prefix).join("db")).await?;
    let marker = ReplayMarker {
        operation_id: request.operation_id.clone(),
        digest: request.digest()?,
        expected_version: *version,
        expected_fingerprint: fingerprint.clone(),
    };
    let table = db.open_table(table).execute().await?;
    match offline_replay::replay(&table, &marker, replay_mutation(request.mutation.clone())?)
        .await?
    {
        ReplayOutcome::Applied {
            version,
            fingerprint,
        } => respond(
            request,
            OfflineReplayStatus::Applied,
            Some(OfflineExpected::TableVersion {
                version,
                fingerprint: Some(fingerprint),
            }),
        ),
        ReplayOutcome::Conflict { .. } => respond(request, OfflineReplayStatus::Conflict, None),
        _ => respond(request, OfflineReplayStatus::OutcomeUnknown, None),
    }
}

pub(crate) fn test_etag(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex()[..16].to_owned()
}

fn apply_file(
    cloud: &Path,
    key: &ScopeKey,
    request: &OfflineReplayRequest,
) -> Result<OfflineReplayResponse> {
    let OfflineResource::File { purpose, path } = &request.resource else {
        anyhow::bail!("not a file replay");
    };
    let prefix = location_prefix(key, *purpose).context("unknown purpose")?;
    let file = cloud.join(format!("{prefix}{path}"));
    let existing = std::fs::read(&file).ok();
    match &request.mutation {
        OfflineMutation::FilePut { data_base64, .. } => {
            let bytes = STANDARD.decode(data_base64)?;
            let conflict = match (&request.expected, &existing) {
                (OfflineExpected::FileAbsent, Some(current)) => *current != bytes,
                (OfflineExpected::FileRevision { e_tag, .. }, Some(current)) => {
                    e_tag.as_deref() != Some(test_etag(current).as_str())
                }
                (OfflineExpected::FileRevision { .. }, None) => true,
                _ => false,
            };
            if conflict {
                return respond(request, OfflineReplayStatus::Conflict, None);
            }
            std::fs::create_dir_all(file.parent().context("a parent directory")?)?;
            std::fs::write(&file, &bytes)?;
            respond(
                request,
                OfflineReplayStatus::Applied,
                Some(OfflineExpected::FileRevision {
                    e_tag: Some(test_etag(&bytes)),
                    version: None,
                }),
            )
        }
        OfflineMutation::FileDelete => {
            let _ = std::fs::remove_file(&file);
            respond(
                request,
                OfflineReplayStatus::Applied,
                Some(OfflineExpected::FileAbsent),
            )
        }
        _ => anyhow::bail!("a table mutation reached the file replay"),
    }
}

pub(crate) fn rows(ids: std::ops::Range<i64>) -> Vec<Value> {
    ids.map(|id| json!({"id": id, "value": id * 10, "label": format!("row {id}")}))
        .collect()
}

/// Rows of random text, so the table's data files stay large on disk.
pub(crate) fn bulky_rows(count: i64) -> Vec<Value> {
    (0..count)
        .map(|id| {
            let text: String = (0..16)
                .map(|_| uuid::Uuid::new_v4().simple().to_string())
                .collect();
            json!({"id": id, "value": id, "label": text})
        })
        .collect()
}

pub(crate) fn selection(table: &str, prefetch: bool) -> OfflineTableSelection {
    OfflineTableSelection {
        purpose: OfflineTablePurpose::Storage,
        table: table.to_owned(),
        primary_key: "id".to_owned(),
        prefetch,
    }
}

pub(crate) struct Fixture {
    pub(crate) root: TempDir,
    pub(crate) cloud_dir: TempDir,
    pub(crate) local: TempDir,
    pub(crate) hub: Hub,
    pub(crate) tokens: Arc<StubTokens>,
    pub(crate) cloud: Arc<DirectoryCloud>,
    pub(crate) events: Arc<Mutex<Vec<(String, Value)>>>,
    pub(crate) registry: OfflineWrites,
    pub(crate) activation_retry: Duration,
}

impl Fixture {
    pub(crate) async fn new() -> Self {
        Self::with_retry(Duration::from_secs(30)).await
    }

    pub(crate) async fn with_retry(activation_retry: Duration) -> Self {
        let root = TempDir::new("root");
        let cloud_dir = TempDir::new("cloud");
        let local = TempDir::new("local");
        for directory in ["project", "user"] {
            std::fs::create_dir_all(local.path().join(directory)).unwrap();
        }
        let hub = Hub::start(cloud_dir.path()).await;
        let tokens = Arc::new(StubTokens::default());
        *tokens.session.lock().unwrap() = Some(jwt(ALICE));
        let cloud = DirectoryCloud::new(cloud_dir.path());
        let events = Arc::new(Mutex::new(Vec::new()));
        let mut fixture = Self {
            registry: OfflineWrites::at(root.path().join("offline-writes")),
            root,
            cloud_dir,
            local,
            hub,
            tokens,
            cloud,
            events,
            activation_retry,
        };
        fixture.registry = fixture.new_registry();
        fixture
    }

    pub(crate) fn services(&self) -> Services {
        let events = self.events.clone();
        Services {
            cloud: self.cloud.clone(),
            tokens: self.tokens.clone(),
            emit: Arc::new(move |event, payload| {
                events.lock().unwrap().push((event.to_owned(), payload));
            }),
            activation_retry: self.activation_retry,
        }
    }

    /// A registry over the same directories, as after a restart.
    pub(crate) fn new_registry(&self) -> OfflineWrites {
        OfflineWrites::with_services(self.root.path().join("offline-writes"), self.services())
    }

    pub(crate) fn dirs(&self) -> CacheDirs {
        CacheDirs {
            project: self.local.path().join("project"),
            user: self.local.path().join("user"),
        }
    }

    pub(crate) fn hub_key(&self) -> String {
        hub_key(&self.hub.url, false).expect("a canonical test hub")
    }

    pub(crate) fn key(&self, subject: &str) -> ScopeKey {
        ScopeKey {
            hub: self.hub_key(),
            subject: subject.to_owned(),
            app_id: APP.to_owned(),
        }
    }

    pub(crate) fn scope(&self, subject: &str) -> Arc<AppOfflineScope> {
        self.registry
            .obtain(None, self.dirs(), self.key(subject))
            .expect("a test scope")
    }

    /// A scope whose hub confirmed desktop offline support.
    pub(crate) async fn supported(&self, subject: &str) -> Arc<AppOfflineScope> {
        let scope = self.scope(subject);
        assert!(scope.refresh_capabilities(&jwt(subject)).await.unwrap());
        scope
    }

    pub(crate) async fn seed(
        &self,
        subject: &str,
        purpose: StoragePurpose,
        table: &str,
        rows: Vec<Value>,
    ) -> Connection {
        let prefix = location_prefix(&self.key(subject), purpose).unwrap();
        let db = connect(&self.cloud_dir.path().join(prefix).join("db"))
            .await
            .unwrap();
        db.create_table(table, value_to_batch_reader(rows).unwrap())
            .execute()
            .await
            .unwrap();
        db
    }

    pub(crate) async fn cloud_table(&self, subject: &str, table: &str) -> lancedb::Table {
        let prefix = location_prefix(&self.key(subject), StoragePurpose::Storage).unwrap();
        connect(&self.cloud_dir.path().join(prefix).join("db"))
            .await
            .unwrap()
            .open_table(table)
            .execute()
            .await
            .unwrap()
    }

    /// Enables `table` and waits for its setup to finish.
    pub(crate) async fn enable(
        &self,
        scope: &Arc<AppOfflineScope>,
        table: &str,
        prefetch: bool,
    ) -> ConfiguredTable {
        let configured = scope
            .begin_table(&selection(table, prefetch))
            .await
            .unwrap();
        scope.clone().configure_table(configured.clone()).await;
        scope
            .configured(configured.purpose, &configured.table)
            .expect("a configured table")
    }

    pub(crate) fn event_count(&self, event: &str) -> usize {
        self.events
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, _)| name == event)
            .count()
    }
}

/// Polls `check` until it holds, failing after 20 s.
pub(crate) async fn eventually<F, Fut>(what: &str, check: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = Instant::now() + Duration::from_secs(20);
    while !check().await {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
