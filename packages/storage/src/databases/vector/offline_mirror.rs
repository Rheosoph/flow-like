//! Lazy offline table mirror: manifest-only snapshots of cloud tables whose immutable data,
//! deletion and index files are cached whole on this device when they are read.

use std::{
    collections::{HashMap, HashSet},
    fmt,
    io::{Read, Seek, SeekFrom},
    ops::Range,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, RwLock, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, anyhow, ensure};
use flow_like_types::{Bytes, reqwest::Url, tokio};
use futures::{StreamExt, TryStreamExt, stream::BoxStream};
use lance::Dataset;
use lance_io::object_store::{
    ObjectStore as LanceStore, ObjectStoreParams, ObjectStoreProvider, ObjectStoreRegistry,
    uri_to_url,
};
use lance_table::{
    feature_flags::apply_feature_flags,
    format::{IndexFile, IndexMetadata, Manifest, RowDatasetVersionMeta, RowIdMeta},
    io::{
        commit::{ManifestNamingScheme, commit_handler_from_url, write_manifest_file_to_path},
        deletion::deletion_file_path,
        manifest::read_manifest_indexes,
    },
    system_index::mem_wal::MEM_WAL_INDEX_NAME,
};
use lancedb::{Connection, Table};
use object_store::{
    GetOptions, GetResult, GetResultPayload, ListResult, ObjectMeta, ObjectStore, ObjectStoreExt,
    path::Path,
};

use super::offline_replay::{MaterializedTable, WriteBudget, fingerprint, sync_directory};

/// Stable part of E36. Lance flattens object-store errors into text, so misses are
/// recognized by this text, never by type.
pub const MIRROR_MISS: &str = "needs data that is not on this device yet";
/// Stable part of E37, carried by the NotFound of a listed key that the cloud no longer has.
pub const MIRROR_STALE: &str = "was reorganized in the cloud after this device took its copy";
/// Stable part of E39.
pub const MIRROR_FULL: &str = "Offline storage on this device is full";

const STORE: &str = "offline-mirror";
/// Charged per cached file on top of its bytes, like every local file.
const FILE_OVERHEAD: u64 = 4096;
const RECENT_USE: Duration = Duration::from_secs(10 * 60);
const FIRST_BYTE: Duration = Duration::from_secs(10);
const BREAKER_POLL: Duration = Duration::from_millis(250);
const MAX_DOWNLOADS: usize = 4;
const MAX_HEADS: usize = 256;
const HEAD_CONCURRENCY: usize = 8;
const AUTO_CLEANUP: &str = "lance.auto_cleanup.";

/// E36 for `table`.
pub fn missing_data_text(table: &str) -> String {
    format!(
        "Table '{table}' {MIRROR_MISS}. Reconnect to the hub, or turn on \"Download everything\" for this table while connected."
    )
}

/// E37 for `table`.
pub fn reorganized_text(table: &str) -> String {
    format!(
        "Part of table '{table}' {MIRROR_STALE}. The copy refreshes automatically once the table has no queued changes; try again then."
    )
}

/// E39 for `table`.
pub fn storage_full_text(table: &str, needed: u64) -> String {
    format!(
        "{MIRROR_FULL}: table '{table}' needs {} more for this change. Increase the limit on the Offline access page, or turn off \"Download everything\" for another table.",
        format_bytes(needed)
    )
}

/// `N MiB` for whole MiB, otherwise `N.N MB`.
pub fn format_bytes(bytes: u64) -> String {
    const MIB: u64 = 1024 * 1024;
    if bytes.is_multiple_of(MIB) {
        format!("{} MiB", bytes / MIB)
    } else {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    }
}

/// True when the text of any cause contains MIRROR_MISS.
pub fn is_mirror_miss(error: &anyhow::Error) -> bool {
    error
        .chain()
        .any(|cause| cause.to_string().contains(MIRROR_MISS))
}

/// The stale key named by an error whose text contains MIRROR_STALE.
pub fn mirror_stale_key(error: &anyhow::Error) -> Option<String> {
    error.chain().find_map(|cause| {
        let text = cause.to_string();
        let after = &text[text.find(MIRROR_STALE)?..];
        let start = after.find(" (")? + 2;
        let end = after[start..].find(')')?;
        Some(after[start..start + end].to_string())
    })
}

/// The table a cloud key belongs to: its `<name>.lance` segment.
fn table_of(key: &Path) -> String {
    key.parts()
        .find_map(|part| part.as_ref().strip_suffix(".lance").map(str::to_string))
        .unwrap_or_else(|| key.to_string())
}

fn unsupported(table: &str, detail: &str) -> anyhow::Error {
    anyhow!(
        "Table '{table}' uses a storage layout that offline access does not support yet: {detail}."
    )
}

fn cache_name(key: &str) -> String {
    blake3::hash(key.as_bytes()).to_hex().to_string()
}

fn recent(used: SystemTime, now: SystemTime) -> bool {
    now.duration_since(used).unwrap_or_default() < RECENT_USE
}

/// A cloud database root: the Lance URI the cloud connection uses and its bucket-relative key.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirrorRoot {
    /// "s3://bucket/apps/p/storage/db"
    pub url: String,
    /// "apps/p/storage/db"
    pub key: String,
}

impl MirrorRoot {
    /// `key` plus the URL path below `url`, verbatim: percent escapes stay key bytes, as in
    /// the lease's Lance binding. None outside the root.
    pub fn key_of(&self, url: &Url) -> Option<Path> {
        let root = uri_to_url(&self.url).ok()?;
        if url.scheme() != root.scheme()
            || url.authority() != root.authority()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        let suffix = url.path().strip_prefix(root.path().trim_end_matches('/'))?;
        if !suffix.is_empty() && !suffix.starts_with('/') {
            return None;
        }
        Path::parse(format!("{}{suffix}", self.key)).ok()
    }

    /// The database root of a cloud table: the parent of its URI and of the object path its
    /// store derived, so keys are spelled like the cloud binding spells them.
    fn of(dataset: &Dataset) -> Result<Self> {
        let (url, _) = dataset
            .uri()
            .trim_end_matches('/')
            .rsplit_once('/')
            .with_context(|| format!("Cloud table URI '{}' has no database", dataset.uri()))?;
        let path = dataset.branch_location().path;
        let parts = path.parts().collect::<Vec<_>>();
        ensure!(
            parts.len() > 1,
            "Cloud table path '{path}' has no database directory"
        );
        let key = Path::from_iter(parts[..parts.len() - 1].iter().cloned());
        Ok(Self {
            url: url.to_string(),
            key: key.to_string(),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MirrorFileKind {
    Data,
    Deletion,
    Index,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MirrorFile {
    pub key: String,
    pub bytes: Option<u64>,
    pub kind: MirrorFileKind,
}

/// Cache classes over listed keys. A cached key that is not listed is garbage.
#[derive(Clone, Debug, Default)]
pub struct MirrorRetention {
    /// Every listed key of current snapshots, retired snapshots within their grace and
    /// snapshots being prepared, with its listed size.
    pub listed: HashMap<String, Option<u64>>,
    /// Listed keys that are never evicted.
    pub pinned: HashSet<String>,
    /// Listed keys of Download everything tables: kept whole whatever their size.
    pub whole: HashSet<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorFetch {
    Succeeded,
    ConnectFailed,
    TimedOut,
}

/// Why `LazyMirror::fetch` downloaded nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MirrorShortage {
    /// The files do not fit the disk limit, even after eviction; `needed` more bytes.
    Disk { needed: u64 },
    /// Today's download allowance is used up.
    Allowance,
}

impl fmt::Display for MirrorShortage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disk { needed } => write!(
                formatter,
                "the offline storage limit is {} short",
                format_bytes(*needed)
            ),
            Self::Allowance => formatter.write_str("the daily download allowance is used up"),
        }
    }
}

impl std::error::Error for MirrorShortage {}

/// Implemented by the engine from its host, breaker and outbox.
#[async_trait::async_trait]
pub trait MirrorHost: Send + Sync {
    /// True while the scope's connectivity breaker is open; misses then fail at once.
    fn is_offline(&self) -> bool;
    /// Bucket-relative cloud store with current credentials.
    async fn cloud(&self) -> Result<Arc<dyn ObjectStore>>;
    /// Some for offline-class errors: ConnectFailed, or TimedOut for timeouts.
    fn classify(&self, error: &object_store::Error) -> Option<MirrorFetch>;
    /// Transport outcome of a cloud call, for the connectivity breaker.
    fn observe(&self, outcome: MirrorFetch);
    /// Reserves `bytes` of today's download allowance; false once it is used up. `force`
    /// (the downloads of writes) always reserves.
    fn allow_download(&self, bytes: u64, force: bool) -> bool;
    /// Ends a reservation and charges the bytes actually received.
    fn downloaded(&self, reserved: u64, transferred: u64);
    /// A download finished, files were evicted or a key went stale.
    fn changed(&self);
}

pub struct LazyMirrorSetup {
    /// `<engine scope>/mirror`
    pub directory: PathBuf,
    pub roots: Vec<MirrorRoot>,
    pub max_lazy_file_bytes: u64,
}

pub struct LazySnapshot {
    /// The local table; source version and fingerprint of the cloud manifest.
    pub table: MaterializedTable,
    /// Every cloud file the snapshot refers to.
    pub files: Vec<MirrorFile>,
    pub root: MirrorRoot,
    /// Deletion file key → the data file keys of its fragment.
    owners: HashMap<String, Vec<String>>,
}

impl LazySnapshot {
    /// The data and deletion files this snapshot lists that `current` does not, without
    /// deletion files of fragments whose data files are neither cached nor in the delta.
    pub fn delta(&self, current: &[MirrorFile], mirror: &LazyMirror) -> Vec<MirrorFile> {
        let known = current
            .iter()
            .map(|file| file.key.as_str())
            .collect::<HashSet<_>>();
        let new = self
            .files
            .iter()
            .filter(|file| file.kind != MirrorFileKind::Index && !known.contains(file.key.as_str()))
            .collect::<Vec<_>>();
        let fetched = new
            .iter()
            .filter(|file| file.kind == MirrorFileKind::Data)
            .map(|file| file.key.as_str())
            .collect::<HashSet<_>>();
        new.into_iter()
            .filter(|file| {
                file.kind == MirrorFileKind::Data
                    || self.owners.get(&file.key).is_none_or(|data| {
                        data.iter().all(|key| {
                            fetched.contains(key.as_str()) || mirror.cached(key).is_some()
                        })
                    })
            })
            .cloned()
            .collect()
    }
}

struct Bound {
    root: MirrorRoot,
    url: Url,
    key: Path,
}

impl Bound {
    fn key_of(&self, url: &Url) -> Option<Path> {
        if url.scheme() != self.url.scheme() {
            return None;
        }
        self.root.key_of(url)
    }
}

#[derive(Default)]
struct Classes {
    /// cache name → (key, listed size)
    listed: HashMap<String, (String, Option<u64>)>,
    pinned: HashSet<String>,
    whole: HashSet<String>,
    /// `_indices/<uuid>` directories with listed files.
    directories: HashSet<String>,
}

struct Cached {
    bytes: u64,
    used: SystemTime,
    reads: u32,
}

struct Scope {
    id: u64,
    table: String,
    keys: Arc<HashSet<String>>,
}

#[derive(Default)]
struct State {
    /// cache name → file
    cached: HashMap<String, Cached>,
    classes: Classes,
    stale: HashSet<String>,
    scopes: Vec<Scope>,
    in_flight: HashSet<String>,
    next_scope: u64,
}

enum How<'a> {
    /// After an online read: skipped when every download slot is busy, the allowance has no
    /// room, or it does not fit without evicting pinned or recent files.
    Background,
    /// The reads of a write: may evict any live file, ignores the allowance.
    Write(&'a str),
    Fetch {
        recent: bool,
    },
}

enum DownloadError {
    Store(object_store::Error),
    Short(MirrorShortage),
}

impl From<object_store::Error> for DownloadError {
    fn from(error: object_store::Error) -> Self {
        Self::Store(error)
    }
}

/// The disk charge and allowance reservation of one download. Unless committed, drop
/// releases the charge and removes the partial and the renamed file. Drop always ends the
/// reservation with the bytes actually transferred.
struct Download<'a> {
    mirror: &'a LazyMirror,
    host: Arc<dyn MirrorHost>,
    name: &'a str,
    reserved: u64,
    charged: u64,
    transferred: AtomicU64,
    committed: bool,
}

impl Drop for Download<'_> {
    fn drop(&mut self) {
        if !self.committed && self.charged > 0 {
            let target = self.mirror.file_path(self.name);
            let _ = std::fs::remove_file(target.with_extension("partial"));
            let _ = std::fs::remove_file(&target);
            self.mirror.budget.release(self.charged);
        }
        self.host
            .downloaded(self.reserved, self.transferred.load(Ordering::Acquire));
    }
}

/// Drops the key's download lock from the lock map once no download holds it.
struct KeyLockRelease<'a> {
    mirror: &'a LazyMirror,
    name: &'a str,
}

impl Drop for KeyLockRelease<'_> {
    fn drop(&mut self) {
        self.mirror.release_key_lock(self.name);
    }
}

/// Counts an open read of a cached file until drop.
struct OpenRead<'a> {
    mirror: &'a LazyMirror,
    name: &'a str,
}

impl Drop for OpenRead<'_> {
    fn drop(&mut self) {
        if let Some(file) = self.mirror.state().cached.get_mut(self.name) {
            file.reads = file.reads.saturating_sub(1);
        }
    }
}

pub struct LazyMirror {
    this: Weak<LazyMirror>,
    directory: PathBuf,
    budget: Arc<WriteBudget>,
    registry: Weak<ObjectStoreRegistry>,
    roots: RwLock<Vec<Bound>>,
    host: RwLock<Option<Weak<dyn MirrorHost>>>,
    state: Mutex<State>,
    locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    downloads: Arc<tokio::sync::Semaphore>,
    max_lazy_file_bytes: AtomicU64,
    first_byte_millis: AtomicU64,
    background: Mutex<Vec<tokio::task::AbortHandle>>,
    closed: AtomicBool,
}

impl fmt::Debug for LazyMirror {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LazyMirror")
            .field("directory", &self.directory)
            .finish_non_exhaustive()
    }
}

/// Ends the write-read mode of its keys on drop.
pub struct WriteScope {
    mirror: Weak<LazyMirror>,
    id: u64,
}

impl Drop for WriteScope {
    fn drop(&mut self) {
        if let Some(mirror) = self.mirror.upgrade() {
            mirror.state().scopes.retain(|scope| scope.id != self.id);
        }
    }
}

