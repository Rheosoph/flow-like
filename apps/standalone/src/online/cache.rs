use anyhow::{Result, ensure};
use bytes::Bytes;
use flow_like_offline_writes::fs::{StoreError, is_offline_error};
use flow_like_types_contracts::authorization::AuthorizationError;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    ops::Range,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

const BLOCK_BYTES: usize = 1024 * 1024;
const LIST_BYTES: usize = 4 * 1024 * 1024;
const ROW_OVERHEAD: u64 = 256;
const MIN_CACHE_BYTES: u64 = 64 * 1024;
const MAX_WRITE_MARKERS: u64 = 64;
/// No provider request outlives this window, so an unresolved write older than
/// it has either committed or never will.
const WRITE_SETTLE_SECONDS: i64 = 3600;
const CLOUD_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

mod stores;
pub(super) use stores::{file_cache, lance_cache};

fn wall_clock() -> CacheResult<i64> {
    crate::enrollment::unix_time().map_err(|error| cache_error(error.to_string()))
}

fn is_scope_digest(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
fn check_private_directory(path: &Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "Cache directory contains a symlink or non-directory"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            metadata.mode() & 0o077 == 0 && metadata.uid() == unsafe { libc::geteuid() },
            "Cache directory must be private and owned by this user"
        );
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
#[error("Cloud object storage is temporarily unavailable")]
struct CloudUnavailable;

#[derive(Debug, thiserror::Error)]
#[error("Cloud storage is offline and this read is not available in the device cache")]
struct OfflineCacheMiss;

#[derive(Debug, thiserror::Error)]
#[error(
    "The project grant deadline has passed; this placement stops serving until it is authorized again"
)]
struct GrantDeadlineElapsed;

/// Preserve server availability as a typed error before the provider hides the
/// HTTP status in its private retry error. Writes retain normal provider behavior.
#[derive(Debug, Clone, Copy)]
pub(super) struct OfflineConnector;

const OUTSIDE_SCOPE: &str = "Object is outside this device cache's project and user scope";

/// Failures of the shared cache, independent of the object_store release a
/// store implements. Each release converts them into its own error variants.
#[derive(Debug, thiserror::Error)]
pub(super) enum CacheFailure {
    #[error(transparent)]
    Generic(Box<dyn std::error::Error + Send + Sync>),
    #[error("{scope}: {0}", scope = OUTSIDE_SCOPE)]
    OutsideScope(String),
}
type CacheResult<T> = std::result::Result<T, CacheFailure>;

fn cache_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> CacheFailure {
    CacheFailure::Generic(error.into())
}
fn denied() -> CacheFailure {
    cache_error(AuthorizationError::Denied)
}

