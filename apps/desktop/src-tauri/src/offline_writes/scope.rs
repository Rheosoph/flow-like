//! One offline scope: (hub, subject, app, installation). Owns the descriptor, the lazily
//! opened engine, the activation fence and the configured tables (design §4.2, §4.3, §4.9).

use super::{
    AppEvent, MIRROR_EVENT, STATUS_EVENT, TABLES_EVENT, api_base, authorization,
    commands::{OfflineSyncState, OfflineTableSelection, OfflineTableState, table_state},
    descriptor_path, engine_dir, ensure_root,
    host::{DesktopHost, Services, TransportStatus},
    object_index::{ContentRoots, Known, ObjectIndex},
    scope_dir, scopes_dir,
    stores::{
        DesktopCloudView, DesktopFileStore, DisconnectedConnectivity, FileLayer, LocalCacheView,
        Refusal, ScopeConnectivity, UnreadyTables, file_options,
    },
    sync_directory, texts,
};
use anyhow::{Context, Result, anyhow, bail, ensure};
use flow_like::{
    credentials::{
        SharedCredentials,
        renewable::{ContentStoreDecorator, RenewableSharedCredentials},
    },
    flow_like_storage::{
        databases::vector::lancedb::LanceDBVectorStore,
        files::store::{FlowLikeStore, local_store::LocalObjectStore},
        object_store::{ObjectStoreExt, path::Path as ObjectPath},
    },
};
use flow_like_device_protocol::{
    DESKTOP_OFFLINE_LIMITS, DesktopOfflineCapabilities, MAX_OFFLINE_OPERATION_BYTES,
    MAX_OFFLINE_REPLAY_HTTP_BYTES, OfflineContentProvider, OfflineLimits, OfflineResource,
    StoragePurpose, desktop_offline_capabilities_path,
};
use flow_like_offline_writes::{
    BufferedTable, BufferingConfig, Connectivity, FastForward, LazyMirrorOptions, MirrorMode,
    Observation, TableActivation, TableSetup, TableState, WriteManager, WriteManagerOptions,
    fs::{private_directory, unix_time},
    outbox::QueueLanes,
};
use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, MutexGuard, OnceLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::AppHandle;
use tokio::sync::{Notify, OnceCell};

const DESCRIPTOR_VERSION: u32 = 1;
const REFRESH_INTERVAL: Duration = Duration::from_secs(120);
const IDLE_POLL: Duration = Duration::from_secs(15);
const FAST_FORWARD: FastForward = FastForward {
    probe_timeout: Duration::from_secs(2),
    min_interval: Duration::from_secs(10),
};
const RETIRED_SNAPSHOT_GRACE: Duration = Duration::from_secs(10 * 60);
const MAX_LAZY_FILE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_REFRESH_FETCH_BYTES: u64 = 64 * 1024 * 1024;
const CAPABILITIES_MAX_AGE: i64 = 24 * 60 * 60;
const CAPABILITIES_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_SINKS: usize = 32;
const STALE_NOTICE_AFTER: i64 = 15 * 60;
const EVENT_INTERVAL: Duration = Duration::from_millis(500);
const MAX_BLOCKED_HEADS: u32 = 256;
const MAX_SKIP_REASON: usize = 1024;
const ELLIPSIS: &str = "…";

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ScopeKey {
    pub(crate) hub: String,
    pub(crate) subject: String,
    pub(crate) app_id: String,
}

/// FlowPath's local cache roots: the project and user directories of this device.
#[derive(Clone, Debug)]
pub(crate) struct CacheDirs {
    pub(crate) project: PathBuf,
    pub(crate) user: PathBuf,
}