fn io_error(error: impl std::error::Error + Send + Sync + 'static) -> object_store::Error {
    object_store::Error::Generic {
        store: STORE,
        source: Box::new(error),
    }
}

fn private_directory(directory: &std::path::Path) -> std::io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory)
}

fn head_result(key: &Path, size: u64) -> GetResult {
    GetResult {
        payload: GetResultPayload::Stream(futures::stream::empty().boxed()),
        meta: ObjectMeta {
            location: key.clone(),
            last_modified: Default::default(),
            size,
            e_tag: None,
            version: None,
        },
        range: 0..0,
        attributes: Default::default(),
    }
}

fn read_range(
    file: &std::path::Path,
    range: Option<object_store::GetRange>,
    size: u64,
) -> std::io::Result<(Range<u64>, Bytes, SystemTime)> {
    let mut handle = std::fs::File::open(file)?;
    let metadata = handle.metadata()?;
    let range = match range {
        Some(range) => range
            .as_range(size)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?,
        None => 0..size,
    };
    handle.seek(SeekFrom::Start(range.start))?;
    let mut buffer = vec![0; (range.end - range.start) as usize];
    handle.read_exact(&mut buffer)?;
    Ok((range, Bytes::from(buffer), metadata.modified()?))
}

impl LazyMirror {
    /// Scans the cache directory (removing interrupted downloads), charges it to `budget`
    /// without a limit and binds the recorded roots.
    pub(super) fn open(
        setup: LazyMirrorSetup,
        budget: Arc<WriteBudget>,
        registry: &Arc<ObjectStoreRegistry>,
    ) -> Result<Arc<Self>> {
        private_directory(&setup.directory)?;
        let mut cached = HashMap::new();
        for group in std::fs::read_dir(&setup.directory)? {
            let group = group?;
            if !group.file_type()?.is_dir() {
                continue;
            }
            for entry in std::fs::read_dir(group.path())? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let metadata = std::fs::symlink_metadata(entry.path())?;
                if !metadata.is_file() {
                    continue;
                }
                if name.ends_with(".partial") {
                    std::fs::remove_file(entry.path())?;
                    continue;
                }
                if name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    cached.insert(
                        name,
                        Cached {
                            bytes: metadata.len(),
                            used: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                            reads: 0,
                        },
                    );
                }
            }
        }
        budget.add(cached.values().map(|file| file.bytes + FILE_OVERHEAD).sum());
        let mirror = Arc::new_cyclic(|this| Self {
            this: this.clone(),
            directory: setup.directory,
            budget,
            registry: Arc::downgrade(registry),
            roots: RwLock::new(Vec::new()),
            host: RwLock::new(None),
            state: Mutex::new(State {
                cached,
                ..State::default()
            }),
            locks: Mutex::new(HashMap::new()),
            downloads: Arc::new(tokio::sync::Semaphore::new(MAX_DOWNLOADS)),
            max_lazy_file_bytes: AtomicU64::new(setup.max_lazy_file_bytes),
            first_byte_millis: AtomicU64::new(FIRST_BYTE.as_millis() as u64),
            background: Mutex::new(Vec::new()),
            closed: AtomicBool::new(false),
        });
        for root in setup.roots {
            mirror.bind(root)?;
        }
        Ok(mirror)
    }

    /// The engine attaches itself once it exists. Until then every miss fails as offline.
    pub fn attach(&self, host: Weak<dyn MirrorHost>) {
        *self.host.write().unwrap_or_else(|error| error.into_inner()) = Some(host);
    }

    /// Stops background downloads; the connection is being closed.
    pub fn close(&self) {
        let mut background = self
            .background
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.closed.store(true, Ordering::Release);
        for task in background.drain(..) {
            task.abort();
        }
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn host(&self) -> Option<Arc<dyn MirrorHost>> {
        self.host
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .and_then(Weak::upgrade)
    }

    fn changed(&self) {
        if let Some(host) = self.host() {
            host.changed();
        }
    }

    fn observe(&self, outcome: MirrorFetch) {
        if let Some(host) = self.host() {
            host.observe(outcome);
        }
    }

    fn file_path(&self, name: &str) -> PathBuf {
        self.directory.join(&name[..2]).join(name)
    }

    fn bind(&self, root: MirrorRoot) -> Result<()> {
        let url = uri_to_url(&root.url)?;
        ensure!(
            url.scheme() != "file-object-store",
            "A device database cannot be an offline mirror root"
        );
        let key = Path::parse(&root.key)?;
        let mut roots = self
            .roots
            .write()
            .unwrap_or_else(|error| error.into_inner());
        if roots.iter().any(|bound| bound.root == root) {
            return Ok(());
        }
        let registry = self
            .registry
            .upgrade()
            .context("The offline table connection was closed")?;
        let mirror = self
            .this
            .upgrade()
            .context("The offline mirror was closed")?;
        registry.insert(url.scheme(), Arc::new(MirrorProvider(mirror)));
        roots.push(Bound { root, url, key });
        Ok(())
    }

    fn bound_key(&self, url: &Url) -> Option<Path> {
        self.roots
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .find_map(|bound| bound.key_of(url))
    }

    /// True for keys under a bound root.
    pub(super) fn routes(&self, key: &Path) -> bool {
        self.roots
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .iter()
            .any(|bound| key.prefix_matches(&bound.key))
    }

    pub(super) fn read_only(&self, key: &Path) -> object_store::Error {
        object_store::Error::NotSupported {
            source: format!(
                "Offline copies never write to the cloud; '{key}' is read-only on this device"
            )
            .into(),
        }
    }

    fn outside(&self, key: &impl fmt::Display) -> object_store::Error {
        object_store::Error::Generic {
            store: STORE,
            source: format!(
                "Offline copies cannot read '{key}': it is outside this project's databases"
            )
            .into(),
        }
    }

    fn miss(&self, key: &Path, cause: &str) -> object_store::Error {
        object_store::Error::Generic {
            store: STORE,
            source: format!("{} ({key}: {cause})", missing_data_text(&table_of(key))).into(),
        }
    }

    fn stale(&self, key: &Path) -> object_store::Error {
        object_store::Error::NotFound {
            path: key.to_string(),
            source: format!("{} ({key})", reorganized_text(&table_of(key))).into(),
        }
    }

    fn listed(&self, name: &str) -> Option<Option<u64>> {
        self.state()
            .classes
            .listed
            .get(name)
            .map(|(_, bytes)| *bytes)
    }

    fn absent_index_file(&self, key: &Path) -> bool {
        let text = key.as_ref();
        text.rsplit_once('/')
            .is_some_and(|(directory, _)| self.state().classes.directories.contains(directory))
    }

    fn keepable(&self, name: &str, bytes: u64) -> bool {
        bytes <= self.max_lazy_file_bytes.load(Ordering::Acquire)
            || self.state().classes.whole.contains(name)
    }

    fn write_table(&self, key: &Path) -> Option<String> {
        self.state()
            .scopes
            .iter()
            .find(|scope| scope.keys.contains(key.as_ref()))
            .map(|scope| scope.table.clone())
    }

    /// Size of a cached file.
    pub fn cached(&self, key: &str) -> Option<u64> {
        self.state()
            .cached
            .get(&cache_name(key))
            .map(|file| file.bytes)
    }

    pub fn cache_bytes(&self) -> u64 {
        self.state().cached.values().map(|file| file.bytes).sum()
    }

    pub fn set_max_lazy_file_bytes(&self, bytes: u64) {
        self.max_lazy_file_bytes.store(bytes, Ordering::Release);
    }

    /// While the guard lives, reads of these listed keys are the reads of a write: misses
    /// download whole first (up to the lazy file cap, or any size for `whole` keys), ignore
    /// the allowance, may evict any live file, and fail with E39 when they cannot fit.
    pub fn write_scope(&self, table: &str, keys: Arc<HashSet<String>>) -> WriteScope {
        let mut state = self.state();
        state.next_scope += 1;
        let id = state.next_scope;
        state.scopes.push(Scope {
            id,
            table: table.to_string(),
            keys,
        });
        WriteScope {
            mirror: self.this.clone(),
            id,
        }
    }

    /// Listed keys that the cloud confirmed missing and that were not cleared yet.
    pub fn stale_keys(&self) -> Vec<String> {
        let mut keys = self.state().stale.iter().cloned().collect::<Vec<_>>();
        keys.sort();
        keys
    }

    /// Called after a checkpoint for the keys that no current snapshot lists any more.
    pub fn clear_stale(&self, keys: &[String]) {
        let mut state = self.state();
        for key in keys {
            state.stale.remove(key);
        }
    }

    /// Replaces the retention and deletes cached files whose size differs from their
    /// listed size, even while they are read.
    pub fn set_retention(&self, retention: MirrorRetention) {
        let mut classes = Classes::default();
        for (key, bytes) in retention.listed {
            let name = cache_name(&key);
            if retention.pinned.contains(&key) {
                classes.pinned.insert(name.clone());
            }
            if retention.whole.contains(&key) {
                classes.whole.insert(name.clone());
            }
            if let Some((directory, _)) = key.rsplit_once('/')
                && directory
                    .rsplit('/')
                    .nth(1)
                    .is_some_and(|parent| parent == "_indices")
            {
                classes.directories.insert(directory.to_string());
            }
            classes.listed.insert(name, (key, bytes));
        }
        let truncated = {
            let mut state = self.state();
            state.classes = classes;
            let state = &mut *state;
            state
                .cached
                .iter()
                .filter(|(name, file)| {
                    state
                        .classes
                        .listed
                        .get(*name)
                        .and_then(|(_, bytes)| *bytes)
                        .is_some_and(|bytes| bytes != file.bytes)
                })
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>()
        };
        let mut removed = false;
        for name in truncated {
            removed |= self.remove_cached(&name, true);
        }
        if removed {
            self.changed();
        }
    }

    /// Deletes a cached file unless it has an open read and `reads` is false. False when it
    /// is gone already or cannot be deleted now.
    fn remove_cached(&self, name: &str, reads: bool) -> bool {
        let file = {
            let mut state = self.state();
            match state.cached.get(name) {
                Some(file) if reads || file.reads == 0 => state.cached.remove(name),
                _ => None,
            }
        };
        let Some(file) = file else {
            return false;
        };
        match std::fs::remove_file(self.file_path(name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                self.state().cached.insert(name.to_string(), file);
                return false;
            }
        }
        self.budget.release(file.bytes + FILE_OVERHEAD);
        true
    }

    /// The cached files `evict` may remove, in eviction order, with their sizes.
    fn evictable(&self, recent_files: bool, unused_for: Option<Duration>) -> Vec<(String, u64)> {
        let now = SystemTime::now();
        let mut candidates = {
            let state = self.state();
            state
                .cached
                .iter()
                .filter_map(|(name, file)| {
                    if state.classes.pinned.contains(name) || (!recent_files && file.reads > 0) {
                        return None;
                    }
                    let garbage = !state.classes.listed.contains_key(name);
                    let evictable = match unused_for {
                        Some(age) => {
                            garbage && now.duration_since(file.used).unwrap_or_default() >= age
                        }
                        None => garbage || recent_files || !recent(file.used, now),
                    };
                    evictable.then(|| (!garbage, file.used, name.clone(), file.bytes))
                })
                .collect::<Vec<_>>()
        };
        candidates.sort();
        candidates
            .into_iter()
            .map(|(_, _, name, bytes)| (name, bytes))
            .collect()
    }

    /// Evicts garbage, then live files by least recent use, never pinned ones, until the
    /// budget's used bytes are at most `target_used`. `recent` also evicts files with an
    /// open read or used in the last 10 minutes. With `unused_for`, only garbage unused for
    /// that long is evicted, whatever the target.
    fn evict(&self, target_used: u64, recent_files: bool, unused_for: Option<Duration>) -> u64 {
        let mut freed = 0;
        for (name, _) in self.evictable(recent_files, unused_for) {
            if unused_for.is_none() && self.budget.used() <= target_used {
                break;
            }
            let bytes = self.state().cached.get(&name).map(|file| file.bytes);
            if let Some(bytes) = bytes
                && self.remove_cached(&name, recent_files)
            {
                freed += bytes + FILE_OVERHEAD;
            }
        }
        if freed > 0 {
            self.changed();
        }
        freed
    }

    /// Evicts garbage, then live files by least recent use, until the budget's used bytes are
    /// at most `target_used`. `recent` also evicts recently used live files. Returns the
    /// freed bytes.
    pub async fn evict_to(&self, target_used: u64, recent: bool) -> Result<u64> {
        Ok(self.evict(target_used, recent, None))
    }

    /// Evicts garbage unused for `unused_for`.
    pub async fn evict_garbage(&self, unused_for: Duration) -> Result<u64> {
        Ok(self.evict(0, false, Some(unused_for)))
    }

    /// Room for a local write of `bytes`: evicts every file that may be evicted if needed.
    pub(super) fn make_room_for_local(&self, bytes: u64) {
        self.evict(self.budget.maximum().saturating_sub(bytes), true, None);
    }

    /// Charges `charge` to the budget, evicting first when needed. Evicts nothing when
    /// evicting every evictable file would not make room. Err: the bytes still missing after
    /// evicting every evictable file.
    fn make_room(&self, charge: u64, recent_files: bool) -> std::result::Result<(), u64> {
        if self.budget.try_charge(charge) {
            return Ok(());
        }
        let evictable = self
            .evictable(recent_files, None)
            .iter()
            .map(|(_, bytes)| bytes + FILE_OVERHEAD)
            .sum::<u64>();
        let shortfall = (self.budget.used() + charge).saturating_sub(self.budget.maximum());
        if evictable < shortfall {
            return Err(shortfall - evictable);
        }
        self.evict(
            self.budget.maximum().saturating_sub(charge),
            recent_files,
            None,
        );
        if self.budget.try_charge(charge) {
            return Ok(());
        }
        Err((self.budget.used() + charge).saturating_sub(self.budget.maximum()))
    }

    async fn cloud(&self, key: &Path) -> object_store::Result<Arc<dyn ObjectStore>> {
        let Some(host) = self.host() else {
            return Err(self.miss(key, "this device keeps no offline changes for it"));
        };
        if host.is_offline() {
            return Err(self.miss(key, "the hub is unreachable"));
        }
        host.cloud()
            .await
            .map_err(|error| self.miss(key, &format!("{error:#}")))
    }

    /// Waits for `call`; after the first-byte timeout it reports `TimedOut` and keeps
    /// waiting until the breaker opens.
    async fn first_byte<T>(
        &self,
        key: &Path,
        call: impl Future<Output = object_store::Result<T>>,
    ) -> object_store::Result<T> {
        tokio::pin!(call);
        let timeout = Duration::from_millis(self.first_byte_millis.load(Ordering::Acquire));
        tokio::select! {
            result = &mut call => return result,
            _ = tokio::time::sleep(timeout) => {}
        }
        self.observe(MirrorFetch::TimedOut);
        loop {
            tokio::select! {
                result = &mut call => return result,
                _ = tokio::time::sleep(BREAKER_POLL) => {
                    if self.host().is_none_or(|host| host.is_offline()) {
                        return Err(self.miss(key, "the cloud did not answer"));
                    }
                }
            }
        }
    }

    /// Offline-class errors become E36; a listed key that the cloud answers NotFound or 403
    /// for is confirmed by a listing and becomes E37 when it is gone.
    async fn failed(&self, key: &Path, error: object_store::Error) -> object_store::Error {
        if let Some(host) = self.host()
            && let Some(outcome) = host.classify(&error)
        {
            host.observe(outcome);
            return self.miss(key, &error.to_string());
        }
        if matches!(
            error,
            object_store::Error::NotFound { .. } | object_store::Error::PermissionDenied { .. }
        ) && self.listed(&cache_name(key.as_ref())).is_some()
            && self.vanished(key).await
        {
            return self.stale(key);
        }
        error
    }

    /// Lists the key's directory from just before the key: object_store lists prefixes as
    /// directories, so this is the listing that can contain the key itself.
    async fn vanished(&self, key: &Path) -> bool {
        let Ok(cloud) = self.cloud(key).await else {
            return false;
        };
        let text = key.as_ref();
        let (Some((parent, _)), Some((last, _))) =
            (text.rsplit_once('/'), text.char_indices().last())
        else {
            return false;
        };
        let (Ok(parent), Ok(offset)) = (Path::parse(parent), Path::parse(&text[..last])) else {
            return false;
        };
        let mut listing = cloud.list_with_offset(Some(&parent), &offset);
        while let Some(entry) = listing.next().await {
            match entry {
                Ok(meta) if meta.location == *key => return false,
                Ok(_) => {}
                Err(_) => return false,
            }
        }
        self.state().stale.insert(key.to_string());
        self.changed();
        true
    }

    async fn cloud_get(&self, key: &Path, options: GetOptions) -> object_store::Result<GetResult> {
        let cloud = self.cloud(key).await?;
        match self.first_byte(key, cloud.get_opts(key, options)).await {
            Ok(result) => {
                self.observe(MirrorFetch::Succeeded);
                Ok(self.classified_body(key, result))
            }
            Err(error) => Err(self.failed(key, error).await),
        }
    }

    /// A connection lost while the body streams becomes E36, like one lost before it.
    fn classified_body(&self, key: &Path, result: GetResult) -> GetResult {
        match result.payload {
            GetResultPayload::Stream(stream) => {
                let mirror = self.this.clone();
                let key = key.clone();
                let stream = stream.map_err(move |error| match mirror.upgrade() {
                    Some(mirror) => mirror.classified(&key, error),
                    None => error,
                });
                GetResult {
                    payload: GetResultPayload::Stream(stream.boxed()),
                    ..result
                }
            }
            payload => GetResult { payload, ..result },
        }
    }

    async fn read_cached(
        &self,
        key: &Path,
        name: &str,
        options: &GetOptions,
    ) -> object_store::Result<Option<GetResult>> {
        let bytes = {
            let mut state = self.state();
            let Some(file) = state.cached.get_mut(name) else {
                return Ok(None);
            };
            file.used = SystemTime::now();
            file.reads += 1;
            file.bytes
        };
        let open = OpenRead { mirror: self, name };
        let path = self.file_path(name);
        let range = options.range.clone();
        let read = tokio::task::spawn_blocking(move || read_range(&path, range, bytes)).await;
        drop(open);
        match read.map_err(io_error)? {
            Ok((range, data, modified)) => Ok(Some(GetResult {
                payload: GetResultPayload::Stream(
                    futures::stream::once(async { Ok(data) }).boxed(),
                ),
                meta: ObjectMeta {
                    location: key.clone(),
                    last_modified: modified.into(),
                    size: bytes,
                    e_tag: None,
                    version: None,
                },
                range,
                attributes: Default::default(),
            })),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(file) = self.state().cached.remove(name) {
                    self.budget.release(file.bytes + FILE_OVERHEAD);
                }
                Ok(None)
            }
            Err(error) => Err(io_error(error)),
        }
    }

    /// Reads a key under a bound root (§2.8 read path).
    pub(super) async fn get(
        &self,
        key: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        if !self.routes(key) {
            return Err(self.outside(key));
        }
        let name = cache_name(key.as_ref());
        let Some(listed) = self.listed(&name) else {
            if self.absent_index_file(key) {
                return Err(object_store::Error::NotFound {
                    path: key.to_string(),
                    source: "the offline copy lists no such index file".into(),
                });
            }
            return self.cloud_get(key, options).await;
        };
        if options.head {
            if let Some(bytes) = listed.or_else(|| self.cached(key.as_ref())) {
                return Ok(head_result(key, bytes));
            }
            return self.cloud_get(key, options).await;
        }
        if let Some(result) = self.read_cached(key, &name, &options).await? {
            return Ok(result);
        }
        let bytes = match listed {
            Some(bytes) => bytes,
            None => {
                self.cloud_get(key, GetOptions::new().with_head(true))
                    .await?
                    .meta
                    .size
            }
        };
        let keepable = self.keepable(&name, bytes);
        if keepable && let Some(table) = self.write_table(key) {
            match self.download(key, &name, bytes, How::Write(&table)).await {
                Ok(_) => {}
                Err(DownloadError::Store(error)) => return Err(error),
                Err(DownloadError::Short(shortage)) => return Err(io_error(shortage)),
            }
            if let Some(result) = self.read_cached(key, &name, &options).await? {
                return Ok(result);
            }
        }
        let result = self.cloud_get(key, options).await?;
        if keepable {
            self.spawn_background(key, &name, bytes);
        }
        Ok(result)
    }

    pub(super) fn list(
        &self,
        prefix: &Path,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        let Some(mirror) = self.this.upgrade() else {
            let error = self.outside(prefix);
            return futures::stream::once(async move { Err(error) }).boxed();
        };
        let prefix = prefix.clone();
        futures::stream::once(async move {
            if !mirror.routes(&prefix) {
                return Err(mirror.outside(&prefix));
            }
            let cloud = mirror.cloud(&prefix).await?;
            let classify = mirror.clone();
            let key = prefix.clone();
            Ok(cloud
                .list(Some(&prefix))
                .map(move |entry| entry.map_err(|error| classify.classified(&key, error))))
        })
        .try_flatten()
        .boxed()
    }

    pub(super) async fn list_with_delimiter(
        &self,
        prefix: &Path,
    ) -> object_store::Result<ListResult> {
        if !self.routes(prefix) {
            return Err(self.outside(prefix));
        }
        let cloud = self.cloud(prefix).await?;
        cloud
            .list_with_delimiter(Some(prefix))
            .await
            .map_err(|error| self.classified(prefix, error))
    }

    /// Offline-class errors become E36; other errors pass through.
    fn classified(&self, key: &Path, error: object_store::Error) -> object_store::Error {
        match self.host() {
            Some(host) => match host.classify(&error) {
                Some(outcome) => {
                    host.observe(outcome);
                    self.miss(key, &error.to_string())
                }
                None => error,
            },
            None => error,
        }
    }

    fn key_lock(&self, name: &str) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .entry(name.to_string())
            .or_default()
            .clone()
    }

    fn release_key_lock(&self, name: &str) {
        let mut locks = self.locks.lock().unwrap_or_else(|error| error.into_inner());
        if locks
            .get(name)
            .is_some_and(|lock| Arc::strong_count(lock) == 1)
        {
            locks.remove(name);
        }
    }

    fn spawn_background(&self, key: &Path, name: &str, bytes: u64) {
        let Some(mirror) = self.this.upgrade() else {
            return;
        };
        let mut background = self
            .background
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        {
            let mut state = self.state();
            if state.cached.contains_key(name) || !state.in_flight.insert(name.to_string()) {
                return;
            }
        }
        let (key, name) = (key.clone(), name.to_string());
        let task = tokio::spawn(async move {
            let _ = mirror.download(&key, &name, bytes, How::Background).await;
            mirror.state().in_flight.remove(&name);
        });
        background.retain(|task| !task.is_finished());
        background.push(task.abort_handle());
    }

    /// Downloads one file whole into the cache. Ok(0) when it is cached already or a
    /// background download was skipped.
    async fn download(
        &self,
        key: &Path,
        name: &str,
        bytes: u64,
        how: How<'_>,
    ) -> std::result::Result<u64, DownloadError> {
        let _release = KeyLockRelease { mirror: self, name };
        self.download_locked(key, name, bytes, how).await
    }

    async fn download_locked(
        &self,
        key: &Path,
        name: &str,
        bytes: u64,
        how: How<'_>,
    ) -> std::result::Result<u64, DownloadError> {
        if self.state().cached.contains_key(name) {
            return Ok(0);
        }
        let _permit = match how {
            How::Background => match self.downloads.clone().try_acquire_owned() {
                Ok(permit) => permit,
                Err(_) => return Ok(0),
            },
            How::Write(_) | How::Fetch { .. } => self
                .downloads
                .clone()
                .acquire_owned()
                .await
                .map_err(io_error)?,
        };
        let lock = self.key_lock(name);
        let _guard = lock.lock().await;
        if self.state().cached.contains_key(name) {
            return Ok(0);
        }
        let host = self
            .host()
            .ok_or_else(|| self.miss(key, "this device keeps no offline changes for it"))?;
        let cloud = self.cloud(key).await?;
        let allowed = host.allow_download(bytes, matches!(how, How::Write(_)));
        match how {
            How::Background if !allowed => return Ok(0),
            How::Fetch { .. } if !allowed => {
                return Err(DownloadError::Short(MirrorShortage::Allowance));
            }
            _ => {}
        }
        let mut download = Download {
            mirror: self,
            host,
            name,
            reserved: bytes,
            charged: 0,
            transferred: AtomicU64::new(0),
            committed: false,
        };
        let charge = bytes + FILE_OVERHEAD;
        let recent_files = match how {
            How::Background => false,
            How::Write(_) => true,
            How::Fetch { recent } => recent,
        };
        if let Err(needed) = self.make_room(charge, recent_files) {
            return match how {
                How::Background => Ok(0),
                How::Write(table) => {
                    Err(io_error(std::io::Error::other(storage_full_text(table, needed))).into())
                }
                How::Fetch { .. } => Err(DownloadError::Short(MirrorShortage::Disk { needed })),
            };
        }
        download.charged = charge;
        self.transfer(cloud, key, name, bytes, &download.transferred)
            .await?;
        if matches!(how, How::Background) && !self.keepable(name, bytes) {
            return Ok(0);
        }
        self.state().cached.insert(
            name.to_string(),
            Cached {
                bytes,
                used: SystemTime::now(),
                reads: 0,
            },
        );
        download.committed = true;
        drop(download);
        self.changed();
        Ok(bytes)
    }

    /// GET into `<name>.partial`, length check, fsync, rename, directory fsync. The caller's
    /// `Download` removes what a failure leaves behind.
    async fn transfer(
        &self,
        cloud: Arc<dyn ObjectStore>,
        key: &Path,
        name: &str,
        bytes: u64,
        transferred: &AtomicU64,
    ) -> object_store::Result<()> {
        let result = match self
            .first_byte(key, cloud.get_opts(key, GetOptions::default()))
            .await
        {
            Ok(result) => {
                self.observe(MirrorFetch::Succeeded);
                result
            }
            Err(error) => return Err(self.failed(key, error).await),
        };
        if result.meta.size != bytes {
            return Err(io_error(std::io::Error::other(format!(
                "Cloud file '{key}' has {} bytes, but the offline copy lists {bytes}",
                result.meta.size
            ))));
        }
        let target = self.file_path(name);
        let partial = target.with_extension("partial");
        let directory = target
            .parent()
            .expect("cache files have a directory")
            .to_path_buf();
        private_directory(&directory).map_err(io_error)?;
        let mut file = tokio::fs::File::create(&partial).await.map_err(io_error)?;
        let mut stream = result.into_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(error) => return Err(self.failed(key, error).await),
            };
            let length = chunk.len() as u64;
            if transferred.fetch_add(length, Ordering::AcqRel) + length > bytes {
                return Err(io_error(std::io::Error::other(format!(
                    "Cloud file '{key}' is longer than the {bytes} bytes the offline copy lists"
                ))));
            }
            tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                .await
                .map_err(io_error)?;
        }
        let received = transferred.load(Ordering::Acquire);
        if received != bytes {
            return Err(io_error(std::io::Error::other(format!(
                "Download of '{key}' ended after {received} of {bytes} bytes"
            ))));
        }
        file.sync_all().await.map_err(io_error)?;
        drop(file);
        tokio::fs::rename(&partial, &target)
            .await
            .map_err(io_error)?;
        tokio::task::spawn_blocking(move || sync_directory(&directory))
            .await
            .map_err(io_error)?
            .map_err(io_error)
    }

    /// Downloads the listed files that are not cached yet, at most 4 at a time. Charges the
    /// disk budget (evicting garbage, then live files; recent live files only when
    /// `recent`) and the download allowance. Nothing is downloaded when the files do not
    /// fit or the allowance has no room (`MirrorShortage`). Stops at the first failure.
    /// Ignores the lazy file cap. Returns the downloaded bytes.
    pub async fn fetch(&self, files: &[MirrorFile], recent: bool) -> Result<u64> {
        let mut seen = HashSet::new();
        let mut pending = Vec::new();
        for file in files {
            if !seen.insert(file.key.as_str()) || self.cached(&file.key).is_some() {
                continue;
            }
            let key = Path::parse(&file.key)?;
            let bytes = match file.bytes {
                Some(bytes) => bytes,
                None => {
                    self.cloud_get(&key, GetOptions::new().with_head(true))
                        .await?
                        .meta
                        .size
                }
            };
            pending.push((key, bytes));
        }
        if pending.is_empty() {
            return Ok(0);
        }
        let total = pending.iter().map(|(_, bytes)| bytes).sum::<u64>();
        let charge = total + FILE_OVERHEAD * pending.len() as u64;
        let host = self
            .host()
            .context("The offline mirror is not attached to its offline changes")?;
        if !host.allow_download(total, false) {
            return Err(MirrorShortage::Allowance.into());
        }
        let room = self.make_room(charge, recent);
        host.downloaded(total, 0);
        if let Err(needed) = room {
            return Err(MirrorShortage::Disk { needed }.into());
        }
        self.budget.release(charge);
        let mut downloads =
            futures::stream::iter(pending.into_iter().map(|(key, bytes)| async move {
                let name = cache_name(key.as_ref());
                self.download(&key, &name, bytes, How::Fetch { recent })
                    .await
            }))
            .buffer_unordered(MAX_DOWNLOADS);
        let mut downloaded = 0;
        while let Some(result) = downloads.next().await {
            match result {
                Ok(bytes) => downloaded += bytes,
                Err(DownloadError::Store(error)) => return Err(error.into()),
                Err(DownloadError::Short(shortage)) => return Err(shortage.into()),
            }
        }
        Ok(downloaded)
    }

    /// Manifest-only snapshot of `source`, pinned at its current version, into `destination`
    /// (this mirror's connection). Binds the table's database root and downloads no table
    /// files. `previous` is the file list of the table's current snapshot; its sizes are
    /// reused for keys the new version still lists.
    pub async fn snapshot(
        &self,
        source: &Table,
        destination: &Connection,
        name: &str,
        previous: &[MirrorFile],
    ) -> Result<LazySnapshot> {
        super::lancedb::LanceDBVectorStore::validate_table_name(name)?;
        ensure!(
            uri_to_url(destination.uri())?.scheme() == "file-object-store",
            "offline snapshots require a budgeted_local_connection destination"
        );
        match destination.open_table(name).execute().await {
            Ok(_) => anyhow::bail!("offline snapshot destination already exists"),
            Err(lancedb::Error::TableNotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        let table = source.name().to_string();
        let dataset = source
            .dataset()
            .ok_or_else(|| anyhow!("offline snapshots require a native Lance table"))?
            .get()
            .await?;
        let dataset = dataset.checkout_version(dataset.manifest().version).await?;
        let root = MirrorRoot::of(&dataset)?;
        let cloud_store = dataset.object_store(None).await?;
        let indices = read_manifest_indexes(
            &cloud_store,
            dataset.manifest_location(),
            dataset.manifest(),
        )
        .await?;
        refuse_unsupported(dataset.manifest(), &indices, &root, &table)?;
        self.bind(root.clone())?;
        let base_id = dataset
            .manifest()
            .base_paths
            .keys()
            .max()
            .map_or(0, |id| id + 1);
        let mut manifest = dataset.manifest().shallow_clone(
            None,
            dataset.uri().into(),
            base_id,
            None,
            String::new(),
        );
        manifest.transaction_file = None;
        manifest
            .config
            .retain(|key, _| !key.starts_with(AUTO_CLEANUP));
        apply_feature_flags(&mut manifest, false, false)?;
        let mut indices = indices
            .into_iter()
            .map(|mut index| {
                index.base_id.get_or_insert(base_id);
                index
            })
            .collect::<Vec<_>>();
        let (files, owners) = self
            .walk(&root, &manifest, &mut indices, previous, &table)
            .await?;
        let source_version = dataset.manifest().version;
        let source_fingerprint = fingerprint(&dataset).await?;
        let uri = format!("{}/{name}.lance", destination.uri().trim_end_matches('/'));
        let registry = self
            .registry
            .upgrade()
            .context("The offline table connection was closed")?;
        let (local, base) =
            LanceStore::from_uri_and_params(registry, &uri, &ObjectStoreParams::default()).await?;
        let committed = commit_handler_from_url(&uri, &None)
            .await?
            .commit(
                &mut manifest,
                (!indices.is_empty()).then_some(indices),
                &base,
                local.as_ref(),
                write_manifest_file_to_path,
                ManifestNamingScheme::V2,
                None,
            )
            .await;
        let opened = match committed {
            Ok(_) => destination
                .open_table(name)
                .execute()
                .await
                .map_err(Into::into),
            Err(error) => Err(anyhow!(
                "Offline snapshot of '{table}' could not be committed on this device: {error:?}"
            )),
        };
        let local_table = match opened {
            Ok(table) => table,
            Err(error) => {
                return match destination.drop_table(name, &[]).await {
                    Ok(()) | Err(lancedb::Error::TableNotFound { .. }) => Err(error),
                    Err(cleanup) => Err(anyhow!(
                        "{error:#}; the partial offline snapshot could not be removed: {cleanup}"
                    )),
                };
            }
        };
        Ok(LazySnapshot {
            table: MaterializedTable {
                table: local_table,
                source_version,
                source_fingerprint,
            },
            files,
            root,
            owners,
        })
    }

    /// Every data and deletion file of the cloned manifest and every file of each index,
    /// keyed like the router keys them. Fills `IndexMetadata.files` where it is missing.
    async fn walk(
        &self,
        root: &MirrorRoot,
        manifest: &Manifest,
        indices: &mut [IndexMetadata],
        previous: &[MirrorFile],
        table: &str,
    ) -> Result<(Vec<MirrorFile>, HashMap<String, Vec<String>>)> {
        let mut bases = HashMap::new();
        for base in manifest.base_paths.values() {
            let key = root
                .key_of(&uri_to_url(&base.path)?)
                .ok_or_else(|| unsupported(table, "data outside this table's database"))?;
            bases.insert(base.id, (key, base.is_dataset_root));
        }
        let base = |id: Option<u32>| {
            id.and_then(|id| bases.get(&id))
                .ok_or_else(|| anyhow!("A file of table '{table}' has no cloud base"))
        };
        let mut files = Vec::new();
        let mut owners = HashMap::new();
        for fragment in manifest.fragments.iter() {
            let mut data = Vec::new();
            for file in &fragment.files {
                let (key, dataset_root) = base(file.base_id)?;
                let directory = if *dataset_root {
                    key.clone().join("data")
                } else {
                    key.clone()
                };
                let key = directory.join(file.path.as_str()).to_string();
                data.push(key.clone());
                files.push(MirrorFile {
                    key,
                    bytes: file.file_size_bytes.get().map(|bytes| bytes.get()),
                    kind: MirrorFileKind::Data,
                });
            }
            if let Some(deletion) = &fragment.deletion_file {
                let (key, _) = base(deletion.base_id)?;
                let key = deletion_file_path(key, fragment.id, deletion).to_string();
                owners.insert(key.clone(), data);
                files.push(MirrorFile {
                    key,
                    bytes: None,
                    kind: MirrorFileKind::Deletion,
                });
            }
        }
        for index in indices.iter_mut() {
            let (key, dataset_root) = base(index.base_id)?;
            let directory = if *dataset_root {
                key.clone().join("_indices")
            } else {
                key.clone()
            }
            .join(index.uuid.to_string());
            if index.files.is_none() {
                index.files = Some(self.list_index(&directory).await?);
            }
            for file in index.files.iter().flatten() {
                files.push(MirrorFile {
                    key: Path::parse(format!("{directory}/{}", file.path))?.to_string(),
                    bytes: Some(file.size_bytes),
                    kind: MirrorFileKind::Index,
                });
            }
        }
        let known = previous
            .iter()
            .filter_map(|file| file.bytes.map(|bytes| (file.key.as_str(), bytes)))
            .collect::<HashMap<_, _>>();
        for file in files.iter_mut().filter(|file| file.bytes.is_none()) {
            file.bytes = known.get(file.key.as_str()).copied();
        }
        self.size_unknown(&mut files).await;
        Ok((files, owners))
    }

    async fn list_index(&self, directory: &Path) -> Result<Vec<IndexFile>> {
        let cloud = self.cloud(directory).await?;
        let prefix = format!("{directory}/");
        cloud
            .list(Some(directory))
            .map_ok(|meta| IndexFile {
                path: meta
                    .location
                    .as_ref()
                    .strip_prefix(&prefix)
                    .unwrap_or_else(|| meta.location.as_ref())
                    .to_string(),
                size_bytes: meta.size,
            })
            .try_collect()
            .await
            .with_context(|| format!("Could not list the cloud index directory '{directory}'"))
    }

    /// HEADs at most 256 unknown keys, 8 at a time; otherwise lists each table directory
    /// once. Keys stay unknown when those requests fail.
    async fn size_unknown(&self, files: &mut [MirrorFile]) {
        let unknown = files
            .iter()
            .filter(|file| file.bytes.is_none())
            .map(|file| file.key.clone())
            .collect::<Vec<_>>();
        if unknown.is_empty() {
            return;
        }
        let Some(first) = unknown.first().and_then(|key| Path::parse(key).ok()) else {
            return;
        };
        let Ok(cloud) = self.cloud(&first).await else {
            return;
        };
        let mut sizes = HashMap::new();
        if unknown.len() <= MAX_HEADS {
            let heads = futures::stream::iter(unknown.into_iter().map(|key| {
                let cloud = cloud.clone();
                async move {
                    let path = Path::parse(&key).ok()?;
                    cloud.head(&path).await.ok().map(|meta| (key, meta.size))
                }
            }))
            .buffered(HEAD_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
            sizes.extend(heads.into_iter().flatten());
        } else {
            let directories = unknown
                .iter()
                .filter_map(|key| {
                    key.find(".lance/")
                        .map(|end| key[..end + ".lance".len()].to_string())
                })
                .collect::<HashSet<_>>();
            for directory in directories {
                let Ok(path) = Path::parse(&directory) else {
                    continue;
                };
                let listed = cloud
                    .list(Some(&path))
                    .try_collect::<Vec<_>>()
                    .await
                    .unwrap_or_default();
                sizes.extend(
                    listed
                        .into_iter()
                        .map(|meta| (meta.location.to_string(), meta.size)),
                );
            }
        }
        for file in files.iter_mut().filter(|file| file.bytes.is_none()) {
            file.bytes = sizes.get(&file.key).copied();
        }
    }
}

/// E38: layouts whose files the walk cannot cover or must not follow.
fn refuse_unsupported(
    manifest: &Manifest,
    indices: &[IndexMetadata],
    root: &MirrorRoot,
    table: &str,
) -> Result<()> {
    if manifest.fragments.iter().any(|fragment| {
        matches!(fragment.row_id_meta, Some(RowIdMeta::External(_)))
            || matches!(
                fragment.last_updated_at_version_meta,
                Some(RowDatasetVersionMeta::External(_))
            )
            || matches!(
                fragment.created_at_version_meta,
                Some(RowDatasetVersionMeta::External(_))
            )
    }) {
        return Err(unsupported(table, "external row metadata files"));
    }
    if manifest
        .schema
        .fields_pre_order()
        .any(|field| field.is_blob_v2())
    {
        return Err(unsupported(table, "blob columns"));
    }
    if indices.iter().any(|index| index.name == MEM_WAL_INDEX_NAME) {
        return Err(unsupported(table, "a MemWAL index"));
    }
    for base in manifest.base_paths.values() {
        if root.key_of(&uri_to_url(&base.path)?).is_none() {
            return Err(unsupported(table, "data outside this table's database"));
        }
    }
    Ok(())
}

/// A BTree or bitmap index on `column` alone, from index details or, for legacy indexes,
/// from their recorded files as Lance's own inference types them. None when an index's
/// files are unknown.
pub async fn key_indexed(
    table: &Table,
    column: &str,
    files: &[MirrorFile],
) -> Result<Option<bool>> {
    let dataset = table
        .dataset()
        .ok_or_else(|| anyhow!("offline snapshots require a native Lance table"))?
        .get()
        .await?;
    let Some(field) = dataset.schema().field(column) else {
        return Ok(Some(false));
    };
    let store = dataset.object_store(None).await?;
    let indices =
        read_manifest_indexes(&store, dataset.manifest_location(), dataset.manifest()).await?;
    let mut unknown = false;
    for index in indices.iter().filter(|index| index.fields == [field.id]) {
        if let Some(details) = &index.index_details {
            if details.type_url.ends_with("BTreeIndexDetails")
                || details.type_url.ends_with("BitmapIndexDetails")
            {
                return Ok(Some(true));
            }
            continue;
        }
        let directory = format!("/_indices/{}/", index.uuid);
        let names = files
            .iter()
            .filter_map(|file| file.key.split_once(&directory).map(|(_, name)| name))
            .collect::<HashSet<_>>();
        if names.is_empty() {
            unknown = true;
        } else if names.contains("bitmap_page_lookup.lance")
            || !["metadata.lance", "invert.lance", "index.idx"]
                .iter()
                .any(|name| names.contains(name))
        {
            return Ok(Some(true));
        }
    }
    Ok((!unknown).then_some(false))
}

#[derive(Debug)]
struct MirrorProvider(Arc<LazyMirror>);

#[async_trait::async_trait]
impl ObjectStoreProvider for MirrorProvider {
    async fn new_store(&self, url: Url, params: &ObjectStoreParams) -> lance::Result<LanceStore> {
        if self.0.bound_key(&url).is_none() {
            return Err(lance::Error::io_source(Box::new(self.0.outside(&url))));
        }
        Ok(LanceStore::new(
            Arc::new(MirrorStore(self.0.clone())),
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

    fn extract_path(&self, url: &Url) -> lance::Result<Path> {
        self.0
            .bound_key(url)
            .ok_or_else(|| lance::Error::io_source(Box::new(self.0.outside(url))))
    }

    fn calculate_object_store_prefix(
        &self,
        url: &Url,
        _: Option<&HashMap<String, String>>,
    ) -> lance::Result<String> {
        Ok(format!(
            "offline-mirror:{}${}",
            url.scheme(),
            url.authority()
        ))
    }
}

/// The object store behind every Lance store of a bound root.
#[derive(Debug)]
struct MirrorStore(Arc<LazyMirror>);

impl fmt::Display for MirrorStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("lazy offline mirror")
    }
}

#[async_trait::async_trait]
impl ObjectStore for MirrorStore {
    async fn put_opts(
        &self,
        location: &Path,
        _: object_store::PutPayload,
        _: object_store::PutOptions,
    ) -> object_store::Result<object_store::PutResult> {
        Err(self.0.read_only(location))
    }

    async fn put_multipart_opts(
        &self,
        location: &Path,
        _: object_store::PutMultipartOptions,
    ) -> object_store::Result<Box<dyn object_store::MultipartUpload>> {
        Err(self.0.read_only(location))
    }

    async fn get_opts(
        &self,
        location: &Path,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.0.get(location, options).await
    }

    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<Path>>,
    ) -> BoxStream<'static, object_store::Result<Path>> {
        let mirror = self.0.clone();
        locations
            .map(move |location| Err(mirror.read_only(&location?)))
            .boxed()
    }

    fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        self.0.list(&prefix.cloned().unwrap_or_default())
    }

    async fn list_with_delimiter(&self, prefix: Option<&Path>) -> object_store::Result<ListResult> {
        self.0
            .list_with_delimiter(&prefix.cloned().unwrap_or_default())
            .await
    }

    async fn copy_opts(
        &self,
        _: &Path,
        to: &Path,
        _: object_store::CopyOptions,
    ) -> object_store::Result<()> {
        Err(self.0.read_only(to))
    }

    async fn rename_opts(
        &self,
        _: &Path,
        to: &Path,
        _: object_store::RenameOptions,
    ) -> object_store::Result<()> {
        Err(self.0.read_only(to))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::databases::vector::offline_replay::{
        LocalConnection, ReplayMarker, ReplayMutation, ReplayOutcome,
        budgeted_local_connection_with_budget, revision,
    };
    use arrow_array::{
        Array, FixedSizeListArray, Int64Array, RecordBatch, StringArray, types::Float32Type,
    };
    use arrow_schema::{DataType, Field, Schema};
    use flow_like_types::authorization::AuthorizationError;
    use lancedb::{
        index::{
            Index,
            scalar::{BTreeIndexBuilder, FtsIndexBuilder},
            vector::IvfFlatIndexBuilder,
        },
        query::{ExecutableQuery, QueryBase, Select},
    };
    use object_store::{
        CopyOptions, MultipartUpload, PutMultipartOptions, PutOptions, PutPayload, PutResult,
        RenameOptions, local::LocalFileSystem,
    };
    use std::sync::atomic::AtomicUsize;

    const DB: &str = "cloudsim://bucket/apps/p/storage/db";
    const DIM: i32 = 4;
    const MIB: u64 = 1024 * 1024;

    #[derive(Debug, Default)]
    struct Faults {
        offline: bool,
        lease: bool,
        stall: bool,
        /// Bodies stop after their first half until this is cleared.
        stall_body: bool,
        /// Bodies lose the connection after their first half.
        cut_body: bool,
        forbidden_missing: bool,
        forbidden: HashSet<String>,
    }

    /// The cloud bucket: a directory behind a switchable store that counts requests.
    #[derive(Debug)]
    struct Cloud {
        inner: LocalFileSystem,
        faults: Arc<Mutex<Faults>>,
        /// (key, ranged, head)
        gets: Mutex<Vec<(String, bool, bool)>>,
        lists: Mutex<Vec<String>>,
    }

    impl Cloud {
        fn faults(&self) -> MutexGuard<'_, Faults> {
            self.faults.lock().unwrap()
        }

        fn fault(&self) -> object_store::Result<()> {
            let faults = self.faults();
            if faults.offline {
                return Err(object_store::Error::Generic {
                    store: "cloudsim",
                    source: Box::new(std::io::Error::new(
                        std::io::ErrorKind::ConnectionRefused,
                        "connection refused",
                    )),
                });
            }
            if faults.lease {
                return Err(object_store::Error::Generic {
                    store: "cloudsim",
                    source: Box::new(AuthorizationError::Unavailable),
                });
            }
            Ok(())
        }

        async fn interrupted(&self, result: GetResult) -> object_store::Result<GetResult> {
            let (meta, range, attributes) = (
                result.meta.clone(),
                result.range.clone(),
                result.attributes.clone(),
            );
            let data = result.bytes().await?;
            let half = data.len() / 2;
            let (first, rest) = (data.slice(..half), data.slice(half..));
            let faults = self.faults.clone();
            let rest = async move {
                if faults.lock().unwrap().cut_body {
                    return Err(object_store::Error::Generic {
                        store: "cloudsim",
                        source: Box::new(std::io::Error::new(
                            std::io::ErrorKind::ConnectionReset,
                            "connection reset",
                        )),
                    });
                }
                while faults.lock().unwrap().stall_body {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Ok(rest)
            };
            Ok(GetResult {
                payload: GetResultPayload::Stream(
                    futures::stream::iter([Ok(first)])
                        .chain(futures::stream::once(rest))
                        .boxed(),
                ),
                meta,
                range,
                attributes,
            })
        }

        fn whole_gets(&self, key: &str) -> usize {
            self.gets
                .lock()
                .unwrap()
                .iter()
                .filter(|(get, ranged, head)| get == key && !ranged && !head)
                .count()
        }

        fn requests(&self) -> usize {
            self.gets.lock().unwrap().len() + self.lists.lock().unwrap().len()
        }

        /// HEADs of table files; Lance itself HEADs manifests.
        fn file_heads(&self) -> usize {
            self.gets
                .lock()
                .unwrap()
                .iter()
                .filter(|(key, _, head)| {
                    *head && (key.contains("/data/") || key.contains("/_deletions/"))
                })
                .count()
        }
    }

    impl fmt::Display for Cloud {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("cloudsim")
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore for Cloud {
        async fn put_opts(
            &self,
            location: &Path,
            payload: PutPayload,
            options: PutOptions,
        ) -> object_store::Result<PutResult> {
            self.fault()?;
            self.inner.put_opts(location, payload, options).await
        }

        async fn put_multipart_opts(
            &self,
            location: &Path,
            options: PutMultipartOptions,
        ) -> object_store::Result<Box<dyn MultipartUpload>> {
            self.fault()?;
            self.inner.put_multipart_opts(location, options).await
        }

        async fn get_opts(
            &self,
            location: &Path,
            options: GetOptions,
        ) -> object_store::Result<GetResult> {
            self.fault()?;
            while self.faults().stall {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            self.gets.lock().unwrap().push((
                location.to_string(),
                options.range.is_some(),
                options.head,
            ));
            if self.faults().forbidden.contains(location.as_ref()) {
                return Err(object_store::Error::PermissionDenied {
                    path: location.to_string(),
                    source: "403 Forbidden".into(),
                });
            }
            let head = options.head;
            let result = match self.inner.get_opts(location, options).await {
                Err(object_store::Error::NotFound { path, .. })
                    if self.faults().forbidden_missing =>
                {
                    return Err(object_store::Error::PermissionDenied {
                        path,
                        source: "403 Forbidden".into(),
                    });
                }
                result => result?,
            };
            let body_fault = {
                let faults = self.faults();
                faults.stall_body || faults.cut_body
            };
            if head || !body_fault {
                return Ok(result);
            }
            self.interrupted(result).await
        }

        fn delete_stream(
            &self,
            locations: BoxStream<'static, object_store::Result<Path>>,
        ) -> BoxStream<'static, object_store::Result<Path>> {
            self.inner.delete_stream(locations)
        }

        fn list(
            &self,
            prefix: Option<&Path>,
        ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
            if let Err(error) = self.fault() {
                return futures::stream::once(async move { Err(error) }).boxed();
            }
            self.lists
                .lock()
                .unwrap()
                .push(prefix.map(ToString::to_string).unwrap_or_default());
            self.inner.list(prefix)
        }

        async fn list_with_delimiter(
            &self,
            prefix: Option<&Path>,
        ) -> object_store::Result<ListResult> {
            self.fault()?;
            self.lists
                .lock()
                .unwrap()
                .push(prefix.map(ToString::to_string).unwrap_or_default());
            self.inner.list_with_delimiter(prefix).await
        }

        async fn copy_opts(
            &self,
            from: &Path,
            to: &Path,
            options: CopyOptions,
        ) -> object_store::Result<()> {
            self.fault()?;
            self.inner.copy_opts(from, to, options).await
        }

        async fn rename_opts(
            &self,
            from: &Path,
            to: &Path,
            options: RenameOptions,
        ) -> object_store::Result<()> {
            self.fault()?;
            self.inner.rename_opts(from, to, options).await
        }
    }

    /// The cloud binding: URL paths map to keys verbatim, or below `prefix` to the issuer's
    /// spelling (`url path prefix`, `key prefix`).
    #[derive(Debug)]
    struct CloudProvider {
        cloud: Arc<Cloud>,
        prefix: Option<(String, String)>,
    }

    #[async_trait::async_trait]
    impl ObjectStoreProvider for CloudProvider {
        async fn new_store(
            &self,
            url: Url,
            params: &ObjectStoreParams,
        ) -> lance::Result<LanceStore> {
            Ok(LanceStore::new(
                self.cloud.clone(),
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

        fn extract_path(&self, url: &Url) -> lance::Result<Path> {
            let path = match &self.prefix {
                Some((url_prefix, key_prefix)) => match url.path().strip_prefix(url_prefix) {
                    Some(suffix) => format!("{key_prefix}{suffix}"),
                    None => url.path().to_string(),
                },
                None => url.path().to_string(),
            };
            Path::parse(path).map_err(|error| lance::Error::invalid_input(error.to_string()))
        }
    }

    struct Host {
        cloud: Arc<Cloud>,
        offline: AtomicBool,
        no_lease: AtomicBool,
        observed: Mutex<Vec<MirrorFetch>>,
        allowance: Mutex<u64>,
        transferred: AtomicU64,
        changes: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl MirrorHost for Host {
        fn is_offline(&self) -> bool {
            self.offline.load(Ordering::Acquire)
        }
        async fn cloud(&self) -> Result<Arc<dyn ObjectStore>> {
            ensure!(
                !self.no_lease.load(Ordering::Acquire),
                "Connect to the hub to download offline data"
            );
            Ok(self.cloud.clone())
        }
        fn classify(&self, error: &object_store::Error) -> Option<MirrorFetch> {
            let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
            while let Some(error) = source {
                if let Some(error) = error.downcast_ref::<std::io::Error>() {
                    return match error.kind() {
                        std::io::ErrorKind::TimedOut => Some(MirrorFetch::TimedOut),
                        std::io::ErrorKind::ConnectionRefused
                        | std::io::ErrorKind::ConnectionReset => Some(MirrorFetch::ConnectFailed),
                        _ => None,
                    };
                }
                if let Some(AuthorizationError::Unavailable) =
                    error.downcast_ref::<AuthorizationError>()
                {
                    return Some(MirrorFetch::ConnectFailed);
                }
                source = error.source();
            }
            None
        }
        fn observe(&self, outcome: MirrorFetch) {
            self.observed.lock().unwrap().push(outcome);
        }
        fn allow_download(&self, bytes: u64, force: bool) -> bool {
            let mut allowance = self.allowance.lock().unwrap();
            if !force && *allowance < bytes {
                return false;
            }
            *allowance = allowance.saturating_sub(bytes);
            true
        }
        fn downloaded(&self, reserved: u64, transferred: u64) {
            *self.allowance.lock().unwrap() += reserved.saturating_sub(transferred);
            self.transferred.fetch_add(transferred, Ordering::AcqRel);
        }
        fn changed(&self) {
            self.changes.fetch_add(1, Ordering::AcqRel);
        }
    }

    struct Fixture {
        root: PathBuf,
        cloud: Arc<Cloud>,
        cloud_registry: Arc<ObjectStoreRegistry>,
        hub: Connection,
        host: Arc<Host>,
        local: LocalConnection,
        maximum: u64,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn cloud_session(
        cloud: &Arc<Cloud>,
        prefix: Option<(String, String)>,
    ) -> Arc<lance::session::Session> {
        let registry = Arc::new(ObjectStoreRegistry::empty());
        registry.insert(
            "cloudsim",
            Arc::new(CloudProvider {
                cloud: cloud.clone(),
                prefix,
            }),
        );
        Arc::new(lance::session::Session::new(
            16 * 1024 * 1024,
            16 * 1024 * 1024,
            registry,
        ))
    }

    fn private(directory: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    impl Fixture {
        async fn new(maximum: u64) -> Result<Self> {
            Self::with_database(maximum, DB, None).await
        }

        async fn with_database(
            maximum: u64,
            database: &str,
            prefix: Option<(String, String)>,
        ) -> Result<Self> {
            let root = std::env::temp_dir().join(format!(
                "flow-like-offline-mirror-{}",
                flow_like_types::create_id()
            ));
            let cloud_dir = root.join("cloud");
            std::fs::create_dir_all(&cloud_dir)?;
            private(&root.join("tables"))?;
            let cloud = Arc::new(Cloud {
                inner: LocalFileSystem::new_with_prefix(&cloud_dir)?,
                faults: Default::default(),
                gets: Default::default(),
                lists: Default::default(),
            });
            let session = cloud_session(&cloud, prefix);
            let cloud_registry = session.store_registry();
            let hub = lancedb::connect(database)
                .session(session)
                .execute()
                .await?;
            let host = Arc::new(Host {
                cloud: cloud.clone(),
                offline: AtomicBool::new(false),
                no_lease: AtomicBool::new(false),
                observed: Default::default(),
                allowance: Mutex::new(u64::MAX / 2),
                transferred: AtomicU64::new(0),
                changes: AtomicUsize::new(0),
            });
            let local = Self::connect(&root, maximum, &host, Vec::new()).await?;
            Ok(Self {
                root,
                cloud,
                cloud_registry,
                hub,
                host,
                local,
                maximum,
            })
        }

        async fn connect(
            root: &std::path::Path,
            maximum: u64,
            host: &Arc<Host>,
            roots: Vec<MirrorRoot>,
        ) -> Result<LocalConnection> {
            let local = budgeted_local_connection_with_budget(
                &root.join("tables"),
                maximum,
                Some(LazyMirrorSetup {
                    directory: root.join("mirror"),
                    roots,
                    max_lazy_file_bytes: 64 * MIB,
                }),
            )
            .await?;
            let attached: Weak<dyn MirrorHost> = Arc::downgrade(host) as Weak<dyn MirrorHost>;
            local
                .mirror
                .as_ref()
                .context("lazy mirror")?
                .attach(attached);
            Ok(local)
        }

        /// A fresh device connection over the same directories, as after a restart.
        async fn restart(&mut self, roots: Vec<MirrorRoot>) -> Result<()> {
            self.mirror().close();
            let local = Self::connect(&self.root, self.maximum, &self.host, roots).await?;
            self.local = local;
            Ok(())
        }

        fn mirror(&self) -> &Arc<LazyMirror> {
            self.local.mirror.as_ref().expect("lazy mirror")
        }

        fn cloud_file(&self, key: &str) -> PathBuf {
            key.split('/')
                .fold(self.root.join("cloud"), |path, part| path.join(part))
        }

        async fn seed(&self, fragments: i64) -> Result<Table> {
            let table = self
                .hub
                .create_table("records", batch(0..100)?)
                .execute()
                .await?;
            for fragment in 1..fragments {
                table
                    .add(batch(fragment * 100..(fragment + 1) * 100)?)
                    .execute()
                    .await?;
            }
            Ok(table)
        }

        async fn snapshot(&self, name: &str, previous: &[MirrorFile]) -> Result<LazySnapshot> {
            self.snapshot_of("records", name, previous).await
        }

        async fn snapshot_of(
            &self,
            table: &str,
            name: &str,
            previous: &[MirrorFile],
        ) -> Result<LazySnapshot> {
            let source = self.hub.open_table(table).execute().await?;
            self.mirror()
                .snapshot(&source, &self.local.connection, name, previous)
                .await
        }

        fn retain(&self, files: &[MirrorFile], pinned: bool, whole: bool) {
            self.mirror().set_retention(retention(files, pinned, whole));
        }

        async fn table(&self, name: &str) -> Result<Table> {
            Ok(self.local.connection.open_table(name).execute().await?)
        }

        fn offline(&self, offline: bool) {
            self.cloud.faults().offline = offline;
            self.host.offline.store(offline, Ordering::Release);
        }
    }

    fn retention(files: &[MirrorFile], pinned: bool, whole: bool) -> MirrorRetention {
        let keys = files.iter().map(|file| file.key.clone());
        MirrorRetention {
            listed: files
                .iter()
                .map(|file| (file.key.clone(), file.bytes))
                .collect(),
            pinned: if pinned {
                keys.clone().collect()
            } else {
                HashSet::new()
            },
            whole: if whole {
                keys.collect()
            } else {
                HashSet::new()
            },
        }
    }

    fn schema() -> Arc<Schema> {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("text", DataType::Utf8, true),
            Field::new(
                "vec",
                DataType::FixedSizeList(Arc::new(Field::new("item", DataType::Float32, true)), DIM),
                true,
            ),
        ]))
    }

    fn vector(id: i64) -> Vec<f32> {
        vec![id as f32, (id % 10) as f32, 1.0, 0.5]
    }

    fn batch(ids: Range<i64>) -> Result<RecordBatch> {
        let ids = ids.collect::<Vec<_>>();
        let vectors = FixedSizeListArray::from_iter_primitive::<Float32Type, _, _>(
            ids.iter()
                .map(|id| Some(vector(*id).into_iter().map(Some).collect::<Vec<_>>())),
            DIM,
        );
        Ok(RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int64Array::from(ids.clone())),
                Arc::new(StringArray::from_iter_values(ids.iter().map(|id| {
                    if id % 7 == 0 {
                        format!("row {id} needle")
                    } else {
                        format!("row {id} hay")
                    }
                }))),
                Arc::new(vectors),
            ],
        )?)
    }

    async fn scan_ids(table: &Table, filter: Option<&str>) -> Result<Vec<i64>> {
        let mut query = table.query().select(Select::Columns(vec!["id".into()]));
        if let Some(filter) = filter {
            query = query.only_if(filter);
        }
        let batches = query.execute().await?.try_collect::<Vec<_>>().await?;
        let mut ids = Vec::new();
        for batch in batches {
            let column = batch
                .column_by_name("id")
                .context("result has no id column")?
                .as_any()
                .downcast_ref::<Int64Array>()
                .context("id column is not Int64")?
                .clone();
            ids.extend((0..column.len()).map(|index| column.value(index)));
        }
        ids.sort_unstable();
        Ok(ids)
    }

    async fn nearest(table: &Table, id: i64) -> Result<Vec<RecordBatch>> {
        Ok(table
            .query()
            .nearest_to(vector(id))?
            .limit(3)
            .select(Select::Columns(vec!["id".into()]))
            .execute()
            .await?
            .try_collect::<Vec<_>>()
            .await?)
    }

    fn files_under(directory: &std::path::Path) -> Result<Vec<String>> {
        let mut files = Vec::new();
        let mut stack = vec![directory.to_path_buf()];
        while let Some(current) = stack.pop() {
            for entry in std::fs::read_dir(&current)? {
                let entry = entry?;
                if entry.file_type()?.is_dir() {
                    stack.push(entry.path());
                } else {
                    files.push(
                        entry
                            .path()
                            .strip_prefix(directory)?
                            .to_string_lossy()
                            .replace('\\', "/"),
                    );
                }
            }
        }
        files.sort();
        Ok(files)
    }

    fn of_kind(files: &[MirrorFile], kind: MirrorFileKind) -> Vec<MirrorFile> {
        files
            .iter()
            .filter(|file| file.kind == kind)
            .cloned()
            .collect()
    }

    async fn eventually(mut done: impl FnMut() -> bool) -> Result<()> {
        for _ in 0..500 {
            if done() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        anyhow::bail!("condition not reached within 5 s")
    }

    async fn apply(table: &Table, mutation: ReplayMutation) -> Result<()> {
        let (expected_version, fingerprint) = revision(table).await?;
        let marker = ReplayMarker {
            operation_id: flow_like_types::create_id(),
            digest: "offline-mirror-test".into(),
            expected_version,
            expected_fingerprint: Some(fingerprint),
        };
        match crate::databases::vector::offline_replay::replay(table, &marker, mutation).await? {
            ReplayOutcome::Applied { .. } => {
                table.checkout_latest().await?;
                Ok(())
            }
            other => anyhow::bail!("local mutation was not applied: {other:?}"),
        }
    }

    fn row(id: i64) -> flow_like_types::Value {
        flow_like_types::json::json!({"id": id, "text": format!("row {id} fresh"), "vec": vector(id)})
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn snapshot_writes_only_version_metadata_and_keeps_cloud_indices() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture
            .seed(3)
            .await?
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
        let source = fixture.hub.open_table("records").execute().await?;
        let before = fixture.cloud.gets.lock().unwrap().len();
        let snapshot = fixture
            .mirror()
            .snapshot(&source, &fixture.local.connection, "lazy", &[])
            .await?;
        let reads = fixture.cloud.gets.lock().unwrap()[before..].to_vec();
        assert!(
            reads.iter().all(|(key, _, _)| key.contains("/_versions/")),
            "snapshot read more than manifests: {reads:?}"
        );
        assert_eq!(fixture.mirror().cache_bytes(), 0);
        let local = files_under(&fixture.root.join("tables").join("lazy.lance"))?;
        assert!(
            local.iter().all(|file| file.starts_with("_versions/")),
            "the device keeps only version metadata: {local:?}"
        );
        let dataset = snapshot
            .table
            .table
            .dataset()
            .context("native")?
            .get()
            .await?;
        let store = dataset.object_store(None).await?;
        let indices =
            read_manifest_indexes(&store, dataset.manifest_location(), dataset.manifest()).await?;
        assert_eq!(indices.len(), 1);
        assert!(indices[0].files.is_some() && indices[0].base_id.is_some());
        assert_eq!(snapshot.table.source_version, dataset.manifest().version);
        assert_eq!(
            snapshot.root,
            MirrorRoot {
                url: DB.into(),
                key: "apps/p/storage/db".into()
            }
        );
        assert_eq!(snapshot.table.table.count_rows(None).await?, 300);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn snapshot_lists_every_cloud_file_with_sizes() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        let table = fixture.seed(3).await?;
        table.delete("id = 5").await?;
        table
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
        table
            .create_index(&["text"], Index::FTS(FtsIndexBuilder::default()))
            .execute()
            .await?;
        let snapshot = fixture.snapshot("listed", &[]).await?;
        assert_eq!(of_kind(&snapshot.files, MirrorFileKind::Data).len(), 3);
        assert_eq!(of_kind(&snapshot.files, MirrorFileKind::Deletion).len(), 1);
        assert!(of_kind(&snapshot.files, MirrorFileKind::Index).len() >= 2);
        for file in &snapshot.files {
            assert!(file.key.starts_with("apps/p/storage/db/records.lance/"));
            let size = std::fs::metadata(fixture.cloud_file(&file.key))?.len();
            assert_eq!(file.bytes, Some(size), "{}", file.key);
        }
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn refresh_reuses_known_sizes_and_heads_only_new_keys() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        let table = fixture.seed(2).await?;
        table.delete("id = 5").await?;
        let heads = fixture.cloud.file_heads();
        let first = fixture.snapshot("first", &[]).await?;
        assert_eq!(
            fixture.cloud.file_heads() - heads,
            1,
            "one deletion file to size"
        );
        table.add(batch(200..300)?).execute().await?;
        table.delete("id = 150").await?;
        let heads = fixture.cloud.file_heads();
        let second = fixture.snapshot("second", &first.files).await?;
        assert_eq!(
            fixture.cloud.file_heads() - heads,
            1,
            "only the deletion file the new version adds is sized"
        );
        assert!(second.files.iter().all(|file| file.bytes.is_some()));
        assert_eq!(of_kind(&second.files, MirrorFileKind::Deletion).len(), 2);
        Ok(())
    }

    async fn cloud_manifest(
        fixture: &Fixture,
    ) -> Result<(Manifest, Vec<IndexMetadata>, MirrorRoot)> {
        let table = fixture.hub.open_table("records").execute().await?;
        let dataset = table.dataset().context("native")?.get().await?;
        let store = dataset.object_store(None).await?;
        let indices =
            read_manifest_indexes(&store, dataset.manifest_location(), dataset.manifest()).await?;
        Ok((
            dataset.manifest().clone(),
            indices,
            MirrorRoot::of(&dataset)?,
        ))
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn snapshot_refuses_external_row_metadata_with_e38() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let (mut manifest, indices, root) = cloud_manifest(&fixture).await?;
        refuse_unsupported(&manifest, &indices, &root, "records")?;
        let mut fragments = manifest.fragments.as_ref().clone();
        fragments[0].row_id_meta = Some(RowIdMeta::External(lance_table::format::ExternalFile {
            path: "row_ids.bin".into(),
            offset: 0,
            size: 16,
        }));
        manifest.fragments = Arc::new(fragments);
        let error = refuse_unsupported(&manifest, &indices, &root, "records")
            .expect_err("external row ids are refused");
        assert_eq!(
            error.to_string(),
            "Table 'records' uses a storage layout that offline access does not support yet: external row metadata files."
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn snapshot_refuses_bases_outside_the_tables_database_with_e38() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let reader = lancedb::connect("cloudsim://bucket/users/u/apps/p/db")
            .session(cloud_session(&fixture.cloud, None))
            .execute()
            .await?;
        reader
            .create_table("private", batch(0..10)?)
            .execute()
            .await?;
        let reader_root = fixture
            .snapshot_of_connection(&reader, "private", "reader_copy")
            .await?
            .root;
        let records = fixture.hub.open_table("records").execute().await?;
        fixture.cloud_clone(&records, DB, "sibling").await?;
        let sibling = fixture.snapshot_of("sibling", "sibling_copy", &[]).await?;
        assert!(
            sibling.files.iter().any(|file| file
                .key
                .starts_with("apps/p/storage/db/records.lance/data/")),
            "a clone within the database root stays supported"
        );
        let private = reader.open_table("private").execute().await?;
        fixture.cloud_clone(&private, DB, "borrowed").await?;
        let error = match fixture.snapshot_of("borrowed", "borrowed_copy", &[]).await {
            Ok(_) => anyhow::bail!("a base under another bound root must be refused"),
            Err(error) => error,
        };
        assert_ne!(reader_root.key, "apps/p/storage/db");
        assert_eq!(
            error.to_string(),
            "Table 'borrowed' uses a storage layout that offline access does not support yet: data outside this table's database."
        );
        assert!(
            fixture
                .local
                .connection
                .open_table("borrowed_copy")
                .execute()
                .await
                .is_err()
        );
        Ok(())
    }

    impl Fixture {
        /// A cloud shallow clone of `source` as `target` in `database`, committed like
        /// Lance's own clone (which cannot open custom schemes).
        async fn cloud_clone(&self, source: &Table, database: &str, target: &str) -> Result<()> {
            let dataset = source.dataset().context("native")?.get().await?;
            let store = dataset.object_store(None).await?;
            let indices =
                read_manifest_indexes(&store, dataset.manifest_location(), dataset.manifest())
                    .await?;
            let base_id = dataset
                .manifest()
                .base_paths
                .keys()
                .max()
                .map_or(0, |id| id + 1);
            let mut manifest = dataset.manifest().shallow_clone(
                None,
                dataset.uri().into(),
                base_id,
                None,
                String::new(),
            );
            manifest.transaction_file = None;
            apply_feature_flags(&mut manifest, false, false)?;
            let indices = indices
                .into_iter()
                .map(|mut index| {
                    index.base_id.get_or_insert(base_id);
                    index
                })
                .collect::<Vec<_>>();
            let uri = format!("{database}/{target}.lance");
            let (store, base) = LanceStore::from_uri_and_params(
                self.cloud_registry.clone(),
                &uri,
                &ObjectStoreParams::default(),
            )
            .await?;
            commit_handler_from_url(&uri, &None)
                .await?
                .commit(
                    &mut manifest,
                    (!indices.is_empty()).then_some(indices),
                    &base,
                    store.as_ref(),
                    write_manifest_file_to_path,
                    ManifestNamingScheme::V2,
                    None,
                )
                .await
                .map_err(|error| anyhow!("cloud clone failed: {error:?}"))?;
            Ok(())
        }

        async fn snapshot_of_connection(
            &self,
            connection: &Connection,
            table: &str,
            name: &str,
        ) -> Result<LazySnapshot> {
            let source = connection.open_table(table).execute().await?;
            self.mirror()
                .snapshot(&source, &self.local.connection, name, &[])
                .await
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn snapshot_refuses_blob_v2_and_mem_wal_tables_with_e38() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture
            .seed(1)
            .await?
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
        let (manifest, indices, root) = cloud_manifest(&fixture).await?;
        let mut blob = manifest.clone();
        let field = blob
            .schema
            .fields
            .iter_mut()
            .find(|field| field.name == "text");
        field
            .context("text field")?
            .metadata
            .insert("ARROW:extension:name".into(), "lance.blob.v2".into());
        assert!(
            refuse_unsupported(&blob, &indices, &root, "records")
                .expect_err("blob v2 is refused")
                .to_string()
                .ends_with("does not support yet: blob columns.")
        );
        let mut wal = indices.clone();
        let mut index = indices.first().context("key index")?.clone();
        index.name = MEM_WAL_INDEX_NAME.into();
        wal.push(index);
        assert!(
            refuse_unsupported(&manifest, &wal, &root, "records")
                .expect_err("MemWAL is refused")
                .to_string()
                .ends_with("does not support yet: a MemWAL index.")
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn snapshot_strips_auto_cleanup_config() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        let table = fixture.seed(1).await?;
        table
            .as_native()
            .context("native")?
            .update_config([
                ("lance.auto_cleanup.interval".to_string(), "1".to_string()),
                (
                    "lance.auto_cleanup.older_than".to_string(),
                    "0s".to_string(),
                ),
                ("custom.kept".to_string(), "yes".to_string()),
            ])
            .await?;
        let snapshot = fixture.snapshot("clean", &[]).await?;
        let dataset = snapshot
            .table
            .table
            .dataset()
            .context("native")?
            .get()
            .await?;
        let config = &dataset.manifest().config;
        assert!(!config.keys().any(|key| key.starts_with(AUTO_CLEANUP)));
        assert_eq!(config.get("custom.kept").map(String::as_str), Some("yes"));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn roots_map_uris_to_keys_like_the_cloud_binding() -> Result<()> {
        let root = MirrorRoot {
            url: "s3://bucket/users/auth0%7C123/apps/a/db".into(),
            key: "users/auth0|123/apps/a/db".into(),
        };
        let key = |url: &str| {
            root.key_of(&Url::parse(url).unwrap())
                .map(|key| key.to_string())
        };
        assert_eq!(
            key("s3://bucket/users/auth0%7C123/apps/a/db/t.lance/data/x.lance").as_deref(),
            Some("users/auth0|123/apps/a/db/t.lance/data/x.lance")
        );
        assert_eq!(
            key("s3://bucket/users/auth0%7C123/apps/a/db2/t.lance"),
            None
        );
        assert_eq!(key("s3://other/users/auth0%7C123/apps/a/db/t.lance"), None);
        assert_eq!(key("gs://bucket/users/auth0%7C123/apps/a/db/t.lance"), None);

        let fixture = Fixture::with_database(
            256 * MIB,
            "cloudsim://bucket/users/auth0%7C123/apps/a/db",
            Some((
                "/users/auth0%7C123/apps/a".into(),
                "users/auth0|123/apps/a".into(),
            )),
        )
        .await?;
        fixture
            .seed(2)
            .await?
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
        let snapshot = fixture.snapshot("user", &[]).await?;
        assert_eq!(snapshot.root.key, "users/auth0|123/apps/a/db");
        assert!(snapshot.files.iter().all(|file| {
            file.key
                .starts_with("users/auth0|123/apps/a/db/records.lance/")
        }));
        fixture.retain(&snapshot.files, true, true);
        fixture.mirror().fetch(&snapshot.files, true).await?;
        fixture.offline(true);
        let requests = fixture.cloud.requests();
        assert_eq!(
            scan_ids(&snapshot.table.table, Some("id = 150")).await?,
            vec![150]
        );
        assert_eq!(scan_ids(&snapshot.table.table, None).await?.len(), 200);
        assert_eq!(
            fixture.cloud.requests(),
            requests,
            "every key Lance requested was listed"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn reads_cache_whole_immutable_files_once_and_pass_through_other_keys() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture
            .seed(3)
            .await?
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
        let snapshot = fixture.snapshot("cached", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        assert_eq!(
            scan_ids(&snapshot.table.table, Some("id = 150")).await?,
            vec![150]
        );
        eventually(|| fixture.mirror().cached(&data[1].key).is_some()).await?;
        assert_eq!(fixture.mirror().cached(&data[0].key), None);
        assert_eq!(fixture.mirror().cached(&data[2].key), None);
        // Background downloads skip while every slot is busy, so a file the first read
        // touched may download on a later read; repeat until nothing new arrives.
        let mut transferred = u64::MAX;
        for _ in 0..10 {
            eventually(|| {
                fixture.host.transferred.load(Ordering::Acquire) == fixture.mirror().cache_bytes()
            })
            .await?;
            let now = fixture.host.transferred.load(Ordering::Acquire);
            if now == transferred {
                break;
            }
            transferred = now;
            assert_eq!(
                scan_ids(&snapshot.table.table, Some("id = 160")).await?,
                vec![160]
            );
        }
        let gets = fixture.cloud.whole_gets(&data[1].key);
        for _ in 0..2 {
            assert_eq!(
                scan_ids(&snapshot.table.table, Some("id = 160")).await?,
                vec![160]
            );
        }
        assert_eq!(fixture.cloud.whole_gets(&data[1].key), gets);
        assert_eq!(
            fixture.host.transferred.load(Ordering::Acquire),
            transferred,
            "each touched file downloads once"
        );
        fixture.offline(true);
        assert_eq!(
            scan_ids(&snapshot.table.table, Some("id = 170")).await?,
            vec![170]
        );
        fixture.offline(false);
        let manifest = fixture
            .cloud
            .inner
            .list(Some(&Path::parse(
                "apps/p/storage/db/records.lance/_versions",
            )?))
            .try_next()
            .await?
            .context("cloud manifest")?
            .location;
        let requests = fixture.cloud.requests();
        let read = fixture
            .mirror()
            .get(&manifest, GetOptions::default())
            .await?
            .bytes()
            .await?;
        assert!(!read.is_empty());
        assert_eq!(
            fixture.cloud.requests(),
            requests + 1,
            "other keys pass through"
        );
        assert!(fixture.mirror().cached(manifest.as_ref()).is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn large_or_unaffordable_files_are_read_by_range_without_caching() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture
            .seed(3)
            .await?
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
        let snapshot = fixture.snapshot("ranged", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        fixture.mirror().set_max_lazy_file_bytes(1);
        assert_eq!(
            scan_ids(&snapshot.table.table, Some("id = 150")).await?,
            vec![150]
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            fixture.mirror().cache_bytes(),
            0,
            "files over the cap stay in the cloud"
        );

        fixture.mirror().set_max_lazy_file_bytes(64 * MIB);
        *fixture.host.allowance.lock().unwrap() = 0;
        assert_eq!(
            scan_ids(&snapshot.table.table, Some("id = 250")).await?,
            vec![250]
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(
            fixture.mirror().cache_bytes(),
            0,
            "no allowance, no download"
        );

        *fixture.host.allowance.lock().unwrap() = u64::MAX / 2;
        fixture
            .local
            .budget
            .set_maximum(fixture.local.budget.used() + 1024);
        assert_eq!(
            scan_ids(&snapshot.table.table, Some("id = 50")).await?,
            vec![50]
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(fixture.mirror().cache_bytes(), 0, "no room, no download");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn offline_miss_text_contains_the_marker_and_the_key() -> Result<()> {
        let mut fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(2).await?;
        let snapshot = fixture.snapshot("missing", &[]).await?;
        let root = snapshot.root.clone();
        fixture.restart(vec![root]).await?;
        fixture.retain(&snapshot.files, false, false);
        fixture.offline(true);
        let table = fixture.table("missing").await?;
        let error = scan_ids(&table, Some("id = 150"))
            .await
            .expect_err("uncached data is not readable offline");
        let text = format!("{error:#}");
        assert!(is_mirror_miss(&error), "{text}");
        assert!(text.contains(&missing_data_text("records")), "{text}");
        assert!(
            of_kind(&snapshot.files, MirrorFileKind::Data)
                .iter()
                .any(|file| text.contains(&file.key)),
            "{text}"
        );
        assert!(mirror_stale_key(&error).is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn lease_refresh_failures_during_a_miss_are_offline_class() -> Result<()> {
        let mut fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("lease", &[]).await?;
        fixture.restart(vec![snapshot.root.clone()]).await?;
        fixture.retain(&snapshot.files, false, false);
        fixture.cloud.faults().lease = true;
        let table = fixture.table("lease").await?;
        let error = scan_ids(&table, None)
            .await
            .expect_err("the lease cannot be renewed");
        assert!(is_mirror_miss(&error), "{error:#}");
        assert!(
            fixture
                .host
                .observed
                .lock()
                .unwrap()
                .contains(&MirrorFetch::ConnectFailed)
        );
        fixture.cloud.faults().lease = false;
        fixture.host.no_lease.store(true, Ordering::Release);
        let error = scan_ids(&table, None)
            .await
            .expect_err("no lease, no token");
        assert!(format!("{error:#}").contains("Connect to the hub to download offline data"));
        assert!(is_mirror_miss(&error));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stalled_downloads_observe_timeouts_and_fail_once_the_breaker_opens() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("stalled", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        fixture
            .mirror()
            .first_byte_millis
            .store(100, Ordering::Release);
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.cloud.faults().stall = true;
        let mirror = fixture.mirror().clone();
        let key = Path::parse(&data[0].key)?;
        let read = tokio::spawn(async move { mirror.get(&key, GetOptions::default()).await });
        let host = fixture.host.clone();
        eventually(|| {
            host.observed
                .lock()
                .unwrap()
                .contains(&MirrorFetch::TimedOut)
        })
        .await?;
        assert!(
            !read.is_finished(),
            "the read keeps waiting while the hub answers"
        );
        fixture.host.offline.store(true, Ordering::Release);
        let error = read.await?.expect_err("the breaker opened");
        assert!(error.to_string().contains(MIRROR_MISS));
        fixture.cloud.faults().stall = false;
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn listed_keys_answer_head_without_the_cloud() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?.delete("id = 3").await?;
        let snapshot = fixture.snapshot("heads", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        let requests = fixture.cloud.requests();
        for file in &snapshot.files {
            let meta = fixture
                .mirror()
                .get(&Path::parse(&file.key)?, GetOptions::new().with_head(true))
                .await?
                .meta;
            assert_eq!(Some(meta.size), file.bytes);
        }
        fixture.offline(true);
        let file = &snapshot.files[0];
        let meta = fixture
            .mirror()
            .get(&Path::parse(&file.key)?, GetOptions::new().with_head(true))
            .await?
            .meta;
        assert_eq!(Some(meta.size), file.bytes);
        assert_eq!(fixture.cloud.requests(), requests);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn absent_index_files_answer_not_found_without_the_cloud() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture
            .seed(1)
            .await?
            .create_index(&["id"], Index::BTree(BTreeIndexBuilder::default()))
            .execute()
            .await?;
        let snapshot = fixture.snapshot("probes", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        let index = of_kind(&snapshot.files, MirrorFileKind::Index)
            .into_iter()
            .next()
            .context("index file")?;
        let (directory, _) = index.key.rsplit_once('/').context("index directory")?;
        let probe = Path::parse(format!("{directory}/bitmap_page_lookup.lance"))?;
        let requests = fixture.cloud.requests();
        for options in [GetOptions::new().with_head(true), GetOptions::default()] {
            assert!(matches!(
                fixture.mirror().get(&probe, options).await,
                Err(object_store::Error::NotFound { .. })
            ));
        }
        fixture.offline(true);
        assert!(matches!(
            fixture
                .mirror()
                .get(&probe, GetOptions::new().with_head(true))
                .await,
            Err(object_store::Error::NotFound { .. })
        ));
        assert_eq!(fixture.cloud.requests(), requests);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn list_under_a_bound_root_passes_through_online_and_fails_offline() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(2).await?;
        let snapshot = fixture.snapshot("listing", &[]).await?;
        fixture.mirror().fetch(&snapshot.files, true).await?;
        let prefix = Path::parse("apps/p/storage/db/records.lance/data")?;
        let listed = fixture
            .mirror()
            .list(&prefix)
            .try_collect::<Vec<_>>()
            .await?;
        assert_eq!(listed.len(), 2);
        assert!(
            fixture
                .cloud
                .lists
                .lock()
                .unwrap()
                .contains(&prefix.to_string())
        );
        fixture.offline(true);
        let error = fixture
            .mirror()
            .list(&prefix)
            .try_collect::<Vec<_>>()
            .await
            .expect_err("listings are never built from the cache");
        assert!(error.to_string().contains(MIRROR_MISS));
        assert!(fixture.mirror().list_with_delimiter(&prefix).await.is_err());
        let outside = Path::parse("apps/other/storage/db")?;
        assert!(
            fixture
                .mirror()
                .get(&outside, GetOptions::default())
                .await
                .expect_err("outside the bound roots")
                .to_string()
                .contains("it is outside this project's databases")
        );
        Ok(())
    }

    async fn vanished(forbidden_missing: bool) -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(2).await?;
        let snapshot = fixture.snapshot("vanished", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        fixture.cloud.faults().forbidden_missing = forbidden_missing;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        std::fs::remove_file(fixture.cloud_file(&data[0].key))?;
        let key = Path::parse(&data[0].key)?;
        let error = anyhow::Error::from(
            fixture
                .mirror()
                .get(&key, GetOptions::default())
                .await
                .expect_err("the file is gone"),
        );
        assert!(
            error.to_string().contains(&reorganized_text("records")),
            "{error:#}"
        );
        assert_eq!(
            mirror_stale_key(&error).as_deref(),
            Some(data[0].key.as_str())
        );
        assert_eq!(fixture.mirror().stale_keys(), vec![data[0].key.clone()]);
        fixture.mirror().clear_stale(&[data[0].key.clone()]);

        fixture.cloud.faults().forbidden.insert(data[1].key.clone());
        let error = fixture
            .mirror()
            .get(&Path::parse(&data[1].key)?, GetOptions::default())
            .await
            .expect_err("a 403 for a present key");
        assert!(matches!(
            error,
            object_store::Error::PermissionDenied { .. }
        ));
        assert!(fixture.mirror().stale_keys().is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn missing_listed_keys_are_confirmed_by_listing_whether_the_cloud_answers_404_or_403()
    -> Result<()> {
        vanished(false).await?;
        vanished(true).await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn router_serves_cloud_keys_from_the_mirror_and_rejects_base_writes() -> Result<()> {
        let mut fixture = Fixture::new(256 * MIB).await?;
        let table = fixture.seed(3).await?;
        table
            .create_index(
                &["vec"],
                Index::IvfFlat(IvfFlatIndexBuilder::default().num_partitions(2)),
            )
            .execute()
            .await?;
        let expected = nearest(&table, 150).await?;
        let snapshot = fixture.snapshot("routed", &[]).await?;
        fixture.retain(&snapshot.files, true, true);
        fixture.mirror().fetch(&snapshot.files, true).await?;
        fixture.restart(vec![snapshot.root.clone()]).await?;
        fixture.retain(&snapshot.files, true, true);
        fixture.offline(true);
        let routed = fixture.table("routed").await?;
        assert_eq!(nearest(&routed, 150).await?, expected);

        let local = routed
            .dataset()
            .context("native")?
            .get()
            .await?
            .object_store(None)
            .await?;
        let index = of_kind(&snapshot.files, MirrorFileKind::Index)
            .into_iter()
            .next()
            .context("index file")?;
        let key = Path::parse(&index.key)?;
        assert_eq!(
            local.inner.head(&key).await?.size,
            index.bytes.context("sized")?
        );
        let error = local
            .inner
            .put(&key, PutPayload::from_static(b"x"))
            .await
            .expect_err("the base is read-only");
        assert!(
            error
                .to_string()
                .contains("Offline copies never write to the cloud")
        );
        assert!(
            MirrorStore(fixture.mirror().clone())
                .put(&key, PutPayload::from_static(b"x"))
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::metadata(fixture.cloud_file(&index.key))?.len(),
            index.bytes.unwrap()
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cache_bytes_are_charged_to_the_shared_budget_at_open_and_on_download() -> Result<()> {
        let mut fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(2).await?;
        let snapshot = fixture.snapshot("charged", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        let before = fixture.local.budget.used();
        let downloaded = fixture.mirror().fetch(&snapshot.files, false).await?;
        let files = snapshot.files.len() as u64;
        assert_eq!(downloaded, fixture.mirror().cache_bytes());
        assert_eq!(
            fixture.local.budget.used(),
            before + downloaded + files * FILE_OVERHEAD
        );
        let used = fixture.local.budget.used();
        fixture.restart(vec![snapshot.root.clone()]).await?;
        assert_eq!(fixture.local.budget.used(), used);
        assert_eq!(fixture.mirror().cache_bytes(), downloaded);
        assert_eq!(fixture.host.transferred.load(Ordering::Acquire), downloaded);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn eviction_order_is_garbage_then_lru_and_never_pinned() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(4).await?;
        let snapshot = fixture.snapshot("evicted", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.mirror().fetch(&data, false).await?;
        let [garbage, older, newer, pinned] = [&data[0], &data[1], &data[2], &data[3]];
        let listed = [older.clone(), newer.clone(), pinned.clone()];
        let mut kept = retention(&listed, false, false);
        kept.pinned.insert(pinned.key.clone());
        fixture.mirror().set_retention(kept);
        let requests = fixture.cloud.requests();
        for file in [older, newer] {
            tokio::time::sleep(Duration::from_millis(20)).await;
            fixture
                .mirror()
                .get(&Path::parse(&file.key)?, GetOptions::default())
                .await?;
        }
        assert_eq!(
            fixture.cloud.requests(),
            requests,
            "cached files are read locally"
        );
        let one = garbage.bytes.unwrap() + FILE_OVERHEAD;
        let used = fixture.local.budget.used();
        fixture.mirror().evict_to(used - 1, false).await?;
        assert_eq!(
            fixture.mirror().cached(&garbage.key),
            None,
            "garbage goes first"
        );
        assert!(
            fixture.mirror().cached(&older.key).is_some(),
            "recent files stay"
        );
        fixture.mirror().evict_to(used - one - 1, true).await?;
        assert_eq!(
            fixture.mirror().cached(&older.key),
            None,
            "then least recent use"
        );
        assert!(fixture.mirror().cached(&newer.key).is_some());
        let freed = fixture.mirror().evict_to(0, true).await?;
        assert!(freed > 0);
        assert_eq!(fixture.mirror().cached(&newer.key), None);
        assert!(
            fixture.mirror().cached(&pinned.key).is_some(),
            "pinned files are never evicted"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn garbage_is_kept_for_a_day_unless_space_is_needed() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("garbage", &[]).await?;
        fixture.mirror().fetch(&snapshot.files, false).await?;
        fixture.mirror().set_retention(MirrorRetention::default());
        let day = Duration::from_secs(24 * 3600);
        assert_eq!(fixture.mirror().evict_garbage(day).await?, 0);
        assert!(fixture.mirror().cache_bytes() > 0);
        fixture.mirror().evict_to(0, false).await?;
        assert_eq!(
            fixture.mirror().cache_bytes(),
            0,
            "space needed evicts garbage at once"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn local_writes_reclaim_unpinned_cache_space() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(3).await?;
        let snapshot = fixture.snapshot("reclaim", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.mirror().fetch(&snapshot.files, true).await?;
        let mut kept = retention(&snapshot.files, false, false);
        kept.pinned.insert(data[0].key.clone());
        fixture.mirror().set_retention(kept);
        fixture
            .local
            .budget
            .set_maximum(fixture.local.budget.used() + 8 * 1024);
        apply(
            &snapshot.table.table,
            ReplayMutation::Insert {
                items: vec![row(1000)],
            },
        )
        .await?;
        assert!(fixture.local.budget.used() <= fixture.local.budget.maximum());
        assert!(
            fixture.mirror().cached(&data[0].key).is_some(),
            "pinned files stay"
        );
        assert!(
            data[1..]
                .iter()
                .any(|file| fixture.mirror().cached(&file.key).is_none()),
            "the local write reclaimed live cache space"
        );
        assert_eq!(
            scan_ids(&snapshot.table.table, Some("id = 1000")).await?,
            vec![1000]
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn downloads_are_single_flight_atomic_and_length_checked() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("single", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        let fetches = (0..4).map(|_| fixture.mirror().fetch(&data, false));
        let downloaded = futures::future::try_join_all(fetches).await?;
        assert_eq!(downloaded.iter().sum::<u64>(), data[0].bytes.unwrap());
        assert_eq!(fixture.cloud.whole_gets(&data[0].key), 1);

        fixture.mirror().set_retention(MirrorRetention::default());
        fixture.mirror().evict_to(0, true).await?;
        let wrong = MirrorFile {
            bytes: data[0].bytes.map(|bytes| bytes + 1),
            ..data[0].clone()
        };
        assert!(fixture.mirror().fetch(&[wrong], false).await.is_err());
        assert_eq!(fixture.mirror().cached(&data[0].key), None);
        let leftovers = files_under(&fixture.root.join("mirror"))?;
        assert!(leftovers.is_empty(), "{leftovers:?}");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn downloads_are_fsynced_and_truncated_files_are_dropped_when_the_retention_is_set()
    -> Result<()> {
        let mut fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("truncated", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.mirror().fetch(&data, false).await?;
        let file = fixture.mirror().file_path(&cache_name(&data[0].key));
        assert_eq!(std::fs::metadata(&file)?.len(), data[0].bytes.unwrap());
        assert!(
            files_under(&fixture.root.join("mirror"))?
                .iter()
                .all(|name| !name.ends_with(".partial"))
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(&file)?
            .set_len(10)?;
        fixture.restart(vec![snapshot.root.clone()]).await?;
        assert_eq!(fixture.mirror().cached(&data[0].key), Some(10));
        fixture
            .mirror()
            .state()
            .cached
            .get_mut(&cache_name(&data[0].key))
            .context("cached")?
            .reads = 1;
        fixture.retain(&snapshot.files, false, false);
        assert_eq!(
            fixture.mirror().cached(&data[0].key),
            None,
            "an open read does not keep a truncated file"
        );
        assert!(!file.exists());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn partial_downloads_are_removed_at_open() -> Result<()> {
        let mut fixture = Fixture::new(256 * MIB).await?;
        let name = cache_name("apps/p/storage/db/records.lance/data/x.lance");
        let partial = fixture
            .root
            .join("mirror")
            .join(&name[..2])
            .join(format!("{name}.partial"));
        std::fs::create_dir_all(partial.parent().context("directory")?)?;
        std::fs::write(&partial, vec![0u8; 4096])?;
        let used = fixture.local.budget.used();
        fixture.restart(Vec::new()).await?;
        assert!(!partial.exists());
        assert_eq!(fixture.local.budget.used(), used);
        assert_eq!(fixture.mirror().cache_bytes(), 0);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_downloads_return_their_charge_and_reservation() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("cancelled", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        let used = fixture.local.budget.used();
        let allowance = *fixture.host.allowance.lock().unwrap();
        let transferred = fixture.host.transferred.load(Ordering::Acquire);
        let partial = fixture
            .mirror()
            .file_path(&cache_name(&data[0].key))
            .with_extension("partial");
        fixture.cloud.faults().stall_body = true;
        let mirror = fixture.mirror().clone();
        let files = data.clone();
        let fetch = tokio::spawn(async move { mirror.fetch(&files, false).await });
        eventually(|| partial.exists()).await?;
        assert!(fixture.local.budget.used() > used);
        fetch.abort();
        assert!(fetch.await.is_err_and(|error| error.is_cancelled()));
        assert!(!partial.exists(), "the partial file is removed");
        assert_eq!(fixture.local.budget.used(), used, "the charge is returned");
        let received = fixture.host.transferred.load(Ordering::Acquire) - transferred;
        assert_eq!(
            *fixture.host.allowance.lock().unwrap() + received,
            allowance,
            "the reservation ends with the bytes received"
        );
        assert!(fixture.mirror().locks.lock().unwrap().is_empty());
        assert!(files_under(&fixture.root.join("mirror"))?.is_empty());

        fixture.cloud.faults().stall_body = false;
        let downloaded = fixture.mirror().fetch(&data, false).await?;
        assert_eq!(Some(downloaded), data[0].bytes);
        assert_eq!(
            fixture.local.budget.used(),
            used + downloaded + FILE_OVERHEAD
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn offline_write_misses_fail_before_evicting() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(3).await?;
        let snapshot = fixture.snapshot("offline_write", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.mirror().fetch(&data[..2], false).await?;
        fixture.retain(&snapshot.files, false, false);
        let used = fixture.local.budget.used();
        fixture.local.budget.set_maximum(used);
        fixture.offline(true);
        let _scope = fixture
            .mirror()
            .write_scope("records", Arc::new(HashSet::from([data[2].key.clone()])));
        let error = fixture
            .mirror()
            .get(&Path::parse(&data[2].key)?, GetOptions::default())
            .await
            .expect_err("the write's file is not on this device");
        assert!(error.to_string().contains(MIRROR_MISS), "{error}");
        assert!(
            data[..2]
                .iter()
                .all(|file| fixture.mirror().cached(&file.key).is_some()),
            "nothing was evicted for a download that could not start"
        );
        assert_eq!(fixture.local.budget.used(), used);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn unaffordable_downloads_evict_nothing() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(3).await?;
        let snapshot = fixture.snapshot("unaffordable", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.mirror().fetch(&data[..2], false).await?;
        fixture.retain(&data[2..], false, false);
        let garbage = data[..2]
            .iter()
            .map(|file| file.bytes.unwrap() + FILE_OVERHEAD)
            .sum::<u64>();
        let file = &data[2];
        let (key, name) = (Path::parse(&file.key)?, cache_name(&file.key));
        let bytes = file.bytes.context("sized")?;
        let charge = bytes + FILE_OVERHEAD;
        let kept = || {
            data[..2]
                .iter()
                .all(|file| fixture.mirror().cached(&file.key).is_some())
        };
        let background = || {
            fixture
                .mirror()
                .download(&key, &name, bytes, How::Background)
        };
        let used = fixture.local.budget.used();

        fixture.local.budget.set_maximum(used);
        *fixture.host.allowance.lock().unwrap() = 0;
        assert!(matches!(background().await, Ok(0)));
        assert!(kept(), "no allowance: nothing evicted");
        assert_eq!(*fixture.host.allowance.lock().unwrap(), 0);

        *fixture.host.allowance.lock().unwrap() = u64::MAX / 2;
        fixture.local.budget.set_maximum(used - garbage);
        assert!(matches!(background().await, Ok(0)));
        assert!(kept(), "no room even without the garbage: nothing evicted");
        assert_eq!(fixture.mirror().make_room(charge, false), Err(charge));
        let error = fixture
            .mirror()
            .fetch(&data[2..], false)
            .await
            .expect_err("the file does not fit");
        assert_eq!(
            error.downcast_ref::<MirrorShortage>(),
            Some(&MirrorShortage::Disk { needed: charge })
        );
        assert!(kept());
        assert_eq!(fixture.local.budget.used(), used);
        assert_eq!(
            *fixture.host.allowance.lock().unwrap(),
            u64::MAX / 2,
            "skipped downloads end their reservations"
        );

        fixture.local.budget.set_maximum(used);
        assert!(matches!(background().await, Ok(downloaded) if downloaded == bytes));
        assert!(!kept(), "garbage makes room when it is enough");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cancelled_cached_reads_end_their_open_read() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("open_reads", &[]).await?;
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.mirror().fetch(&data, false).await?;
        let (key, name) = (Path::parse(&data[0].key)?, cache_name(&data[0].key));
        let reads = || {
            fixture
                .mirror()
                .state()
                .cached
                .get(&name)
                .map(|file| file.reads)
        };
        let options = GetOptions::default();
        {
            let mut read = std::pin::pin!(fixture.mirror().read_cached(&key, &name, &options));
            let mut context = std::task::Context::from_waker(std::task::Waker::noop());
            assert!(read.as_mut().poll(&mut context).is_pending());
            assert_eq!(reads(), Some(1));
        }
        assert_eq!(reads(), Some(0));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn background_downloads_never_wait_for_a_download_slot() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(2).await?;
        let snapshot = fixture.snapshot("slots", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.mirror().fetch(&data[..1], false).await?;
        let slots = fixture
            .mirror()
            .downloads
            .clone()
            .acquire_many_owned(MAX_DOWNLOADS as u32)
            .await?;
        let wait = Duration::from_secs(1);
        let download = |file: &MirrorFile, how: How<'static>| {
            let key = Path::parse(&file.key).expect("key");
            let name = cache_name(&file.key);
            let bytes = file.bytes.expect("sized");
            let mirror = fixture.mirror().clone();
            async move { mirror.download(&key, &name, bytes, how).await }
        };
        let skipped = tokio::time::timeout(wait, download(&data[1], How::Background)).await?;
        assert!(matches!(skipped, Ok(0)));
        assert_eq!(fixture.mirror().cached(&data[1].key), None);
        let cached = tokio::time::timeout(wait, download(&data[0], How::Write("records"))).await?;
        assert!(matches!(cached, Ok(0)), "a cached file needs no slot");
        drop(slots);
        let written = download(&data[1], How::Write("records")).await;
        assert!(matches!(written, Ok(bytes) if Some(bytes) == data[1].bytes));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn connections_lost_during_an_online_read_body_are_offline_class() -> Result<()> {
        let fixture = Fixture::new(256 * MIB).await?;
        fixture.seed(1).await?;
        let snapshot = fixture.snapshot("cut", &[]).await?;
        fixture.retain(&snapshot.files, false, false);
        fixture.mirror().set_max_lazy_file_bytes(1);
        let data = of_kind(&snapshot.files, MirrorFileKind::Data);
        fixture.cloud.faults().cut_body = true;
        let error = fixture
            .mirror()
            .get(&Path::parse(&data[0].key)?, GetOptions::default())
            .await?
            .bytes()
            .await
            .expect_err("the connection was lost");
        assert!(
            error.to_string().contains(&missing_data_text("records")),
            "{error}"
        );
        assert!(
            fixture
                .host
                .observed
                .lock()
                .unwrap()
                .contains(&MirrorFetch::ConnectFailed)
        );
        Ok(())
    }

    #[test]
    fn lazy_connection_uses_default_lance_cache_sizes() {
        assert_eq!(
            super::super::offline_replay::local_cache_sizes(true),
            (
                lance::dataset::DEFAULT_INDEX_CACHE_SIZE,
                lance::dataset::DEFAULT_METADATA_CACHE_SIZE
            )
        );
        assert_eq!(
            super::super::offline_replay::local_cache_sizes(false),
            (16 * 1024 * 1024, 16 * 1024 * 1024)
        );
    }
}
