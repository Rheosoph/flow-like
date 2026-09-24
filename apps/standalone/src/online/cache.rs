use anyhow::{Result, ensure};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use flow_like_storage::object_store::{
    self, Attribute, Attributes, CopyOptions, GetOptions, GetResult, GetResultPayload, ListResult,
    MultipartUpload, ObjectMeta, ObjectStore, ObjectStoreExt, PutMultipartOptions, PutOptions,
    PutPayload, PutResult, RenameOptions, UploadPart, path::Path as ObjectPath,
};
use flow_like_types::async_stream::try_stream;
use flow_like_types_contracts::authorization::AuthorizationError;
use futures_util::{StreamExt, stream::BoxStream};
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
const CLOUD_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

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

/// Preserve server availability as a typed error before the provider hides the
/// HTTP status in its private retry error. Writes retain normal provider behavior.
#[derive(Debug, Clone, Copy)]
pub(super) struct OfflineConnector;
impl object_store::client::HttpConnector for OfflineConnector {
    fn connect(
        &self,
        options: &object_store::ClientOptions,
    ) -> object_store::Result<object_store::client::HttpClient> {
        use object_store::client::{HttpClient, ReqwestConnector};
        Ok(HttpClient::new(OfflineHttpService(
            ReqwestConnector::default().connect(options)?,
        )))
    }
}
#[derive(Debug)]
struct OfflineHttpService(object_store::client::HttpClient);
#[async_trait]
impl object_store::client::HttpService for OfflineHttpService {
    async fn call(
        &self,
        request: object_store::client::HttpRequest,
    ) -> std::result::Result<object_store::client::HttpResponse, object_store::client::HttpError>
    {
        let read = matches!(request.method().as_str(), "GET" | "HEAD");
        let response = self.0.execute(request).await?;
        if read && response.status().is_server_error() {
            return Err(object_store::client::HttpError::new(
                object_store::client::HttpErrorKind::Request,
                CloudUnavailable,
            ));
        }
        Ok(response)
    }
}

