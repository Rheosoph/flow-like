use crate::{
    BufferedTable, BufferingConfig, Outbox,
    files::{self, FileOverlay, FileOverlayOptions},
    fs,
    host::{Connectivity, OfflineHost, ReplayErrorKind},
    limits::{ReplayLimits, RequestError, validate_request},
    outbox::{CLOSED, OutboxOptions, QueueLanes, QueuedOperation},
    table::TableOverlay,
};
use anyhow::{Context, Result, ensure};
use bytes::Bytes;
use flow_like_device_protocol::{
    INSTANCE_OFFLINE_LIMITS, OfflineExpected, OfflineLimits, OfflineMutation, OfflineReplayRequest,
    OfflineReplayStatus, OfflineResource,
};
use flow_like_storage::{
    databases::vector::{
        lancedb::{DatabaseSelector, LanceDBVectorStore},
        offline_replay,
    },
    lancedb::Connection,
    object_store::{ObjectStore, path::Path as ObjectPath},
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Mutex, MutexGuard, Notify},
    task::JoinHandle,
};

const KEY_VALIDATION_MEMORY: usize = 256 * 1024 * 1024;
const MAX_COALESCED: usize = 1024;
const REFRESH_FAILED: &str = "Cloud table refresh failed; the last complete local snapshot remains active. Check connectivity and the mirror disk budget.";

/// Fast-forward refresh before a write to an idle table.
#[derive(Clone, Copy, Debug)]
pub struct FastForward {
    /// Bound of the cloud revision probe (desktop 2 s).
    pub probe_timeout: Duration,
    /// Minimum time between probes of one table (desktop 10 s).
    pub min_interval: Duration,
}

pub struct WriteManagerOptions {
    /// Parent of ".standalone-outbox".
    pub parent: PathBuf,
    /// Standalone: placement id; desktop: app id.
    pub namespace: String,
    /// 64 lowercase hex characters.
    pub scope: String,
    /// Only `validate_limits()` is applied here.
    pub limits: BufferingConfig,
    pub replay_limits: OfflineLimits,
    pub refresh_interval: Duration,
    pub idle_poll: Duration,
    pub quarantine_other_scopes: bool,
    pub lanes: QueueLanes,
    /// Merge adjacent never-sent row batches of one lane at dispatch.
    pub coalesce_row_batches: bool,
    pub fast_forward: Option<FastForward>,
    pub connectivity: Option<Arc<dyn Connectivity>>,
    /// False: a table deleted in the cloud is marked missing instead of re-created.
    pub recreate_dropped_tables: bool,
    /// Superseded local snapshots are deleted after this grace; None: at startup only.
    pub retired_snapshot_grace: Option<Duration>,
}

