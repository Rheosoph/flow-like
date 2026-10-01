//! The engine's lazy mirror: retention, the mirror task, refresh orders and fetch rules.

use crate::{
    fs,
    manager::{RefreshOutcome, WriteManager},
    outbox::RecordedSnapshot,
    table::TableOverlay,
};
use anyhow::{Context, Result, anyhow};
use flow_like_device_protocol::{OfflineExpected, format_limit};
use flow_like_storage::{
    databases::vector::{
        offline_mirror::{
            self, LazyMirror, LazySnapshot, MIRROR_FULL, MirrorFetch, MirrorFile, MirrorHost,
            MirrorRetention, MirrorShortage, WriteScope, is_mirror_miss, mirror_stale_key,
            missing_data_text, reorganized_text,
        },
        offline_replay,
    },
    lancedb::Table,
    object_store::{self, ObjectStore},
};
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex, MutexGuard, Weak, atomic::Ordering},
    time::{Duration, Instant},
};
use tokio::{
    sync::{Notify, oneshot},
    task::JoinHandle,
};

/// Probe and manifest-only snapshot of one refresh.
const METADATA_STEP: Duration = Duration::from_secs(30);
/// A refresh that lost its checkpoint or was dropped retries after this.
const REFRESH_RETRY: Duration = Duration::from_secs(10);
const GARBAGE_GRACE: Duration = Duration::from_secs(24 * 3600);
/// Files per Download everything step; the mirror task checks for jobs between steps.
const PREFETCH_STEP: usize = 4;
const ALLOWANCE_USED: &str = "Daily download limit for offline copies reached.";
const REORGANIZED_PENDING: &str = "Part of this table was reorganized in the cloud. The copy refreshes once its queued changes are synced.";
const NOT_RECORDED: &str = "its cloud location is not recorded on this device; turn offline access for it off and on again";
const MIRROR_STOPPED: &str =
    "The offline mirror stopped before this refresh finished; try the refresh again.";

/// §2.8 trigger table: idle refreshes fetch before they switch; writes, activation and stale
/// reports switch first and fetch in the mirror task.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Order {
    FetchFirst,
    MetadataFirst,
}

/// A read of an offline table view needs cloud data this device cannot get now. Drain
/// recovery backs off only the lane that hit it.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct MirrorUnavailable(pub(crate) String);

type Waiter = oneshot::Sender<std::result::Result<RefreshOutcome, String>>;
type Preparing = HashMap<String, (Arc<Vec<MirrorFile>>, bool)>;

enum Job {
    Refresh {
        resource: String,
        order: Order,
        waiters: Vec<Waiter>,
    },
    Fetch {
        resource: String,
        files: Vec<MirrorFile>,
    },
}

#[derive(Default)]
struct Jobs {
    queue: VecDeque<Job>,
    /// The mirror task ended: new jobs are refused.
    stopped: bool,
}

/// Ends the jobs of a mirror task that returned, was aborted or panicked.
struct TaskExit(Weak<WriteManager>);

impl Drop for TaskExit {
    fn drop(&mut self) {
        if let Some(manager) = self.0.upgrade()
            && let Some(lazy) = manager.lazy()
        {
            drop(lazy.end_jobs());
        }
    }
}

/// What `retention` read besides the retention itself.
struct Retained {
    retention: MirrorRetention,
    /// Current, retired and preparing snapshot names.
    names: HashSet<String>,
    current: Vec<Arc<RecordedSnapshot>>,
}

enum Step {
    Missing,
    Unchanged,
    Snapshot(String, LazySnapshot),
}