struct ListingJson(Vec<u8>);
impl std::io::Write for ListingJson {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > LIST_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other(
                "Listing exceeds the device cache entry budget",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn is_denied(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(error) = source {
        if error.downcast_ref::<AuthorizationError>() == Some(&AuthorizationError::Denied) {
            return true;
        }
        source = error.source();
    }
    false
}
/// A 404, failed condition, invalid credential, or access denial must never
/// turn into a successful stale read.
pub(super) fn is_offline(error: &impl StoreError) -> bool {
    is_offline_error(error, &|error| {
        error.is::<CloudUnavailable>() || error.is::<OfflineCacheMiss>()
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedMeta {
    path: String,
    modified: String,
    size: u64,
    etag: Option<String>,
    version: Option<String>,
    attributes: Vec<(String, String)>,
}
impl CachedMeta {
    fn generation(&self) -> CacheResult<String> {
        let value = (&self.modified, self.size, &self.etag, &self.version);
        Ok(
            blake3::hash(&serde_json::to_vec(&value).map_err(cache_error)?)
                .to_hex()
                .to_string(),
        )
    }
}

/// Shared by the credential refresher, normal file stores, and Lance stores.
/// The durable denial marker fences other processes using the same scope.
pub(super) struct CacheControl {
    connection: Mutex<Connection>,
    revoked: AtomicBool,
    suspended: AtomicBool,
    prefixes: Vec<String>,
    max_bytes: u64,
    availability: Mutex<ReadAvailability>,
}
#[derive(Default)]
struct ReadAvailability {
    failures: u8,
    retry_at: Option<Instant>,
}
impl fmt::Debug for CacheControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CacheControl")
            .field("max_bytes", &self.max_bytes)
            .finish_non_exhaustive()
    }
}
impl CacheControl {
    pub(super) fn revoke_placement(root: &Path, placement: &str) -> Result<()> {
        Self::visit_placement_scopes(root, placement, None)
    }
    /// Call after the API authorizes the placement's current grant generation.
    /// Each placement retains one shared budget across its cloud stores.
    pub(super) fn retain_placement_scope(root: &Path, placement: &str, keep: &str) -> Result<()> {
        ensure!(is_scope_digest(keep), "Invalid cache scope identifier");
        Self::visit_placement_scopes(root, placement, Some(keep))
    }
    fn visit_placement_scopes(root: &Path, placement: &str, keep: Option<&str>) -> Result<()> {
        crate::config::validate_id("placement", placement)?;
        let base = root.join(".standalone-cache").join(placement);
        match std::fs::symlink_metadata(&base) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        }
        check_private_directory(&root.join(".standalone-cache"))?;
        check_private_directory(&base)?;
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let lock = options.open(base.join("read-cache.lock"))?;
        ensure!(
            lock.metadata()?.is_file(),
            "Invalid cache coordination lock"
        );
        fs2::FileExt::lock_exclusive(&lock)?;
        let entries = match std::fs::read_dir(&base) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        for (index, entry) in entries.enumerate() {
            ensure!(
                index < 65_536,
                "Placement has too many cache scopes to invalidate"
            );
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str().filter(|name| is_scope_digest(name)) else {
                continue;
            };
            if keep == Some(name) {
                continue;
            }
            let root = base.join(name);
            check_private_directory(&root)?;
            let cache = root.join(".standalone-cache");
            if !cache.try_exists()? {
                if keep.is_some() {
                    std::fs::remove_dir_all(&root)?;
                }
                continue;
            }
            check_private_directory(&cache)?;
            let read_through = cache.join("read-through");
            if !read_through.try_exists()? {
                if keep.is_some() {
                    std::fs::remove_dir_all(&root)?;
                }
                continue;
            }
            check_private_directory(&read_through)?;
            for (index, entry) in std::fs::read_dir(&read_through)?.enumerate() {
                ensure!(
                    index < 65_536,
                    "Placement has too many read cache scopes to invalidate"
                );
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str().filter(|name| is_scope_digest(name)) else {
                    continue;
                };
                let directory = read_through.join(name);
                check_private_directory(&directory)?;
                let path = directory.join("objects.sqlite");
                if !path.try_exists()? {
                    continue;
                }
                let metadata = std::fs::symlink_metadata(&path)?;
                ensure!(
                    metadata.is_file() && !metadata.file_type().is_symlink(),
                    "Invalid cache database"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    ensure!(
                        metadata.mode() & 0o077 == 0
                            && metadata.uid() == unsafe { libc::geteuid() },
                        "Cache database must be private and owned by this user"
                    );
                }
                let mut connection = Connection::open_with_flags(
                    &path,
                    rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
                )?;
                connection.busy_timeout(Duration::from_secs(5))?;
                connection.execute_batch("PRAGMA secure_delete=ON;")?;
                let tx = connection
                    .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
                tx.execute(
                    "UPDATE cache_state SET revoked=1,epoch=epoch+1 WHERE singleton=1",
                    [],
                )?;
                tx.execute("DELETE FROM cache_entries", [])?;
                tx.commit()?;
            }
            // Old readers retain an open database with a committed denial
            // marker. Removing the path cannot make that handle authorized.
            if keep.is_some() {
                std::fs::remove_dir_all(&root)?;
            }
        }
        Ok(())
    }
    pub(super) fn new(
        root: &Path,
        scope: &str,
        prefixes: Vec<String>,
        max_bytes: u64,
    ) -> Result<Arc<Self>> {
        ensure!(
            !scope.is_empty() && max_bytes >= MIN_CACHE_BYTES && max_bytes <= i64::MAX as u64 / 4,
            "Invalid read cache size or scope"
        );
        ensure!(
            !prefixes.is_empty()
                && prefixes.iter().all(|prefix| prefix.ends_with('/')
                    && flow_like_storage::object_store::path::Path::parse(prefix)
                        .is_ok_and(|path| format!("{path}/") == *prefix)),
            "Invalid read cache prefixes"
        );
        // Only a digest enters the filesystem. Callers bind scope to the cloud
        // location, device, project, grant version, and delegating user.
        let directory = super::private_cache(
            root,
            "read-through",
            &blake3::hash(scope.as_bytes()).to_hex().to_string(),
        )?;
        let path = directory.join("objects.sqlite");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(&path)?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "Cache database is not a regular file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.mode() & 0o077 == 0 && metadata.uid() == unsafe { libc::geteuid() },
                "Cache database must be private and owned by this user"
            );
        }
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=NORMAL; PRAGMA secure_delete=ON; PRAGMA auto_vacuum=FULL;
            CREATE TABLE IF NOT EXISTS cache_state (singleton INTEGER PRIMARY KEY CHECK(singleton=1), revoked INTEGER NOT NULL, clock INTEGER NOT NULL, epoch INTEGER NOT NULL, bytes INTEGER NOT NULL);
            INSERT OR IGNORE INTO cache_state VALUES (1,0,0,0,0);
            CREATE TABLE IF NOT EXISTS cache_entries (id TEXT PRIMARY KEY, path TEXT NOT NULL, kind INTEGER NOT NULL, generation TEXT NOT NULL, start INTEGER NOT NULL, finish INTEGER NOT NULL, data BLOB NOT NULL, cost INTEGER NOT NULL, accessed INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS cache_write_state (singleton INTEGER PRIMARY KEY CHECK(singleton=1), immutable_uncertain INTEGER NOT NULL);
            INSERT OR IGNORE INTO cache_write_state VALUES (1,0);
            CREATE INDEX IF NOT EXISTS cache_lru ON cache_entries(accessed);
            CREATE INDEX IF NOT EXISTS cache_object ON cache_entries(path,kind,generation,start,finish);
            CREATE INDEX IF NOT EXISTS cache_entry_kind ON cache_entries(kind,accessed);
            CREATE TRIGGER IF NOT EXISTS cache_inserted AFTER INSERT ON cache_entries BEGIN UPDATE cache_state SET bytes=bytes+NEW.cost WHERE singleton=1; END;
            CREATE TRIGGER IF NOT EXISTS cache_removed AFTER DELETE ON cache_entries BEGIN UPDATE cache_state SET bytes=bytes-OLD.cost WHERE singleton=1; END;")?;
        // SQLite pages and indexes are bounded as well as payloads. A rollback
        // journal can temporarily add at most one database worth of pages.
        connection.pragma_update(
            None,
            "max_page_count",
            (max_bytes.saturating_mul(2) / 4096 + 64).min(u32::MAX as u64),
        )?;
        let revoked = connection.query_row(
            "SELECT revoked FROM cache_state WHERE singleton=1",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        // Unresolved writes recorded before write times were kept settle one
        // window after this start instead of immediately.
        let now = crate::enrollment::unix_time()?;
        connection.execute(
            "UPDATE cache_entries SET start=?1 WHERE kind=3 AND start=0",
            [now],
        )?;
        connection.execute(
            "UPDATE cache_write_state SET immutable_uncertain=?1 WHERE singleton=1 AND immutable_uncertain=1",
            [now],
        )?;
        let control = Arc::new(Self {
            connection: Mutex::new(connection),
            revoked: AtomicBool::new(revoked),
            suspended: AtomicBool::new(false),
            prefixes,
            max_bytes,
            availability: Mutex::new(ReadAvailability::default()),
        });
        if !revoked {
            control.transaction(|tx| {
                control.reserve_marker(tx, 0)?;
                control.evict(tx, 0)
            })?;
        }
        Ok(control)
    }
    /// Returns an error when the local database cannot be read. Only a durable
    /// revocation that was actually read latches this process as revoked.
    pub(super) fn is_revoked(&self) -> Result<bool> {
        if self.revoked.load(Ordering::Acquire) {
            return Ok(true);
        }
        let connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Device read cache lock poisoned"))?;
        let revoked = connection.query_row(
            "SELECT revoked FROM cache_state WHERE singleton=1",
            [],
            |row| row.get::<_, bool>(0),
        )?;
        if revoked {
            self.revoked.store(true, Ordering::Release);
        }
        Ok(revoked)
    }
    #[cfg(test)]
    pub(super) fn database_path(&self) -> std::path::PathBuf {
        std::path::PathBuf::from(
            self.connection
                .lock()
                .unwrap()
                .path()
                .expect("file-backed cache database"),
        )
    }
    /// Stops serving cached data in this process without a durable revocation,
    /// for a grant deadline that elapsed with no confirmed denial.
    pub(super) fn suspend(&self) {
        self.suspended.store(true, Ordering::Release);
    }
    fn authorized(&self) -> CacheResult<()> {
        if self.suspended.load(Ordering::Acquire) {
            return Err(cache_error(GrantDeadlineElapsed));
        }
        if self
            .is_revoked()
            .map_err(|error| cache_error(format!("{error:#}")))?
        {
            return Err(denied());
        }
        Ok(())
    }
    fn try_cloud_read(&self) -> CacheResult<bool> {
        self.authorized()?;
        let mut availability = self
            .availability
            .lock()
            .map_err(|_| cache_error("Cache availability lock poisoned"))?;
        if let Some(deadline) = availability.retry_at {
            let now = Instant::now();
            if now < deadline {
                return Ok(false);
            }
            // One request probes recovery; a cancelled probe releases this
            // reservation after a bounded interval without blocking a worker.
            availability.retry_at = Some(now + Duration::from_secs(30));
        }
        Ok(true)
    }
    fn cloud_succeeded(&self) {
        if let Ok(mut availability) = self.availability.lock() {
            *availability = ReadAvailability::default();
        }
    }
    fn cloud_failed(&self, error: &impl StoreError) {
        if !is_offline(error) {
            return;
        }
        if let Ok(mut availability) = self.availability.lock() {
            availability.failures = availability.failures.saturating_add(1).min(6);
            let seconds = (1u64 << (availability.failures - 1)).min(30);
            availability.retry_at = Some(Instant::now() + Duration::from_secs(seconds));
        }
    }
    pub(super) fn revoke(&self) -> Result<()> {
        self.revoked.store(true, Ordering::Release);
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| anyhow::anyhow!("Cache lock poisoned"))?;
        let tx = connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute(
            "UPDATE cache_state SET revoked=1,epoch=epoch+1 WHERE singleton=1",
            [],
        )?;
        tx.execute("DELETE FROM cache_entries", [])?;
        tx.commit()?;
        Ok(())
    }
    fn scope(&self, path: &(impl AsRef<str> + ?Sized), directory: bool) -> CacheResult<()> {
        self.authorized()?;
        let path = path.as_ref();
        if !self.prefixes.iter().any(|prefix| {
            path.starts_with(prefix) || (directory && path == prefix.trim_end_matches('/'))
        }) {
            return Err(CacheFailure::OutsideScope(path.to_owned()));
        }
        Ok(())
    }
    fn transaction<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> CacheResult<T>,
    ) -> CacheResult<T> {
        if self.suspended.load(Ordering::Acquire) {
            return Err(cache_error(GrantDeadlineElapsed));
        }
        if self.revoked.load(Ordering::Acquire) {
            return Err(denied());
        }
        let mut connection = self
            .connection
            .lock()
            .map_err(|_| cache_error("Cache lock poisoned"))?;
        let tx = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(cache_error)?;
        let revoked = tx
            .query_row(
                "SELECT revoked FROM cache_state WHERE singleton=1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .map_err(cache_error)?;
        if revoked {
            self.revoked.store(true, Ordering::Release);
            return Err(denied());
        }
        let value = operation(&tx)?;
        tx.commit().map_err(cache_error)?;
        Ok(value)
    }
    fn epoch(&self) -> CacheResult<i64> {
        self.transaction(|tx| {
            tx.query_row(
                "SELECT epoch FROM cache_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(cache_error)
        })
    }
    fn tick(tx: &Transaction<'_>) -> CacheResult<i64> {
        tx.execute("UPDATE cache_state SET clock=clock+1 WHERE singleton=1", [])
            .map_err(cache_error)?;
        tx.query_row(
            "SELECT clock FROM cache_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(cache_error)
    }
    fn read(&self, id: &str) -> CacheResult<Option<Vec<u8>>> {
        self.transaction(|tx| {
            let data = tx
                .query_row("SELECT data FROM cache_entries WHERE id=?1", [id], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(cache_error)?;
            if data.is_some() {
                let now = Self::tick(tx)?;
                tx.execute(
                    "UPDATE cache_entries SET accessed=?1 WHERE id=?2",
                    params![now, id],
                )
                .map_err(cache_error)?;
            }
            Ok(data)
        })
    }
    #[allow(clippy::too_many_arguments)]
    fn insert(
        &self,
        tx: &Transaction<'_>,
        id: &str,
        path: &str,
        kind: i64,
        generation: &str,
        start: u64,
        finish: u64,
        data: &[u8],
    ) -> CacheResult<()> {
        let cost =
            data.len() as u64 + (id.len() + path.len() + generation.len()) as u64 + ROW_OVERHEAD;
        if kind == 3 {
            if !self.reserve_marker(tx, cost)? {
                return Ok(());
            }
        } else if cost > self.data_budget() {
            return Ok(());
        }
        tx.execute("DELETE FROM cache_entries WHERE id=?1", [id])
            .map_err(cache_error)?;
        if kind != 3 {
            self.evict(tx, cost)?;
        }
        let now = Self::tick(tx)?;
        tx.execute(
            "INSERT INTO cache_entries VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![id, path, kind, generation, start, finish, data, cost, now],
        )
        .map_err(cache_error)?;
        Ok(())
    }
    fn marker_budget(&self) -> u64 {
        (self.max_bytes / 64).min(64 * 1024)
    }
    fn data_budget(&self) -> u64 {
        self.max_bytes - self.marker_budget()
    }
    fn marker_usage(tx: &Transaction<'_>) -> CacheResult<(u64, u64)> {
        tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(cost),0) FROM cache_entries WHERE kind=3",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(cache_error)
    }
    fn uncertain_since(tx: &Transaction<'_>) -> CacheResult<i64> {
        tx.query_row(
            "SELECT immutable_uncertain FROM cache_write_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(cache_error)
    }
    fn reserve_marker(&self, tx: &Transaction<'_>, cost: u64) -> CacheResult<bool> {
        let now = wall_clock()?;
        Self::settle_writes(tx, now)?;
        let (count, bytes) = Self::marker_usage(tx)?;
        if count + u64::from(cost > 0) > MAX_WRITE_MARKERS
            || bytes.saturating_add(cost) > self.marker_budget()
        {
            // An overflow replaces all unresolved operation history with one
            // timestamp that settles a window after the last untracked write.
            // Later writes are tracked again, so steady traffic cannot keep
            // the scope uncertain. The reserved control slice keeps
            // bookkeeping out of the data LRU.
            tx.execute(
                "UPDATE cache_write_state SET immutable_uncertain=?1 WHERE singleton=1",
                [now],
            )
            .map_err(cache_error)?;
            tx.execute("DELETE FROM cache_entries WHERE kind=3", [])
                .map_err(cache_error)?;
            return Ok(false);
        }
        Ok(true)
    }
    /// A write unresolved for the settle window has committed or never will.
    /// Its path's cached data may predate a late commit, so it is dropped with
    /// the marker and revalidated on the next read.
    fn settle_writes(tx: &Transaction<'_>, now: i64) -> CacheResult<()> {
        let cutoff = now - WRITE_SETTLE_SECONDS;
        let settled: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM cache_entries WHERE kind=3 AND start<=?1)",
                [cutoff],
                |row| row.get(0),
            )
            .map_err(cache_error)?;
        let uncertain = Self::uncertain_since(tx)?;
        let resolved = uncertain != 0 && uncertain <= cutoff;
        if !settled && !resolved {
            return Ok(());
        }
        tx.execute("UPDATE cache_state SET epoch=epoch+1 WHERE singleton=1", [])
            .map_err(cache_error)?;
        tx.execute("DELETE FROM cache_entries WHERE kind=2", [])
            .map_err(cache_error)?;
        if settled {
            tx.execute(
                "DELETE FROM cache_entries WHERE kind IN (0,1) AND path IN (SELECT path FROM cache_entries WHERE kind=3 AND start<=?1)",
                [cutoff],
            )
            .map_err(cache_error)?;
            tx.execute(
                "DELETE FROM cache_entries WHERE kind=3 AND start<=?1",
                [cutoff],
            )
            .map_err(cache_error)?;
        }
        if resolved {
            // Untracked writes left no paths, so drop every cached entry whose
            // trust depends on immutability before trusting it again.
            tx.execute(
                "DELETE FROM cache_entries WHERE kind IN (0,1) AND path GLOB '*.lance/data/*.lance'",
                [],
            )
            .map_err(cache_error)?;
            tx.execute(
                "UPDATE cache_write_state SET immutable_uncertain=0 WHERE singleton=1",
                [],
            )
            .map_err(cache_error)?;
        }
        Ok(())
    }
    fn evict(&self, tx: &Transaction<'_>, cost: u64) -> CacheResult<()> {
        let mut total: u64 = tx
            .query_row(
                "SELECT bytes FROM cache_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(cache_error)?;
        total = total.saturating_sub(Self::marker_usage(tx)?.1);
        while total + cost > self.data_budget() {
            let (old, bytes): (String, u64) = tx
                .query_row(
                    "SELECT id,cost FROM cache_entries WHERE kind<>3 ORDER BY accessed LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(cache_error)?;
            tx.execute("DELETE FROM cache_entries WHERE id=?1", [old])
                .map_err(cache_error)?;
            total = total.saturating_sub(bytes);
        }
        Ok(())
    }
    fn observe(&self, meta: &CachedMeta, epoch: i64) -> CacheResult<Option<String>> {
        let generation = meta.generation()?;
        let data = serde_json::to_vec(meta).map_err(cache_error)?;
        self.transaction(|tx| {
            let current: i64 = tx
                .query_row(
                    "SELECT epoch FROM cache_state WHERE singleton=1",
                    [],
                    |row| row.get(0),
                )
                .map_err(cache_error)?;
            if current != epoch {
                return Ok(None);
            }
            tx.execute(
                "DELETE FROM cache_entries WHERE path=?1 AND kind IN (0,1) AND generation<>?2",
                params![meta.path, generation],
            )
            .map_err(cache_error)?;
            self.insert(
                tx,
                &format!("meta:{}", meta.path),
                &meta.path,
                0,
                &generation,
                0,
                0,
                &data,
            )?;
            Ok(Some(generation))
        })
    }
    fn block(
        &self,
        path: &str,
        generation: &str,
        start: u64,
        data: &[u8],
        epoch: i64,
    ) -> CacheResult<()> {
        self.transaction(|tx| {
            let current: i64 = tx
                .query_row(
                    "SELECT epoch FROM cache_state WHERE singleton=1",
                    [],
                    |row| row.get(0),
                )
                .map_err(cache_error)?;
            if current != epoch {
                return Ok(());
            }
            let expected: Option<String> = tx
                .query_row(
                    "SELECT generation FROM cache_entries WHERE id=?1",
                    [format!("meta:{path}")],
                    |row| row.get(0),
                )
                .optional()
                .map_err(cache_error)?;
            if expected.as_deref() != Some(generation) {
                return Ok(());
            }
            let now = Self::tick(tx)?;
            tx.execute(
                "UPDATE cache_entries SET accessed=?1 WHERE id=?2",
                params![now, format!("meta:{path}")],
            )
            .map_err(cache_error)?;
            self.insert(
                tx,
                &format!("block:{path}:{generation}:{start}"),
                path,
                1,
                generation,
                start,
                start + data.len() as u64,
                data,
            )
        })
    }
    fn read_block(
        &self,
        path: &str,
        generation: &str,
        start: u64,
        end: u64,
    ) -> CacheResult<Option<Bytes>> {
        self.transaction(|tx| {
            let block: Option<(String,u64,Vec<u8>)> = tx.query_row("SELECT id,start,data FROM cache_entries WHERE path=?1 AND kind=1 AND generation=?2 AND start<=?3 AND finish>?3 ORDER BY finish DESC LIMIT 1", params![path,generation,start], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).optional().map_err(cache_error)?;
            let Some((id,offset,data)) = block else { return Ok(None); };
            let now = Self::tick(tx)?;
            tx.execute("UPDATE cache_entries SET accessed=?1 WHERE id=?2", params![now,id]).map_err(cache_error)?;
            let from = (start-offset) as usize;
            let to = data.len().min((end-offset) as usize);
            Ok(Some(Bytes::from(data).slice(from..to)))
        })
    }
    fn covers(&self, path: &str, generation: &str, range: &Range<u64>) -> CacheResult<bool> {
        self.transaction(|tx| {
            let mut start = range.start;
            while start < range.end {
                let end: Option<u64> = tx.query_row("SELECT MAX(finish) FROM cache_entries WHERE path=?1 AND kind=1 AND generation=?2 AND start<=?3 AND finish>?3", params![path,generation,start], |row| row.get(0)).map_err(cache_error)?;
                let Some(end) = end else { return Ok(false); };
                start = end;
            }
            Ok(true)
        })
    }
    fn invalidate<P: AsRef<str> + ?Sized>(&self, paths: &[&P]) -> CacheResult<()> {
        self.transaction(|tx| Self::invalidate_data(tx, paths))
    }
    /// Retain an acknowledged logical file write before its outbox payload is
    /// released. The normal cache budget and authorization fence still apply.
    pub(super) fn remember_file(
        &self,
        meta: &flow_like_storage::object_store::ObjectMeta,
        bytes: &[u8],
    ) -> CacheResult<()> {
        self.scope(&meta.location, false)?;
        if bytes.len() as u64 != meta.size {
            return Err(cache_error(
                "Acknowledged file size differs from its payload",
            ));
        }
        let stored = file_cache::cached_meta(meta, &Default::default());
        let generation = stored.generation()?;
        let metadata = serde_json::to_vec(&stored).map_err(cache_error)?;
        self.transaction(|tx| {
            self.acknowledge_file(tx, meta.location.as_ref(), Some(&stored))?;
            self.insert(
                tx,
                &format!("meta:{}", meta.location),
                meta.location.as_ref(),
                0,
                &generation,
                0,
                0,
                &metadata,
            )?;
            for (index, block) in bytes.chunks(BLOCK_BYTES).enumerate() {
                let start = (index * BLOCK_BYTES) as u64;
                self.insert(
                    tx,
                    &format!("block:{}:{generation}:{start}", meta.location),
                    meta.location.as_ref(),
                    1,
                    &generation,
                    start,
                    start + block.len() as u64,
                    block,
                )?;
            }
            Ok(())
        })
    }
    pub(super) fn remember_file_deleted(
        &self,
        path: &(impl AsRef<str> + ?Sized),
    ) -> CacheResult<()> {
        self.scope(path, false)?;
        self.transaction(|tx| self.acknowledge_file(tx, path.as_ref(), None))
    }
    fn acknowledge_file(
        &self,
        tx: &Transaction<'_>,
        path: &str,
        meta: Option<&CachedMeta>,
    ) -> CacheResult<()> {
        tx.execute("UPDATE cache_state SET epoch=epoch+1 WHERE singleton=1", [])
            .map_err(cache_error)?;
        tx.execute(
            "DELETE FROM cache_entries WHERE path=?1 AND kind IN (0,1)",
            [path],
        )
        .map_err(cache_error)?;
        // Update complete snapshots that already exist. Bound work per replay
        // acknowledgement, even when the configured cache budget is very large.
        let mut remaining = 8 * 1024 * 1024usize;
        let mut after = String::new();
        loop {
            let next: Option<(String, Vec<u8>)> = tx
                .query_row(
                    "SELECT id,data FROM cache_entries WHERE kind=2 AND id>?1 ORDER BY id LIMIT 1",
                    [&after],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(cache_error)?;
            let Some((id, bytes)) = next else { break };
            if bytes.len() > remaining {
                tx.execute("DELETE FROM cache_entries WHERE kind=2 AND id>?1", [&after])
                    .map_err(cache_error)?;
                break;
            }
            remaining = remaining.saturating_sub(bytes.len() + id.len() + ROW_OVERHEAD as usize);
            after = id.clone();
            let Some((kind, prefix)) = id.strip_prefix("list:").and_then(|key| key.split_once(':'))
            else {
                continue;
            };
            if !prefix_matches(path, prefix) {
                continue;
            }
            tx.execute("DELETE FROM cache_entries WHERE id=?1", [&id])
                .map_err(cache_error)?;
            // A delimiter listing cannot tell whether a removed object was
            // its directory's last child. Rebuild that listing online.
            if kind != "0" {
                continue;
            }
            let mut entries: Vec<CachedMeta> =
                serde_json::from_slice(&bytes).map_err(cache_error)?;
            entries.retain(|entry| entry.path != path);
            entries.extend(meta.cloned());
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            let mut encoded = ListingJson(Vec::new());
            if serde_json::to_writer(&mut encoded, &entries).is_ok() {
                self.insert(tx, &id, "", 2, "", 0, 0, &encoded.0)?;
            }
        }
        Ok(())
    }
    fn invalidate_data<P: AsRef<str> + ?Sized>(
        tx: &Transaction<'_>,
        paths: &[&P],
    ) -> CacheResult<()> {
        tx.execute("UPDATE cache_state SET epoch=epoch+1 WHERE singleton=1", [])
            .map_err(cache_error)?;
        tx.execute("DELETE FROM cache_entries WHERE kind=2", [])
            .map_err(cache_error)?;
        for path in paths {
            tx.execute(
                "DELETE FROM cache_entries WHERE path=?1 AND kind IN (0,1)",
                [path.as_ref()],
            )
            .map_err(cache_error)?;
        }
        Ok(())
    }
    fn begin_write<P: AsRef<str> + ?Sized>(&self, paths: &[&P]) -> CacheResult<String> {
        let operation = uuid::Uuid::new_v4().to_string();
        let started = wall_clock()?.max(1) as u64;
        self.transaction(|tx| {
            tx.execute("UPDATE cache_state SET epoch=epoch+1 WHERE singleton=1", [])
                .map_err(cache_error)?;
            for path in paths {
                let path = path.as_ref();
                self.insert(
                    tx,
                    &format!("write:{operation}:{path}"),
                    path,
                    3,
                    &operation,
                    started,
                    0,
                    &[],
                )?;
            }
            Ok(operation)
        })
    }
    /// A provider response settles a write whether or not it succeeded; only a
    /// transport failure or cancellation leaves it unresolved.
    fn finish_write<T, P: AsRef<str> + ?Sized, E: StoreError>(
        &self,
        operation: &str,
        paths: &[&P],
        result: &std::result::Result<T, E>,
    ) -> CacheResult<()> {
        match result {
            Ok(_) => {
                self.cloud_succeeded();
                self.complete_write(operation, paths)
            }
            Err(error) => {
                self.handle_error(error);
                if !is_offline(error) && !is_denied(error) {
                    if let Err(cleanup) = self.complete_write(operation, paths) {
                        tracing::warn!("Unable to settle a rejected device cache write: {cleanup}");
                    }
                }
                Ok(())
            }
        }
    }
    fn complete_write<P: AsRef<str> + ?Sized>(
        &self,
        operation: &str,
        paths: &[&P],
    ) -> CacheResult<()> {
        self.transaction(|tx| {
            Self::invalidate_data(tx, paths)?;
            // A separate concurrent or cancelled write remains uncertain.
            tx.execute(
                "DELETE FROM cache_entries WHERE kind=3 AND generation=?1",
                [operation],
            )
            .map_err(cache_error)?;
            Ok(())
        })
    }
    fn immutable_trusted(&self, path: &(impl AsRef<str> + ?Sized)) -> CacheResult<bool> {
        let now = wall_clock()?;
        self.transaction(|tx| {
            Self::settle_writes(tx, now)?;
            if Self::uncertain_since(tx)? != 0 {
                return Ok(false);
            }
            let pending: bool = tx
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM cache_entries WHERE path=?1 AND kind=3)",
                    [path.as_ref()],
                    |row| row.get(0),
                )
                .map_err(cache_error)?;
            Ok(!pending)
        })
    }
    fn listing(&self, key: &str, data: &[u8], epoch: i64) -> CacheResult<()> {
        if data.len() > LIST_BYTES {
            return Ok(());
        }
        self.transaction(|tx| {
            let current: i64 = tx
                .query_row(
                    "SELECT epoch FROM cache_state WHERE singleton=1",
                    [],
                    |row| row.get(0),
                )
                .map_err(cache_error)?;
            if current == epoch {
                self.insert(tx, key, "", 2, "", 0, 0, data)?;
            }
            Ok(())
        })
    }
    fn handle_error(&self, error: &impl StoreError) {
        if is_offline(error) {
            self.cloud_failed(error);
        } else {
            // Conditional and missing-object responses complete a recovery
            // probe just as successful reads do. Denial remains a separate,
            // permanent authorization fence below.
            self.cloud_succeeded();
        }
        if is_denied(error) {
            if let Err(error) = self.revoke() {
                tracing::error!("Unable to erase denied device read cache: {error}");
            }
        }
    }
}

// Lance 13.0.0: dataset/write.rs::prepare_data_file_path names fresh data files
// with fragment/write.rs::generate_random_filename, and staged writes promote
// them with copy_if_not_exists. Manifests, indexes, blobs, and ordinary uploads
// remain revalidated.
fn immutable_lance_data(path: &(impl AsRef<str> + ?Sized)) -> bool {
    let parts: Vec<_> = path.as_ref().split('/').collect();
    let database = matches!(
        parts.as_slice(),
        ["apps", _, "storage", "db", ..] | ["users", _, "apps", _, "db", ..]
    );
    if !database {
        return false;
    }
    let [.., table, directory, filename] = parts.as_slice() else {
        return false;
    };
    let Some(stem) = filename.strip_suffix(".lance") else {
        return false;
    };
    table.ends_with(".lance")
        && *directory == "data"
        && stem.len() == 50
        && stem.as_bytes()[..24]
            .iter()
            .all(|byte| matches!(byte, b'0' | b'1'))
        && stem.as_bytes()[24..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

// Published model folders are write-once: weights.bin and preprocessing.json are written
// before the manifest that records their sha256, and none of them is ever replaced.
// Pointer files under models/{experiments,learning,deployments}/ remain revalidated.
fn immutable_model_artifact(path: &(impl AsRef<str> + ?Sized)) -> bool {
    let parts: Vec<_> = path.as_ref().split('/').collect();
    matches!(
        parts.as_slice(),
        ["apps", _, "storage", "models", "artifacts", _, file]
            | ["users", _, "apps", _, "models", "artifacts", _, file]
            if matches!(*file, "weights.bin" | "manifest.json" | "preprocessing.json")
    )
}

/// `Path::prefix_matches` on the normalized keys both object_store releases share.
fn prefix_matches(path: &str, prefix: &str) -> bool {
    prefix.is_empty()
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::{
        file_cache::{OfflineHttpService, ReadCache, cache_error, delimiter_snapshot},
        *,
    };
    use async_trait::async_trait;
    use flow_like_storage::object_store::{
        self, CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta,
        ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions, PutPayload, PutResult,
        RenameOptions, path::Path as ObjectPath,
    };
    use futures_util::{StreamExt, TryStreamExt, stream::BoxStream};
    use std::sync::atomic::{AtomicU8, AtomicUsize};

    #[tokio::test]
    async fn cloud_disconnect_after_accepting_a_request_allows_offline_reads() {
        use tokio::io::AsyncReadExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/object", listener.local_addr().unwrap());
        let peer = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            assert!(stream.read(&mut bytes).await.unwrap() > 0);
        });
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            crate::enrollment::http_client()
                .unwrap()
                .get(endpoint)
                .send(),
        )
        .await
        .unwrap()
        .unwrap_err();
        peer.await.unwrap();
        assert!(error.is_request());
        assert!(error.status().is_none());
        assert!(is_offline(&cache_error(error)));
    }

    #[derive(Debug, Default)]
    struct Cloud {
        memory: object_store::memory::InMemory,
        historical: object_store::memory::InMemory,
        versioned: AtomicBool,
        mode: AtomicU8,
        reads: AtomicUsize,
        write_started: tokio::sync::Notify,
    }
    impl fmt::Display for Cloud {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("cache-test-cloud")
        }
    }
    impl Cloud {
        fn check(&self, path: &ObjectPath) -> object_store::Result<()> {
            match self.mode.load(Ordering::SeqCst) {
                1 => Err(cache_error(AuthorizationError::Unavailable)),
                2 => Err(cache_error(AuthorizationError::Denied)),
                3 => Err(object_store::Error::NotFound {
                    path: path.to_string(),
                    source: "removed".into(),
                }),
                4 => Err(object_store::Error::PermissionDenied {
                    path: path.to_string(),
                    source: "not allowed".into(),
                }),
                _ => Ok(()),
            }
        }
    }
    #[async_trait]
    impl ObjectStore for Cloud {
        async fn put_opts(
            &self,
            path: &ObjectPath,
            payload: PutPayload,
            options: PutOptions,
        ) -> object_store::Result<PutResult> {
            self.check(path)?;
            if self.mode.load(Ordering::SeqCst) == 5 {
                self.write_started.notify_one();
                futures_util::future::pending::<()>().await;
            }
            self.memory.put_opts(path, payload, options).await
        }
        async fn put_multipart_opts(
            &self,
            path: &ObjectPath,
            options: PutMultipartOptions,
        ) -> object_store::Result<Box<dyn MultipartUpload>> {
            self.check(path)?;
            self.memory.put_multipart_opts(path, options).await
        }
        async fn get_opts(
            &self,
            path: &ObjectPath,
            mut options: GetOptions,
        ) -> object_store::Result<GetResult> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            self.check(path)?;
            if self.versioned.load(Ordering::SeqCst) {
                let version = options.version.take().unwrap_or_else(|| "latest".into());
                let mut result = match version.as_str() {
                    "historical" => self.historical.get_opts(path, options).await?,
                    "latest" => self.memory.get_opts(path, options).await?,
                    _ => {
                        return Err(object_store::Error::NotFound {
                            path: path.to_string(),
                            source: "Version does not exist".into(),
                        });
                    }
                };
                result.meta.version = Some(version);
                return Ok(result);
            }
            self.memory.get_opts(path, options).await
        }
        fn delete_stream(
            &self,
            paths: BoxStream<'static, object_store::Result<ObjectPath>>,
        ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
            if let Err(error) = self.check(&ObjectPath::default()) {
                return Box::pin(futures_util::stream::once(async { Err(error) }));
            }
            self.memory.delete_stream(paths)
        }
        fn list(
            &self,
            prefix: Option<&ObjectPath>,
        ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
            if let Err(error) = self.check(prefix.unwrap_or(&ObjectPath::default())) {
                return Box::pin(futures_util::stream::once(async { Err(error) }));
            }
            self.memory.list(prefix)
        }
        async fn list_with_delimiter(
            &self,
            prefix: Option<&ObjectPath>,
        ) -> object_store::Result<ListResult> {
            self.check(prefix.unwrap_or(&ObjectPath::default()))?;
            self.memory.list_with_delimiter(prefix).await
        }
        async fn copy_opts(
            &self,
            from: &ObjectPath,
            to: &ObjectPath,
            options: CopyOptions,
        ) -> object_store::Result<()> {
            self.check(from)?;
            self.memory.copy_opts(from, to, options).await
        }
    }
    fn control(root: &Path, scope: &str, max: u64) -> Arc<CacheControl> {
        CacheControl::new(
            root,
            scope,
            vec![
                "apps/project/storage/".into(),
                "users/alice/apps/project/".into(),
            ],
            max,
        )
        .unwrap()
    }
    fn immutable() -> ObjectPath {
        ObjectPath::from(format!(
            "apps/project/storage/db/table.lance/data/{}.lance",
            "0".repeat(24) + "a".repeat(26).as_str()
        ))
    }
    async fn body(store: &dyn ObjectStore, path: &ObjectPath) -> Bytes {
        store.get(path).await.unwrap().bytes().await.unwrap()
    }