impl WriteManagerOptions {
    pub fn standalone(
        parent: PathBuf,
        placement: String,
        scope: String,
        limits: BufferingConfig,
    ) -> Self {
        Self {
            parent,
            namespace: placement,
            scope,
            limits,
            replay_limits: INSTANCE_OFFLINE_LIMITS,
            refresh_interval: Duration::from_secs(30),
            idle_poll: Duration::from_secs(15),
            quarantine_other_scopes: true,
            lanes: QueueLanes::Global,
            coalesce_row_batches: false,
            fast_forward: None,
            connectivity: None,
            recreate_dropped_tables: true,
            retired_snapshot_grace: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TableActivation {
    /// Registered, but runs cannot open it until `activate_table`.
    Held,
    Active,
}

#[derive(Clone, Copy, Debug)]
pub struct TableSetup {
    pub activation: TableActivation,
    /// Absent baseline only: check the key column before registering.
    pub validate_key: bool,
    /// Absent baseline only: download every file of the table (lazy mirror only).
    pub prefetch: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RefreshOutcome {
    Unchanged,
    Refreshed {
        version: u64,
        downloaded_bytes: u64,
    },
    /// See the table's mirror error for the reason.
    Deferred,
    RemoteMissing,
}

#[derive(Clone, Debug)]
pub struct TableState {
    pub table: BufferedTable,
    /// "apps/p/storage/db"
    pub database_path: String,
    /// None: not registered.
    pub activation: Option<TableActivation>,
    pub revision: Option<OfflineExpected>,
    pub refreshed_at: Option<i64>,
    pub pending: u64,
    pub mirror_error: Option<String>,
    pub remote_missing: bool,
    /// The current local snapshot directory.
    pub local_bytes: u64,
    pub prefetch: bool,
    pub cached_bytes: u64,
    pub total_bytes: Option<u64>,
    pub offline_complete: bool,
    pub downloading: bool,
    pub key_indexed: Option<bool>,
}

#[derive(Default)]
struct Registry {
    by_resource: BTreeMap<String, Arc<TableOverlay>>,
    by_location: BTreeMap<(String, String), Arc<TableOverlay>>,
}

#[derive(Default)]
struct Backoff {
    global: Option<(u32, Instant)>,
    lanes: HashMap<String, (u32, Instant)>,
}

fn backoff_after(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.min(6)).min(60))
}

enum Wait {
    Now,
    Backoff(Duration),
    Idle(Duration),
}

pub struct WriteManager {
    pub(crate) queue: Arc<Outbox>,
    pub(crate) host: Arc<dyn OfflineHost>,
    local: RwLock<Option<Connection>>,
    tables: RwLock<Registry>,
    pub(crate) gate: Arc<Mutex<()>>,
    pub(crate) wake: Arc<Notify>,
    lock: std::sync::Mutex<Option<File>>,
    pub(crate) max_mirror_bytes: AtomicU64,
    pub(crate) replay_limits: ReplayLimits,
    pub(crate) refresh_interval: Duration,
    idle_poll: Duration,
    lanes: QueueLanes,
    coalesce_row_batches: bool,
    pub(crate) fast_forward: Option<FastForward>,
    pub(crate) connectivity: Option<Arc<dyn Connectivity>>,
    pub(crate) recreate_dropped_tables: bool,
    retired_snapshot_grace: Option<Duration>,
    closed: AtomicBool,
    stop: Notify,
    backoff: std::sync::Mutex<Backoff>,
    drain: std::sync::Mutex<Option<JoinHandle<()>>>,
}

impl WriteManager {
    /// Opens the outbox, takes the exclusive writer lock, optionally quarantines sibling
    /// scopes, removes orphaned local tables and opens the budgeted local connection.
    pub async fn open(
        options: WriteManagerOptions,
        host: Arc<dyn OfflineHost>,
    ) -> Result<Arc<Self>> {
        options.limits.validate_limits()?;
        let queue = Arc::new(Outbox::open_with(
            &options.parent,
            &options.namespace,
            &options.scope,
            options.limits.clone(),
            OutboxOptions {
                lanes: options.lanes,
            },
        )?);
        let lock = queue.acquire_writer()?;
        if options.quarantine_other_scopes {
            queue.quarantine_other_scopes()?;
        }
        let tables_root = queue.root().join("tables");
        fs::private_directory(&tables_root)?;
        fs::clean_orphaned_tables(&tables_root, &queue.retained_table_names()?)?;
        let spill = queue.root().join("spill");
        if spill.exists() {
            std::fs::remove_dir_all(&spill)?;
        }
        fs::private_directory(&spill)?;
        let local = offline_replay::budgeted_local_connection(
            &tables_root,
            options.limits.max_mirror_bytes,
        )
        .await?;
        Ok(Arc::new(Self {
            queue,
            host,
            local: RwLock::new(Some(local)),
            tables: RwLock::new(Registry::default()),
            gate: Arc::new(Mutex::new(())),
            wake: Arc::new(Notify::new()),
            lock: std::sync::Mutex::new(Some(lock)),
            max_mirror_bytes: AtomicU64::new(options.limits.max_mirror_bytes),
            replay_limits: ReplayLimits::new(options.replay_limits),
            refresh_interval: options.refresh_interval,
            idle_poll: options.idle_poll,
            lanes: options.lanes,
            coalesce_row_batches: options.coalesce_row_batches,
            fast_forward: options.fast_forward,
            connectivity: options.connectivity,
            recreate_dropped_tables: options.recreate_dropped_tables,
            retired_snapshot_grace: options.retired_snapshot_grace,
            closed: AtomicBool::new(false),
            stop: Notify::new(),
            backoff: std::sync::Mutex::new(Backoff::default()),
            drain: std::sync::Mutex::new(None),
        }))
    }

    pub(crate) fn local(&self) -> Result<Connection> {
        self.local
            .read()
            .map_err(|_| anyhow::anyhow!("Offline table connection poisoned"))?
            .clone()
            .context(CLOSED)
    }

    pub(crate) fn spill_directory(&self) -> PathBuf {
        self.queue.root().join("spill")
    }

    pub(crate) fn key_validation_memory(&self) -> usize {
        KEY_VALIDATION_MEMORY
    }

    fn database_path(&self, table: &BufferedTable) -> Result<ObjectPath> {
        let prefix = self
            .host
            .location_prefix(table.purpose)
            .context("Missing buffered database scope")?;
        Ok(ObjectPath::parse(format!("{prefix}{}", table.database))?)
    }

    fn resource_key(table: &BufferedTable) -> Result<(OfflineResource, String)> {
        let resource = OfflineResource::Table {
            purpose: table.purpose,
            database: table.database.clone(),
            table: table.table.clone(),
        };
        let key = serde_json::to_string(&resource)?;
        Ok((resource, key))
    }

    /// Initializes the table's local snapshot and registers its overlay. An absent baseline
    /// is materialized from the cloud and checked by `setup` before anything is recorded or
    /// registered; a failure leaves nothing behind. Idempotent.
    pub async fn add_table(
        self: &Arc<Self>,
        table: BufferedTable,
        setup: TableSetup,
    ) -> Result<TableState> {
        self.authorize()?;
        table.validate()?;
        ensure!(
            !setup.prefetch,
            "Download everything needs a lazy offline mirror"
        );
        let path = self.database_path(&table)?;
        let (resource, key) = Self::resource_key(&table)?;
        if let Some(existing) = self.overlay(&key) {
            return self.state_of(&existing);
        }
        let overlay = Arc::new(TableOverlay::new(
            Arc::downgrade(self),
            key.clone(),
            resource,
            table,
            path.to_string(),
            setup.activation,
        ));
        overlay.initialize(self, setup).await?;
        {
            let mut tables = self.registry_mut()?;
            tables.by_location.insert(
                (path.to_string(), overlay.selection.table.clone()),
                overlay.clone(),
            );
            tables.by_resource.insert(key, overlay.clone());
        }
        self.state_of(&overlay)
    }

    /// Held → Active after one refresh that catches the cloud's current version. A failed
    /// refresh keeps the table held.
    pub async fn activate_table(self: &Arc<Self>, table: &BufferedTable) -> Result<TableState> {
        let overlay = self.registered(table)?;
        if overlay.activation() == TableActivation::Held {
            overlay.refresh(self).await.inspect_err(|error| {
                let _ = self.queue.mirror_error(
                    &overlay.key,
                    Some(&format!("The cloud table could not be read: {error}")),
                );
            })?;
            overlay.activate();
        }
        self.state_of(&overlay)
    }

    /// One refresh now, ignoring `refresh_interval`. Deferred while the lane has queued changes.
    pub async fn refresh_table(self: &Arc<Self>, table: &BufferedTable) -> Result<RefreshOutcome> {
        let overlay = self.registered(table)?;
        overlay.refresh(self).await.inspect_err(|_| {
            let _ = self.queue.mirror_error(&overlay.key, Some(REFRESH_FAILED));
        })
    }

    /// Only when the resource has no non-terminal operations. Handles that runs still hold
    /// fail afterwards.
    pub async fn remove_table(&self, table: &BufferedTable) -> Result<()> {
        self.authorize()?;
        let (_, key) = Self::resource_key(table)?;
        let _guard = self.gate.lock().await;
        ensure!(
            !self.queue.has_pending(&key)?,
            "Table '{}' still has queued changes",
            table.table
        );
        let names = [
            self.queue
                .local_view(&key)
                .ok()
                .and_then(|(name, _, _)| name),
            self.overlay(&key).map(|overlay| overlay.local_name.clone()),
        ];
        if let Some(overlay) = self.overlay(&key) {
            overlay.remove();
            let mut tables = self.registry_mut()?;
            tables.by_resource.remove(&key);
            tables
                .by_location
                .retain(|_, registered| !Arc::ptr_eq(registered, &overlay));
        }
        ensure!(
            self.queue.forget_resource(&key)?,
            "Table '{}' still has queued changes",
            table.table
        );
        let local = self.local()?;
        for name in names.into_iter().flatten() {
            match local.drop_table(&name, &[]).await {
                Ok(()) | Err(flow_like_storage::lancedb::Error::TableNotFound { .. }) => (),
                Err(error) => return Err(error.into()),
            }
        }
        self.host.queue_changed();
        Ok(())
    }

    /// Hot limits: queue limits, the snapshot budget of later refreshes and the replay
    /// limits of later freezes and dispatches.
    pub async fn set_limits(
        &self,
        limits: BufferingConfig,
        replay_limits: OfflineLimits,
    ) -> Result<()> {
        self.authorize()?;
        limits.validate_limits()?;
        self.queue.set_limits(limits.clone())?;
        self.max_mirror_bytes
            .store(limits.max_mirror_bytes, Ordering::Release);
        self.replay_limits.set(replay_limits);
        Ok(())
    }

    /// Marks the manager closed, stops the drain loop and releases the writer lock, the
    /// SQLite connection and the local Lance connection.
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let _guard = self.gate.lock().await;
        if let Some(drain) = self
            .drain
            .lock()
            .map_err(|_| anyhow::anyhow!("Offline drain handle poisoned"))?
            .take()
        {
            drain.abort();
        }
        self.stop.notify_waiters();
        let overlays = {
            let mut tables = self.registry_mut()?;
            let overlays = tables.by_resource.values().cloned().collect::<Vec<_>>();
            *tables = Registry::default();
            overlays
        };
        for overlay in overlays {
            overlay.remove();
        }
        self.queue.close()?;
        drop(
            self.local
                .write()
                .map_err(|_| anyhow::anyhow!("Offline table connection poisoned"))?
                .take(),
        );
        drop(
            self.lock
                .lock()
                .map_err(|_| anyhow::anyhow!("Offline writer lock poisoned"))?
                .take(),
        );
        Ok(())
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// True for registered tables (held or active).
    pub fn table_is_managed(&self, database_path: &ObjectPath, table: &str) -> bool {
        self.overlay_at(database_path, table).is_some()
    }

    /// Installs the offline adapter on stores of active tables; unregistered tables pass
    /// through. Held, removed and cloud-deleted tables fail.
    pub async fn decorate(
        &self,
        database_path: &ObjectPath,
        store: LanceDBVectorStore,
    ) -> Result<LanceDBVectorStore> {
        match self.overlay_at(database_path, store.table_name()) {
            Some(overlay) => {
                overlay.usable()?;
                LanceDBVectorStore::validate_overlay_selector(&store.selector())?;
                Ok(store.with_mutation_adapter(overlay))
            }
            None => Ok(store),
        }
    }

    /// A managed store on the manager's private connection. None = not registered.
    pub async fn managed_store(
        &self,
        database_path: &ObjectPath,
        table: &str,
        selector: DatabaseSelector,
    ) -> Result<Option<LanceDBVectorStore>> {
        let Some(overlay) = self.overlay_at(database_path, table) else {
            return Ok(None);
        };
        overlay.usable()?;
        Ok(Some(
            LanceDBVectorStore::from_connection_for_overlay(self.local()?, table.into(), selector)?
                .with_mutation_adapter(overlay),
        ))
    }

    /// Active tables only.
    pub fn managed_table_names(&self, database_path: &ObjectPath) -> Vec<String> {
        self.tables
            .read()
            .map(|tables| {
                tables
                    .by_location
                    .iter()
                    .filter(|((root, _), overlay)| {
                        root == database_path.as_ref()
                            && overlay.activation() == TableActivation::Active
                    })
                    .map(|((_, table), _)| table.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Cloud tables plus active managed tables; only the managed ones while the cloud
    /// inventory is unavailable.
    pub async fn table_names(&self, database_path: &ObjectPath) -> Result<Vec<String>> {
        let local = self.managed_table_names(database_path);
        let mut names = match self.host.remote_table_names(database_path).await {
            Ok(names) => names,
            Err(error) if !local.is_empty() => {
                tracing::debug!(%error, "Showing initialized offline tables while cloud inventory is unavailable");
                local.clone()
            }
            Err(error) => return Err(error),
        };
        names.extend(local);
        names.sort();
        names.dedup();
        Ok(names)
    }

    pub async fn table_states(&self) -> Result<Vec<TableState>> {
        self.overlays()
            .iter()
            .map(|overlay| self.state_of(overlay))
            .collect()
    }

    fn state_of(&self, overlay: &TableOverlay) -> Result<TableState> {
        let revision = self
            .queue
            .resource_revision(&overlay.key)?
            .map(serde_json::from_value)
            .transpose()?;
        let (name, _, _) = self.queue.local_view(&overlay.key)?;
        let name = name.unwrap_or_else(|| overlay.local_name.clone());
        Ok(TableState {
            table: overlay.selection.clone(),
            database_path: overlay.database_path.clone(),
            activation: Some(overlay.activation()),
            revision,
            refreshed_at: self.queue.refreshed_at(&overlay.key)?,
            pending: self.queue.pending_states(&overlay.key)?.len() as u64,
            mirror_error: self.queue.table_mirror_error(&overlay.key)?,
            remote_missing: overlay.remote_missing(),
            local_bytes: fs::directory_bytes(
                &self
                    .queue
                    .root()
                    .join("tables")
                    .join(format!("{name}.lance")),
            )?,
            prefetch: false,
            cached_bytes: 0,
            total_bytes: None,
            offline_complete: true,
            downloading: false,
            key_indexed: None,
        })
    }

    pub fn file_overlay(
        self: &Arc<Self>,
        inner: Arc<dyn ObjectStore>,
        options: FileOverlayOptions,
    ) -> Result<Arc<FileOverlay>> {
        self.authorize()?;
        Ok(Arc::new(FileOverlay::new(inner, self.clone(), options)))
    }

    /// True once the resource has no non-terminal operations; false after `timeout`.
    pub async fn wait_resource_idle(&self, resource: &OfflineResource, timeout: Duration) -> bool {
        match serde_json::to_string(resource) {
            Ok(key) => files::wait_idle(&self.queue, &key, timeout).await,
            Err(_) => false,
        }
    }

    /// Digest-checked bytes of a queued FilePut. None when no payload is retained.
    pub fn file_payload(&self, operation_id: &str) -> Result<Option<(OfflineResource, Bytes)>> {
        let Some(operation) = self.queue.operation(operation_id)? else {
            return Ok(None);
        };
        let request: OfflineReplayRequest = match serde_json::from_value(operation.payload) {
            Ok(request) => request,
            Err(_) => return Ok(None),
        };
        Ok(files::payload_bytes(&request)?.map(|bytes| (request.resource, bytes)))
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn queue(&self) -> &Outbox {
        &self.queue
    }

    pub fn root(&self) -> &Path {
        self.queue.root()
    }

    /// One drain step, for tests and hosts.
    #[doc(hidden)]
    pub async fn drain_once(&self) -> Result<bool> {
        self.drain_one().await
    }

    /// The drain loop holds the manager weakly and stops once it is dropped or closed.
    pub fn spawn_drain(self: &Arc<Self>) {
        let weak = Arc::downgrade(self);
        let lanes = self.lanes;
        let handle = tokio::spawn(async move {
            match lanes {
                QueueLanes::Global => Self::drain_global(weak).await,
                QueueLanes::PerResource => Self::drain_lanes(weak).await,
            }
        });
        if let Ok(mut drain) = self.drain.lock() {
            if let Some(previous) = drain.replace(handle) {
                previous.abort();
            }
        }
    }

    async fn drain_global(weak: std::sync::Weak<Self>) {
        let mut failures = 0u32;
        let mut immediate = false;
        let mut retrying = false;
        loop {
            let Some(manager) = weak.upgrade() else { break };
            if manager.is_closed() {
                break;
            }
            if immediate {
                // Drain healthy queues at network speed while yielding to local flows.
                tokio::task::yield_now().await;
            } else if retrying {
                // New local writes must not bypass an outage's retry backoff.
                tokio::select! { _ = tokio::time::sleep(backoff_after(failures)) => (), _ = manager.stop.notified() => () }
            } else {
                tokio::select! { _ = tokio::time::sleep(manager.idle_poll) => (), _ = manager.wake.notified() => (), _ = manager.stop.notified() => () }
            }
            if manager.is_closed() {
                break;
            }
            let result = manager.drain_one().await;
            if matches!(result, Ok(false)) {
                manager.idle_pass().await;
            }
            match result {
                Ok(true) => {
                    failures = 0;
                    immediate = true;
                    retrying = false;
                }
                Ok(false) => {
                    immediate = false;
                    retrying = manager.queue.head_state().ok().flatten().as_deref()
                        == Some("outcome_unknown");
                    failures = if retrying {
                        failures.saturating_add(1)
                    } else {
                        0
                    };
                }
                Err(error) => {
                    tracing::warn!(%error,"Offline replay paused");
                    immediate = false;
                    retrying = true;
                    failures = failures.saturating_add(1);
                }
            }
            drop(manager);
        }
    }

    async fn drain_lanes(weak: std::sync::Weak<Self>) {
        let mut wait = Wait::Idle(Duration::ZERO);
        loop {
            let Some(manager) = weak.upgrade() else { break };
            if manager.is_closed() {
                break;
            }
            match wait {
                Wait::Now => tokio::task::yield_now().await,
                Wait::Backoff(delay) => {
                    tokio::select! { _ = tokio::time::sleep(delay) => (), _ = manager.stop.notified() => () }
                }
                Wait::Idle(delay) => {
                    tokio::select! { _ = tokio::time::sleep(delay) => (), _ = manager.wake.notified() => (), _ = manager.stop.notified() => () }
                }
            }
            if manager.is_closed() {
                break;
            }
            wait = match manager.drain_one().await {
                Ok(true) => Wait::Now,
                Ok(false) => {
                    manager.idle_pass().await;
                    Wait::Idle(manager.next_lane_ready().min(manager.idle_poll))
                }
                Err(error) => {
                    tracing::warn!(%error, "Offline replay of one change paused");
                    match manager.global_backoff() {
                        Some(delay) => Wait::Backoff(delay),
                        None => Wait::Now,
                    }
                }
            };
            drop(manager);
        }
    }

    /// Idle refreshes and retired snapshot cleanup.
    pub(crate) async fn idle_pass(&self) {
        for table in self.overlays() {
            if let Err(error) = table.refresh_idle(self).await {
                tracing::debug!(%error,"Keeping the last complete offline table snapshot");
                let _ = self.queue.mirror_error(&table.key, Some(REFRESH_FAILED));
            }
        }
        if let Err(error) = self.prune_retired().await {
            tracing::debug!(%error, "Could not delete retired offline table snapshots");
        }
    }

    async fn prune_retired(&self) -> Result<()> {
        let Some(grace) = self.retired_snapshot_grace else {
            return Ok(());
        };
        let now = fs::unix_time()?;
        let local = self.local()?;
        for (name, retired_at) in self.queue.retired_tables()? {
            if now.saturating_sub(retired_at) < grace.as_secs() as i64 {
                continue;
            }
            match local.drop_table(&name, &[]).await {
                Ok(()) | Err(flow_like_storage::lancedb::Error::TableNotFound { .. }) => {
                    self.queue.forget_retired(&name)?
                }
                Err(error) => {
                    tracing::debug!(%error, table = %name, "Retired offline snapshot is still in use")
                }
            }
        }
        Ok(())
    }

    pub(crate) fn retire(&self, name: &str) -> Result<()> {
        if self.retired_snapshot_grace.is_some() {
            self.queue.retire_table(name, fs::unix_time()?)?;
        }
        Ok(())
    }

    fn registry_mut(&self) -> Result<std::sync::RwLockWriteGuard<'_, Registry>> {
        self.tables
            .write()
            .map_err(|_| anyhow::anyhow!("Offline table registry poisoned"))
    }

    fn overlay_at(&self, database_path: &ObjectPath, table: &str) -> Option<Arc<TableOverlay>> {
        self.tables.read().ok().and_then(|tables| {
            tables
                .by_location
                .get(&(database_path.to_string(), table.to_owned()))
                .cloned()
        })
    }

    pub(crate) fn overlay(&self, resource: &str) -> Option<Arc<TableOverlay>> {
        self.tables
            .read()
            .ok()
            .and_then(|tables| tables.by_resource.get(resource).cloned())
    }

    fn registered(&self, table: &BufferedTable) -> Result<Arc<TableOverlay>> {
        let (_, key) = Self::resource_key(table)?;
        self.overlay(&key).with_context(|| {
            format!(
                "Table '{}' is not set up for offline use on this device",
                table.table
            )
        })
    }

    fn overlays(&self) -> Vec<Arc<TableOverlay>> {
        self.tables
            .read()
            .map(|tables| tables.by_resource.values().cloned().collect())
            .unwrap_or_default()
    }

    pub(crate) fn authorize(&self) -> Result<()> {
        ensure!(!self.is_closed(), CLOSED);
        if self.host.authorization_current().is_err() && self.queue.check_authorized().is_ok() {
            self.queue
                .quarantine("Project resource authorization revoked")?;
            self.host.queue_changed();
        }
        self.queue.check_authorized()
    }

    fn lane_backoff(&self) -> std::sync::MutexGuard<'_, Backoff> {
        self.backoff
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lane_ready(&self, resource: &str) -> bool {
        self.lane_backoff()
            .lanes
            .get(resource)
            .is_none_or(|(_, until)| *until <= Instant::now())
    }

    fn next_lane_ready(&self) -> Duration {
        let now = Instant::now();
        self.lane_backoff()
            .lanes
            .values()
            .filter(|(_, until)| *until > now)
            .map(|(_, until)| *until - now)
            .min()
            .unwrap_or(Duration::MAX)
    }

    fn global_backoff(&self) -> Option<Duration> {
        let now = Instant::now();
        self.lane_backoff()
            .global
            .filter(|(_, until)| *until > now)
            .map(|(_, until)| until - now)
    }

    fn record_backoff(&self, picked: Option<&(String, String)>, result: &Result<bool>) {
        if self.lanes == QueueLanes::Global {
            return;
        }
        let stalled = match (picked, result) {
            (Some((_, id)), Ok(false)) => self
                .queue
                .operation_state(id)
                .ok()
                .flatten()
                .is_some_and(|lookup| lookup.state == "outcome_unknown"),
            (_, Err(_)) => true,
            (_, Ok(_)) => false,
        };
        let now = Instant::now();
        let mut backoff = self.lane_backoff();
        match picked {
            Some((resource, _)) => {
                backoff.global = None;
                if stalled {
                    let failures = backoff
                        .lanes
                        .get(resource)
                        .map_or(0, |(failures, _)| failures.saturating_add(1));
                    backoff
                        .lanes
                        .insert(resource.clone(), (failures, now + backoff_after(failures)));
                } else {
                    backoff.lanes.remove(resource);
                }
            }
            None if stalled => {
                let failures = backoff
                    .global
                    .map_or(0, |(failures, _)| failures.saturating_add(1));
                backoff.global = Some((failures, now + backoff_after(failures)));
            }
            None => backoff.global = None,
        }
    }

    pub(crate) async fn drain_one(&self) -> Result<bool> {
        let mut picked = None;
        let result = self.drain_step(&mut picked).await;
        self.record_backoff(picked.as_ref(), &result);
        result
    }

    async fn complete_skips(&self) -> Result<()> {
        for (id, reason) in self.queue.requested_skips()? {
            let head = match self.lanes {
                QueueLanes::Global => self.queue.head()?.filter(|head| head.operation_id == id),
                QueueLanes::PerResource => self.queue.operation(&id)?,
            };
            let Some(head) = head else {
                self.queue.clear_finished_skip(&id)?;
                continue;
            };
            if let Some(table) = self.overlay(&head.resource) {
                table.skip(self, &id, &reason).await?;
            } else {
                self.queue.complete_skip(&id, &reason, None, "main", None)?;
                let resource = serde_json::from_str::<OfflineResource>(&head.resource).ok();
                if let Some(path) = resource
                    .as_ref()
                    .map(|resource| files::file_path(self, resource))
                    .transpose()?
                    .flatten()
                    && let Err(error) = self.host.file_discarded(&path, &id)
                {
                    tracing::warn!(%error, "Could not discard the local copy of a skipped offline file");
                }
            }
            self.host.queue_changed();
        }
        Ok(())
    }

    fn next_head(&self) -> Result<Option<QueuedOperation>> {
        let dispatchable =
            |state: &str| matches!(state, "pending" | "attempting" | "outcome_unknown");
        match self.lanes {
            QueueLanes::Global => Ok(self.queue.head()?.filter(|head| dispatchable(&head.state))),
            QueueLanes::PerResource => {
                for head in self.queue.lane_heads()? {
                    if dispatchable(&head.state) && self.lane_ready(&head.resource) {
                        return self.queue.operation(&head.operation_id);
                    }
                }
                Ok(None)
            }
        }
    }

    async fn drain_step(&self, picked: &mut Option<(String, String)>) -> Result<bool> {
        self.authorize()?;
        let guard = self.gate.lock().await;
        self.complete_skips().await?;
        let Some(operation) = self.next_head()? else {
            return Ok(false);
        };
        *picked = Some((operation.resource.clone(), operation.operation_id.clone()));
        self.dispatch(operation, guard).await
    }

    async fn dispatch(
        &self,
        mut operation: QueuedOperation,
        guard: MutexGuard<'_, ()>,
    ) -> Result<bool> {
        if let Some(table) = self.overlay(&operation.resource) {
            table.recover_local(self).await?;
            operation = self
                .queue
                .operation(&operation.operation_id)?
                .context("Queue head disappeared")?;
        }
        let mut request: OfflineReplayRequest = serde_json::from_value(operation.payload.clone())?;
        if matches!(request.resource, OfflineResource::File { .. })
            && operation.local_version.is_none()
        {
            // File bytes/tombstones are already durable in the queue payload.
            // Recover a crash between enqueue and the ready marker.
            self.queue.mark_local(&operation.operation_id, 1)?;
        }
        request.operation_id = operation.operation_id.clone();
        if operation.attempts == 0 {
            request.expected = serde_json::from_value(
                self.queue
                    .resource_revision(&operation.resource)?
                    .context("Missing cloud base revision")?,
            )?;
            let merged = if self.coalesce_row_batches {
                self.coalesce(&operation, &mut request)?
            } else {
                Vec::new()
            };
            match validate_request(&request, &self.replay_limits.get()) {
                Ok(()) => (),
                Err(RequestError::TooLarge(message)) => {
                    self.queue
                        .block_unclaimed(&operation.operation_id, "hub_limit", &message)?;
                    self.host.queue_changed();
                    return Ok(false);
                }
                Err(RequestError::Invalid(error)) => return Err(error.into()),
            }
            self.queue.prepare_attempt_merged(
                &operation.operation_id,
                &serde_json::to_value(&request)?,
                &merged,
            )?;
        }
        drop(guard);
        let result = self.host.replay(&request).await;
        let _guard = self.gate.lock().await;
        match result {
            Ok(response) => {
                ensure!(
                    response.operation_id == request.operation_id
                        && response.digest == request.digest()?,
                    "Replay response does not match the queued operation"
                );
                match response.status {
                    OfflineReplayStatus::Applied => {
                        let revision = response
                            .result
                            .as_ref()
                            .context("Replay acknowledgement lacks a resource revision")?;
                        files::remember_acknowledged(self, &request, revision)?;
                        self.queue.acknowledge(
                            &operation.operation_id,
                            &serde_json::to_value(revision)?,
                            &serde_json::to_value(&response)?,
                        )?;
                        self.host.queue_changed();
                        Ok(true)
                    }
                    status => {
                        let state = match status {
                            OfflineReplayStatus::Conflict => "conflict",
                            OfflineReplayStatus::OutcomeUnknown => "outcome_unknown",
                            _ => "blocked",
                        };
                        self.queue.block(
                            &operation.operation_id,
                            state,
                            response
                                .message
                                .as_deref()
                                .unwrap_or("Cloud write is blocked"),
                        )?;
                        self.host.queue_changed();
                        Ok(false)
                    }
                }
            }
            Err(error) => {
                match error.kind {
                    ReplayErrorKind::Denied => {
                        self.queue.quarantine(
                            "Replay authorization denied; queued data remains on this device",
                        )?;
                        self.host.queue_changed();
                    }
                    ReplayErrorKind::Rejected => {
                        self.queue.block_with_code(
                            &operation.operation_id,
                            "blocked",
                            error.code.as_deref(),
                            &error.message,
                        )?;
                        self.host.queue_changed();
                    }
                    ReplayErrorKind::NotClaimed => {
                        self.queue.block_unclaimed(
                            &operation.operation_id,
                            error.code.as_deref().unwrap_or("invalid"),
                            &error.message,
                        )?;
                        self.host.queue_changed();
                    }
                    ReplayErrorKind::Unavailable => (),
                }
                // Attempting remains immutable after a lost response. The API
                // reconciles the same operation ID before any repeated effect.
                Err(error.into())
            }
        }
    }

    /// Merges the never-sent operations that follow `head` in its lane into `request`
    /// while they are of the same kind and the merged request stays within the limits.
    fn coalesce(
        &self,
        head: &QueuedOperation,
        request: &mut OfflineReplayRequest,
    ) -> Result<Vec<String>> {
        if !matches!(request.resource, OfflineResource::Table { .. }) {
            return Ok(Vec::new());
        }
        let limits = self.replay_limits.get();
        let mut merged = Vec::new();
        let mut sequence = head.sequence;
        while merged.len() < MAX_COALESCED {
            let next = match self.lanes {
                QueueLanes::PerResource => self.queue.next_in_lane(&head.resource, sequence)?,
                QueueLanes::Global => self
                    .queue
                    .next_after(sequence)?
                    .filter(|next| next.resource == head.resource),
            };
            let Some(next) = next else { break };
            if next.state != "pending" || next.attempts != 0 || next.local_version.is_none() {
                break;
            }
            let follower: OfflineReplayRequest = serde_json::from_value(next.payload)?;
            let Some(mutation) = merge_mutations(&request.mutation, &follower.mutation) else {
                break;
            };
            let candidate = OfflineReplayRequest {
                mutation,
                ..request.clone()
            };
            if validate_request(&candidate, &limits).is_err() {
                break;
            }
            *request = candidate;
            merged.push(next.operation_id);
            sequence = next.sequence;
        }
        Ok(merged)
    }
}

/// Same-kind row batches as one mutation; upserts merge last-wins per key.
fn merge_mutations(first: &OfflineMutation, next: &OfflineMutation) -> Option<OfflineMutation> {
    match (first, next) {
        (OfflineMutation::TableInsert { rows }, OfflineMutation::TableInsert { rows: more }) => {
            Some(OfflineMutation::TableInsert {
                rows: rows.iter().chain(more).cloned().collect(),
            })
        }
        (
            OfflineMutation::TableUpsert { id_field, rows },
            OfflineMutation::TableUpsert {
                id_field: next_field,
                rows: more,
            },
        ) if id_field == next_field => {
            let mut merged: Vec<Value> = Vec::with_capacity(rows.len() + more.len());
            let mut positions = HashMap::new();
            for row in rows.iter().chain(more) {
                let key = serde_json::to_string(row.get(id_field)?).ok()?;
                match positions.get(&key) {
                    Some(position) => merged[*position] = row.clone(),
                    None => {
                        positions.insert(key, merged.len());
                        merged.push(row.clone());
                    }
                }
            }
            Some(OfflineMutation::TableUpsert {
                id_field: id_field.clone(),
                rows: merged,
            })
        }
        (
            OfflineMutation::TableDelete { filter },
            OfflineMutation::TableDelete { filter: more },
        ) => Some(OfflineMutation::TableDelete {
            filter: format!("{filter} OR {more}"),
        }),
        _ => None,
    }
}