/// Lazy mode state of a manager.
pub(crate) struct Lazy {
    pub(crate) mirror: Arc<LazyMirror>,
    jobs: Mutex<Jobs>,
    notify: Notify,
    task: Mutex<Option<JoinHandle<()>>>,
    /// Open download allowance reservations.
    reserved: Mutex<u64>,
    /// Snapshots a refresh or `add_table` is preparing: files and Download everything. Held
    /// while snapshot rows are pruned and while a checkpoint retires the snapshot it replaced.
    preparing: Mutex<Preparing>,
    recorded: Mutex<HashMap<String, Arc<RecordedSnapshot>>>,
    scopes: Mutex<HashMap<String, Arc<HashSet<String>>>>,
    downloading: Mutex<HashSet<String>>,
    retry_at: Mutex<HashMap<String, Instant>>,
    prefetch_retry: Mutex<HashMap<String, Instant>>,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Lazy {
    pub(crate) fn new(mirror: Arc<LazyMirror>) -> Self {
        Self {
            mirror,
            jobs: Mutex::new(Jobs::default()),
            notify: Notify::new(),
            task: Mutex::new(None),
            reserved: Mutex::new(0),
            preparing: Mutex::new(HashMap::new()),
            recorded: Mutex::new(HashMap::new()),
            scopes: Mutex::new(HashMap::new()),
            downloading: Mutex::new(HashSet::new()),
            retry_at: Mutex::new(HashMap::new()),
            prefetch_retry: Mutex::new(HashMap::new()),
        }
    }

    pub(crate) fn wake(&self) {
        self.notify.notify_one();
    }

    fn running(&self) -> bool {
        locked(&self.task)
            .as_ref()
            .is_some_and(|task| !task.is_finished())
    }

    pub(crate) fn stop(&self) {
        if let Some(task) = locked(&self.task).take() {
            task.abort();
        }
        self.mirror.close();
        for waiter in self.end_jobs() {
            let _ = waiter.send(Err(crate::outbox::CLOSED.into()));
        }
    }

    /// Refuses later jobs and returns the waiters of the queued ones.
    fn end_jobs(&self) -> Vec<Waiter> {
        let mut jobs = locked(&self.jobs);
        jobs.stopped = true;
        jobs.queue
            .drain(..)
            .flat_map(|job| match job {
                Job::Refresh { waiters, .. } => waiters,
                Job::Fetch { .. } => Vec::new(),
            })
            .collect()
    }

    fn prepare(&self, name: &str, files: &[MirrorFile], prefetch: bool) {
        locked(&self.preparing).insert(name.to_string(), (Arc::new(files.to_vec()), prefetch));
    }

    fn unprepare(&self, name: &str) {
        locked(&self.preparing).remove(name);
    }

    /// False once the mirror task ended; the job is dropped.
    fn queue_refresh(&self, resource: &str, order: Order, waiter: Option<Waiter>) -> bool {
        let mut jobs = locked(&self.jobs);
        if jobs.stopped {
            return false;
        }
        let queued = jobs.queue.iter_mut().find_map(|job| match job {
            Job::Refresh {
                resource: queued,
                order: queued_order,
                waiters,
            } if queued == resource => Some((queued_order, waiters)),
            _ => None,
        });
        match queued {
            Some((queued_order, waiters)) => {
                if order == Order::MetadataFirst {
                    *queued_order = Order::MetadataFirst;
                }
                waiters.extend(waiter);
            }
            None => jobs.queue.push_back(Job::Refresh {
                resource: resource.to_string(),
                order,
                waiters: waiter.into_iter().collect(),
            }),
        }
        drop(jobs);
        self.wake();
        true
    }

    fn queue_fetch(&self, resource: &str, files: Vec<MirrorFile>) {
        if files.is_empty() {
            return;
        }
        let mut jobs = locked(&self.jobs);
        if jobs.stopped {
            return;
        }
        jobs.queue.push_back(Job::Fetch {
            resource: resource.to_string(),
            files,
        });
        drop(jobs);
        self.wake();
    }

    fn retry_soon(&self, resource: &str) {
        locked(&self.retry_at).insert(resource.to_string(), Instant::now() + REFRESH_RETRY);
    }

    pub(crate) fn forget(&self, resource: &str, names: &[String]) {
        for name in names {
            locked(&self.recorded).remove(name);
            locked(&self.scopes).remove(name);
        }
        locked(&self.retry_at).remove(resource);
        locked(&self.prefetch_retry).remove(resource);
        locked(&self.downloading).remove(resource);
    }

    /// Download everything was turned off: the table no longer counts as downloading.
    pub(crate) fn stop_prefetch(&self, resource: &str) {
        locked(&self.downloading).remove(resource);
    }

    /// Download everything was turned on or the limits changed: try again at once.
    pub(crate) fn resume_prefetch(&self) {
        locked(&self.prefetch_retry).clear();
        self.wake();
    }
}

/// Offline-class errors of mirror cloud calls: timeouts are TimedOut, everything else that
/// `is_offline_error` accepts is ConnectFailed.
fn classify(error: &object_store::Error) -> Option<MirrorFetch> {
    if !fs::is_offline_error(error, &|_| false) {
        return None;
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = source {
        let timed_out = error
            .downcast_ref::<object_store::client::HttpError>()
            .is_some_and(|error| error.kind() == object_store::client::HttpErrorKind::Timeout)
            || error
                .downcast_ref::<reqwest::Error>()
                .is_some_and(reqwest::Error::is_timeout)
            || error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut);
        if timed_out {
            return Some(MirrorFetch::TimedOut);
        }
        source = error.source();
    }
    Some(MirrorFetch::ConnectFailed)
}

/// E36, E37 or E39 naming the configured table; other errors unchanged.
pub(crate) fn mirror_failure(table: &str, error: anyhow::Error) -> anyhow::Error {
    if is_mirror_miss(&error) {
        return anyhow!(missing_data_text(table));
    }
    if mirror_stale_key(&error).is_some() {
        return anyhow!(reorganized_text(table));
    }
    const END: &str = "for another table.";
    let full = error.chain().find_map(|cause| {
        let text = cause.to_string();
        let tail = &text[text.find(MIRROR_FULL)?..];
        let end = tail.find(END).map_or(tail.len(), |end| end + END.len());
        Some(tail[..end].to_string())
    });
    match full {
        Some(text) => anyhow!(text),
        None => error,
    }
}

fn is_mirror_failure(error: &anyhow::Error) -> bool {
    is_mirror_miss(error)
        || mirror_stale_key(error).is_some()
        || error
            .chain()
            .any(|cause| cause.to_string().contains(MIRROR_FULL))
}

fn shortage_text(table: &str, error: &anyhow::Error) -> String {
    match error.downcast_ref::<MirrorShortage>() {
        Some(MirrorShortage::Disk { needed }) => format!(
            "Not enough offline storage on this device to download the latest version of '{table}' ({} needed).",
            offline_mirror::format_bytes(*needed)
        ),
        Some(MirrorShortage::Allowance) => ALLOWANCE_USED.into(),
        None => format!("Could not refresh from the cloud: {error}"),
    }
}

fn is_shortage(error: &str) -> bool {
    error == ALLOWANCE_USED || error.starts_with("Not enough offline storage")
}

fn limit(bytes: u64) -> String {
    format_limit(usize::try_from(bytes).unwrap_or(usize::MAX))
}

/// The Lazy-mode fields of a table's state.
pub(crate) struct MirrorState {
    pub(crate) cached_bytes: u64,
    pub(crate) total_bytes: Option<u64>,
    pub(crate) offline_complete: bool,
    pub(crate) downloading: bool,
    pub(crate) key_indexed: Option<bool>,
}

/// A table counted as Download everything before it is registered or flagged.
pub(crate) struct Candidate<'a> {
    pub(crate) resource: &'a str,
    pub(crate) local_name: &'a str,
    pub(crate) files: &'a [MirrorFile],
}

impl WriteManager {
    pub(crate) fn lazy(&self) -> Option<&Lazy> {
        self.lazy.as_ref()
    }