    #[tokio::test]
    async fn cloud_cache_revalidates_mutable_objects_and_never_masks_deletion_or_denial() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let control = control(root.path(), "scope-alice", 4 * 1024 * 1024);
        let cache = ReadCache::new(cloud.clone(), control.clone());
        let path = ObjectPath::from("apps/project/storage/report.json");
        cloud.memory.put(&path, "first".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "first");
        cloud.memory.put(&path, "next".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "next");
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "next");
        assert_eq!(cache.head(&path).await.unwrap().size, 4);
        let missing = cache
            .get(&ObjectPath::from("apps/project/storage/missing"))
            .await
            .unwrap_err();
        assert!(missing.to_string().contains("offline"));
        cloud.mode.store(3, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        assert!(matches!(
            cache.get(&path).await.unwrap_err(),
            object_store::Error::NotFound { .. }
        ));
        cloud.mode.store(1, Ordering::SeqCst);
        assert!(
            cache
                .get(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        assert_eq!(body(cache.as_ref(), &path).await, "next");
        cloud.mode.store(2, Ordering::SeqCst);
        assert!(is_denied(&cache.get(&path).await.unwrap_err()));
        assert!(control.is_revoked().unwrap());
        assert!(
            control
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM cache_entries", [], |row| row
                    .get::<_, u64>(0))
                .unwrap()
                == 0
        );
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        assert!(is_denied(&cache.get(&path).await.unwrap_err()));
    }

    #[tokio::test]
    async fn historical_version_reads_never_replace_or_evict_the_current_offline_copy() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        cloud.versioned.store(true, Ordering::SeqCst);
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "versions", 4 * 1024 * 1024),
        );
        let path = ObjectPath::from("apps/project/storage/versioned");
        cloud
            .memory
            .put(&path, "current value".into())
            .await
            .unwrap();
        cloud
            .historical
            .put(&path, "old value".into())
            .await
            .unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "current value");
        let old = cache
            .get_opts(
                &path,
                GetOptions {
                    version: Some("historical".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert_eq!(old.meta.version.as_deref(), Some("historical"));
        assert_eq!(old.bytes().await.unwrap(), "old value");
        let missing = cache
            .get_opts(
                &path,
                GetOptions {
                    version: Some("removed-version".into()),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(missing, object_store::Error::NotFound { .. }));
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "current value");
        assert_eq!(
            cache.head(&path).await.unwrap().version.as_deref(),
            Some("latest")
        );
        assert!(
            cache
                .get_opts(
                    &path,
                    GetOptions {
                        version: Some("historical".into()),
                        ..Default::default()
                    }
                )
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        cache.control.revoke().unwrap();
        assert!(is_denied(
            &cache
                .get_opts(
                    &path,
                    GetOptions {
                        version: Some("latest".into()),
                        ..Default::default()
                    }
                )
                .await
                .unwrap_err()
        ));
    }

    #[tokio::test]
    async fn cache_scope_persists_between_instances_and_never_crosses_user_or_project() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let first = control(root.path(), "device:grant:project:alice", 4 * 1024 * 1024);
        let cache = ReadCache::new(cloud.clone(), first.clone());
        let path = ObjectPath::from("users/alice/apps/project/state");
        cloud.memory.put(&path, "private".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "private");
        let other_process = control(root.path(), "device:grant:project:alice", 4 * 1024 * 1024);
        let same = ReadCache::new(cloud.clone(), other_process.clone());
        let different = ReadCache::new(
            cloud.clone(),
            control(root.path(), "device:grant:project:bob", 4 * 1024 * 1024),
        );
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(body(same.as_ref(), &path).await, "private");
        assert!(
            different
                .get(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        for outside in [
            "users/bob/apps/project/state",
            "apps/another/storage/state",
            "apps/project/storage-other/state",
            "apps/project/metadata/manifest",
        ] {
            let before = cloud.reads.load(Ordering::SeqCst);
            assert!(matches!(
                cache.get(&ObjectPath::from(outside)).await.unwrap_err(),
                object_store::Error::PermissionDenied { .. }
            ));
            assert_eq!(cloud.reads.load(Ordering::SeqCst), before);
        }
        let mut pending = same.get(&path).await.unwrap().into_stream();
        first.revoke().unwrap();
        assert!(other_process.is_revoked().unwrap());
        assert!(is_denied(&pending.next().await.unwrap().unwrap_err()));
        let reloaded = control(root.path(), "device:grant:project:alice", 4 * 1024 * 1024);
        assert!(reloaded.is_revoked().unwrap());
    }

    #[tokio::test]
    async fn cache_reads_ranges_without_buffering_whole_objects_and_lru_is_bounded() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let control = control(root.path(), "ranges", 4 * 1024 * 1024);
        let cache = ReadCache::new(cloud.clone(), control.clone());
        let path = immutable();
        let payload = vec![42u8; 6 * BLOCK_BYTES];
        cloud
            .memory
            .put(&path, Bytes::from(payload).into())
            .await
            .unwrap();
        let first = cache.get_range(&path, 0..4096).await.unwrap();
        assert_eq!(first, Bytes::from(vec![42; 4096]));
        let before = cloud.reads.load(Ordering::SeqCst);
        assert_eq!(
            cache.get_range(&path, 100..4000).await.unwrap(),
            Bytes::from(vec![42; 3900])
        );
        assert_eq!(
            cloud.reads.load(Ordering::SeqCst),
            before,
            "verified immutable range should not hit cloud"
        );
        cloud.mode.store(1, Ordering::SeqCst);
        assert!(
            cache
                .get_range(&path, 4096..8192)
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        let mut stream = cache.get(&path).await.unwrap().into_stream();
        let mut total = 0;
        while let Some(bytes) = stream.next().await {
            let bytes = bytes.unwrap();
            assert!(bytes.len() <= BLOCK_BYTES);
            total += bytes.len();
        }
        assert_eq!(total, 6 * BLOCK_BYTES);
        let cost = control
            .connection
            .lock()
            .unwrap()
            .query_row("SELECT SUM(cost) FROM cache_entries", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        assert!(cost <= control.max_bytes);
        cloud.mode.store(1, Ordering::SeqCst);
        assert!(
            cache
                .get_range(&path, 0..4096)
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        assert_eq!(
            cache
                .get_range(&path, 5 * BLOCK_BYTES as u64..6 * BLOCK_BYTES as u64)
                .await
                .unwrap(),
            Bytes::from(vec![42; BLOCK_BYTES])
        );
    }

    #[tokio::test]
    async fn cache_lists_revalidate_and_online_mutations_invalidate_entries() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "lists", 4 * 1024 * 1024),
        );
        let path = ObjectPath::from("apps/project/storage/file");
        let root = ObjectPath::from("apps/project/storage");
        cloud.memory.put(&path, "old".into()).await.unwrap();
        assert_eq!(
            cache
                .list(Some(&root))
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            cache
                .list_with_delimiter(Some(&root))
                .await
                .unwrap()
                .objects
                .len(),
            1
        );
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(
            cache
                .list(Some(&root))
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            cache
                .list_with_delimiter(Some(&root))
                .await
                .unwrap()
                .objects
                .len(),
            1
        );
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        assert_eq!(body(cache.as_ref(), &path).await, "old");
        cache.put(&path, "replacement".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "replacement");
        let added = ObjectPath::from("apps/project/storage/new");
        cloud.memory.put(&added, "new".into()).await.unwrap();
        assert_eq!(
            cache
                .list(Some(&root))
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            2
        );
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(
            cache
                .list(Some(&root))
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            2
        );
        assert!(cache.put(&added, "not queued".into()).await.is_err());
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        assert_eq!(body(&cloud.memory, &added).await, "new");
        cache.delete(&path).await.unwrap();
        cloud.mode.store(1, Ordering::SeqCst);
        assert!(
            cache
                .list(Some(&root))
                .try_collect::<Vec<_>>()
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        assert!(
            cache
                .get(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
    }

    #[tokio::test]
    async fn failed_offline_mutations_preserve_last_known_objects_and_listings() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "failed-writes", 4 * 1024 * 1024),
        );
        let from = ObjectPath::from("apps/project/storage/source");
        let to = ObjectPath::from("apps/project/storage/target");
        let directory = ObjectPath::from("apps/project/storage");
        cloud
            .memory
            .put(&from, "source value".into())
            .await
            .unwrap();
        cloud.memory.put(&to, "target value".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &from).await, "source value");
        assert_eq!(body(cache.as_ref(), &to).await, "target value");
        assert_eq!(
            cache
                .list(Some(&directory))
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            cache
                .list_with_delimiter(Some(&directory))
                .await
                .unwrap()
                .objects
                .len(),
            2
        );
        cloud.mode.store(1, Ordering::SeqCst);
        assert!(cache.put(&to, "rejected".into()).await.is_err());
        assert!(cache.delete(&from).await.is_err());
        assert!(
            cache
                .copy_opts(&from, &to, CopyOptions::default())
                .await
                .is_err()
        );
        assert!(
            cache
                .rename_opts(&from, &to, RenameOptions::default())
                .await
                .is_err()
        );
        assert!(
            cache
                .put_multipart_opts(&to, PutMultipartOptions::default())
                .await
                .is_err()
        );
        assert_eq!(body(cache.as_ref(), &from).await, "source value");
        assert_eq!(body(cache.as_ref(), &to).await, "target value");
        assert_eq!(
            cache
                .list(Some(&directory))
                .try_collect::<Vec<_>>()
                .await
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            cache
                .list_with_delimiter(Some(&directory))
                .await
                .unwrap()
                .objects
                .len(),
            2
        );
        cloud.mode.store(0, Ordering::SeqCst);
        cache.put(&to, "confirmed".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &to).await, "confirmed");
    }

    #[tokio::test]
    async fn cancelled_writes_keep_cached_data_but_never_restore_immutable_trust() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "cancelled-write", 4 * 1024 * 1024),
        );
        let path = immutable();
        cloud.memory.put(&path, "before".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "before");
        cloud.mode.store(5, Ordering::SeqCst);
        let task = tokio::spawn({
            let cache = cache.clone();
            let path = path.clone();
            async move { cache.put(&path, "pending".into()).await }
        });
        tokio::time::timeout(Duration::from_secs(5), cloud.write_started.notified())
            .await
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "before");
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        cloud
            .memory
            .put(&path, "after first response".into())
            .await
            .unwrap();
        let before = cloud.reads.load(Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "after first response");
        assert_eq!(cloud.reads.load(Ordering::SeqCst), before + 1);
        // The cancelled request could commit after an intervening read.
        cloud.memory.put(&path, "late commit".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "late commit");
        let reopened = ReadCache::new(
            cloud.clone(),
            control(root.path(), "cancelled-write", 4 * 1024 * 1024),
        );
        cloud
            .memory
            .put(&path, "after restart".into())
            .await
            .unwrap();
        assert_eq!(body(reopened.as_ref(), &path).await, "after restart");
    }

    #[test]
    fn write_intent_compaction_and_concurrent_success_cannot_restore_immutable_trust() {
        let root = tempfile::tempdir().unwrap();
        let control = control(root.path(), "write-intents", 4 * MIN_CACHE_BYTES);
        let path = immutable();
        let first = control.begin_write(&[&path]).unwrap();
        let second = control.begin_write(&[&path]).unwrap();
        control.complete_write(&second, &[&path]).unwrap();
        assert!(!control.immutable_trusted(&path).unwrap());
        // Data eviction cannot consume the reserved write-control slice.
        for name in ["a", "b", "c", "d"] {
            control
                .transaction(|tx| control.insert(tx, name, "", 2, "", 0, 0, &vec![7u8; 100 * 1024]))
                .unwrap();
        }
        assert!(
            control
                .read(&format!("write:{first}:{path}"))
                .unwrap()
                .is_some()
        );
        assert!(!control.immutable_trusted(&path).unwrap());
        for _ in 0..MAX_WRITE_MARKERS + 1 {
            control.begin_write(&[&path]).unwrap();
        }
        let (markers, marker_bytes) = control
            .transaction(|tx| CacheControl::marker_usage(tx))
            .unwrap();
        assert!(markers <= MAX_WRITE_MARKERS && marker_bytes <= control.marker_budget());
        assert!(
            control
                .read(&format!("write:{first}:{path}"))
                .unwrap()
                .is_none(),
            "Overflow compacts unresolved history into the uncertainty timestamp"
        );
        assert!(
            !control
                .immutable_trusted(&ObjectPath::from("apps/project/storage/another"))
                .unwrap()
        );
        let bytes: u64 = control
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT bytes FROM cache_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(bytes <= control.max_bytes);
    }

    #[test]
    fn a_full_data_cache_reserves_space_for_normal_successful_writes() {
        let root = tempfile::tempdir().unwrap();
        let control = control(root.path(), "full-data", MIN_CACHE_BYTES);
        let payload =
            vec![7u8; (control.data_budget() - ROW_OVERHEAD - "filled".len() as u64) as usize];
        control
            .transaction(|tx| {
                control.insert(tx, "filled", "", 1, "", 0, payload.len() as u64, &payload)
            })
            .unwrap();
        let path = immutable();
        let operation = control.begin_write(&[&path]).unwrap();
        assert_eq!(control.read("filled").unwrap().unwrap(), payload);
        assert!(
            control
                .immutable_trusted(&ObjectPath::from("apps/project/storage/another"))
                .unwrap()
        );
        control.complete_write(&operation, &[&path]).unwrap();
        assert!(control.immutable_trusted(&path).unwrap());
        assert_eq!(control.read("filled").unwrap().unwrap(), payload);
        assert_eq!(
            control
                .transaction(|tx| CacheControl::marker_usage(tx))
                .unwrap(),
            (0, 0)
        );
    }

    #[tokio::test]
    async fn repeated_offline_writes_cannot_fill_the_cache_with_marker_history() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "repeated-failures", MIN_CACHE_BYTES),
        );
        let path = immutable();
        cloud
            .memory
            .put(&path, "last known value".into())
            .await
            .unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "last known value");
        let before: u64 = cache
            .control
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT bytes FROM cache_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        cloud.mode.store(1, Ordering::SeqCst);
        for _ in 0..256 {
            assert!(cache.put(&path, "not queued".into()).await.is_err());
        }
        assert_eq!(
            cache
                .control
                .transaction(|tx| CacheControl::marker_usage(tx))
                .unwrap(),
            (0, 0)
        );
        let after: u64 = cache
            .control
            .connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT bytes FROM cache_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            after, before,
            "Failed-write history must not displace cached data"
        );
        assert_eq!(body(cache.as_ref(), &path).await, "last known value");
        assert!(!cache.control.immutable_trusted(&path).unwrap());
    }

    #[derive(Debug, Clone)]
    struct StatusConnector(Arc<std::sync::atomic::AtomicU16>);
    impl object_store::client::HttpConnector for StatusConnector {
        fn connect(
            &self,
            _: &object_store::ClientOptions,
        ) -> object_store::Result<object_store::client::HttpClient> {
            use object_store::client::HttpClient;
            Ok(HttpClient::new(OfflineHttpService(HttpClient::new(
                self.clone(),
            ))))
        }
    }
    #[async_trait]
    impl object_store::client::HttpService for StatusConnector {
        async fn call(
            &self,
            _: object_store::client::HttpRequest,
        ) -> std::result::Result<object_store::client::HttpResponse, object_store::client::HttpError>
        {
            let mut response =
                object_store::client::HttpResponse::new(Bytes::from_static(b"one").into());
            *response.status_mut() =
                reqwest::StatusCode::from_u16(self.0.load(Ordering::SeqCst)).unwrap();
            for (key, value) in [
                ("content-length", "3"),
                ("etag", "\"one\""),
                ("last-modified", "Tue, 02 Jan 2024 00:00:00 GMT"),
            ] {
                response.headers_mut().insert(key, value.parse().unwrap());
            }
            Ok(response)
        }
    }
    #[tokio::test]
    async fn provider_http_503_uses_cache_but_400_403_and_416_never_do() {
        let root = tempfile::tempdir().unwrap();
        let status = Arc::new(std::sync::atomic::AtomicU16::new(200));
        let s3 = object_store::aws::AmazonS3Builder::new()
            .with_bucket_name("bucket")
            .with_region("us-east-1")
            .with_access_key_id("test-access")
            .with_secret_access_key("test-secret")
            .with_http_connector(StatusConnector(status.clone()))
            .with_retry(object_store::RetryConfig {
                max_retries: 0,
                ..Default::default()
            })
            .build()
            .unwrap();
        let cache = ReadCache::new(Arc::new(s3), control(root.path(), "http", 4 * 1024 * 1024));
        let path = ObjectPath::from("apps/project/storage/http");
        assert_eq!(body(cache.as_ref(), &path).await, "one");
        status.store(503, Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "one");
        for code in [400, 416] {
            status.store(code, Ordering::SeqCst);
            cache.control.cloud_succeeded();
            assert!(!is_offline(&cache.get(&path).await.unwrap_err()));
        }
        status.store(403, Ordering::SeqCst);
        assert!(matches!(
            cache.get(&path).await.unwrap_err(),
            object_store::Error::PermissionDenied { .. }
        ));
        status.store(503, Ordering::SeqCst);
        assert!(
            cache
                .get(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
    }

    #[tokio::test]
    async fn transient_outages_use_bounded_probe_backoff_and_writes_can_recover_it() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let control = control(root.path(), "backoff", 4 * 1024 * 1024);
        let cache = ReadCache::new(cloud.clone(), control.clone());
        let path = ObjectPath::from("apps/project/storage/value");
        cloud.memory.put(&path, "cached".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "cached");
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "cached");
        let reads = cloud.reads.load(Ordering::SeqCst);
        for _ in 0..20 {
            assert_eq!(body(cache.as_ref(), &path).await, "cached");
        }
        assert_eq!(cloud.reads.load(Ordering::SeqCst), reads);
        assert!(
            cache
                .get(&ObjectPath::from("apps/project/storage/missing"))
                .await
                .unwrap_err()
                .to_string()
                .contains("offline")
        );
        assert_eq!(cloud.reads.load(Ordering::SeqCst), reads);
        assert!(!control.try_cloud_read().unwrap());
        control.availability.lock().unwrap().retry_at =
            Some(Instant::now() - Duration::from_secs(1));
        assert!(control.try_cloud_read().unwrap());
        assert!(
            !control.try_cloud_read().unwrap(),
            "only one recovery probe should run"
        );
        for _ in 0..10 {
            control.cloud_failed(&cache_error(CloudUnavailable));
        }
        {
            let availability = control.availability.lock().unwrap();
            assert_eq!(availability.failures, 6);
            assert!(
                availability
                    .retry_at
                    .unwrap()
                    .saturating_duration_since(Instant::now())
                    <= Duration::from_secs(30)
            );
        }
        cloud.mode.store(0, Ordering::SeqCst);
        // Writes always reach the provider even while reads use cached data.
        cache.put(&path, "online".into()).await.unwrap();
        assert_eq!(control.availability.lock().unwrap().failures, 0);
        assert_eq!(body(cache.as_ref(), &path).await, "online");
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "online");
        control.revoke().unwrap();
        assert!(is_denied(&cache.get(&path).await.unwrap_err()));
    }

    #[tokio::test]
    async fn conditional_recovery_response_releases_probe_and_restores_fresh_reads() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "conditional-recovery", 4 * 1024 * 1024),
        );
        let path = ObjectPath::from("apps/project/storage/conditional");
        cloud
            .memory
            .put(&path, "before outage".into())
            .await
            .unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "before outage");
        let etag = cache.head(&path).await.unwrap().e_tag.unwrap();
        cloud.mode.store(1, Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &path).await, "before outage");
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.availability.lock().unwrap().retry_at =
            Some(Instant::now() - Duration::from_secs(1));
        let response = cache
            .get_opts(
                &path,
                GetOptions {
                    if_none_match: Some(etag),
                    ..Default::default()
                },
            )
            .await
            .unwrap_err();
        assert!(matches!(response, object_store::Error::NotModified { .. }));
        assert!(
            cache
                .control
                .availability
                .lock()
                .unwrap()
                .retry_at
                .is_none()
        );
        cloud
            .memory
            .put(&path, "after recovery".into())
            .await
            .unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "after recovery");
    }

    #[test]
    fn eviction_uses_last_access_and_counts_metadata_and_listings() {
        let root = tempfile::tempdir().unwrap();
        let control = control(root.path(), "lru", MIN_CACHE_BYTES);
        let payload = vec![7u8; 20 * 1024];
        for name in ["a", "b", "c"] {
            control
                .transaction(|tx| control.insert(tx, name, "", 2, "", 0, 0, &payload))
                .unwrap();
        }
        assert!(control.read("a").unwrap().is_some());
        control
            .transaction(|tx| control.insert(tx, "d", "", 2, "", 0, 0, &payload))
            .unwrap();
        assert!(control.read("b").unwrap().is_none());
        for name in ["a", "c", "d"] {
            assert!(control.read(name).unwrap().is_some());
        }
    }

    #[test]
    fn delimiter_snapshot_stops_serialization_at_the_entry_budget() {
        let result = ListResult {
            common_prefixes: vec![ObjectPath::from("apps/project/storage/folder")],
            objects: Vec::new(),
        };
        let (prefixes, objects): (Vec<String>, Vec<CachedMeta>) =
            serde_json::from_slice(&delimiter_snapshot(&result).unwrap()).unwrap();
        assert_eq!(prefixes, vec!["apps/project/storage/folder"]);
        assert!(objects.is_empty());
        let mut writer = ListingJson(Vec::new());
        use std::io::Write;
        writer.write_all(&vec![b'x'; LIST_BYTES]).unwrap();
        assert!(writer.write_all(b"overflow").is_err());
        assert_eq!(writer.0.len(), LIST_BYTES);
        let result = ListResult {
            common_prefixes: vec![ObjectPath::from("x".repeat(1024)); LIST_BYTES / 1024 + 1],
            objects: Vec::new(),
        };
        assert!(delimiter_snapshot(&result).is_none());
    }

    #[test]
    fn startup_denial_invalidates_existing_private_placement_scopes() {
        let root = tempfile::tempdir().unwrap();
        let scope = blake3::hash(b"scope").to_hex().to_string();
        let directory = super::super::private_cache(root.path(), "placement", &scope).unwrap();
        let control = control(&directory, &scope, MIN_CACHE_BYTES);
        control
            .transaction(|tx| control.insert(tx, "cached", "", 2, "", 0, 0, b"private"))
            .unwrap();
        CacheControl::revoke_placement(root.path(), "placement").unwrap();
        assert!(control.is_revoked().unwrap());
        assert!(
            control
                .connection
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM cache_entries", [], |row| row
                    .get::<_, u64>(0))
                .unwrap()
                == 0
        );
    }

    #[test]
    fn new_authorized_scope_reclaims_old_scope_budget_and_fences_open_handles() {
        let root = tempfile::tempdir().unwrap();
        let old = blake3::hash(b"old-grant").to_hex().to_string();
        let current = blake3::hash(b"current-grant").to_hex().to_string();
        let old_root = super::super::private_cache(root.path(), "placement", &old).unwrap();
        let old_cache = control(&old_root, &old, MIN_CACHE_BYTES);
        old_cache
            .transaction(|tx| old_cache.insert(tx, "old", "", 2, "", 0, 0, b"private"))
            .unwrap();
        let current_root = super::super::private_cache(root.path(), "placement", &current).unwrap();
        let current_cache = control(&current_root, &current, MIN_CACHE_BYTES);
        let metadata_root =
            super::super::private_cache(root.path(), "placement", "instance-id").unwrap();
        std::fs::write(metadata_root.join("manifest"), b"keep").unwrap();
        CacheControl::retain_placement_scope(root.path(), "placement", &current).unwrap();
        assert!(!old_root.exists());
        assert!(old_cache.is_revoked().unwrap());
        assert!(!current_cache.is_revoked().unwrap());
        assert_eq!(
            std::fs::read(metadata_root.join("manifest")).unwrap(),
            b"keep"
        );
        // A later, newly authorized rollback may recreate the former scope.
        let old_root = super::super::private_cache(root.path(), "placement", &old).unwrap();
        CacheControl::retain_placement_scope(root.path(), "placement", &old).unwrap();
        assert!(
            !control(&old_root, &old, MIN_CACHE_BYTES)
                .is_revoked()
                .unwrap()
        );
        assert!(current_cache.is_revoked().unwrap());
    }

    #[test]
    fn only_verified_lance_fragment_names_skip_revalidation() {
        assert!(immutable_lance_data(&immutable()));
        assert!(!immutable_lance_data(&ObjectPath::from(
            immutable().to_string().replace("storage/db/", "upload/")
        )));
        assert!(immutable_lance_data(&ObjectPath::from(
            immutable()
                .to_string()
                .replace("apps/project/storage/db/", "users/alice/apps/project/db/")
        )));
        for name in [
            "apps/project/storage/table.lance/_versions/1.manifest",
            "apps/project/storage/table.lance/_latest.manifest",
            "apps/project/storage/table.lance/_indices/id/index.idx",
            "apps/project/storage/table.lance/data/custom.lance",
            "apps/project/storage/table.lance/data/00000000-0000-0000-0000-000000000000.lance",
            "apps/project/upload/table.lance",
            "apps/project/storage/file",
        ] {
            assert!(!immutable_lance_data(&ObjectPath::from(name)), "{name}");
        }
    }

    #[test]
    fn only_published_model_files_inside_artifact_folders_skip_revalidation() {
        for name in [
            "apps/project/storage/models/artifacts/model-1/weights.bin",
            "apps/project/storage/models/artifacts/model-1/manifest.json",
            "apps/project/storage/models/artifacts/model-1/preprocessing.json",
            "users/alice/apps/project/models/artifacts/model-1/weights.bin",
            "users/alice/apps/project/models/artifacts/model-1/manifest.json",
            "users/alice/apps/project/models/artifacts/model-1/preprocessing.json",
        ] {
            assert!(immutable_model_artifact(&ObjectPath::from(name)), "{name}");
        }
        for name in [
            "apps/project/storage/models/experiments/experiment.json",
            "apps/project/storage/models/learning/project.json",
            "apps/project/storage/models/deployments/deployment.json",
            "apps/project/storage/models/artifacts/model-1/weights.bin.s3flowEtag",
            "apps/project/storage/models/artifacts/model-1/model.bin",
            "apps/project/storage/models/artifacts/model-1/dependencies/source/weights.bin",
            "apps/project/storage/models/artifacts/weights.bin",
            "apps/project/storage/artifacts/model-1/weights.bin",
            "apps/project/upload/models/artifacts/model-1/weights.bin",
            "apps/project/models/artifacts/model-1/weights.bin",
            "users/alice/apps/project/storage/models/artifacts/model-1/weights.bin",
            "users/alice/models/artifacts/model-1/weights.bin",
        ] {
            assert!(!immutable_model_artifact(&ObjectPath::from(name)), "{name}");
        }
    }

    #[tokio::test]
    async fn trusted_cached_model_files_are_served_without_a_cloud_read() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "models", 4 * 1024 * 1024),
        );
        let weights = ObjectPath::from("apps/project/storage/models/artifacts/model-1/weights.bin");
        let private =
            ObjectPath::from("users/alice/apps/project/models/artifacts/model-1/manifest.json");
        let pointer = ObjectPath::from("apps/project/storage/models/learning/project.json");
        for path in [&weights, &private, &pointer] {
            cloud.memory.put(path, "stored".into()).await.unwrap();
            assert_eq!(body(cache.as_ref(), path).await, "stored");
        }
        let before = cloud.reads.load(Ordering::SeqCst);
        assert_eq!(body(cache.as_ref(), &weights).await, "stored");
        assert_eq!(body(cache.as_ref(), &private).await, "stored");
        assert_eq!(
            cloud.reads.load(Ordering::SeqCst),
            before,
            "verified model artifact files should not hit cloud"
        );
        assert_eq!(body(cache.as_ref(), &pointer).await, "stored");
        assert_eq!(
            cloud.reads.load(Ordering::SeqCst),
            before + 1,
            "pointer files are revalidated"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cache_refuses_symlinks_and_public_database_files() {
        use std::os::unix::{
            fs::{PermissionsExt, symlink},
            prelude::MetadataExt,
        };
        let root = tempfile::tempdir().unwrap();
        let cache = control(root.path(), "private", MIN_CACHE_BYTES);
        let connection = cache.connection.lock().unwrap();
        let filename = connection.path().unwrap().to_string();
        let meta = std::fs::metadata(&filename).unwrap();
        assert_eq!(meta.mode() & 0o077, 0);
        drop(connection);
        drop(cache);
        std::fs::set_permissions(&filename, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(
            CacheControl::new(
                root.path(),
                "private",
                vec!["apps/project/storage/".into()],
                MIN_CACHE_BYTES
            )
            .is_err()
        );
        std::fs::remove_file(&filename).unwrap();
        let other = root.path().join("public");
        std::fs::write(&other, b"untouched").unwrap();
        symlink(&other, &filename).unwrap();
        assert!(
            CacheControl::new(
                root.path(),
                "private",
                vec!["apps/project/storage/".into()],
                MIN_CACHE_BYTES
            )
            .is_err()
        );
        assert_eq!(std::fs::read(other).unwrap(), b"untouched");
    }

    fn age(control: &CacheControl, sql: &str) {
        control
            .connection
            .lock()
            .unwrap()
            .execute(sql, [WRITE_SETTLE_SECONDS + 1])
            .unwrap();
    }

    #[tokio::test]
    async fn rejected_and_settled_writes_restore_immutable_trust() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let cache = ReadCache::new(
            cloud.clone(),
            control(root.path(), "settled-writes", 4 * 1024 * 1024),
        );
        let path = immutable();
        cloud.memory.put(&path, "stored".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "stored");
        cloud.mode.store(4, Ordering::SeqCst);
        assert!(cache.put(&path, "refused".into()).await.is_err());
        cloud.mode.store(0, Ordering::SeqCst);
        assert!(
            cache.control.immutable_trusted(&path).unwrap(),
            "A provider rejection settles its write"
        );
        cloud.mode.store(1, Ordering::SeqCst);
        assert!(cache.put(&path, "unknown".into()).await.is_err());
        cloud.mode.store(0, Ordering::SeqCst);
        cache.control.cloud_succeeded();
        assert!(!cache.control.immutable_trusted(&path).unwrap());
        age(
            &cache.control,
            "UPDATE cache_entries SET start=start-?1 WHERE kind=3",
        );
        assert!(cache.control.immutable_trusted(&path).unwrap());
        assert_eq!(
            cache
                .control
                .transaction(|tx| CacheControl::marker_usage(tx))
                .unwrap(),
            (0, 0)
        );
        assert!(
            cache
                .control
                .read(&format!("meta:{path}"))
                .unwrap()
                .is_none(),
            "Data cached before a settled write is revalidated"
        );
        cloud.memory.put(&path, "late commit".into()).await.unwrap();
        assert_eq!(body(cache.as_ref(), &path).await, "late commit");
    }

    #[test]
    fn untracked_write_uncertainty_resets_after_the_last_write_settles() {
        let root = tempfile::tempdir().unwrap();
        let control = control(root.path(), "uncertain", 4 * MIN_CACHE_BYTES);
        let path = immutable();
        let other = ObjectPath::from(format!(
            "apps/project/storage/db/table.lance/data/{}.lance",
            "1".repeat(24) + "b".repeat(26).as_str()
        ));
        control
            .transaction(|tx| {
                control.insert(
                    tx,
                    &format!("meta:{other}"),
                    other.as_ref(),
                    0,
                    "g",
                    0,
                    0,
                    b"{}",
                )
            })
            .unwrap();
        for _ in 0..=MAX_WRITE_MARKERS {
            control.begin_write(&[&path]).unwrap();
        }
        assert!(!control.immutable_trusted(&other).unwrap());
        age(
            &control,
            "UPDATE cache_write_state SET immutable_uncertain=immutable_uncertain-?1",
        );
        assert!(control.immutable_trusted(&other).unwrap());
        assert!(
            control.read(&format!("meta:{other}")).unwrap().is_none(),
            "Immutable entries cached during uncertainty are revalidated"
        );
        control.begin_write(&[&path]).unwrap();
        assert!(!control.immutable_trusted(&path).unwrap());
        assert!(control.immutable_trusted(&other).unwrap());
    }

    #[test]
    fn steady_tracked_writes_do_not_keep_an_overflowed_scope_uncertain() {
        let root = tempfile::tempdir().unwrap();
        let control = control(root.path(), "steady-writes", 4 * 1024 * 1024);
        let path = immutable();
        let other = ObjectPath::from(format!(
            "apps/project/storage/db/table.lance/data/{}.lance",
            "2".repeat(24) + "c".repeat(26).as_str()
        ));
        for _ in 0..=MAX_WRITE_MARKERS {
            control.begin_write(&[&path]).unwrap();
        }
        assert!(!control.immutable_trusted(&other).unwrap());
        let elapse = |seconds: i64| {
            control
                .connection
                .lock()
                .unwrap()
                .execute_batch(&format!(
                    "UPDATE cache_write_state SET immutable_uncertain=immutable_uncertain-{seconds} WHERE immutable_uncertain<>0;
                     UPDATE cache_entries SET start=start-{seconds} WHERE kind=3;"
                ))
                .unwrap();
        };
        let pending = control.begin_write(&[&other]).unwrap();
        assert!(
            control
                .read(&format!("write:{pending}:{other}"))
                .unwrap()
                .is_some(),
            "Writes stay tracked while the overflow settles"
        );
        control.complete_write(&pending, &[&other]).unwrap();
        for _ in 0..=WRITE_SETTLE_SECONDS / 600 {
            elapse(600);
            let operation = control.begin_write(&[&path]).unwrap();
            control.complete_write(&operation, &[&path]).unwrap();
        }
        assert!(
            control.immutable_trusted(&other).unwrap(),
            "Writing every ten minutes still lets the last untracked write settle"
        );
        assert!(control.immutable_trusted(&path).unwrap());
    }

    #[tokio::test]
    async fn unreadable_cache_state_fails_the_operation_without_revoking() {
        let root = tempfile::tempdir().unwrap();
        let cloud = Arc::new(Cloud::default());
        let control = control(root.path(), "unreadable", 4 * 1024 * 1024);
        let cache = ReadCache::new(cloud.clone(), control.clone());
        let path = ObjectPath::from("apps/project/storage/report.json");
        cloud.memory.put(&path, "value".into()).await.unwrap();
        let rename = |from: &str, to: &str| {
            control
                .connection
                .lock()
                .unwrap()
                .execute_batch(&format!("ALTER TABLE {from} RENAME TO {to}"))
                .unwrap();
        };
        rename("cache_state", "cache_state_moved");
        assert!(control.is_revoked().is_err());
        let error = cache.get(&path).await.unwrap_err();
        assert!(!is_denied(&error) && !is_offline(&error));
        rename("cache_state_moved", "cache_state");
        assert!(!control.is_revoked().unwrap());
        assert_eq!(body(cache.as_ref(), &path).await, "value");
        control.suspend();
        let error = cache.get(&path).await.unwrap_err();
        assert!(!is_denied(&error) && !is_offline(&error));
        assert!(
            !control.is_revoked().unwrap(),
            "Suspension is never durable"
        );
    }

    #[tokio::test]
    async fn file_and_lance_stores_share_one_device_cache() {
        use flow_like_storage::lance_object_store::{self as lance, ObjectStoreExt as _};
        let root = tempfile::tempdir().unwrap();
        let control = control(root.path(), "shared", 4 * 1024 * 1024);
        let files = ReadCache::new(Arc::new(Cloud::default()), control.clone());
        let tables = super::lance_cache::ReadCache::new(
            Arc::new(lance::memory::InMemory::new()),
            control.clone(),
        );
        let path = ObjectPath::from("apps/project/storage/db/shared.lance/_versions/1.manifest");
        let lance_path = lance::path::Path::from(path.as_ref());
        let offline = || {
            control.availability.lock().unwrap().retry_at =
                Some(Instant::now() + Duration::from_secs(60));
        };
        files.put(&path, "from files".into()).await.unwrap();
        assert_eq!(body(files.as_ref(), &path).await, "from files");
        offline();
        assert_eq!(
            tables
                .get(&lance_path)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap(),
            "from files"
        );
        control.cloud_succeeded();
        tables.put(&lance_path, "from tables".into()).await.unwrap();
        offline();
        assert!(
            files
                .get(&path)
                .await
                .unwrap_err()
                .to_string()
                .contains("offline"),
            "A Lance write invalidates what the file store cached"
        );
    }
}