fn cache_error(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> object_store::Error {
    object_store::Error::Generic {
        store: "StandaloneReadCache",
        source: error.into(),
    }
}
fn denied() -> object_store::Error {
    cache_error(AuthorizationError::Denied)
}
fn offline() -> object_store::Error {
    cache_error(OfflineCacheMiss)
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
fn delimiter_snapshot(result: &ListResult) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut json = ListingJson(Vec::new());
    json.write_all(b"[[").ok()?;
    for (index, prefix) in result.common_prefixes.iter().enumerate() {
        if index != 0 {
            json.write_all(b",").ok()?;
        }
        serde_json::to_writer(&mut json, prefix.as_ref()).ok()?;
    }
    json.write_all(b"],[").ok()?;
    for (index, meta) in result.objects.iter().enumerate() {
        if index != 0 {
            json.write_all(b",").ok()?;
        }
        // Bound each clone before constructing its cache representation.
        if meta.location.as_ref().len() > LIST_BYTES
            || meta
                .e_tag
                .as_ref()
                .is_some_and(|value| value.len() > LIST_BYTES)
            || meta
                .version
                .as_ref()
                .is_some_and(|value| value.len() > LIST_BYTES)
        {
            return None;
        }
        serde_json::to_writer(&mut json, &CachedMeta::new(meta, &Attributes::default())).ok()?;
    }
    json.write_all(b"]]").ok()?;
    Some(json.0)
}
fn is_denied(error: &object_store::Error) -> bool {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = source {
        if error.downcast_ref::<AuthorizationError>() == Some(&AuthorizationError::Denied) {
            return true;
        }
        source = error.source();
    }
    false
}
pub(super) fn is_offline(error: &object_store::Error) -> bool {
    // A 404, failed condition, invalid credential, or access denial must never
    // turn into a successful stale read.
    if !matches!(error, object_store::Error::Generic { .. }) {
        return false;
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = source {
        if error.is::<CloudUnavailable>() || error.is::<OfflineCacheMiss>() {
            return true;
        }
        if let Some(error) = error.downcast_ref::<AuthorizationError>() {
            return matches!(
                error,
                AuthorizationError::Unavailable | AuthorizationError::Expired
            );
        }
        if let Some(error) = error.downcast_ref::<object_store::client::HttpError>() {
            use object_store::client::HttpErrorKind;
            if matches!(
                error.kind(),
                HttpErrorKind::Connect | HttpErrorKind::Timeout | HttpErrorKind::Interrupted
            ) {
                return true;
            }
        }
        if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            return error.is_connect()
                || error.is_timeout()
                || error
                    .status()
                    .is_some_and(|status| status.is_server_error());
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            return matches!(
                error.kind(),
                std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::NetworkUnreachable
                    | std::io::ErrorKind::HostUnreachable
            );
        }
        source = error.source();
    }
    false
}

#[derive(Debug, Serialize, Deserialize)]
struct CachedMeta {
    path: String,
    modified: String,
    size: u64,
    etag: Option<String>,
    version: Option<String>,
    attributes: Vec<(String, String)>,
}
impl CachedMeta {
    fn new(meta: &ObjectMeta, attributes: &Attributes) -> Self {
        Self {
            path: meta.location.to_string(),
            modified: meta.last_modified.to_rfc3339(),
            size: meta.size,
            etag: meta.e_tag.clone(),
            version: meta.version.clone(),
            attributes: attributes
                .iter()
                .filter_map(|(key, value)| {
                    let key = match key {
                        Attribute::ContentDisposition => "disposition".into(),
                        Attribute::ContentEncoding => "encoding".into(),
                        Attribute::ContentLanguage => "language".into(),
                        Attribute::ContentType => "type".into(),
                        Attribute::CacheControl => "control".into(),
                        Attribute::StorageClass => "class".into(),
                        Attribute::Metadata(name) => format!("metadata:{name}"),
                        _ => return None,
                    };
                    Some((key, value.as_ref().to_owned()))
                })
                .collect(),
        }
    }
    fn meta(&self) -> object_store::Result<ObjectMeta> {
        Ok(ObjectMeta {
            location: ObjectPath::parse(&self.path)?,
            last_modified: self.modified.parse().map_err(cache_error)?,
            size: self.size,
            e_tag: self.etag.clone(),
            version: self.version.clone(),
        })
    }
    fn attributes(&self) -> Attributes {
        self.attributes
            .iter()
            .filter_map(|(key, value)| {
                let key = match key.as_str() {
                    "disposition" => Attribute::ContentDisposition,
                    "encoding" => Attribute::ContentEncoding,
                    "language" => Attribute::ContentLanguage,
                    "type" => Attribute::ContentType,
                    "control" => Attribute::CacheControl,
                    "class" => Attribute::StorageClass,
                    key => Attribute::Metadata(key.strip_prefix("metadata:")?.to_owned().into()),
                };
                Some((key, object_store::AttributeValue::from(value.clone())))
            })
            .collect()
    }
    fn generation(&self) -> object_store::Result<String> {
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
                    && ObjectPath::parse(prefix).is_ok_and(|path| format!("{path}/") == *prefix)),
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
        let control = Arc::new(Self {
            connection: Mutex::new(connection),
            revoked: AtomicBool::new(revoked),
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
    pub(super) fn is_revoked(&self) -> bool {
        if self.revoked.load(Ordering::Acquire) {
            return true;
        }
        let Ok(connection) = self.connection.lock() else {
            return true;
        };
        let revoked = connection
            .query_row(
                "SELECT revoked FROM cache_state WHERE singleton=1",
                [],
                |row| row.get::<_, bool>(0),
            )
            .unwrap_or(true);
        if revoked {
            self.revoked.store(true, Ordering::Release);
        }
        revoked
    }
    fn try_cloud_read(&self) -> object_store::Result<bool> {
        if self.is_revoked() {
            return Err(denied());
        }
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
    fn cloud_failed(&self, error: &object_store::Error) {
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
    fn scope(&self, path: &ObjectPath, directory: bool) -> object_store::Result<()> {
        if self.is_revoked() {
            return Err(denied());
        }
        if !self.prefixes.iter().any(|prefix| {
            path.as_ref().starts_with(prefix)
                || (directory && path.as_ref() == prefix.trim_end_matches('/'))
        }) {
            return Err(object_store::Error::PermissionDenied {
                path: path.to_string(),
                source: "Object is outside this device cache's project and user scope".into(),
            });
        }
        Ok(())
    }
    fn transaction<T>(
        &self,
        operation: impl FnOnce(&Transaction<'_>) -> object_store::Result<T>,
    ) -> object_store::Result<T> {
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
    fn epoch(&self) -> object_store::Result<i64> {
        self.transaction(|tx| {
            tx.query_row(
                "SELECT epoch FROM cache_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(cache_error)
        })
    }
    fn tick(tx: &Transaction<'_>) -> object_store::Result<i64> {
        tx.execute("UPDATE cache_state SET clock=clock+1 WHERE singleton=1", [])
            .map_err(cache_error)?;
        tx.query_row(
            "SELECT clock FROM cache_state WHERE singleton=1",
            [],
            |row| row.get(0),
        )
        .map_err(cache_error)
    }
    fn read(&self, id: &str) -> object_store::Result<Option<Vec<u8>>> {
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
    ) -> object_store::Result<()> {
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
    fn marker_usage(tx: &Transaction<'_>) -> object_store::Result<(u64, u64)> {
        tx.query_row(
            "SELECT COUNT(*),COALESCE(SUM(cost),0) FROM cache_entries WHERE kind=3",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(cache_error)
    }
    fn reserve_marker(&self, tx: &Transaction<'_>, cost: u64) -> object_store::Result<bool> {
        let uncertain: bool = tx
            .query_row(
                "SELECT immutable_uncertain FROM cache_write_state WHERE singleton=1",
                [],
                |row| row.get(0),
            )
            .map_err(cache_error)?;
        let (count, bytes) = Self::marker_usage(tx)?;
        if uncertain
            || count + u64::from(cost > 0) > MAX_WRITE_MARKERS
            || bytes.saturating_add(cost) > self.marker_budget()
        {
            // A bounded flag replaces all unresolved operation history. The
            // reserved control slice keeps bookkeeping out of the data LRU.
            tx.execute(
                "UPDATE cache_write_state SET immutable_uncertain=1 WHERE singleton=1",
                [],
            )
            .map_err(cache_error)?;
            tx.execute("DELETE FROM cache_entries WHERE kind=3", [])
                .map_err(cache_error)?;
            return Ok(false);
        }
        Ok(true)
    }
    fn evict(&self, tx: &Transaction<'_>, cost: u64) -> object_store::Result<()> {
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
    fn observe(&self, meta: &CachedMeta, epoch: i64) -> object_store::Result<Option<String>> {
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
    ) -> object_store::Result<()> {
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
    ) -> object_store::Result<Option<Bytes>> {
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
    fn covers(
        &self,
        path: &str,
        generation: &str,
        range: &Range<u64>,
    ) -> object_store::Result<bool> {
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
    pub(super) fn invalidate(&self, paths: &[&ObjectPath]) -> object_store::Result<()> {
        self.transaction(|tx| Self::invalidate_data(tx, paths))
    }
    /// Retain an acknowledged logical file write before its outbox payload is
    /// released. The normal cache budget and authorization fence still apply.
    pub(super) fn remember_file(
        &self,
        meta: &ObjectMeta,
        bytes: &[u8],
    ) -> object_store::Result<()> {
        self.scope(&meta.location, false)?;
        if bytes.len() as u64 != meta.size {
            return Err(cache_error(
                "Acknowledged file size differs from its payload",
            ));
        }
        let stored = CachedMeta::new(meta, &Attributes::default());
        let generation = stored.generation()?;
        let metadata = serde_json::to_vec(&stored).map_err(cache_error)?;
        self.transaction(|tx| {
            self.acknowledge_file(tx, &meta.location, Some(meta))?;
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
    pub(super) fn remember_file_deleted(&self, path: &ObjectPath) -> object_store::Result<()> {
        self.scope(path, false)?;
        self.transaction(|tx| self.acknowledge_file(tx, path, None))
    }
    fn acknowledge_file(
        &self,
        tx: &Transaction<'_>,
        path: &ObjectPath,
        meta: Option<&ObjectMeta>,
    ) -> object_store::Result<()> {
        tx.execute("UPDATE cache_state SET epoch=epoch+1 WHERE singleton=1", [])
            .map_err(cache_error)?;
        tx.execute(
            "DELETE FROM cache_entries WHERE path=?1 AND kind IN (0,1)",
            [path.as_ref()],
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
            let prefix = ObjectPath::parse(prefix)?;
            if !path.prefix_matches(&prefix) {
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
            entries.retain(|entry| entry.path != path.as_ref());
            if let Some(meta) = meta {
                entries.push(CachedMeta::new(meta, &Attributes::default()));
            }
            entries.sort_by(|left, right| left.path.cmp(&right.path));
            let mut encoded = ListingJson(Vec::new());
            if serde_json::to_writer(&mut encoded, &entries).is_ok() {
                self.insert(tx, &id, "", 2, "", 0, 0, &encoded.0)?;
            }
        }
        Ok(())
    }
    fn invalidate_data(tx: &Transaction<'_>, paths: &[&ObjectPath]) -> object_store::Result<()> {
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
    fn begin_write(&self, paths: &[&ObjectPath]) -> object_store::Result<String> {
        let operation = uuid::Uuid::new_v4().to_string();
        self.transaction(|tx| {
            tx.execute("UPDATE cache_state SET epoch=epoch+1 WHERE singleton=1", [])
                .map_err(cache_error)?;
            for path in paths {
                self.insert(
                    tx,
                    &format!("write:{operation}:{path}"),
                    path.as_ref(),
                    3,
                    &operation,
                    0,
                    0,
                    &[],
                )?;
            }
            Ok(operation)
        })
    }
    fn complete_write(&self, operation: &str, paths: &[&ObjectPath]) -> object_store::Result<()> {
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
    fn immutable_trusted(&self, path: &ObjectPath) -> object_store::Result<bool> {
        self.transaction(|tx| {
            let uncertain: bool = tx
                .query_row(
                    "SELECT immutable_uncertain FROM cache_write_state WHERE singleton=1",
                    [],
                    |row| row.get(0),
                )
                .map_err(cache_error)?;
            if uncertain {
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
    fn listing(&self, key: &str, data: &[u8], epoch: i64) -> object_store::Result<()> {
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
    fn handle_error(&self, error: &object_store::Error) {
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

// Lance 8.0.0 @ 82a122e: dataset/write.rs::open_writer_with_options
// creates fresh data files using fragment/write.rs::generate_random_filename.
// Manifests, indexes, blobs, and ordinary uploads remain revalidated.
fn immutable_lance_data(path: &ObjectPath) -> bool {
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

#[derive(Debug, Clone)]
pub(super) struct ReadCache {
    inner: Arc<dyn ObjectStore>,
    control: Arc<CacheControl>,
}
impl fmt::Display for ReadCache {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("StandaloneCloudReadCache")
    }
}
impl ReadCache {
    pub(super) fn new(inner: Arc<dyn ObjectStore>, control: Arc<CacheControl>) -> Arc<Self> {
        Arc::new(Self { inner, control })
    }
    fn cached(
        &self,
        path: &ObjectPath,
        options: &GetOptions,
    ) -> object_store::Result<Option<GetResult>> {
        let Some(data) = self.control.read(&format!("meta:{path}"))? else {
            return Ok(None);
        };
        let stored: CachedMeta = serde_json::from_slice(&data).map_err(cache_error)?;
        let meta = stored.meta()?;
        if options
            .version
            .as_ref()
            .is_some_and(|version| meta.version.as_ref() != Some(version))
        {
            return Ok(None);
        }
        options.check_preconditions(&meta)?;
        let range = if options.head {
            0..0
        } else {
            options
                .range
                .as_ref()
                .map(|range| range.as_range(meta.size))
                .transpose()
                .map_err(cache_error)?
                .unwrap_or(0..meta.size)
        };
        let generation = stored.generation()?;
        if !self.control.covers(path.as_ref(), &generation, &range)? {
            return Ok(None);
        }
        let control = self.control.clone();
        let path = path.to_string();
        let mut position = range.start;
        let end = range.end;
        let stream = try_stream! {
            while position < end {
                let bytes = control.read_block(&path,&generation,position,end)?.ok_or_else(offline)?;
                position += bytes.len() as u64;
                yield bytes;
            }
        };
        Ok(Some(GetResult {
            payload: GetResultPayload::Stream(Box::pin(stream)),
            meta,
            range,
            attributes: stored.attributes(),
        }))
    }
    fn wrap_result(
        &self,
        result: GetResult,
        epoch: i64,
        head: bool,
        cache_current: bool,
    ) -> object_store::Result<GetResult> {
        let meta = result.meta.clone();
        self.control.scope(&meta.location, false)?;
        let attributes = result.attributes.clone();
        let range = result.range.clone();
        // An explicit historical version must not replace the current object
        // that an unversioned offline read will use.
        let generation = if cache_current {
            self.control
                .observe(&CachedMeta::new(&meta, &attributes), epoch)?
        } else {
            None
        };
        let control = self.control.clone();
        let path = meta.location.to_string();
        let mut position = range.start;
        let end = range.end;
        let mut source = result.into_stream();
        let stream = try_stream! {
            let mut pending = BytesMut::new();
            let mut block_start = position;
            while let Some(result) = source.next().await {
                let mut bytes = match result {
                    Ok(bytes) => bytes,
                    Err(error) => { control.handle_error(&error); Err(error)?; unreachable!() }
                };
                if control.is_revoked() { Err(denied())?; }
                if !head && bytes.len() as u64 > end.saturating_sub(position) { Err(cache_error("Cloud object returned an invalid byte range"))?; }
                while !bytes.is_empty() {
                    let chunk = bytes.split_to(bytes.len().min(BLOCK_BYTES-pending.len()));
                    if !head && generation.is_some() {
                        pending.extend_from_slice(&chunk);
                        if pending.len() == BLOCK_BYTES {
                            control.block(&path,generation.as_deref().unwrap(),block_start,&pending,epoch)?;
                            pending.clear();
                            block_start = position + chunk.len() as u64;
                        }
                    }
                    position += chunk.len() as u64;
                    yield chunk;
                }
            }
            if !head && position != end { Err(cache_error("Cloud object ended before its declared range"))?; }
            if !pending.is_empty() { control.block(&path,generation.as_deref().unwrap(),block_start,&pending,epoch)?; }
            control.cloud_succeeded();
        };
        Ok(GetResult {
            payload: GetResultPayload::Stream(Box::pin(stream)),
            meta,
            range,
            attributes,
        })
    }
    fn list_key(
        &self,
        prefix: Option<&ObjectPath>,
        delimiter: bool,
    ) -> object_store::Result<String> {
        let prefix = prefix.ok_or_else(|| {
            cache_error("Device listing requires an authorized project or user directory")
        })?;
        self.control.scope(prefix, true)?;
        Ok(format!("list:{}:{prefix}", u8::from(delimiter)))
    }
    fn cached_delimiter(&self, key: &str) -> object_store::Result<ListResult> {
        let bytes = self.control.read(key)?.ok_or_else(offline)?;
        let (prefixes, objects): (Vec<String>, Vec<CachedMeta>) =
            serde_json::from_slice(&bytes).map_err(cache_error)?;
        let result = ListResult {
            common_prefixes: prefixes
                .iter()
                .map(ObjectPath::parse)
                .collect::<std::result::Result<_, _>>()?,
            objects: objects
                .iter()
                .map(CachedMeta::meta)
                .collect::<object_store::Result<_>>()?,
        };
        for prefix in &result.common_prefixes {
            self.control.scope(prefix, true)?;
        }
        for meta in &result.objects {
            self.control.scope(&meta.location, false)?;
        }
        Ok(result)
    }
}

#[async_trait]
impl ObjectStore for ReadCache {
    async fn get_opts(
        &self,
        path: &ObjectPath,
        options: GetOptions,
    ) -> object_store::Result<GetResult> {
        self.control.scope(path, false)?;
        if immutable_lance_data(path) && self.control.immutable_trusted(path)? {
            if let Some(result) = self.cached(path, &options)? {
                return Ok(result);
            }
        }
        if !self.control.try_cloud_read()? {
            return self.cached(path, &options)?.ok_or_else(offline);
        }
        let epoch = self.control.epoch()?;
        let result = tokio::time::timeout(
            CLOUD_PROBE_TIMEOUT,
            self.inner.get_opts(path, options.clone()),
        )
        .await
        .unwrap_or_else(|_| Err(cache_error(CloudUnavailable)));
        match result {
            Ok(result) => {
                if result.meta.location != *path {
                    return Err(cache_error(
                        "Cloud object response changed its requested path",
                    ));
                }
                if options
                    .version
                    .as_ref()
                    .is_some_and(|version| result.meta.version.as_ref() != Some(version))
                {
                    return Err(cache_error(
                        "Cloud object response changed its requested version",
                    ));
                }
                self.control.cloud_succeeded();
                self.wrap_result(result, epoch, options.head, options.version.is_none())
            }
            Err(error) => {
                self.control.handle_error(&error);
                if is_offline(&error) {
                    return self.cached(path, &options)?.ok_or_else(offline);
                }
                if options.version.is_none()
                    && matches!(
                        error,
                        object_store::Error::NotFound { .. }
                            | object_store::Error::PermissionDenied { .. }
                            | object_store::Error::Unauthenticated { .. }
                    )
                {
                    self.control.invalidate(&[path])?;
                }
                Err(error)
            }
        }
    }
    async fn put_opts(
        &self,
        path: &ObjectPath,
        payload: PutPayload,
        options: PutOptions,
    ) -> object_store::Result<PutResult> {
        self.control.scope(path, false)?;
        let operation = self.control.begin_write(&[path])?;
        let result = self.inner.put_opts(path, payload, options).await;
        if let Err(error) = &result {
            self.control.handle_error(error);
        } else {
            self.control.cloud_succeeded();
            self.control.complete_write(&operation, &[path])?;
        }
        result
    }
    async fn put_multipart_opts(
        &self,
        path: &ObjectPath,
        options: PutMultipartOptions,
    ) -> object_store::Result<Box<dyn MultipartUpload>> {
        self.control.scope(path, false)?;
        let upload = self
            .inner
            .put_multipart_opts(path, options)
            .await
            .map_err(|error| {
                self.control.handle_error(&error);
                error
            })?;
        self.control.cloud_succeeded();
        Ok(Box::new(CachedUpload {
            inner: upload,
            control: self.control.clone(),
            path: path.clone(),
        }))
    }
    fn delete_stream(
        &self,
        locations: BoxStream<'static, object_store::Result<ObjectPath>>,
    ) -> BoxStream<'static, object_store::Result<ObjectPath>> {
        let store = self.clone();
        Box::pin(locations.then(move |path| {
            let store = store.clone();
            async move {
                let path = path?;
                store.control.scope(&path, false)?;
                let operation = store.control.begin_write(&[&path])?;
                let result = store.inner.delete(&path).await;
                if let Err(error) = &result {
                    store.control.handle_error(error);
                } else {
                    store.control.cloud_succeeded();
                    store.control.complete_write(&operation, &[&path])?;
                }
                result?;
                Ok(path)
            }
        }))
    }
    fn list(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
        let store = self.clone();
        let prefix = prefix.cloned();
        Box::pin(try_stream! {
            let key=store.list_key(prefix.as_ref(),false)?;
            if !store.control.try_cloud_read()? {
                let bytes=store.control.read(&key)?.ok_or_else(offline)?;
                let stored: Vec<CachedMeta>=serde_json::from_slice(&bytes).map_err(cache_error)?;
                for meta in stored { store.control.scope(&ObjectPath::parse(&meta.path)?,false)?; yield meta.meta()?; }
                return;
            }
            let epoch=store.control.epoch()?;
            let mut source=store.inner.list(prefix.as_ref());
            let mut complete=true;
            let mut seen=false;
            let mut data=Vec::<CachedMeta>::new();
            let mut size=0;
            while let Some(result)=tokio::time::timeout(CLOUD_PROBE_TIMEOUT,source.next()).await.unwrap_or_else(|_|Some(Err(cache_error(CloudUnavailable)))) {
                let meta=match result {
                    Ok(meta)=>meta,
                    Err(error)=>{
                        store.control.handle_error(&error);
                        if is_offline(&error) && !seen {
                            let bytes=store.control.read(&key)?.ok_or_else(offline)?;
                            let stored: Vec<CachedMeta>=serde_json::from_slice(&bytes).map_err(cache_error)?;
                            for meta in stored { store.control.scope(&ObjectPath::parse(&meta.path)?,false)?; yield meta.meta()?; }
                            return;
                        }
                        if !is_offline(&error) { store.control.invalidate(&[])?; }
                        Err(error)?;
                        unreachable!()
                    }
                };
                store.control.scope(&meta.location,false)?;
                seen=true;
                if complete {
                    let stored=CachedMeta::new(&meta,&Attributes::default());
                    size+=serde_json::to_vec(&stored).map_err(cache_error)?.len()+1;
                    if size <= LIST_BYTES { data.push(stored); } else { complete=false; data.clear(); }
                }
                yield meta;
            }
            if complete { store.control.listing(&key,&serde_json::to_vec(&data).map_err(cache_error)?,epoch)?; }
            store.control.cloud_succeeded();
        })
    }
    async fn list_with_delimiter(
        &self,
        prefix: Option<&ObjectPath>,
    ) -> object_store::Result<ListResult> {
        let key = self.list_key(prefix, true)?;
        if !self.control.try_cloud_read()? {
            return self.cached_delimiter(&key);
        }
        let epoch = self.control.epoch()?;
        let result =
            tokio::time::timeout(CLOUD_PROBE_TIMEOUT, self.inner.list_with_delimiter(prefix))
                .await
                .unwrap_or_else(|_| Err(cache_error(CloudUnavailable)));
        match result {
            Ok(result) => {
                for meta in &result.objects {
                    self.control.scope(&meta.location, false)?;
                }
                for prefix in &result.common_prefixes {
                    self.control.scope(prefix, true)?;
                }
                if let Some(snapshot) = delimiter_snapshot(&result) {
                    self.control.listing(&key, &snapshot, epoch)?;
                }
                self.control.cloud_succeeded();
                Ok(result)
            }
            Err(error) => {
                self.control.handle_error(&error);
                if !is_offline(&error) {
                    self.control.invalidate(&[])?;
                    return Err(error);
                }
                self.cached_delimiter(&key)
            }
        }
    }
    async fn copy_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: CopyOptions,
    ) -> object_store::Result<()> {
        self.control.scope(from, false)?;
        self.control.scope(to, false)?;
        let operation = self.control.begin_write(&[to])?;
        let result = self.inner.copy_opts(from, to, options).await;
        if let Err(error) = &result {
            self.control.handle_error(error);
        } else {
            self.control.cloud_succeeded();
            self.control.complete_write(&operation, &[to])?;
        }
        result
    }
    async fn rename_opts(
        &self,
        from: &ObjectPath,
        to: &ObjectPath,
        options: RenameOptions,
    ) -> object_store::Result<()> {
        self.control.scope(from, false)?;
        self.control.scope(to, false)?;
        let operation = self.control.begin_write(&[from, to])?;
        let result = self.inner.rename_opts(from, to, options).await;
        if let Err(error) = &result {
            self.control.handle_error(error);
        } else {
            self.control.cloud_succeeded();
            self.control.complete_write(&operation, &[from, to])?;
        }
        result
    }
}
#[derive(Debug)]
struct CachedUpload {
    inner: Box<dyn MultipartUpload>,
    control: Arc<CacheControl>,
    path: ObjectPath,
}
#[async_trait]
impl MultipartUpload for CachedUpload {
    fn put_part(&mut self, data: PutPayload) -> UploadPart {
        if self.control.is_revoked() {
            return Box::pin(async { Err(denied()) });
        }
        let result = self.inner.put_part(data);
        let control = self.control.clone();
        Box::pin(async move {
            let result = result.await;
            if let Err(error) = &result {
                control.handle_error(error);
            } else {
                control.cloud_succeeded();
            }
            result
        })
    }
    async fn complete(&mut self) -> object_store::Result<PutResult> {
        self.control.scope(&self.path, false)?;
        let operation = self.control.begin_write(&[&self.path])?;
        let result = self.inner.complete().await;
        if let Err(error) = &result {
            self.control.handle_error(error);
        } else {
            self.control.cloud_succeeded();
            self.control.complete_write(&operation, &[&self.path])?;
        }
        result
    }
    async fn abort(&mut self) -> object_store::Result<()> {
        self.inner.abort().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::TryStreamExt;
    use std::sync::atomic::{AtomicU8, AtomicUsize};

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
            "0".repeat(24) + &"a".repeat(26)
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
        assert!(control.is_revoked());
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
        assert!(other_process.is_revoked());
        assert!(is_denied(&pending.next().await.unwrap().unwrap_err()));
        let reloaded = control(root.path(), "device:grant:project:alice", 4 * 1024 * 1024);
        assert!(reloaded.is_revoked());
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
        assert_eq!(
            control
                .transaction(|tx| CacheControl::marker_usage(tx))
                .unwrap(),
            (0, 0)
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
        let availability = control.availability.lock().unwrap();
        assert_eq!(availability.failures, 6);
        assert!(
            availability
                .retry_at
                .unwrap()
                .saturating_duration_since(Instant::now())
                <= Duration::from_secs(30)
        );
        drop(availability);
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
        assert!(control.is_revoked());
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
        assert!(old_cache.is_revoked());
        assert!(!current_cache.is_revoked());
        assert_eq!(
            std::fs::read(metadata_root.join("manifest")).unwrap(),
            b"keep"
        );
        // A later, newly authorized rollback may recreate the former scope.
        let old_root = super::super::private_cache(root.path(), "placement", &old).unwrap();
        CacheControl::retain_placement_scope(root.path(), "placement", &old).unwrap();
        assert!(!control(&old_root, &old, MIN_CACHE_BYTES).is_revoked());
        assert!(current_cache.is_revoked());
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
}