    pub(crate) fn recorded(&self, name: &str) -> Result<Option<Arc<RecordedSnapshot>>> {
        let Some(lazy) = self.lazy() else {
            return Ok(None);
        };
        if let Some(recorded) = locked(&lazy.recorded).get(name) {
            return Ok(Some(recorded.clone()));
        }
        let Some(recorded) = self.queue.snapshot(name)? else {
            return Ok(None);
        };
        let recorded = Arc::new(recorded);
        locked(&lazy.recorded).insert(name.to_string(), recorded.clone());
        Ok(Some(recorded))
    }

    /// §1.8 classes over the listed keys of every table resource of the outbox, retired
    /// snapshots within their grace and snapshots being prepared.
    fn retention(&self) -> Result<Retained> {
        let mut retention = MirrorRetention::default();
        let mut names = HashSet::new();
        let mut current = Vec::new();
        let mut add = |files: &[MirrorFile], pinned: bool, whole: bool| {
            for file in files {
                retention.listed.insert(file.key.clone(), file.bytes);
                if pinned {
                    retention.pinned.insert(file.key.clone());
                }
                if whole {
                    retention.whole.insert(file.key.clone());
                }
            }
        };
        for table in self.queue.mirror_tables()? {
            if let Some(recorded) = self.recorded(&table.local_name)? {
                add(
                    &recorded.files,
                    table.prefetch || table.pending,
                    table.prefetch,
                );
                current.push(recorded);
            }
            names.insert(table.local_name);
        }
        for (name, _) in self.queue.retired_tables()? {
            if let Some(recorded) = self.recorded(&name)? {
                add(&recorded.files, false, false);
            }
            names.insert(name);
        }
        if let Some(lazy) = self.lazy() {
            for (name, (files, prefetch)) in locked(&lazy.preparing).iter() {
                add(files, true, *prefetch);
                names.insert(name.clone());
            }
        }
        Ok(Retained {
            retention,
            names,
            current,
        })
    }

    /// Pushes the retention to the mirror, forgets the cached file lists of snapshots it no
    /// longer covers and clears stale keys that no current snapshot lists. Lazy mode only.
    pub(crate) fn push_retention(&self) -> Result<()> {
        let Some(lazy) = self.lazy() else {
            return Ok(());
        };
        let Retained {
            retention,
            names,
            current,
        } = self.retention()?;
        lazy.mirror.set_retention(retention);
        locked(&lazy.recorded).retain(|name, _| names.contains(name));
        locked(&lazy.scopes).retain(|name, _| names.contains(name));
        let unlisted = lazy
            .mirror
            .stale_keys()
            .into_iter()
            .filter(|key| {
                !current
                    .iter()
                    .any(|recorded| recorded.files.iter().any(|file| file.key == *key))
            })
            .collect::<Vec<_>>();
        lazy.mirror.clear_stale(&unlisted);
        Ok(())
    }

    /// Deletes unused snapshot rows while holding the preparing set, so a snapshot prepared
    /// meanwhile records its row only after the prune.
    pub(crate) fn prune_snapshots(&self) -> Result<()> {
        let Some(lazy) = self.lazy() else {
            return Ok(());
        };
        let preparing = locked(&lazy.preparing);
        self.queue
            .prune_snapshots(&preparing.keys().cloned().collect::<Vec<_>>())
    }