pub(crate) async fn cache_dirs(app_handle: &AppHandle) -> Result<CacheDirs> {
    let settings = crate::state::TauriSettingsState::construct(app_handle).await?;
    let settings = settings.lock().await;
    Ok(CacheDirs {
        project: settings.project_dir.clone(),
        user: settings.user_dir.clone(),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum OfflineTablePurpose {
    Storage,
    User,
}

impl OfflineTablePurpose {
    pub(crate) fn storage(self) -> StoragePurpose {
        match self {
            Self::Storage => StoragePurpose::Storage,
            Self::User => StoragePurpose::User,
        }
    }

    pub(crate) fn from_storage(purpose: StoragePurpose) -> Option<Self> {
        match purpose {
            StoragePurpose::Storage => Some(Self::Storage),
            StoragePurpose::User => Some(Self::User),
            _ => None,
        }
    }

    pub(crate) fn scoped(user_scoped: bool) -> Self {
        if user_scoped {
            Self::User
        } else {
            Self::Storage
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum OfflineTableStatus {
    Preparing,
    Settling,
    Ready,
    Error,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConfiguredTable {
    pub(crate) purpose: OfflineTablePurpose,
    pub(crate) database: String,
    pub(crate) table: String,
    pub(crate) primary_key: String,
    pub(crate) prefetch: bool,
    pub(crate) status: OfflineTableStatus,
    pub(crate) error: Option<String>,
    pub(crate) enabled_at: i64,
}

impl ConfiguredTable {
    pub(crate) fn from_selection(selection: &OfflineTableSelection) -> Result<Self> {
        let table = Self {
            purpose: selection.purpose,
            database: "db".into(),
            table: selection.table.trim().to_owned(),
            primary_key: selection.primary_key.trim().to_owned(),
            prefetch: selection.prefetch,
            status: OfflineTableStatus::Preparing,
            error: None,
            enabled_at: unix_time()?,
        };
        table.selection().validate()?;
        Ok(table)
    }

    pub(crate) fn selection(&self) -> BufferedTable {
        BufferedTable {
            purpose: self.purpose.storage(),
            database: self.database.clone(),
            table: self.table.clone(),
            primary_key: self.primary_key.clone(),
        }
    }

    /// The engine's resource key of this table.
    pub(crate) fn resource(&self) -> String {
        serde_json::to_string(&OfflineResource::Table {
            purpose: self.purpose.storage(),
            database: self.database.clone(),
            table: self.table.clone(),
        })
        .unwrap_or_default()
    }
}

/// Queue and mirror limits of a scope (`OfflineLimitsDto` of the commands).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OfflineLimitsDto {
    pub(crate) max_queue_bytes: u64,
    pub(crate) max_operations: u32,
    pub(crate) max_age_seconds: u64,
    pub(crate) max_mirror_bytes: u64,
}

impl Default for OfflineLimitsDto {
    fn default() -> Self {
        Self {
            max_queue_bytes: 256 * 1024 * 1024,
            max_operations: 10_000,
            max_age_seconds: 7 * 24 * 60 * 60,
            max_mirror_bytes: 1024 * 1024 * 1024,
        }
    }
}

impl OfflineLimitsDto {
    pub(crate) fn config(&self) -> BufferingConfig {
        BufferingConfig {
            max_queue_bytes: self.max_queue_bytes,
            max_operations: self.max_operations,
            max_age_seconds: self.max_age_seconds,
            max_mirror_bytes: self.max_mirror_bytes,
            ..BufferingConfig::default()
        }
    }

    pub(crate) fn lazy(&self) -> LazyMirrorOptions {
        LazyMirrorOptions {
            max_lazy_file_bytes: MAX_LAZY_FILE_BYTES,
            max_refresh_fetch_bytes: MAX_REFRESH_FETCH_BYTES,
            max_download_bytes_per_day: self.max_mirror_bytes.saturating_mul(2),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SinkBinding {
    pub(crate) event_id: String,
    pub(crate) token_digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScopeDescriptor {
    pub(crate) version: u32,
    pub(crate) scope: String,
    pub(crate) hub: String,
    pub(crate) subject: String,
    pub(crate) app_id: String,
    #[serde(default)]
    pub(crate) provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) hub_support: Option<bool>,
    #[serde(default)]
    pub(crate) hub_limits: Option<OfflineLimits>,
    #[serde(default)]
    pub(crate) capabilities_checked_at: Option<i64>,
    #[serde(default)]
    pub(crate) limits: OfflineLimitsDto,
    #[serde(default)]
    pub(crate) tables: Vec<ConfiguredTable>,
    #[serde(default)]
    pub(crate) sinks: Vec<SinkBinding>,
}

impl ScopeDescriptor {
    pub(crate) fn new(scope: &str, key: &ScopeKey) -> Self {
        Self {
            version: DESCRIPTOR_VERSION,
            scope: scope.to_owned(),
            hub: key.hub.clone(),
            subject: key.subject.clone(),
            app_id: key.app_id.clone(),
            provider: None,
            hub_support: None,
            hub_limits: None,
            capabilities_checked_at: None,
            limits: OfflineLimitsDto::default(),
            tables: Vec::new(),
            sinks: Vec::new(),
        }
    }

    pub(crate) fn key(&self) -> ScopeKey {
        ScopeKey {
            hub: self.hub.clone(),
            subject: self.subject.clone(),
            app_id: self.app_id.clone(),
        }
    }

    pub(crate) fn replay_limits(&self) -> OfflineLimits {
        self.hub_limits.unwrap_or(DESKTOP_OFFLINE_LIMITS)
    }

    pub(crate) fn table(
        &self,
        purpose: OfflineTablePurpose,
        table: &str,
    ) -> Option<&ConfiguredTable> {
        self.tables
            .iter()
            .find(|entry| entry.purpose == purpose && entry.table == table)
    }
}

pub(crate) fn read_descriptor(root: &Path, scope: &str) -> Result<Option<ScopeDescriptor>> {
    let path = descriptor_path(root, scope);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let descriptor: ScopeDescriptor = serde_json::from_slice(&bytes).with_context(|| {
        format!(
            "The offline scope descriptor {} is unreadable",
            path.display()
        )
    })?;
    ensure!(
        descriptor.scope == scope,
        "The offline scope descriptor {} names another scope",
        path.display()
    );
    Ok(Some(descriptor))
}

pub(crate) fn read_descriptors(root: &Path) -> Vec<ScopeDescriptor> {
    let Ok(entries) = std::fs::read_dir(scopes_dir(root)) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let scope = name.strip_suffix(".json")?.to_owned();
            if !super::valid_scope(&scope)
                || scope_dir(root, &scope).join(super::TOMBSTONE_FILE).exists()
            {
                return None;
            }
            match read_descriptor(root, &scope) {
                Ok(descriptor) => descriptor,
                Err(error) => {
                    tracing::warn!(%error, "Skipping an unreadable offline scope descriptor");
                    None
                }
            }
        })
        .collect()
}

/// tmp + fsync + rename + directory fsync, 0600.
pub(crate) fn write_descriptor(root: &Path, descriptor: &ScopeDescriptor) -> Result<()> {
    ensure_root(root)?;
    let directory = scopes_dir(root);
    private_directory(&directory)?;
    let path = descriptor_path(root, &descriptor.scope);
    let temporary = directory.join(format!("{}.json.tmp", descriptor.scope));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(descriptor)?)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, &path)?;
    sync_directory(&directory)
}

/// Hosted runs that started before an activation epoch must end before the table activates.
#[derive(Default)]
pub(crate) struct ActivationFence {
    epoch: AtomicU64,
    next: AtomicU64,
    alive: Mutex<BTreeMap<u64, u64>>,
    changed: Notify,
}

impl ActivationFence {
    fn alive(&self) -> MutexGuard<'_, BTreeMap<u64, u64>> {
        self.alive
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn older_than(&self, epoch: u64) -> u32 {
        self.alive()
            .values()
            .filter(|started| **started < epoch)
            .count() as u32
    }
}

/// Dropping it releases the run's hold on activations (D10).
pub(crate) struct HostedRunTicket {
    fence: Arc<ActivationFence>,
    id: u64,
}

impl Drop for HostedRunTicket {
    fn drop(&mut self) {
        self.fence.alive().remove(&self.id);
        self.fence.changed.notify_waiters();
    }
}

/// Emits at most once per interval; the last change always gets its own emission.
pub(crate) struct Coalescer {
    interval: Duration,
    dirty: AtomicBool,
    scheduled: AtomicBool,
    last: Mutex<Option<Instant>>,
    emit: Box<dyn Fn() + Send + Sync>,
}

impl Coalescer {
    pub(crate) fn new(interval: Duration, emit: impl Fn() + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            interval,
            dirty: AtomicBool::new(false),
            scheduled: AtomicBool::new(false),
            last: Mutex::new(None),
            emit: Box::new(emit),
        })
    }

    pub(crate) fn fire(self: &Arc<Self>) {
        self.dirty.store(true, Ordering::SeqCst);
        if self.scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            self.scheduled.store(false, Ordering::SeqCst);
            self.dirty.store(false, Ordering::SeqCst);
            (self.emit)();
            return;
        };
        let this = self.clone();
        runtime.spawn(async move { this.run().await });
    }

    fn last(&self) -> MutexGuard<'_, Option<Instant>> {
        self.last
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn run(self: Arc<Self>) {
        loop {
            let wait = self
                .last()
                .map(|last| (last + self.interval).saturating_duration_since(Instant::now()))
                .unwrap_or_default();
            tokio::time::sleep(wait).await;
            if self.dirty.swap(false, Ordering::SeqCst) {
                (self.emit)();
                *self.last() = Some(Instant::now());
                continue;
            }
            self.scheduled.store(false, Ordering::SeqCst);
            if !self.dirty.load(Ordering::SeqCst) || self.scheduled.swap(true, Ordering::SeqCst) {
                return;
            }
        }
    }
}

/// How a run uses a scope's tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RunMode {
    Hosted,
    Disconnected,
}

/// One setup of a table (§4.9 step 6): a later enable, key change, removal or `forget`
/// replaces its generation, and every await of the setup ends once it no longer matches.
#[derive(Clone, Debug)]
struct PendingActivation {
    generation: u64,
    primary_key: String,
    /// Set once the table is registered held; activation waits for older Hosted runs.
    epoch: Option<u64>,
    last_attempt: Option<Instant>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StatusEvent {
    pub(crate) app_id: String,
    pub(crate) pending_count: u64,
    pub(crate) sync_state: OfflineSyncState,
    pub(crate) head_state: Option<String>,
    pub(crate) blocked_count: u64,
}

pub(crate) struct AppOfflineScope {
    this: Weak<Self>,
    root: PathBuf,
    installation: String,
    id: String,
    key: ScopeKey,
    dirs: CacheDirs,
    roots: ContentRoots,
    pub(crate) services: Services,
    http: Result<flow_like_types::reqwest::Client, String>,
    descriptor: Mutex<ScopeDescriptor>,
    manager: OnceCell<Arc<WriteManager>>,
    manager_error: Mutex<Option<String>>,
    host: OnceLock<Arc<DesktopHost>>,
    index: OnceLock<Arc<ObjectIndex>>,
    connectivity: Arc<ScopeConnectivity>,
    lease: Mutex<Weak<RenewableSharedCredentials>>,
    fence: Arc<ActivationFence>,
    activations: Mutex<HashMap<String, PendingActivation>>,
    generations: AtomicU64,
    /// Serializes engine registration and activation per table resource.
    setup_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    recovery: Mutex<Vec<ConfiguredTable>>,
    status_events: Arc<Coalescer>,
    mirror_events: Arc<Coalescer>,
    closed: AtomicBool,
}

impl AppOfflineScope {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        root: PathBuf,
        installation: String,
        id: String,
        key: ScopeKey,
        descriptor: ScopeDescriptor,
        dirs: CacheDirs,
        services: Services,
    ) -> Arc<Self> {
        let connectivity = ScopeConnectivity::new(&key.hub);
        let roots = ContentRoots::new(&key.app_id, Some(&key.subject));
        let scope = Arc::new_cyclic(|this: &Weak<Self>| {
            let status = this.clone();
            let mirror = this.clone();
            Self {
                this: this.clone(),
                root,
                installation,
                id,
                key,
                dirs,
                roots,
                services,
                http: flow_like_types::reqwest::Client::builder()
                    .redirect(flow_like_types::reqwest::redirect::Policy::none())
                    .connect_timeout(CAPABILITIES_TIMEOUT)
                    .timeout(CAPABILITIES_TIMEOUT)
                    .build()
                    .map_err(|error| error.to_string()),
                descriptor: Mutex::new(descriptor),
                manager: OnceCell::new(),
                manager_error: Mutex::new(None),
                host: OnceLock::new(),
                index: OnceLock::new(),
                connectivity,
                lease: Mutex::new(Weak::new()),
                fence: Arc::new(ActivationFence::default()),
                activations: Mutex::new(HashMap::new()),
                generations: AtomicU64::new(0),
                setup_locks: Mutex::new(HashMap::new()),
                recovery: Mutex::new(Vec::new()),
                status_events: Coalescer::new(EVENT_INTERVAL, move || {
                    if let Some(scope) = status.upgrade() {
                        scope.emit_status_now();
                    }
                }),
                mirror_events: Coalescer::new(EVENT_INTERVAL, move || {
                    if let Some(scope) = mirror.upgrade() {
                        scope.send_event(MIRROR_EVENT, scope.app_event());
                    }
                }),
                closed: AtomicBool::new(false),
            }
        });
        let reconnected = Arc::downgrade(&scope);
        scope.connectivity.on_reconnect(Arc::new(move || {
            if let Some(scope) = reconnected.upgrade() {
                scope.wake();
                scope.retry_activations();
            }
        }));
        scope
    }

    fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
        mutex
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn app_id(&self) -> &str {
        &self.key.app_id
    }

    pub(crate) fn subject(&self) -> &str {
        &self.key.subject
    }

    pub(crate) fn hub(&self) -> &str {
        &self.key.hub
    }

    pub(crate) fn engine_root(&self) -> PathBuf {
        engine_dir(&self.root, &self.key.app_id, &self.id)
    }

    #[cfg(test)]
    pub(crate) fn connectivity(&self) -> Arc<ScopeConnectivity> {
        self.connectivity.clone()
    }

    pub(crate) fn descriptor(&self) -> ScopeDescriptor {
        Self::lock(&self.descriptor).clone()
    }

    /// Applies `change` and persists the descriptor; memory changes only after the write.
    /// A forgotten scope never writes its descriptor again.
    pub(crate) fn update_descriptor(
        &self,
        change: impl FnOnce(&mut ScopeDescriptor),
    ) -> Result<ScopeDescriptor> {
        let mut current = Self::lock(&self.descriptor);
        ensure!(!self.is_closed(), texts::FORGOTTEN);
        let mut next = current.clone();
        change(&mut next);
        write_descriptor(&self.root, &next)?;
        *current = next.clone();
        Ok(next)
    }

    /// Like `update_descriptor`, but writes only when `change` returns true.
    fn modify_descriptor(&self, change: impl FnOnce(&mut ScopeDescriptor) -> bool) -> Result<bool> {
        let mut current = Self::lock(&self.descriptor);
        ensure!(!self.is_closed(), texts::FORGOTTEN);
        let mut next = current.clone();
        if !change(&mut next) {
            return Ok(false);
        }
        write_descriptor(&self.root, &next)?;
        *current = next;
        Ok(true)
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Whether the account of this scope has a signed-in session on this device.
    pub(crate) fn signed_in(&self) -> bool {
        self.services
            .tokens
            .session_token(&self.key.hub, &self.key.subject)
            .is_some()
    }

    fn app_event(&self) -> AppEvent {
        AppEvent {
            app_id: self.key.app_id.clone(),
        }
    }

    pub(crate) fn send_event<T: Serialize>(&self, event: &str, payload: T) {
        if let Ok(value) = serde_json::to_value(payload) {
            (self.services.emit)(event, value);
        }
    }

    pub(crate) fn emit_tables(&self) {
        self.send_event(TABLES_EVENT, self.app_event());
    }

    pub(crate) fn emit_status(&self) {
        self.status_events.fire();
    }

    pub(crate) fn emit_mirror(&self) {
        self.mirror_events.fire();
    }

    fn emit_status_now(&self) {
        if let Some(payload) = self.status_event() {
            self.send_event(STATUS_EVENT, payload);
        }
    }

    pub(crate) fn status_event(&self) -> Option<StatusEvent> {
        let manager = self.manager.get()?;
        let status = manager.queue().status().ok()?;
        let blocked = manager
            .queue()
            .blocked_heads(MAX_BLOCKED_HEADS)
            .map(|heads| heads.len())
            .unwrap_or_default();
        Some(StatusEvent {
            app_id: self.key.app_id.clone(),
            pending_count: status.pending_count,
            sync_state: sync_state(blocked, self.transport(), status.pending_count),
            head_state: status.head.map(|head| head.state),
            blocked_count: blocked as u64,
        })
    }

    pub(crate) fn transport(&self) -> TransportStatus {
        self.host
            .get()
            .map(|host| host.transport())
            .unwrap_or(TransportStatus::Idle)
    }

    pub(crate) fn index(&self) -> Result<Arc<ObjectIndex>> {
        if let Some(index) = self.index.get() {
            return Ok(index.clone());
        }
        ensure!(!self.is_closed(), texts::FORGOTTEN);
        ensure_root(&self.root)?;
        private_directory(&scopes_dir(&self.root))?;
        let directory = scope_dir(&self.root, &self.id);
        private_directory(&directory)?;
        let index = Arc::new(ObjectIndex::open(&directory, self.roots.clone())?);
        Ok(self.index.get_or_init(|| index).clone())
    }

    pub(crate) fn cache_view(&self) -> Result<LocalCacheView> {
        Ok(LocalCacheView {
            index: Some(self.index()?),
            project: Arc::new(LocalObjectStore::new(self.dirs.project.clone())?),
            user: Arc::new(LocalObjectStore::new(self.dirs.user.clone())?),
            roots: self.roots.clone(),
        })
    }

    pub(crate) fn host(&self) -> Result<Arc<DesktopHost>> {
        if let Some(host) = self.host.get() {
            return Ok(host.clone());
        }
        let host = Arc::new(DesktopHost::new(
            self.this.clone(),
            &self.key,
            &self.installation,
            self.cache_view()?,
            self.connectivity.clone(),
        ));
        Ok(self.host.get_or_init(|| host).clone())
    }

    /// Object-key prefix of a buffered purpose, ending in '/'.
    pub(crate) fn location_prefix(&self, purpose: StoragePurpose) -> Option<String> {
        location_prefix(&self.key, purpose)
    }

    pub(crate) fn database_path(&self, purpose: OfflineTablePurpose) -> ObjectPath {
        let prefix = self.location_prefix(purpose.storage()).unwrap_or_default();
        ObjectPath::parse(format!("{prefix}db")).unwrap_or_default()
    }

    fn purpose_of(&self, database_path: &ObjectPath) -> Option<OfflineTablePurpose> {
        [OfflineTablePurpose::Storage, OfflineTablePurpose::User]
            .into_iter()
            .find(|purpose| self.database_path(*purpose) == *database_path)
    }

    pub(crate) fn provider(&self) -> String {
        self.descriptor()
            .provider
            .unwrap_or_else(|| "s3".to_owned())
    }

    fn file_routes(&self) -> Vec<(StoragePurpose, String)> {
        [
            StoragePurpose::Files,
            StoragePurpose::Storage,
            StoragePurpose::User,
        ]
        .into_iter()
        .filter_map(|purpose| Some((purpose, self.location_prefix(purpose)?)))
        .collect()
    }

    pub(crate) fn configured(
        &self,
        purpose: OfflineTablePurpose,
        table: &str,
    ) -> Option<ConfiguredTable> {
        self.descriptor().table(purpose, table).cloned()
    }

    pub(crate) fn configured_at(
        &self,
        database_path: &ObjectPath,
        table: &str,
    ) -> Option<ConfiguredTable> {
        self.configured(self.purpose_of(database_path)?, table)
    }

    /// Configured tables, an accepted hub, or queued work on disk need the engine.
    pub(crate) fn needs_manager(&self) -> bool {
        let descriptor = self.descriptor();
        !descriptor.tables.is_empty()
            || descriptor.hub_support == Some(true)
            || self.engine_root().exists()
    }

    pub(crate) fn manager_error(&self) -> Option<String> {
        Self::lock(&self.manager_error).clone()
    }

    pub(crate) fn open_manager(&self) -> Option<Arc<WriteManager>> {
        self.manager.get().cloned()
    }

    pub(crate) async fn manager(&self) -> Result<Arc<WriteManager>> {
        ensure!(!self.is_closed(), texts::FORGOTTEN);
        let start =
            || -> BoxFuture<'_, Result<Arc<WriteManager>>> { Box::pin(self.start_manager()) };
        match self.manager.get_or_try_init(start).await {
            Ok(manager) => {
                *Self::lock(&self.manager_error) = None;
                Ok(manager.clone())
            }
            Err(error) => {
                *Self::lock(&self.manager_error) = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn manager_options(&self) -> WriteManagerOptions {
        let descriptor = self.descriptor();
        WriteManagerOptions {
            parent: self.root.clone(),
            namespace: self.key.app_id.clone(),
            scope: self.id.clone(),
            limits: descriptor.limits.config(),
            replay_limits: descriptor.replay_limits(),
            refresh_interval: REFRESH_INTERVAL,
            idle_poll: IDLE_POLL,
            quarantine_other_scopes: false,
            lanes: QueueLanes::PerResource,
            coalesce_row_batches: true,
            fast_forward: Some(FAST_FORWARD),
            connectivity: Some(self.connectivity.clone() as Arc<dyn Connectivity>),
            recreate_dropped_tables: false,
            retired_snapshot_grace: Some(RETIRED_SNAPSHOT_GRACE),
            mirror: MirrorMode::Lazy(descriptor.limits.lazy()),
        }
    }

    async fn start_manager(&self) -> Result<Arc<WriteManager>> {
        ensure_root(&self.root)?;
        let manager = WriteManager::open(self.manager_options(), self.host()?)
            .await
            .context("Offline changes of this project could not be opened")?;
        self.recover(&manager).await;
        manager.spawn_drain();
        Ok(manager)
    }

    /// Startup recovery (§4.3): ready tables register active, settling ones held and then
    /// activate, preparing and failed ones are set up again once a lease is offered.
    async fn recover(&self, manager: &Arc<WriteManager>) {
        for table in self.descriptor().tables {
            let activation = match table.status {
                OfflineTableStatus::Ready => TableActivation::Active,
                OfflineTableStatus::Settling => TableActivation::Held,
                OfflineTableStatus::Preparing | OfflineTableStatus::Error => {
                    Self::lock(&self.recovery).push(table);
                    continue;
                }
            };
            let setup = TableSetup {
                activation,
                validate_key: false,
                prefetch: false,
            };
            match manager.add_table(table.selection(), setup).await {
                Ok(_) if activation == TableActivation::Held => {
                    if let Some(generation) = self.claim_setup(&table) {
                        let epoch = self.bump_activation_epoch();
                        self.schedule_activation(manager.clone(), table, generation, epoch);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    let text = setup_error(&table, &error);
                    self.set_status(&table, OfflineTableStatus::Error, Some(text));
                }
            }
        }
        if let Ok(states) = manager.table_states().await {
            let flags: Vec<(OfflineTablePurpose, String, bool)> = states
                .iter()
                .filter_map(|state| {
                    Some((
                        OfflineTablePurpose::from_storage(state.table.purpose)?,
                        state.table.table.clone(),
                        state.prefetch,
                    ))
                })
                .collect();
            let changed = self.descriptor().tables.iter().any(|table| {
                flags.iter().any(|(purpose, name, prefetch)| {
                    table.purpose == *purpose && table.table == *name && table.prefetch != *prefetch
                })
            });
            if changed
                && let Err(error) = self.update_descriptor(|descriptor| {
                    for table in &mut descriptor.tables {
                        if let Some((_, _, prefetch)) = flags.iter().find(|(purpose, name, _)| {
                            table.purpose == *purpose && table.table == *name
                        }) {
                            table.prefetch = *prefetch;
                        }
                    }
                })
            {
                tracing::warn!(%error, "Could not repair the Download everything flags of offline tables");
            }
        }
    }

    /// Refuses every later descriptor write, then closes the engine, waiting for an opening one.
    pub(crate) async fn close(&self) {
        {
            let _descriptor = Self::lock(&self.descriptor);
            self.closed.store(true, Ordering::Release);
        }
        let refuse = || async { Err::<Arc<WriteManager>, _>(anyhow!(texts::FORGOTTEN)) };
        if let Ok(manager) = self.manager.get_or_try_init(refuse).await
            && let Err(error) = manager.close().await
        {
            tracing::warn!(%error, "Offline changes of a forgotten project did not close cleanly");
        }
    }

    pub(crate) fn wake(&self) {
        if let Some(manager) = self.manager.get() {
            manager.wake();
        }
    }

    pub(crate) fn hosted_run_started(&self) -> HostedRunTicket {
        let id = self.fence.next.fetch_add(1, Ordering::AcqRel);
        let epoch = self.fence.epoch.load(Ordering::Acquire);
        self.fence.alive().insert(id, epoch);
        HostedRunTicket {
            fence: self.fence.clone(),
            id,
        }
    }

    pub(crate) fn bump_activation_epoch(&self) -> u64 {
        self.fence.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) async fn wait_for_hosted_runs_before(&self, epoch: u64) {
        loop {
            let changed = self.fence.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.fence.older_than(epoch) == 0 {
                return;
            }
            changed.await;
        }
    }

    /// Hosted runs a settling table still waits for.
    pub(crate) fn waiting_for_runs(&self, table: &ConfiguredTable) -> u32 {
        Self::lock(&self.activations)
            .get(&table.resource())
            .filter(|pending| pending.primary_key == table.primary_key)
            .and_then(|pending| pending.epoch)
            .map_or(0, |epoch| self.fence.older_than(epoch))
    }

    pub(crate) fn lease(&self) -> Option<Arc<RenewableSharedCredentials>> {
        Self::lock(&self.lease).upgrade()
    }

    /// Keeps a lease weakly for mirror downloads and replays; its idle pruning keeps working.
    pub(crate) fn remember_lease(&self, lease: Weak<RenewableSharedCredentials>) {
        *Self::lock(&self.lease) = lease;
    }

    /// A Hosted run or the enable command proved the hub reachable with a fresh lease.
    pub(crate) fn offer_lease(&self, lease: Weak<RenewableSharedCredentials>) {
        let provider = lease
            .upgrade()
            .and_then(|live| provider_of(live.initial()).map(str::to_owned));
        self.remember_lease(lease);
        if let Some(provider) = provider
            && self.descriptor().provider.is_none()
            && let Err(error) = self.update_descriptor(|descriptor| {
                descriptor.provider.get_or_insert(provider);
            })
        {
            tracing::debug!(%error, "Could not record the storage provider of an offline scope");
        }
        self.resume_setup();
    }

    /// Retries setups interrupted at startup and activations that could not read the cloud.
    pub(crate) fn resume_setup(&self) {
        let pending: Vec<ConfiguredTable> = std::mem::take(&mut *Self::lock(&self.recovery));
        for table in pending {
            if let Some(scope) = self.this.upgrade() {
                tokio::spawn(async move { scope.configure_table(table).await });
            }
        }
        self.retry_activations();
    }

    /// Adds `{eventId, blake3(token)}` for replay token lookup; oldest dropped beyond 32.
    pub(crate) fn record_sink(&self, event_id: &str, token: &str) -> Result<()> {
        let binding = SinkBinding {
            event_id: event_id.to_owned(),
            token_digest: blake3::hash(token.as_bytes()).to_hex().to_string(),
        };
        if self.descriptor().sinks.contains(&binding) {
            return Ok(());
        }
        self.update_descriptor(|descriptor| {
            descriptor
                .sinks
                .retain(|sink| sink.event_id != binding.event_id);
            descriptor.sinks.push(binding);
            let excess = descriptor.sinks.len().saturating_sub(MAX_SINKS);
            descriptor.sinks.drain(..excess);
        })?;
        Ok(())
    }

    pub(crate) fn capabilities_stale(&self) -> bool {
        let checked = self.descriptor().capabilities_checked_at;
        let now = unix_time().unwrap_or_default();
        checked.is_none_or(|checked| now.saturating_sub(checked) > CAPABILITIES_MAX_AGE)
    }

    /// `GET …/invoke/offline/capabilities` (§4.9). Some(true) on 200, Some(false) on 404/405;
    /// other failures keep the previous values and return the error.
    pub(crate) async fn refresh_capabilities(&self, token: &str) -> Result<bool> {
        let url = format!(
            "{}{}",
            api_base(&self.key.hub),
            desktop_offline_capabilities_path(&self.key.app_id)
        );
        let response = self
            .http
            .as_ref()
            .map_err(|error| {
                anyhow!("The offline capabilities client could not be created: {error}")
            })?
            .get(&url)
            .header(
                flow_like_types::reqwest::header::AUTHORIZATION,
                authorization(token),
            )
            .send()
            .await?;
        let status = response.status().as_u16();
        let now = unix_time()?;
        match status {
            200 => {
                let capabilities: DesktopOfflineCapabilities = response.json().await?;
                let limits = clamp_limits(capabilities.limits);
                let provider = match capabilities.provider {
                    OfflineContentProvider::S3 => "s3",
                    OfflineContentProvider::Az => "az",
                    OfflineContentProvider::Gs => "gs",
                };
                let descriptor = self.update_descriptor(|descriptor| {
                    descriptor.provider = Some(provider.to_owned());
                    descriptor.hub_support = Some(true);
                    descriptor.hub_limits = Some(limits);
                    descriptor.capabilities_checked_at = Some(now);
                })?;
                if let Some(manager) = self.manager.get()
                    && let Err(error) = manager
                        .set_limits(descriptor.limits.config(), limits, None)
                        .await
                {
                    tracing::warn!(%error, "Could not apply the hub's offline sync limits");
                }
                Ok(true)
            }
            404 | 405 => {
                self.update_descriptor(|descriptor| {
                    descriptor.hub_support = Some(false);
                    descriptor.capabilities_checked_at = Some(now);
                })?;
                Ok(false)
            }
            status => bail!("the hub answered HTTP {status} for {url}"),
        }
    }

    /// Hosted runs refresh a day-old capabilities answer in the background.
    pub(crate) fn refresh_capabilities_in_background(&self, token: &str) {
        if self.is_closed() || !self.capabilities_stale() {
            return;
        }
        let Some(scope) = self.this.upgrade() else {
            return;
        };
        let token = token.to_owned();
        tokio::spawn(async move {
            if let Err(error) = scope.refresh_capabilities(&token).await {
                tracing::debug!(%error, "Offline capabilities of the hub could not be refreshed");
            }
        });
    }

    /// Sync: managed means registered with the engine; while the engine cannot open,
    /// configured tables count as managed so Open Database reports E7.
    pub(crate) fn is_managed(&self, database_path: &ObjectPath, table: &str) -> bool {
        match self.manager.get() {
            Some(manager) => manager.table_is_managed(database_path, table),
            None => {
                self.manager_error().is_some() && self.configured_at(database_path, table).is_some()
            }
        }
    }

    /// The per-run database decorator of online apps (§4.4).
    pub(crate) async fn decorate(
        &self,
        database_path: &ObjectPath,
        store: LanceDBVectorStore,
        mode: RunMode,
    ) -> flow_like_types::Result<LanceDBVectorStore> {
        let table = store.table_name().to_owned();
        if self.configured_at(database_path, &table).is_none() {
            return match mode {
                RunMode::Hosted => Ok(store),
                RunMode::Disconnected => Err(anyhow!(texts::table_unavailable(&table))),
            };
        }
        if let Some(error) = self.manager_error() {
            bail!(texts::unavailable(error));
        }
        let registered = self
            .manager
            .get()
            .cloned()
            .filter(|manager| manager.table_is_managed(database_path, &table));
        let Some(manager) = registered else {
            return match mode {
                RunMode::Hosted => Ok(store),
                RunMode::Disconnected => Err(anyhow!(texts::table_not_ready(&table))),
            };
        };
        if !manager.managed_table_names(database_path).contains(&table) {
            return manager.decorate(database_path, store).await;
        }
        LanceDBVectorStore::validate_overlay_selector(&store.selector())?;
        manager
            .decorate(database_path, store)
            .await
            .map_err(|error| match engine_refusal(&error) {
                true => error,
                false => anyhow!(texts::local_copy_unavailable(&table, &error)),
            })
    }

    /// E32 for an active table whose copy was confirmed more than 15 minutes ago.
    pub(crate) fn table_notice(&self, database_path: &ObjectPath, table: &str) -> Option<String> {
        let configured = self.configured_at(database_path, table)?;
        if self.connectivity.is_offline() {
            return None;
        }
        let manager = self.manager.get()?;
        if !manager
            .managed_table_names(database_path)
            .iter()
            .any(|name| name == table)
        {
            return None;
        }
        let resource = configured.resource();
        let refreshed = manager.queue().refreshed_at(&resource).ok().flatten()?;
        if unix_time().ok()?.saturating_sub(refreshed) <= STALE_NOTICE_AFTER {
            return None;
        }
        let reason = manager
            .queue()
            .table_mirror_error(&resource)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                if manager.queue().has_pending(&resource).unwrap_or(false) {
                    "its queued changes are not synced yet".to_owned()
                } else {
                    "the next refresh has not run yet".to_owned()
                }
            });
        Some(texts::stale_copy(table, &format_time(refreshed), &reason))
    }

    /// Configured tables that are not registered with the engine yet, for E3 texts.
    pub(crate) fn unready_tables(&self) -> UnreadyTables {
        let mut unready = UnreadyTables::default();
        let manager = self.manager.get();
        for table in self.descriptor().tables {
            let path = self.database_path(table.purpose);
            if manager.is_some_and(|manager| manager.table_is_managed(&path, &table.table)) {
                continue;
            }
            match table.purpose {
                OfflineTablePurpose::Storage => unready.project.insert(table.table),
                OfflineTablePurpose::User => unready.user.insert(table.table),
            };
        }
        unready
    }

    pub(crate) fn content_decorator(&self) -> Arc<dyn ContentStoreDecorator> {
        Arc::new(ScopeContentDecorator {
            scope: self.this.clone(),
        })
    }

    fn overlay_store(
        &self,
        manager: &Arc<WriteManager>,
        cloud: Option<FlowLikeStore>,
        cache: LocalCacheView,
        connectivity: Arc<dyn Connectivity>,
    ) -> Result<DesktopFileStore> {
        let view = DesktopCloudView::new(
            cloud.as_ref().map(FlowLikeStore::as_generic),
            cache.clone(),
            self.connectivity.clone(),
        );
        let options = file_options(
            &self.file_routes(),
            &self.provider(),
            connectivity,
            self.descriptor().replay_limits().max_file_bytes,
        );
        let overlay = manager.file_overlay(Arc::new(view), options)?;
        Ok(DesktopFileStore {
            layer: FileLayer::Overlay(overlay),
            cloud,
            cache,
        })
    }

    /// Hosted content decoration: the scope's current manager, or the store unchanged.
    pub(crate) fn decorate_content(&self, store: FlowLikeStore) -> FlowLikeStore {
        if self.is_closed()
            || self.descriptor().hub_support != Some(true)
            || self.manager_error().is_some()
        {
            return store;
        }
        let Some(manager) = self.manager.get().cloned() else {
            return store;
        };
        let decorated = self.cache_view().and_then(|cache| {
            self.overlay_store(
                &manager,
                Some(store.clone()),
                cache,
                self.connectivity.clone(),
            )
        });
        match decorated {
            Ok(decorated) => FlowLikeStore::Signed(Arc::new(decorated)),
            Err(error) => {
                tracing::warn!(%error, "Offline file buffering is unavailable for this run");
                store
            }
        }
    }

    /// The content store of a Disconnected run: every buffered write is queued.
    pub(crate) fn disconnected_store(&self) -> Result<DesktopFileStore> {
        let cache = self.cache_view()?;
        let refusal = if self.descriptor().hub_support != Some(true) {
            Some(Refusal::HubUnsupported)
        } else {
            self.manager_error().map(Refusal::Unavailable)
        };
        let manager = match (refusal, self.manager.get()) {
            (Some(refusal), _) => {
                return Ok(DesktopFileStore {
                    layer: FileLayer::Refused(refusal),
                    cloud: None,
                    cache,
                });
            }
            (None, Some(manager)) => manager.clone(),
            (None, None) => {
                return Ok(DesktopFileStore {
                    layer: FileLayer::Refused(Refusal::Unavailable(
                        "the offline queue of this project is not open".into(),
                    )),
                    cloud: None,
                    cache,
                });
            }
        };
        self.overlay_store(
            &manager,
            None,
            cache,
            Arc::new(DisconnectedConnectivity(self.connectivity.clone())),
        )
    }

    fn set_status(
        &self,
        table: &ConfiguredTable,
        status: OfflineTableStatus,
        error: Option<String>,
    ) {
        let result = self.update_descriptor(|descriptor| {
            if let Some(entry) = descriptor
                .tables
                .iter_mut()
                .find(|entry| entry.purpose == table.purpose && entry.table == table.table)
            {
                entry.status = status;
                entry.error = error;
            }
        });
        if let Err(error) = result {
            tracing::warn!(%error, table = %table.table, "Could not record the offline status of a table");
        }
        self.emit_tables();
    }

    /// Enable steps 1-5 after the command's checks: records the table as preparing.
    pub(crate) async fn begin_table(
        &self,
        selection: &OfflineTableSelection,
    ) -> Result<ConfiguredTable> {
        let table = ConfiguredTable::from_selection(selection)?;
        if let Some(existing) = self.configured(table.purpose, &table.table) {
            if existing.primary_key == table.primary_key
                && existing.status != OfflineTableStatus::Error
            {
                return Ok(existing);
            }
            if existing.primary_key != table.primary_key {
                let manager = self.manager().await?;
                ensure!(
                    !manager.queue().has_pending(&existing.resource())?,
                    texts::key_change_pending(&existing.table)
                );
                if manager.table_is_managed(&self.database_path(existing.purpose), &existing.table)
                {
                    manager.remove_table(&existing.selection()).await?;
                }
            }
            Self::lock(&self.activations).remove(&existing.resource());
        }
        self.update_descriptor(|descriptor| {
            descriptor
                .tables
                .retain(|entry| !(entry.purpose == table.purpose && entry.table == table.table));
            descriptor.tables.push(table.clone());
        })?;
        self.emit_tables();
        Ok(table)
    }

    /// A preparing table whose setup no task of this process runs: new, failed or interrupted.
    pub(crate) fn setup_pending(&self, table: &ConfiguredTable) -> bool {
        table.status == OfflineTableStatus::Preparing
            && !Self::lock(&self.activations)
                .get(&table.resource())
                .is_some_and(|pending| pending.primary_key == table.primary_key)
    }

    /// Starts a setup generation of `table`; None while a setup of the same key runs.
    fn claim_setup(&self, table: &ConfiguredTable) -> Option<u64> {
        let mut activations = Self::lock(&self.activations);
        if activations
            .get(&table.resource())
            .is_some_and(|pending| pending.primary_key == table.primary_key)
        {
            return None;
        }
        let generation = self.generations.fetch_add(1, Ordering::AcqRel) + 1;
        activations.insert(
            table.resource(),
            PendingActivation {
                generation,
                primary_key: table.primary_key.clone(),
                epoch: None,
                last_attempt: None,
            },
        );
        Some(generation)
    }

    /// Applies `change` to the setup of `generation`; false once it was replaced or forgotten.
    fn update_setup(
        &self,
        table: &ConfiguredTable,
        generation: u64,
        change: impl FnOnce(&mut PendingActivation),
    ) -> bool {
        if self.is_closed() {
            return false;
        }
        let mut activations = Self::lock(&self.activations);
        match activations.get_mut(&table.resource()) {
            Some(pending)
                if pending.generation == generation && pending.primary_key == table.primary_key =>
            {
                change(pending);
                true
            }
            _ => false,
        }
    }

    fn setup_current(&self, table: &ConfiguredTable, generation: u64) -> bool {
        self.update_setup(table, generation, |_| ())
    }

    fn end_setup(&self, table: &ConfiguredTable, generation: u64) {
        let mut activations = Self::lock(&self.activations);
        if activations
            .get(&table.resource())
            .is_some_and(|pending| pending.generation == generation)
        {
            activations.remove(&table.resource());
        }
    }

    fn setup_lock(&self, table: &ConfiguredTable) -> Arc<tokio::sync::Mutex<()>> {
        Self::lock(&self.setup_locks)
            .entry(table.resource())
            .or_default()
            .clone()
    }

    /// Changes the setup's descriptor entry unless a newer setup, a removal or `forget`
    /// replaced it; emits `tables-changed` when written.
    fn record_setup(
        &self,
        table: &ConfiguredTable,
        generation: u64,
        update: impl FnOnce(&mut ConfiguredTable),
    ) -> bool {
        let written = self.modify_descriptor(|descriptor| {
            if !self.setup_current(table, generation) {
                return false;
            }
            let Some(entry) = descriptor.tables.iter_mut().find(|entry| {
                entry.purpose == table.purpose
                    && entry.table == table.table
                    && entry.primary_key == table.primary_key
            }) else {
                return false;
            };
            update(entry);
            true
        });
        match written {
            Ok(written) => {
                if written {
                    self.emit_tables();
                }
                written
            }
            Err(error) => {
                tracing::warn!(%error, table = %table.table, "Could not record the offline status of a table");
                false
            }
        }
    }

    fn fail_setup(&self, table: &ConfiguredTable, generation: u64, error: String) {
        self.record_setup(table, generation, |entry| {
            entry.status = OfflineTableStatus::Error;
            entry.error = Some(error);
        });
        self.end_setup(table, generation);
    }

    /// Enable step 6: validate and register held, wait for older Hosted runs, activate.
    /// Stops after any await once a newer setup, a removal or `forget` replaced it.
    pub(crate) async fn configure_table(self: Arc<Self>, table: ConfiguredTable) {
        let Some(generation) = self.claim_setup(&table) else {
            return;
        };
        let manager = match self.manager().await {
            Ok(manager) => manager,
            Err(error) => {
                self.fail_setup(&table, generation, texts::unavailable(&error));
                return;
            }
        };
        let lock = self.setup_lock(&table);
        let registration = lock.lock().await;
        if !self.setup_current(&table, generation) {
            return;
        }
        let state = match self.register_held(&manager, &table).await {
            Ok(state) => state,
            Err(error) => {
                self.fail_setup(&table, generation, setup_error(&table, &error));
                return;
            }
        };
        if !self.setup_current(&table, generation) {
            self.discard_registration(&manager, &table).await;
            return;
        }
        let notice = self
            .apply_prefetch(&manager, &table, generation, &state)
            .await;
        drop(registration);
        if state.activation == Some(TableActivation::Active) {
            self.record_setup(&table, generation, |entry| {
                entry.status = OfflineTableStatus::Ready;
                entry.error = notice;
            });
            self.end_setup(&table, generation);
            return;
        }
        let epoch = self.bump_activation_epoch();
        if !self.update_setup(&table, generation, |pending| pending.epoch = Some(epoch)) {
            return;
        }
        self.record_setup(&table, generation, |entry| {
            entry.status = OfflineTableStatus::Settling;
            entry.error = notice;
        });
        self.wait_for_hosted_runs_before(epoch).await;
        self.activate(&manager, &table, generation).await;
    }

    /// Registers `table` held; a stale registration of another key column is replaced.
    async fn register_held(
        &self,
        manager: &Arc<WriteManager>,
        table: &ConfiguredTable,
    ) -> Result<TableState> {
        let wanted = self
            .configured(table.purpose, &table.table)
            .filter(|current| current.primary_key == table.primary_key)
            .is_none_or(|current| current.prefetch);
        let setup = TableSetup {
            activation: TableActivation::Held,
            validate_key: true,
            prefetch: table.prefetch && wanted,
        };
        let state = manager.add_table(table.selection(), setup).await?;
        if state.table == table.selection() {
            return Ok(state);
        }
        manager.remove_table(&state.table).await?;
        manager.add_table(table.selection(), setup).await
    }

    /// A setup replaced while it registered the table unregisters it, unless the current
    /// entry keeps the same key column.
    async fn discard_registration(&self, manager: &Arc<WriteManager>, table: &ConfiguredTable) {
        let kept = self
            .configured(table.purpose, &table.table)
            .is_some_and(|current| current.primary_key == table.primary_key);
        if kept || self.is_closed() {
            return;
        }
        if let Err(error) = manager.remove_table(&table.selection()).await {
            tracing::warn!(%error, table = %table.table, "Could not unregister a table turned off during setup");
        }
    }

    /// Applies a Download everything choice made while the table was being prepared. A
    /// refusal (E31) turns it off again and is returned as the table's error text.
    async fn apply_prefetch(
        &self,
        manager: &Arc<WriteManager>,
        table: &ConfiguredTable,
        generation: u64,
        state: &TableState,
    ) -> Option<String> {
        let wanted = self
            .configured(table.purpose, &table.table)
            .filter(|current| current.primary_key == table.primary_key)?
            .prefetch;
        if wanted == state.prefetch {
            return None;
        }
        let refusal = manager
            .set_prefetch(&table.selection(), wanted)
            .await
            .err()?
            .to_string();
        let engine = state.prefetch;
        self.record_setup(table, generation, |entry| entry.prefetch = engine);
        Some(refusal)
    }

    fn schedule_activation(
        &self,
        manager: Arc<WriteManager>,
        table: ConfiguredTable,
        generation: u64,
        epoch: u64,
    ) {
        if !self.update_setup(&table, generation, |pending| pending.epoch = Some(epoch)) {
            return;
        }
        let Some(scope) = self.this.upgrade() else {
            return;
        };
        tokio::spawn(async move {
            scope.wait_for_hosted_runs_before(epoch).await;
            scope.activate(&manager, &table, generation).await;
        });
    }

    async fn activate(
        &self,
        manager: &Arc<WriteManager>,
        table: &ConfiguredTable,
        generation: u64,
    ) -> bool {
        let lock = self.setup_lock(table);
        let _registration = lock.lock().await;
        if !self.update_setup(table, generation, |pending| {
            pending.last_attempt = Some(Instant::now());
        }) {
            return false;
        }
        match manager.activate_table(&table.selection()).await {
            Ok(_) => {
                self.record_setup(table, generation, |entry| {
                    entry.status = OfflineTableStatus::Ready;
                });
                self.end_setup(table, generation);
                self.emit_status();
                true
            }
            Err(error) => {
                tracing::info!(%error, table = %table.table, "Offline table activation waits for the cloud table");
                self.emit_tables();
                false
            }
        }
    }

    /// Retries failed activations whose older runs ended, at most every retry interval.
    pub(crate) fn retry_activations(&self) {
        let Some(manager) = self.manager.get().cloned() else {
            return;
        };
        let Some(scope) = self.this.upgrade() else {
            return;
        };
        let tables = self.descriptor().tables;
        let retry = self.services.activation_retry;
        let due: Vec<(ConfiguredTable, u64)> = {
            let mut activations = Self::lock(&self.activations);
            activations
                .iter_mut()
                .filter(|(_, pending)| {
                    pending
                        .last_attempt
                        .is_some_and(|attempt| attempt.elapsed() >= retry)
                        && pending
                            .epoch
                            .is_some_and(|epoch| self.fence.older_than(epoch) == 0)
                })
                .filter_map(|(resource, pending)| {
                    let table = tables.iter().find(|table| {
                        table.resource() == *resource && table.primary_key == pending.primary_key
                    })?;
                    pending.last_attempt = Some(Instant::now());
                    Some((table.clone(), pending.generation))
                })
                .collect()
        };
        for (table, generation) in due {
            let scope = scope.clone();
            let manager = manager.clone();
            tokio::spawn(async move {
                scope.activate(&manager, &table, generation).await;
            });
        }
    }

    /// Disable (§4.9): refuses while the table has queued changes.
    pub(crate) async fn remove_table(
        &self,
        purpose: OfflineTablePurpose,
        table: &str,
    ) -> Result<()> {
        let configured = self
            .configured(purpose, table)
            .with_context(|| format!("Table '{table}' is not available offline on this device"))?;
        let manager = self.manager().await?;
        let path = self.database_path(purpose);
        if manager.queue().has_pending(&configured.resource())? {
            bail!(texts::disable_pending(table));
        }
        if manager.table_is_managed(&path, table) {
            manager
                .remove_table(&configured.selection())
                .await
                .map_err(
                    |error| match error.to_string().contains("still has queued changes") {
                        true => anyhow!(texts::disable_pending(table)),
                        false => error,
                    },
                )?;
        }
        Self::lock(&self.activations).remove(&configured.resource());
        self.update_descriptor(|descriptor| {
            descriptor
                .tables
                .retain(|entry| !(entry.purpose == purpose && entry.table == table));
        })?;
        self.emit_tables();
        self.emit_status();
        Ok(())
    }

    /// Download everything: the engine's flag first, the descriptor after it accepted.
    pub(crate) async fn set_prefetch(
        &self,
        purpose: OfflineTablePurpose,
        table: &str,
        prefetch: bool,
    ) -> Result<OfflineTableState> {
        let configured = self
            .configured(purpose, table)
            .with_context(|| format!("Table '{table}' is not available offline on this device"))?;
        let manager = self.manager().await?;
        let registered = manager.table_is_managed(&self.database_path(purpose), table);
        let engine = if registered {
            Some(
                manager
                    .set_prefetch(&configured.selection(), prefetch)
                    .await?,
            )
        } else {
            None
        };
        self.update_descriptor(|descriptor| {
            if let Some(entry) = descriptor
                .tables
                .iter_mut()
                .find(|entry| entry.purpose == purpose && entry.table == table)
            {
                entry.prefetch = engine.as_ref().map_or(prefetch, |state| state.prefetch);
                if engine.is_some() && entry.status != OfflineTableStatus::Error {
                    entry.error = None;
                }
            }
        })?;
        self.emit_tables();
        let configured = self.configured(purpose, table).unwrap_or(configured);
        Ok(table_state(
            &configured,
            engine.as_ref(),
            self.waiting_for_runs(&configured),
        ))
    }

    /// Queue and mirror limits; the engine applies them first, the descriptor follows.
    pub(crate) async fn set_limits(&self, limits: OfflineLimitsDto) -> Result<OfflineLimitsDto> {
        limits.config().validate_limits()?;
        self.manager()
            .await?
            .set_limits(
                limits.config(),
                self.descriptor().replay_limits(),
                Some(limits.lazy()),
            )
            .await?;
        self.update_descriptor(|descriptor| descriptor.limits = limits)?;
        self.emit_mirror();
        Ok(limits)
    }

    /// Wakes the drain and refreshes every idle active table in the background.
    pub(crate) async fn sync_now(&self) -> Result<()> {
        let manager = self.manager().await?;
        manager.wake();
        let idle: Vec<BufferedTable> = manager
            .table_states()
            .await?
            .into_iter()
            .filter(|state| state.activation == Some(TableActivation::Active) && state.pending == 0)
            .map(|state| state.table)
            .collect();
        let scope = self.this.upgrade();
        tokio::spawn(async move {
            for table in idle {
                if let Err(error) = manager.refresh_table(&table).await {
                    tracing::debug!(%error, table = %table.table, "Sync now could not refresh an offline table");
                }
            }
            if let Some(scope) = scope {
                scope.emit_tables();
                scope.emit_mirror();
            }
        });
        Ok(())
    }

    pub(crate) async fn retry(&self, operation_id: &str) -> Result<()> {
        let manager = self.manager().await?;
        manager.queue().retry(operation_id)?;
        manager.wake();
        self.emit_status();
        Ok(())
    }

    pub(crate) async fn skip(
        &self,
        operation_id: &str,
        reason: &str,
        acknowledge_uncertain: bool,
    ) -> Result<()> {
        let manager = self.manager().await?;
        manager.queue().request_skip(
            operation_id,
            &format!("desktop:{}: {reason}", self.key.subject),
            acknowledge_uncertain,
        )?;
        manager.wake();
        self.emit_status();
        Ok(())
    }

    /// Keep both (§4.9): uploads the conflicting bytes under a new name and skips the change.
    pub(crate) async fn keep_both(&self, operation_id: &str) -> Result<String> {
        let manager = self.manager().await?;
        let lookup = manager
            .queue()
            .operation_state(operation_id)?
            .with_context(|| {
                format!("Offline change '{operation_id}' does not exist on this device")
            })?;
        ensure!(
            lookup.state == "conflict",
            "Keep both needs a file change in conflict; offline change '{operation_id}' is {}",
            lookup.state
        );
        let (resource, bytes) = manager
            .file_payload(operation_id)?
            .with_context(|| format!("Offline change '{operation_id}' keeps no file contents"))?;
        let OfflineResource::File { purpose, path } = resource else {
            bail!(
                "Keep both applies to file changes; offline change '{operation_id}' changes a table"
            );
        };
        let prefix = self
            .location_prefix(purpose)
            .with_context(|| format!("Files of purpose {purpose:?} cannot be kept offline"))?;
        let index = self.index()?;
        let taken = |candidate: &str| {
            ObjectPath::parse(format!("{prefix}{candidate}"))
                .ok()
                .and_then(|key| index.lookup(&key).ok())
                .is_some_and(|known| matches!(known, Known::Present(_)))
        };
        let new_path = keep_both_name(&path, chrono::Utc::now(), taken);
        let reason = kept_as_reason(&self.key.subject, &new_path)
            .with_context(|| format!("Keep both cannot skip offline change '{operation_id}'"))?;
        let target = ObjectPath::parse(format!("{prefix}{new_path}"))?;
        let store = self.keep_both_store(&manager).await?;
        store.put(&target, bytes.into()).await.with_context(|| {
            format!("Could not save '{new_path}' for offline change '{operation_id}'")
        })?;
        manager.queue().request_skip(operation_id, &reason, true)?;
        manager.wake();
        self.emit_status();
        Ok(new_path)
    }

    /// The lease's content store behind the overlay while the hub is reachable, else the queue.
    async fn keep_both_store(&self, manager: &Arc<WriteManager>) -> Result<DesktopFileStore> {
        if !self.connectivity.is_offline()
            && let Ok(Some(cloud)) = self.services.cloud.objects(&*self.host()?).await
        {
            return self.overlay_store(
                manager,
                Some(FlowLikeStore::Other(cloud)),
                self.cache_view()?,
                self.connectivity.clone(),
            );
        }
        self.overlay_store(
            manager,
            None,
            self.cache_view()?,
            Arc::new(DisconnectedConnectivity(self.connectivity.clone())),
        )
    }

    pub(crate) fn observe_unreachable_hub(&self) {
        self.connectivity.observe(Observation::ConnectFailed);
    }
}

/// Installed on a lease; resolves the scope and its current manager at every call.
struct ScopeContentDecorator {
    scope: Weak<AppOfflineScope>,
}

impl ContentStoreDecorator for ScopeContentDecorator {
    fn decorate(&self, store: FlowLikeStore) -> flow_like_types::Result<FlowLikeStore> {
        Ok(match self.scope.upgrade() {
            Some(scope) => scope.decorate_content(store),
            None => store,
        })
    }
}

pub(crate) fn location_prefix(key: &ScopeKey, purpose: StoragePurpose) -> Option<String> {
    match purpose {
        StoragePurpose::Files => Some(format!("apps/{}/upload/", key.app_id)),
        StoragePurpose::Storage => Some(format!("apps/{}/storage/", key.app_id)),
        StoragePurpose::User => Some(format!(
            "{}/",
            ObjectPath::from("users")
                .join(key.subject.as_str())
                .join("apps")
                .join(key.app_id.as_str())
        )),
        StoragePurpose::Temporary => None,
    }
}

pub(crate) fn provider_of(credentials: &SharedCredentials) -> Option<&'static str> {
    match credentials {
        SharedCredentials::Aws(_) => Some("s3"),
        SharedCredentials::Azure(_) => Some("az"),
        SharedCredentials::Gcp(_) => Some("gs"),
        SharedCredentials::Mixed(mixed) => provider_of(&mixed.content),
        SharedCredentials::Renewable(live) => provider_of(live.initial()),
    }
}

fn clamp_limits(limits: OfflineLimits) -> OfflineLimits {
    OfflineLimits {
        max_operation_bytes: limits.max_operation_bytes.min(MAX_OFFLINE_OPERATION_BYTES),
        max_file_bytes: limits.max_file_bytes.min(MAX_OFFLINE_OPERATION_BYTES),
        max_request_bytes: limits
            .max_request_bytes
            .map(|bytes| bytes.min(MAX_OFFLINE_REPLAY_HTTP_BYTES)),
    }
}

/// E34 for a table the cloud does not have; other setup failures keep the engine's text.
fn setup_error(table: &ConfiguredTable, error: &anyhow::Error) -> String {
    let text = error.to_string();
    if text.starts_with(&format!(
        "Table '{}' does not exist in the cloud",
        table.table
    )) {
        return texts::missing_in_cloud(&table.table);
    }
    text
}

/// Engine refusals that already name the table's state (E23, E24, E25, selectors, closed).
fn engine_refusal(error: &anyhow::Error) -> bool {
    let text = error.to_string();
    [
        "is being prepared for offline use",
        "was turned off while this run was using it",
        "was deleted in the cloud",
        "expose only their current version",
        texts::FORGOTTEN,
    ]
    .iter()
    .any(|marker| text.contains(marker))
}

fn format_time(seconds: i64) -> String {
    chrono::DateTime::from_timestamp(seconds, 0)
        .map(|time| time.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_else(|| seconds.to_string())
}

/// `{dir}/{stem} (offline copy {yyyy-mm-dd HHmmss}){.ext}`, numbered while `taken`.
pub(crate) fn keep_both_name(
    path: &str,
    now: chrono::DateTime<chrono::Utc>,
    taken: impl Fn(&str) -> bool,
) -> String {
    let (directory, name) = match path.rsplit_once('/') {
        Some((directory, name)) => (format!("{directory}/"), name),
        None => (String::new(), path),
    };
    let (stem, extension) = match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => (stem, format!(".{extension}")),
        _ => (name, String::new()),
    };
    let stamp = now.format("%Y-%m-%d %H%M%S");
    (1..)
        .map(|attempt| match attempt {
            1 => format!("{directory}{stem} (offline copy {stamp}){extension}"),
            n => format!("{directory}{stem} (offline copy {stamp} {n}){extension}"),
        })
        .find(|candidate| !taken(candidate))
        .unwrap_or_default()
}

/// `desktop:{subject}: kept as {path}` within the engine's skip reason limit; a long path
/// keeps its end.
pub(crate) fn kept_as_reason(subject: &str, new_path: &str) -> Result<String> {
    let prefix = format!("desktop:{subject}: kept as ");
    if prefix.len() + new_path.len() <= MAX_SKIP_REASON {
        return Ok(format!("{prefix}{new_path}"));
    }
    let room = MAX_SKIP_REASON.saturating_sub(prefix.len() + ELLIPSIS.len());
    ensure!(
        room > 0,
        "its skip reason for account '{subject}' would be longer than {MAX_SKIP_REASON} bytes"
    );
    let mut start = new_path.len() - room;
    while !new_path.is_char_boundary(start) {
        start += 1;
    }
    Ok(format!("{prefix}{ELLIPSIS}{}", &new_path[start..]))
}

/// `Blocked` > transport waits > `Syncing` > `Idle` (§4.11).
pub(crate) fn sync_state(
    blocked_heads: usize,
    transport: TransportStatus,
    pending: u64,
) -> OfflineSyncState {
    if blocked_heads > 0 {
        return OfflineSyncState::Blocked;
    }
    if pending == 0 {
        return OfflineSyncState::Idle;
    }
    match transport {
        TransportStatus::WaitingForSignIn => OfflineSyncState::WaitingForSignIn,
        TransportStatus::WaitingForConnection => OfflineSyncState::WaitingForConnection,
        TransportStatus::HubError => OfflineSyncState::HubError,
        TransportStatus::Idle => OfflineSyncState::Syncing,
    }
}