    /// Keeps snapshot rows from being pruned while a checkpoint retires the snapshot it
    /// replaced. None in Materialize mode, which records no snapshot rows.
    pub(crate) fn hold_snapshot_rows(&self) -> Option<MutexGuard<'_, Preparing>> {
        self.lazy().map(|lazy| locked(&lazy.preparing))
    }

    /// Bytes of the current local snapshot directory of `name`.
    pub(crate) fn snapshot_directory_bytes(&self, name: &str) -> Result<u64> {
        fs::directory_bytes(
            &self
                .queue
                .root()
                .join("tables")
                .join(format!("{name}.lance")),
        )
    }

    /// §1.8 required size, with `candidate` counted as a Download everything table.
    /// Unknown sizes count as 0.
    pub(crate) fn required(&self, candidate: Option<Candidate<'_>>) -> Result<u64> {
        let mut totals = Vec::new();
        let mut counted = HashSet::new();
        let mut snapshots = 0u64;
        let mut pending = Vec::new();
        let mut candidate_registered = false;
        let total = |files: &[MirrorFile]| {
            files
                .iter()
                .map(|file| file.bytes.unwrap_or(0))
                .sum::<u64>()
        };
        for table in self.queue.mirror_tables()? {
            snapshots = snapshots.saturating_add(self.snapshot_directory_bytes(&table.local_name)?);
            let recorded = self.recorded(&table.local_name)?;
            let is_candidate = candidate
                .as_ref()
                .is_some_and(|candidate| candidate.resource == table.resource);
            candidate_registered |= is_candidate;
            let files = match (&candidate, is_candidate) {
                (Some(candidate), true) => candidate.files.to_vec(),
                _ => recorded
                    .map(|recorded| recorded.files.clone())
                    .unwrap_or_default(),
            };
            if table.prefetch || is_candidate {
                totals.push(total(&files));
                counted.extend(files.into_iter().map(|file| file.key));
            } else if table.pending {
                pending.extend(files);
            }
        }
        if let Some(candidate) = candidate.as_ref().filter(|_| !candidate_registered) {
            snapshots =
                snapshots.saturating_add(self.snapshot_directory_bytes(candidate.local_name)?);
            totals.push(total(candidate.files));
            counted.extend(candidate.files.iter().map(|file| file.key.clone()));
        }
        let mut pinned = 0u64;
        if let Some(lazy) = self.lazy() {
            for file in pending {
                if counted.insert(file.key.clone()) {
                    pinned = pinned.saturating_add(lazy.mirror.cached(&file.key).unwrap_or(0));
                }
            }
        }
        Ok(totals
            .iter()
            .fold(0u64, |sum, total| sum.saturating_add(*total))
            .saturating_add(totals.iter().copied().max().unwrap_or(0))
            .saturating_add(pinned)
            .saturating_add(snapshots)
            .saturating_add(crate::manager::REQUIRED_HEADROOM))
    }

    /// E31 unless the required size with `candidate` downloading everything fits the limit.
    pub(crate) fn check_download_everything(
        &self,
        table: &str,
        candidate: Candidate<'_>,
    ) -> Result<()> {
        let needed = self.required(Some(candidate))?;
        let maximum = self.max_mirror_bytes.load(Ordering::Acquire);
        anyhow::ensure!(
            needed <= maximum,
            "Downloading all of table '{table}' needs {} of offline storage on this device, including room to update the largest fully downloaded table; the limit is {}. Increase the limit, or turn the table on without \"Download everything\".",
            limit(needed),
            limit(maximum)
        );
        Ok(())
    }

    /// Cached bytes of the pinned class.
    pub(crate) fn pinned_bytes(&self) -> Result<u64> {
        let Some(lazy) = self.lazy() else {
            return Ok(0);
        };
        Ok(self
            .retention()?
            .retention
            .pinned
            .iter()
            .filter_map(|key| lazy.mirror.cached(key))
            .sum())
    }

    /// While the scope lives, reads of the table's listed keys are the reads of a write.
    pub(crate) fn write_scope(&self, overlay: &TableOverlay) -> Result<Option<WriteScope>> {
        let Some(lazy) = self.lazy() else {
            return Ok(None);
        };
        let (name, _, _) = overlay.local_view(self)?;
        let known = locked(&lazy.scopes).get(&name).cloned();
        let keys = match known {
            Some(keys) => keys,
            None => {
                let keys = Arc::new(
                    self.recorded(&name)?
                        .map(|recorded| {
                            recorded
                                .files
                                .iter()
                                .map(|file| file.key.clone())
                                .collect::<HashSet<_>>()
                        })
                        .unwrap_or_default(),
                );
                locked(&lazy.scopes).insert(name, keys.clone());
                keys
            }
        };
        Ok(Some(
            lazy.mirror.write_scope(&overlay.selection.table, keys),
        ))
    }

    /// E4 for a checkpointed lazy snapshot whose cloud root is not recorded.
    pub(crate) fn check_recorded(&self, overlay: &TableOverlay) -> Result<()> {
        if self.lazy().is_none() {
            return Ok(());
        }
        let (name, _, _) = overlay.local_view(self)?;
        anyhow::ensure!(
            !name.starts_with("snapshot_") || self.recorded(&name)?.is_some(),
            "Table '{}' is available offline on this device, but its local copy could not be opened: {NOT_RECORDED}",
            overlay.selection.table
        );
        Ok(())
    }

    pub(crate) async fn mirror_state(&self, overlay: &TableOverlay) -> Result<MirrorState> {
        let lazy = self.lazy().context("Offline table mirror is not lazy")?;
        let (name, _, _) = overlay.local_view(self)?;
        let files = self
            .recorded(&name)?
            .map(|recorded| recorded.files.clone())
            .unwrap_or_default();
        let cached = files
            .iter()
            .map(|file| lazy.mirror.cached(&file.key))
            .collect::<Vec<_>>();
        let key_indexed = match overlay.local_table(self).await {
            Ok(Some(table)) => {
                offline_mirror::key_indexed(&table, &overlay.selection.primary_key, &files)
                    .await
                    .unwrap_or(None)
            }
            _ => None,
        };
        Ok(MirrorState {
            cached_bytes: cached.iter().flatten().sum(),
            total_bytes: files.iter().map(|file| file.bytes).sum::<Option<u64>>(),
            offline_complete: cached.iter().all(Option::is_some),
            downloading: locked(&lazy.downloading).contains(&overlay.key),
            key_indexed,
        })
    }

    /// The Lazy-mode idle pass: garbage eviction, snapshot rows and due refresh jobs. No
    /// network I/O; the mirror task runs the jobs.
    pub(crate) async fn lazy_idle_pass(&self) -> Result<()> {
        let Some(lazy) = self.lazy() else {
            return Ok(());
        };
        self.push_retention()?;
        lazy.mirror.evict_garbage(GARBAGE_GRACE).await?;
        self.prune_snapshots()?;
        let stale = lazy.mirror.stale_keys().into_iter().collect::<HashSet<_>>();
        let now = fs::unix_time()?;
        let instant = Instant::now();
        for overlay in self.overlays() {
            if self.queue.has_pending(&overlay.key)? {
                continue;
            }
            let (name, branch, base) = overlay.local_view(self)?;
            let recorded = self.recorded(&name)?;
            if recorded
                .as_ref()
                .is_some_and(|recorded| recorded.files.iter().any(|file| stale.contains(&file.key)))
            {
                lazy.queue_refresh(&overlay.key, Order::MetadataFirst, None);
                continue;
            }
            let retry = locked(&lazy.retry_at).get(&overlay.key).copied();
            match retry {
                Some(at) if at <= instant => {
                    locked(&lazy.retry_at).remove(&overlay.key);
                    lazy.queue_refresh(&overlay.key, Order::FetchFirst, None);
                    continue;
                }
                Some(_) => continue,
                None => {}
            }
            let since = now.saturating_sub(overlay.last_refresh.load(Ordering::Acquire));
            let rebase = branch != "main"
                || recorded
                    .as_ref()
                    .is_some_and(|recorded| Some(recorded.version) != base);
            if (rebase && since >= REFRESH_RETRY.as_secs() as i64)
                || since >= self.refresh_interval.as_secs() as i64
            {
                lazy.queue_refresh(&overlay.key, Order::FetchFirst, None);
            }
        }
        lazy.wake();
        Ok(())
    }

    /// Runs one refresh job on the mirror task and waits for it; inline when the task does
    /// not run or has ended.
    pub(crate) async fn refresh_lazy(&self, overlay: &Arc<TableOverlay>) -> Result<RefreshOutcome> {
        let lazy = self.lazy().context("Offline table mirror is not lazy")?;
        let (sender, receiver) = oneshot::channel();
        if lazy.running() && lazy.queue_refresh(&overlay.key, Order::FetchFirst, Some(sender)) {
            return match receiver.await {
                Ok(outcome) => outcome.map_err(|error| anyhow!(error)),
                Err(_) if self.is_closed() => Err(anyhow!(crate::outbox::CLOSED)),
                Err(_) => Err(anyhow!(MIRROR_STOPPED)),
            };
        }
        overlay
            .refresh_lazy(self, Order::FetchFirst, None)
            .await
            .inspect_err(|error| self.refresh_failed(&overlay.key, error))
    }

    /// Records the mirror error of a registered table; removed tables keep no settings.
    fn refresh_failed(&self, resource: &str, error: &anyhow::Error) {
        if self.overlay(resource).is_none() {
            return;
        }
        let _ = self.queue.mirror_error(
            resource,
            Some(&format!("Could not refresh from the cloud: {error}")),
        );
        self.host.mirror_changed();
    }

    /// Switches a held table to the cloud's current version metadata-first. A checkpoint
    /// that lost a race probes again.
    pub(crate) async fn activate_lazy(&self, overlay: &TableOverlay) -> Result<()> {
        for _ in 0..crate::manager::ACTIVATION_ATTEMPTS {
            let outcome = overlay
                .refresh_lazy(self, Order::MetadataFirst, None)
                .await?;
            if outcome != RefreshOutcome::Deferred || self.queue.has_pending(&overlay.key)? {
                return Ok(());
            }
        }
        anyhow::bail!(
            "concurrent refreshes replaced the local copy {} times; try again",
            crate::manager::ACTIVATION_ATTEMPTS
        )
    }

    pub(crate) fn spawn_mirror(self: &Arc<Self>) {
        let Some(lazy) = self.lazy() else {
            return;
        };
        let mut task = locked(&lazy.task);
        if task.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        locked(&lazy.jobs).stopped = self.is_closed();
        *task = Some(tokio::spawn(Self::mirror_loop(Arc::downgrade(self))));
    }

    async fn mirror_loop(weak: Weak<Self>) {
        let _exit = TaskExit(weak.clone());
        loop {
            let Some(manager) = weak.upgrade() else { break };
            if manager.is_closed() {
                break;
            }
            let Some(lazy) = manager.lazy() else { break };
            let job = locked(&lazy.jobs).queue.pop_front();
            if let Some(job) = job {
                manager.run_job(job).await;
                continue;
            }
            if manager.prefetch_step().await {
                continue;
            }
            tokio::select! {
                _ = lazy.notify.notified() => (),
                _ = tokio::time::sleep(manager.idle_poll) => (),
                _ = manager.stop.notified() => (),
            }
        }
    }

    async fn run_job(&self, job: Job) {
        let Some(lazy) = self.lazy() else { return };
        match job {
            Job::Refresh {
                resource,
                order,
                waiters,
            } => {
                let result = match self.overlay(&resource) {
                    Some(overlay) => overlay.refresh_lazy(self, order, None).await,
                    None => Err(anyhow!("Offline access for this table was turned off")),
                };
                if let Err(error) = &result {
                    tracing::debug!(%error, "Keeping the current offline table snapshot");
                    self.refresh_failed(&resource, error);
                }
                for waiter in waiters {
                    let _ = waiter.send(
                        result
                            .as_ref()
                            .map(|outcome| *outcome)
                            .map_err(|error| error.to_string()),
                    );
                }
            }
            Job::Fetch { resource, files } => {
                let Some(options) = self.lazy_options() else {
                    return;
                };
                let uncached = files
                    .iter()
                    .filter(|file| lazy.mirror.cached(&file.key).is_none())
                    .map(|file| file.bytes.unwrap_or(0))
                    .sum::<u64>();
                if uncached > options.max_refresh_fetch_bytes {
                    return;
                }
                locked(&lazy.downloading).insert(resource.clone());
                if let Err(error) = lazy.mirror.fetch(&files, false).await
                    && !error.is::<MirrorShortage>()
                {
                    tracing::debug!(%error, "Offline table files will download on use");
                }
                locked(&lazy.downloading).remove(&resource);
                self.host.mirror_changed();
            }
        }
    }

    /// One Download everything step of one table. False when no table needs files now.
    async fn prefetch_step(&self) -> bool {
        let Some(lazy) = self.lazy() else {
            return false;
        };
        if self
            .connectivity
            .as_ref()
            .is_some_and(|connectivity| connectivity.is_offline())
        {
            return false;
        }
        let now = Instant::now();
        for overlay in self.overlays() {
            let key = &overlay.key;
            if !self.queue.prefetch(key).unwrap_or(false) {
                if locked(&lazy.downloading).remove(key) {
                    self.host.mirror_changed();
                }
                continue;
            }
            if locked(&lazy.prefetch_retry)
                .get(key)
                .is_some_and(|at| *at > now)
            {
                continue;
            }
            let Ok((name, _, _)) = overlay.local_view(self) else {
                continue;
            };
            let Ok(Some(recorded)) = self.recorded(&name) else {
                continue;
            };
            let files = self.next_prefetch(lazy, &recorded.files);
            if files.is_empty() {
                if locked(&lazy.downloading).remove(key) {
                    self.prefetch_caught_up(key);
                }
                continue;
            }
            locked(&lazy.downloading).insert(key.clone());
            if let Err(error) = lazy.mirror.fetch(&files, true).await {
                locked(&lazy.downloading).remove(key);
                locked(&lazy.prefetch_retry).insert(key.clone(), now + self.idle_poll);
                let _ = self
                    .queue
                    .mirror_error(key, Some(&shortage_text(&overlay.selection.table, &error)));
                self.host.mirror_changed();
            }
            return true;
        }
        false
    }

    /// The next uncached files of a Download everything table.
    fn next_prefetch(&self, lazy: &Lazy, files: &[MirrorFile]) -> Vec<MirrorFile> {
        files
            .iter()
            .filter(|file| lazy.mirror.cached(&file.key).is_none())
            .take(PREFETCH_STEP)
            .cloned()
            .collect()
    }

    fn prefetch_caught_up(&self, resource: &str) {
        if let Ok(Some(error)) = self.queue.table_mirror_error(resource)
            && is_shortage(&error)
        {
            let _ = self.queue.mirror_error(resource, None);
        }
        self.host.mirror_changed();
    }

    /// Frees cache space down to `target_used`.
    pub(crate) async fn evict_lazy(&self, target_used: u64) -> Result<u64> {
        match self.lazy() {
            Some(lazy) => lazy.mirror.evict_to(target_used, true).await,
            None => Ok(0),
        }
    }
}

#[cfg(test)]
impl WriteManager {
    /// When a dropped refresh of `resource` is due again.
    pub(crate) fn refresh_retry_due(&self, resource: &str) -> Option<Instant> {
        locked(&self.lazy()?.retry_at).get(resource).copied()
    }

    /// Jobs waiting for the mirror task.
    pub(crate) fn queued_jobs(&self) -> usize {
        self.lazy().map_or(0, |lazy| locked(&lazy.jobs).queue.len())
    }
}

#[async_trait::async_trait]
impl MirrorHost for WriteManager {
    fn is_offline(&self) -> bool {
        self.connectivity
            .as_ref()
            .is_some_and(|connectivity| connectivity.is_offline())
    }

    async fn cloud(&self) -> Result<Arc<dyn ObjectStore>> {
        self.host.remote_objects().await
    }

    fn classify(&self, error: &object_store::Error) -> Option<MirrorFetch> {
        classify(error)
    }

    fn observe(&self, outcome: MirrorFetch) {
        if let Some(connectivity) = &self.connectivity {
            connectivity.observe(outcome);
        }
    }

    fn allow_download(&self, bytes: u64, force: bool) -> bool {
        let (Some(lazy), Some(options)) = (self.lazy(), self.lazy_options()) else {
            return force;
        };
        let today = fs::unix_time()
            .and_then(|now| self.queue.downloaded_today(now))
            .unwrap_or(u64::MAX);
        let mut reserved = locked(&lazy.reserved);
        if !force
            && today.saturating_add(*reserved).saturating_add(bytes)
                > options.max_download_bytes_per_day
        {
            return false;
        }
        *reserved = reserved.saturating_add(bytes);
        true
    }

    fn downloaded(&self, reserved: u64, transferred: u64) {
        if let Some(lazy) = self.lazy() {
            let mut open = locked(&lazy.reserved);
            *open = open.saturating_sub(reserved);
        }
        if let Err(error) =
            fs::unix_time().and_then(|now| self.queue.record_download(now, transferred))
        {
            tracing::warn!(%error, "Could not record offline mirror download bytes");
        }
    }

    fn changed(&self) {
        self.host.mirror_changed();
        if self
            .lazy()
            .is_some_and(|lazy| !lazy.mirror.stale_keys().is_empty())
        {
            self.wake();
        }
    }
}

impl TableOverlay {
    /// A refresh of the lazy mirror (§2.8): the metadata step, then the fetch rules and the
    /// checkpoint in `order`. Deferred while the lane has queued changes.
    pub(crate) async fn refresh_lazy(
        &self,
        manager: &WriteManager,
        order: Order,
        remote: Option<Table>,
    ) -> Result<RefreshOutcome> {
        let lazy = manager.lazy().context("Offline table mirror is not lazy")?;
        manager.authorize()?;
        if manager.queue.has_pending(&self.key)? {
            return Ok(RefreshOutcome::Deferred);
        }
        let now = fs::unix_time()?;
        self.last_refresh.store(now, Ordering::Release);
        let previous = manager
            .queue
            .resource_revision(&self.key)?
            .context("Missing offline table revision")?;
        let (current_name, branch, local_base) = self.local_view(manager)?;
        let current = manager.recorded(&current_name)?;
        let step = tokio::time::timeout(
            METADATA_STEP,
            self.metadata_step(
                manager,
                lazy,
                remote,
                &previous,
                branch != "main"
                    || current
                        .as_ref()
                        .is_some_and(|current| Some(current.version) != local_base),
                current.as_deref(),
            ),
        )
        .await
        .map_err(|_| {
            anyhow!(
                "the cloud table did not answer within {} s",
                METADATA_STEP.as_secs()
            )
        })??;
        let (name, snapshot) = match step {
            Step::Missing => return self.remote_absent(manager, &previous).await,
            Step::Unchanged => {
                let files = current.as_ref().map_or(&[][..], |current| &current.files);
                let listed = lazy
                    .mirror
                    .stale_keys()
                    .into_iter()
                    .filter(|key| files.iter().any(|file| file.key == *key))
                    .collect::<Vec<_>>();
                lazy.mirror.clear_stale(&listed);
                self.confirm_lazy(manager, lazy, files, now)?;
                return Ok(RefreshOutcome::Unchanged);
            }
            Step::Snapshot(name, snapshot) => (name, snapshot),
        };
        let prefetch = manager.queue.prefetch(&self.key)?;
        lazy.prepare(&name, &snapshot.files, prefetch);
        let outcome = self
            .switch(
                manager,
                lazy,
                &name,
                &snapshot,
                current.as_deref(),
                &previous,
                prefetch,
                order,
                now,
            )
            .await;
        lazy.unprepare(&name);
        if !matches!(outcome, Ok(RefreshOutcome::Refreshed { .. })) {
            drop(snapshot);
            if let Err(error) = manager.local()?.drop_table(&name, &[]).await {
                tracing::debug!(%error, "Could not drop a dropped offline snapshot");
            }
            manager.prune_snapshots()?;
        }
        if let Err(error) = manager.push_retention() {
            tracing::warn!(%error, "Could not update the offline mirror retention after a refresh");
        }
        outcome
    }

    /// `confirm_remote`, except that a Download everything table keeps its shortage error
    /// until its download caught up.
    fn confirm_lazy(
        &self,
        manager: &WriteManager,
        lazy: &Lazy,
        files: &[MirrorFile],
        now: i64,
    ) -> Result<()> {
        let short = manager.queue.prefetch(&self.key)?
            && manager
                .queue
                .table_mirror_error(&self.key)?
                .is_some_and(|error| is_shortage(&error))
            && files
                .iter()
                .any(|file| lazy.mirror.cached(&file.key).is_none());
        if short {
            self.record_confirmed(manager, now)
        } else {
            self.confirm_remote(manager, now)
        }
    }

    async fn metadata_step(
        &self,
        manager: &WriteManager,
        lazy: &Lazy,
        remote: Option<Table>,
        previous: &Value,
        rebase: bool,
        current: Option<&RecordedSnapshot>,
    ) -> Result<Step> {
        let remote = match remote {
            Some(remote) => remote,
            None => match manager.host.remote_table(&self.selection).await? {
                Some(remote) => remote,
                None => return Ok(Step::Missing),
            },
        };
        let (version, fingerprint) = offline_replay::revision(&remote).await?;
        let expected = serde_json::to_value(OfflineExpected::TableVersion {
            version,
            fingerprint: Some(fingerprint),
        })?;
        if expected == *previous && !rebase {
            return Ok(Step::Unchanged);
        }
        let name = format!("snapshot_{}", uuid::Uuid::new_v4().simple());
        let previous_files: &[MirrorFile] = match current {
            Some(current) => &current.files,
            None => &[],
        };
        let snapshot = lazy
            .mirror
            .snapshot(&remote, &manager.local()?, &name, previous_files)
            .await?;
        Ok(Step::Snapshot(name, snapshot))
    }

    #[allow(clippy::too_many_arguments)]
    async fn switch(
        &self,
        manager: &WriteManager,
        lazy: &Lazy,
        name: &str,
        snapshot: &LazySnapshot,
        current: Option<&RecordedSnapshot>,
        previous: &Value,
        prefetch: bool,
        order: Order,
        now: i64,
    ) -> Result<RefreshOutcome> {
        let local_version = snapshot.table.table.version().await?;
        manager
            .queue
            .record_snapshot(name, &snapshot.root, local_version, &snapshot.files)?;
        manager.push_retention()?;
        let current_files: &[MirrorFile] = match current {
            Some(current) => &current.files,
            None => &[],
        };
        let delta = snapshot.delta(current_files, &lazy.mirror);
        let mut downloaded = 0;
        if order == Order::FetchFirst {
            locked(&lazy.downloading).insert(self.key.clone());
            let fetched = self
                .fetch_first(manager, lazy, snapshot, &delta, prefetch)
                .await;
            locked(&lazy.downloading).remove(&self.key);
            match fetched? {
                Some(bytes) => downloaded = bytes,
                None => return Ok(RefreshOutcome::Deferred),
            }
        }
        let revision = serde_json::to_value(OfflineExpected::TableVersion {
            version: snapshot.table.source_version,
            fingerprint: Some(snapshot.table.source_fingerprint.clone()),
        })?;
        let replaced = {
            let _guard = manager.gate.lock().await;
            manager.authorize()?;
            let (old_name, _, _) = self.local_view(manager)?;
            let _rows = manager.hold_snapshot_rows();
            let replaced = !manager.queue.has_pending(&self.key)?
                && manager.queue.checkpoint(
                    &self.key,
                    name,
                    Some(local_version),
                    &revision,
                    previous,
                )?;
            if replaced && let Err(error) = manager.retire(&old_name) {
                tracing::warn!(%error, table = %old_name, "Could not retire a replaced offline snapshot");
            }
            replaced
        };
        if !replaced {
            lazy.retry_soon(&self.key);
            return Ok(RefreshOutcome::Deferred);
        }
        self.generation.fetch_add(1, Ordering::Release);
        if let Err(error) = self.confirm_lazy(manager, lazy, &snapshot.files, now) {
            tracing::warn!(%error, "Could not record the refreshed offline table state");
        }
        if let Err(error) = manager.push_retention() {
            tracing::warn!(%error, "Could not update the offline mirror retention after a refresh");
        }
        if order == Order::MetadataFirst {
            if prefetch {
                lazy.wake();
            } else {
                lazy.queue_fetch(&self.key, delta);
            }
        }
        manager.host.mirror_changed();
        Ok(RefreshOutcome::Refreshed {
            version: snapshot.table.source_version,
            downloaded_bytes: downloaded,
        })
    }

    /// The fetch of an idle refresh: everything for Download everything tables, the capped
    /// delta for the others. None: deferred, with the table's mirror error.
    async fn fetch_first(
        &self,
        manager: &WriteManager,
        lazy: &Lazy,
        snapshot: &LazySnapshot,
        delta: &[MirrorFile],
        prefetch: bool,
    ) -> Result<Option<u64>> {
        if prefetch {
            return match lazy.mirror.fetch(&snapshot.files, true).await {
                Ok(bytes) => Ok(Some(bytes)),
                Err(error) => {
                    manager.queue.mirror_error(
                        &self.key,
                        Some(&shortage_text(&self.selection.table, &error)),
                    )?;
                    manager.host.mirror_changed();
                    Ok(None)
                }
            };
        }
        let cap = manager
            .lazy_options()
            .map_or(0, |options| options.max_refresh_fetch_bytes);
        let uncached = delta
            .iter()
            .filter(|file| lazy.mirror.cached(&file.key).is_none())
            .map(|file| file.bytes.unwrap_or(0))
            .sum::<u64>();
        if uncached > cap {
            return Ok(Some(0));
        }
        match lazy.mirror.fetch(delta, false).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.is::<MirrorShortage>() => Ok(Some(0)),
            Err(error) => {
                manager.queue.mirror_error(
                    &self.key,
                    Some(&format!("Could not refresh from the cloud: {error}")),
                )?;
                manager.host.mirror_changed();
                Ok(None)
            }
        }
    }

    /// A lazy snapshot of an absent baseline, checked by `setup` before anything is recorded.
    pub(crate) async fn initialize_lazy(
        &self,
        manager: &WriteManager,
        remote: Table,
        setup: crate::manager::TableSetup,
    ) -> Result<()> {
        let lazy = manager.lazy().context("Offline table mirror is not lazy")?;
        let name = format!("snapshot_{}", uuid::Uuid::new_v4().simple());
        let local = manager.local()?;
        let snapshot = lazy.mirror.snapshot(&remote, &local, &name, &[]).await?;
        lazy.prepare(&name, &snapshot.files, setup.prefetch);
        let recorded = async {
            manager.push_retention()?;
            if setup.validate_key {
                crate::table::validate_key(
                    remote,
                    snapshot.table.source_version,
                    &self.selection.table,
                    &self.selection.primary_key,
                    &manager.spill_directory(),
                    manager.key_validation_memory(),
                )
                .await?;
            }
            if setup.prefetch {
                manager.check_download_everything(
                    &self.selection.table,
                    Candidate {
                        resource: &self.key,
                        local_name: &name,
                        files: &snapshot.files,
                    },
                )?;
            }
            let local_version = snapshot.table.table.version().await?;
            manager
                .queue
                .record_snapshot(&name, &snapshot.root, local_version, &snapshot.files)?;
            manager.queue.record_refresh(&self.key, fs::unix_time()?)?;
            manager.queue.initialize_table(
                &self.key,
                &serde_json::to_value(OfflineExpected::TableVersion {
                    version: snapshot.table.source_version,
                    fingerprint: Some(snapshot.table.source_fingerprint.clone()),
                })?,
                Some(local_version),
                Some(&name),
                setup.prefetch,
            )
        }
        .await;
        lazy.unprepare(&name);
        if let Err(error) = recorded {
            drop(snapshot);
            if let Err(drop_error) = local.drop_table(&name, &[]).await {
                tracing::warn!(%drop_error, "Could not drop a rejected offline snapshot");
            }
            manager.prune_snapshots()?;
            manager.push_retention()?;
            return Err(error);
        }
        Ok(())
    }

    /// E36 for misses; for a stale key this table lists, one immediate stale refresh and a
    /// retry when the lane is idle, E37 otherwise.
    pub(crate) async fn recover_read(
        &self,
        manager: &WriteManager,
        error: anyhow::Error,
        retried: bool,
    ) -> flow_like_storage::databases::vector::lancedb::ReadRecovery {
        use flow_like_storage::databases::vector::lancedb::ReadRecovery;
        let table = &self.selection.table;
        if is_mirror_miss(&error) {
            return ReadRecovery::Fail(anyhow!(missing_data_text(table)));
        }
        let Some(key) = mirror_stale_key(&error) else {
            return ReadRecovery::Fail(error);
        };
        let listed = self
            .local_view(manager)
            .and_then(|(name, _, _)| manager.recorded(&name))
            .ok()
            .flatten()
            .is_some_and(|recorded| recorded.files.iter().any(|file| file.key == key));
        if !listed {
            return ReadRecovery::Fail(error);
        }
        let pending = manager.queue.has_pending(&self.key).unwrap_or(true);
        if !retried && !pending && !MirrorHost::is_offline(manager) {
            match self.refresh_lazy(manager, Order::MetadataFirst, None).await {
                Ok(_) => return ReadRecovery::Retry,
                Err(refresh) => {
                    tracing::debug!(%refresh, "Could not refresh a reorganized offline table")
                }
            }
        }
        if pending {
            let _ = manager
                .queue
                .mirror_error(&self.key, Some(REORGANIZED_PENDING));
            manager.host.mirror_changed();
        }
        ReadRecovery::Fail(anyhow!(reorganized_text(table)))
    }

    /// The freeze's reads, outside the manager gate, so their misses download before the
    /// gate is taken. Misses and full disks fail here; other errors surface in the freeze.
    pub(crate) async fn warm_up(
        &self,
        manager: &WriteManager,
        mutation: &flow_like_storage::databases::vector::lancedb::LogicalTableMutation,
    ) -> Result<()> {
        let Ok(Some(table)) = self.local_table(manager).await else {
            return Ok(());
        };
        match self.freeze(manager, Some(&table), mutation.clone()).await {
            Err(error) if is_mirror_failure(&error) => {
                Err(mirror_failure(&self.selection.table, error))
            }
            _ => Ok(()),
        }
    }

    /// Maps mirror misses and stale reads of the local view to a lane-local failure.
    pub(crate) fn lazy_recovery_error(
        &self,
        manager: &WriteManager,
        error: anyhow::Error,
    ) -> std::result::Result<anyhow::Error, anyhow::Error> {
        if manager.lazy().is_some()
            && (is_mirror_miss(&error) || mirror_stale_key(&error).is_some())
        {
            Ok(MirrorUnavailable(mirror_failure(&self.selection.table, error).to_string()).into())
        } else {
            Err(error)
        }
    }
}
