use crate::fs::{private_directory, relative_path, validate_namespace};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::StoragePurpose;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{Mutex, MutexGuard, RwLock},
};

const MAX_OPERATION_BYTES: usize = flow_like_device_protocol::MAX_OFFLINE_REPLAY_HTTP_BYTES;
const RECORD_OVERHEAD: u64 = 4096;
const NON_TERMINAL: &str = "state NOT IN ('applied','skipped','superseded')";
pub(crate) const CLOSED: &str = "Offline changes for this project were removed from this device";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct BufferingConfig {
    pub tables: Vec<BufferedTable>,
    pub files: Vec<BufferedFiles>,
    pub max_queue_bytes: u64,
    pub max_operations: u32,
    pub max_age_seconds: u64,
    pub max_mirror_bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BufferedTable {
    pub purpose: StoragePurpose,
    /// Relative to the authorized project or user storage prefix.
    pub database: String,
    pub table: String,
    pub primary_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BufferedFiles {
    pub purpose: StoragePurpose,
    pub prefix: String,
}

impl Default for BufferingConfig {
    fn default() -> Self {
        Self {
            tables: Vec::new(),
            files: Vec::new(),
            max_queue_bytes: 256 * 1024 * 1024,
            max_operations: 10_000,
            max_age_seconds: 7 * 24 * 3600,
            max_mirror_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

impl BufferingConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.tables.is_empty() || !self.files.is_empty(),
            "Select tables or file directories for offline writes"
        );
        ensure!(
            self.tables.len() + self.files.len() <= 64,
            "At most 64 buffered resources are allowed"
        );
        self.validate_limits()?;
        let mut seen = std::collections::HashSet::new();
        for table in &self.tables {
            table.validate()?;
            ensure!(
                seen.insert((table.purpose, &table.database, &table.table)),
                "Duplicate offline table"
            );
        }
        for file in &self.files {
            ensure!(
                relative_path(&file.prefix, false)
                    && !file.prefix.split('/').any(|part| part.ends_with(".lance")),
                "Buffered files require an explicit directory outside Lance tables"
            );
            ensure!(
                !matches!(file.purpose, StoragePurpose::Storage | StoragePurpose::User)
                    || file.prefix.split('/').next() != Some("db"),
                "Lance database directories cannot enter the file outbox"
            );
        }
        Ok(())
    }

    /// Range checks of the queue and mirror limits.
    pub fn validate_limits(&self) -> Result<()> {
        ensure!(
            (1024 * 1024..=64 * 1024 * 1024 * 1024).contains(&self.max_queue_bytes),
            "Offline queue budget must be between 1 MiB and 64 GiB"
        );
        ensure!(
            (1..=100_000).contains(&self.max_operations),
            "Offline queue operation limit must be between 1 and 100000"
        );
        ensure!(
            (60..=30 * 24 * 3600).contains(&self.max_age_seconds),
            "Offline queue age limit must be between one minute and 30 days"
        );
        ensure!(
            (1024 * 1024..=1024 * 1024 * 1024 * 1024).contains(&self.max_mirror_bytes),
            "Offline table mirror budget must be between 1 MiB and 1 TiB"
        );
        Ok(())
    }
}

impl BufferedTable {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            matches!(self.purpose, StoragePurpose::Storage | StoragePurpose::User),
            "Only project and user databases support offline tables"
        );
        ensure!(
            self.database == "db" && relative_path(&self.table, false) && !self.table.contains('/'),
            "Offline tables must use the project or user db directory"
        );
        flow_like_device_protocol::validate_instance_identifier(&self.table)?;
        ensure!(
            !self.table.contains('.'),
            "Buffered table identifiers cannot contain dots"
        );
        ensure!(
            !self.primary_key.is_empty()
                && self.primary_key.len() <= 128
                && self
                    .primary_key
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "Buffered tables require a primary-key column"
        );
        Ok(())
    }
}

/// How lane heads are chosen. `Global` is one FIFO per scope (standalone); `PerResource`
/// keeps one FIFO per table or file path (desktop).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum QueueLanes {
    #[default]
    Global,
    PerResource,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OutboxOptions {
    pub lanes: QueueLanes,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct QueuedOperation {
    pub sequence: u64,
    pub operation_id: String,
    pub resource: String,
    pub payload: Value,
    pub state: String,
    pub attempts: u32,
    pub created_at: i64,
    pub error: Option<String>,
    pub local_version: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutboxStatus {
    pub scope: String,
    pub quarantined: bool,
    pub pending_count: u64,
    pub pending_bytes: u64,
    pub oldest_at: Option<i64>,
    pub head: Option<QueuedOperation>,
    pub mirror_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationSummary {
    pub sequence: u64,
    pub operation_id: String,
    /// JSON of `OfflineResource`.
    pub resource: String,
    /// `mutation.kind` of the payload, e.g. "table_upsert".
    pub mutation_kind: Option<String>,
    pub state: String,
    pub attempts: u32,
    pub created_at: i64,
    pub bytes: u64,
    pub error: Option<String>,
    pub error_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OperationLookup {
    /// Any state, including the applied, skipped and superseded tombstones.
    pub state: String,
    pub superseded_by: Option<String>,
    pub error: Option<String>,
    pub error_code: Option<String>,
}

pub struct Outbox {
    db: Mutex<Option<Connection>>,
    root: PathBuf,
    scope: String,
    limits: RwLock<BufferingConfig>,
    lanes: QueueLanes,
}

/// Read-only view of a scope's queue that never creates or writes files.
pub struct OutboxReader {
    db: Connection,
}

struct Db<'a>(MutexGuard<'a, Option<Connection>>);
impl std::ops::Deref for Db<'_> {
    type Target = Connection;
    fn deref(&self) -> &Connection {
        self.0.as_ref().expect("an open outbox connection")
    }
}
impl std::ops::DerefMut for Db<'_> {
    fn deref_mut(&mut self) -> &mut Connection {
        self.0.as_mut().expect("an open outbox connection")
    }
}

fn operation(row: &rusqlite::Row<'_>) -> rusqlite::Result<QueuedOperation> {
    let payload: Vec<u8> = row.get(3)?;
    Ok(QueuedOperation {
        sequence: row.get(0)?,
        operation_id: row.get(1)?,
        resource: row.get(2)?,
        payload: serde_json::from_slice(&payload).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Blob, Box::new(e))
        })?,
        state: row.get(4)?,
        attempts: row.get(5)?,
        created_at: row.get(6)?,
        error: row.get(7)?,
        local_version: row.get(8)?,
    })
}
const SELECT_OPERATION: &str = "SELECT sequence,operation_id,resource,payload,state,attempts,created_at,error,local_version FROM operations";

fn summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<OperationSummary> {
    Ok(OperationSummary {
        sequence: row.get(0)?,
        operation_id: row.get(1)?,
        resource: row.get(2)?,
        mutation_kind: row.get(3)?,
        state: row.get(4)?,
        attempts: row.get(5)?,
        created_at: row.get(6)?,
        bytes: row.get(7)?,
        error: row.get(8)?,
        error_code: row.get(9)?,
    })
}
const SELECT_SUMMARY: &str = "SELECT sequence,operation_id,resource,json_extract(CAST(payload AS TEXT),'$.mutation.kind'),state,attempts,created_at,bytes,error,error_code FROM operations";
const LANE_HEADS: &str = "sequence IN (SELECT MIN(sequence) FROM operations WHERE state NOT IN ('applied','skipped','superseded') GROUP BY resource)";
const GLOBAL_HEAD: &str = "sequence=(SELECT MIN(sequence) FROM operations WHERE state NOT IN ('applied','skipped','superseded'))";

fn status_of(db: &Connection, scope: String) -> Result<OutboxStatus> {
    let (pending_count,pending_bytes,oldest_at) = db.query_row("SELECT COUNT(*),COALESCE(SUM(bytes),0),MIN(first_queued_at) FROM operations WHERE state NOT IN ('applied','skipped','superseded')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
    let quarantined = Outbox::authorized(db).is_err();
    let mirror_error = db
        .query_row(
            "SELECT value FROM settings WHERE key LIKE 'mirror_error:%' ORDER BY key LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    let mut head = db.query_row(&format!("{SELECT_OPERATION} WHERE state NOT IN ('applied','skipped','superseded') ORDER BY sequence LIMIT 1"), [], operation).optional()?;
    // Management status never includes user rows or file bodies.
    if let Some(head) = &mut head {
        head.payload = Value::Null;
    }
    Ok(OutboxStatus {
        scope,
        quarantined,
        pending_count,
        pending_bytes,
        oldest_at,
        head,
        mirror_error,
    })
}

fn add_column(db: &Connection, table: &str, column: &str, declaration: &str) -> Result<()> {
    let exists: bool = db.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM pragma_table_info('{table}') WHERE name=?1)"),
        [column],
        |row| row.get(0),
    )?;
    if !exists {
        db.execute_batch(&format!(
            "ALTER TABLE {table} ADD COLUMN {column} {declaration}"
        ))?;
    }
    Ok(())
}

fn sqlite_uri(path: &Path, immutable: bool) -> Result<String> {
    let path = std::path::absolute(path)?;
    let text = path
        .to_str()
        .context("Offline queue path is not valid UTF-8")?;
    let mut uri = String::from("file:");
    if !text.starts_with('/') {
        uri.push('/');
    }
    for character in text.chars() {
        match character {
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3f"),
            '#' => uri.push_str("%23"),
            '\\' => uri.push('/'),
            other => uri.push(other),
        }
    }
    uri.push_str(if immutable {
        "?mode=ro&immutable=1"
    } else {
        "?mode=ro"
    });
    Ok(uri)
}

fn setting_key(prefix: &str, resource: &str) -> String {
    format!("{prefix}:{resource}")
}

impl Outbox {
    /// Opens `<parent>/.standalone-outbox/<namespace>/<scope>/queue.sqlite`.
    pub fn open(
        parent: &Path,
        namespace: &str,
        scope: &str,
        limits: BufferingConfig,
    ) -> Result<Self> {
        Self::open_with(parent, namespace, scope, limits, OutboxOptions::default())
    }

    pub fn open_with(
        parent: &Path,
        namespace: &str,
        scope: &str,
        limits: BufferingConfig,
        options: OutboxOptions,
    ) -> Result<Self> {
        validate_namespace(namespace)?;
        ensure!(
            scope.len() == 64
                && scope
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "Invalid outbox authorization scope"
        );
        let mut root = parent.to_path_buf();
        for part in [".standalone-outbox", namespace, scope] {
            root.push(part);
            private_directory(&root)?;
        }
        for name in [
            "queue.sqlite",
            "queue.sqlite-wal",
            "queue.sqlite-shm",
            "queue.sqlite-journal",
        ] {
            if let Ok(meta) = std::fs::symlink_metadata(root.join(name)) {
                ensure!(
                    meta.is_file() && !meta.file_type().is_symlink(),
                    "Outbox files must not be symlinks"
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    ensure!(
                        meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o077 == 0,
                        "Outbox files must be private and owned by this user"
                    );
                }
            }
        }
        let path = root.join("queue.sqlite");
        let mut file_options = std::fs::OpenOptions::new();
        file_options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            file_options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        drop(file_options.open(&path)?);
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_size_limit=16777216; PRAGMA wal_autocheckpoint=64;
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS resources(resource TEXT PRIMARY KEY,revision BLOB NOT NULL,local_base INTEGER,local_head INTEGER,branch TEXT,local_name TEXT);
            CREATE TABLE IF NOT EXISTS operations(sequence INTEGER PRIMARY KEY AUTOINCREMENT,operation_id TEXT NOT NULL UNIQUE,resource TEXT NOT NULL,payload BLOB NOT NULL,bytes INTEGER NOT NULL,state TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,first_queued_at INTEGER NOT NULL,error TEXT,coalesce_key TEXT,local_version INTEGER,local_expected BLOB,superseded_by TEXT,receipt BLOB);
            CREATE INDEX IF NOT EXISTS operations_pending ON operations(state,sequence);")?;
        add_column(&db, "operations", "error_code", "TEXT")?;
        if options.lanes == QueueLanes::PerResource {
            db.execute_batch(
                "CREATE INDEX IF NOT EXISTS operations_lane ON operations(resource,sequence);",
            )?;
        }
        ensure!(
            limits.max_queue_bytes <= 64 * 1024 * 1024 * 1024,
            "Outbox byte budget exceeds 64 GiB"
        );
        Self::apply_page_limit(&db, &limits)?;
        #[cfg(unix)]
        std::fs::File::open(&root)?.sync_all()?;
        db.execute("INSERT OR IGNORE INTO settings VALUES('scope',?1)", [scope])?;
        let stored: String =
            db.query_row("SELECT value FROM settings WHERE key='scope'", [], |r| {
                r.get(0)
            })?;
        ensure!(
            stored == scope,
            "Outbox belongs to another authorization scope"
        );
        Ok(Self {
            db: Mutex::new(Some(db)),
            root,
            scope: scope.into(),
            limits: RwLock::new(limits),
            lanes: options.lanes,
        })
    }

    /// SQLITE_OPEN_READ_ONLY. Never creates directories or files, sets no PRAGMA and writes
    /// nothing. Ok(None) when the scope directory has no queue.sqlite.
    pub fn open_read_only(scope_root: &Path) -> Result<Option<OutboxReader>> {
        let path = scope_root.join("queue.sqlite");
        match std::fs::symlink_metadata(&path) {
            Ok(meta) => ensure!(
                meta.is_file() && !meta.file_type().is_symlink(),
                "Outbox files must not be symlinks"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        // Without a write-ahead log no writer has the queue open; a read-only open would
        // otherwise create the -wal and -shm files.
        let immutable = !scope_root.join("queue.sqlite-wal").exists();
        let db = Connection::open_with_flags(
            sqlite_uri(&path, immutable)?,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
                | rusqlite::OpenFlags::SQLITE_OPEN_URI
                | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        Ok(Some(OutboxReader { db }))
    }

    fn apply_page_limit(db: &Connection, limits: &BufferingConfig) -> Result<()> {
        let page_size: u64 = db.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        let existing_pages: u64 = db.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        // Keep fixed headroom for bounded receipts and one SQLite transaction.
        // A lower configuration still permits draining an existing larger queue.
        let pages =
            ((limits.max_queue_bytes + 16 * 1024 * 1024).div_ceil(page_size)).max(existing_pages);
        db.execute_batch(&format!("PRAGMA max_page_count={pages}"))?;
        Ok(())
    }

    /// Hot limits: later enqueues use them; the page limit never drops below current usage.
    pub fn set_limits(&self, limits: BufferingConfig) -> Result<()> {
        limits.validate_limits()?;
        let db = self.lock()?;
        Self::apply_page_limit(&db, &limits)?;
        *self
            .limits
            .write()
            .map_err(|_| anyhow::anyhow!("Outbox limits poisoned"))? = limits;
        Ok(())
    }

    fn limits(&self) -> Result<BufferingConfig> {
        Ok(self
            .limits
            .read()
            .map_err(|_| anyhow::anyhow!("Outbox limits poisoned"))?
            .clone())
    }

    pub fn lanes(&self) -> QueueLanes {
        self.lanes
    }

    /// Releases the SQLite connection. Every later call fails.
    pub fn close(&self) -> Result<()> {
        let connection = self
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("Outbox mutex poisoned"))?
            .take();
        drop(connection);
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn acquire_writer(&self) -> Result<std::fs::File> {
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let file = options.open(self.root.join("writer.lock"))?;
        fs2::FileExt::try_lock_exclusive(&file)
            .context("This offline queue already has a writer")?;
        Ok(file)
    }
    pub fn quarantine_other_scopes(&self) -> Result<()> {
        let base = self
            .root
            .parent()
            .context("Missing outbox placement root")?;
        let placement = base
            .file_name()
            .and_then(|name| name.to_str())
            .context("Invalid outbox placement")?;
        let parent = base
            .parent()
            .and_then(Path::parent)
            .context("Missing outbox data root")?;
        for (count, entry) in std::fs::read_dir(base)?.enumerate() {
            ensure!(count < 64, "Too many retained outbox authorization scopes");
            let entry = entry?;
            let scope = entry.file_name().to_string_lossy().into_owned();
            if scope == self.scope {
                continue;
            }
            ensure!(
                entry.file_type()?.is_dir(),
                "Invalid outbox scope directory"
            );
            Self::open_with(
                parent,
                placement,
                &scope,
                self.limits()?,
                OutboxOptions { lanes: self.lanes },
            )?
            .quarantine(
                "Another authorization scope replaced this queue; its payloads remain on this device",
            )?;
        }
        Ok(())
    }
    pub fn scope(&self) -> &str {
        &self.scope
    }
    fn lock(&self) -> Result<Db<'_>> {
        let guard = self
            .db
            .lock()
            .map_err(|_| anyhow::anyhow!("Outbox mutex poisoned"))?;
        ensure!(guard.is_some(), CLOSED);
        Ok(Db(guard))
    }
    fn authorized(db: &Connection) -> Result<()> {
        ensure!(
            !db.query_row(
                "SELECT EXISTS(SELECT 1 FROM settings WHERE key='quarantined')",
                [],
                |r| r.get::<_, bool>(0)
            )?,
            "Offline writes are quarantined after authorization was revoked"
        );
        Ok(())
    }
    pub fn check_authorized(&self) -> Result<()> {
        Self::authorized(&*self.lock()?)
    }
    pub fn quarantine(&self, reason: &str) -> Result<()> {
        self.lock()?.execute(
            "INSERT OR REPLACE INTO settings VALUES('quarantined',?1)",
            [reason],
        )?;
        Ok(())
    }
    fn set_setting(&self, key: &str, value: Option<&str>) -> Result<()> {
        let db = self.lock()?;
        match value {
            Some(value) => {
                db.execute(
                    "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
                    params![key, value],
                )?;
            }
            None => {
                db.execute("DELETE FROM settings WHERE key=?1", [key])?;
            }
        }
        Ok(())
    }
    fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .lock()?
            .query_row("SELECT value FROM settings WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }
    pub fn mirror_error(&self, resource: &str, error: Option<&str>) -> Result<()> {
        let error = error.map(|error| error.chars().take(1024).collect::<String>());
        self.set_setting(&setting_key("mirror_error", resource), error.as_deref())
    }
    pub fn table_mirror_error(&self, resource: &str) -> Result<Option<String>> {
        self.setting(&setting_key("mirror_error", resource))
    }
    /// (resource, error) of every table with a mirror error.
    pub fn mirror_errors(&self) -> Result<Vec<(String, String)>> {
        let db = self.lock()?;
        let mut statement = db.prepare(
            "SELECT substr(key,14),value FROM settings WHERE substr(key,1,13)='mirror_error:' ORDER BY key",
        )?;
        let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Settings "refreshed_at:{resource}": the last refresh or confirmation of the cloud version.
    pub fn record_refresh(&self, resource: &str, at: i64) -> Result<()> {
        self.set_setting(
            &setting_key("refreshed_at", resource),
            Some(&at.to_string()),
        )
    }
    pub fn refreshed_at(&self, resource: &str) -> Result<Option<i64>> {
        self.setting(&setting_key("refreshed_at", resource))?
            .map(|value| value.parse::<i64>().map_err(Into::into))
            .transpose()
    }
    pub fn set_remote_missing(&self, resource: &str, missing: bool) -> Result<()> {
        self.set_setting(
            &setting_key("remote_missing", resource),
            missing.then_some("1"),
        )
    }
    pub fn remote_missing(&self, resource: &str) -> Result<bool> {
        Ok(self
            .setting(&setting_key("remote_missing", resource))?
            .is_some())
    }
    /// Records a local table that a checkpoint replaced, for deletion after its grace.
    pub fn retire_table(&self, name: &str, at: i64) -> Result<()> {
        self.set_setting(&setting_key("retired", name), Some(&at.to_string()))
    }
    /// (local table name, retired at) of retired tables that no resource uses any more.
    pub fn retired_tables(&self) -> Result<Vec<(String, i64)>> {
        let db = self.lock()?;
        let mut statement = db.prepare("SELECT substr(key,9),value FROM settings WHERE substr(key,1,8)='retired:' AND substr(key,9) NOT IN (SELECT local_name FROM resources WHERE local_name IS NOT NULL) ORDER BY key")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (name, at) = row?;
            Ok((name, at.parse()?))
        })
        .collect()
    }
    pub fn forget_retired(&self, name: &str) -> Result<()> {
        self.set_setting(&setting_key("retired", name), None)
    }
    /// Deletes the resource row only when it has no non-terminal operations.
    pub fn forget_resource(&self, resource: &str) -> Result<bool> {
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.query_row(
            &format!(
                "SELECT EXISTS(SELECT 1 FROM operations WHERE resource=?1 AND {NON_TERMINAL})"
            ),
            [resource],
            |r| r.get::<_, bool>(0),
        )? {
            return Ok(false);
        }
        tx.execute("DELETE FROM resources WHERE resource=?1", [resource])?;
        for prefix in ["mirror_error", "skip", "refreshed_at", "remote_missing"] {
            tx.execute(
                "DELETE FROM settings WHERE key=?1",
                [setting_key(prefix, resource)],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    pub fn resource_revision(&self, resource: &str) -> Result<Option<Value>> {
        let bytes: Option<Vec<u8>> = self
            .lock()?
            .query_row(
                "SELECT revision FROM resources WHERE resource=?1",
                [resource],
                |r| r.get(0),
            )
            .optional()?;
        bytes
            .map(|b| serde_json::from_slice(&b).map_err(Into::into))
            .transpose()
    }
    pub fn initialize_resource(
        &self,
        resource: &str,
        revision: &Value,
        local_base: Option<u64>,
    ) -> Result<()> {
        self.lock()?.execute("INSERT OR IGNORE INTO resources(resource,revision,local_base,local_head) VALUES(?1,?2,?3,?3)", params![resource, serde_json::to_vec(revision)?, local_base])?;
        Ok(())
    }
    pub fn initialize_table(
        &self,
        resource: &str,
        revision: &Value,
        local_base: u64,
        name: &str,
    ) -> Result<()> {
        self.lock()?.execute("INSERT OR IGNORE INTO resources(resource,revision,local_base,local_head,local_name) VALUES(?1,?2,?3,?3,?4)",params![resource,serde_json::to_vec(revision)?,local_base,name])?;
        Ok(())
    }
    pub fn retained_table_names(&self) -> Result<std::collections::HashSet<String>> {
        let db = self.lock()?;
        let mut statement = db.prepare("SELECT resource,local_name FROM resources WHERE json_extract(resource,'$.kind')='table'")?;
        let entries = statement.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut names = entries
            .map(|entry| {
                let (resource, name) = entry?;
                Ok(name.unwrap_or_else(|| {
                    format!("table_{}", blake3::hash(resource.as_bytes()).to_hex())
                }))
            })
            .collect::<rusqlite::Result<std::collections::HashSet<_>>>()?;
        let mut retired =
            db.prepare("SELECT substr(key,9) FROM settings WHERE substr(key,1,8)='retired:'")?;
        for name in retired.query_map([], |row| row.get::<_, String>(0))? {
            names.insert(name?);
        }
        Ok(names)
    }
    pub fn local_head(&self, resource: &str) -> Result<Option<u64>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT local_head FROM resources WHERE resource=?1",
                [resource],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
    }
    pub fn local_view(&self, resource: &str) -> Result<(Option<String>, String, Option<u64>)> {
        Ok(self.lock()?.query_row(
            "SELECT local_name,COALESCE(branch,'main'),local_base FROM resources WHERE resource=?1",
            [resource],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?)
    }
    pub fn checkpoint(
        &self,
        resource: &str,
        name: &str,
        local_version: Option<u64>,
        revision: &Value,
        previous: &Value,
    ) -> Result<bool> {
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        if tx.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE resource=?1 AND state NOT IN ('applied','skipped','superseded'))",[resource],|r| r.get::<_,bool>(0))? { return Ok(false) }
        let current: Vec<u8> = tx.query_row(
            "SELECT revision FROM resources WHERE resource=?1",
            [resource],
            |r| r.get(0),
        )?;
        if serde_json::from_slice::<Value>(&current)? != *previous {
            return Ok(false);
        }
        tx.execute("UPDATE resources SET local_name=?2,branch='main',local_base=?3,local_head=?3,revision=?4 WHERE resource=?1",params![resource,name,local_version,serde_json::to_vec(revision)?])?;
        tx.commit()?;
        Ok(true)
    }
    fn skip_key(&self, resource: &str) -> String {
        match self.lanes {
            QueueLanes::Global => "skip".into(),
            QueueLanes::PerResource => setting_key("skip", resource),
        }
    }
    pub fn request_skip(&self, id: &str, reason: &str, acknowledge_uncertain: bool) -> Result<()> {
        ensure!(
            !reason.trim().is_empty() && reason.len() <= 1024,
            "Skipping requires a bounded operator reason"
        );
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        Self::authorized(&tx)?;
        let previous: Option<(String, Option<String>, String)> = tx
            .query_row(
                "SELECT state,error,resource FROM operations WHERE operation_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((state, previous_reason, _)) = &previous {
            if state == "skipped" {
                ensure!(
                    previous_reason.as_deref() == Some(reason),
                    "Skipped operation belongs to another request"
                );
                return Ok(());
            }
        }
        let head = match self.lanes {
            QueueLanes::Global => {
                let head = tx.query_row(&format!("{SELECT_OPERATION} WHERE state NOT IN ('applied','skipped','superseded') ORDER BY sequence LIMIT 1"),[],operation).optional()?.context("Offline queue is empty")?;
                ensure!(
                    head.operation_id == id,
                    "Only the queue head may be skipped"
                );
                head
            }
            QueueLanes::PerResource => {
                let resource = previous
                    .as_ref()
                    .map(|(_, _, resource)| resource.clone())
                    .context("Offline change does not exist")?;
                let head = tx
                    .query_row(
                        &format!("{SELECT_OPERATION} WHERE resource=?1 AND {NON_TERMINAL} ORDER BY sequence LIMIT 1"),
                        [&resource],
                        operation,
                    )
                    .optional()?
                    .context("Offline queue is empty")?;
                ensure!(
                    head.operation_id == id,
                    "Only the oldest queued change of a resource may be skipped"
                );
                head
            }
        };
        ensure!(
            head.attempts == 0 || acknowledge_uncertain,
            "This write may already have reached the cloud; acknowledge that skipping cannot undo a cloud effect"
        );
        tx.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
            params![
                self.skip_key(&head.resource),
                serde_json::to_string(&serde_json::json!({"operation_id":id,"reason":reason}))?
            ],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn clear_finished_skip(&self, id: &str) -> Result<()> {
        match self.lanes {
            QueueLanes::Global => self.lock()?.execute("DELETE FROM settings WHERE key='skip' AND json_extract(value,'$.operation_id')=?1 AND EXISTS(SELECT 1 FROM operations WHERE operation_id=?1 AND state IN ('applied','skipped','superseded'))",[id])?,
            QueueLanes::PerResource => self.lock()?.execute("DELETE FROM settings WHERE substr(key,1,5)='skip:' AND json_extract(value,'$.operation_id')=?1 AND EXISTS(SELECT 1 FROM operations WHERE operation_id=?1 AND state IN ('applied','skipped','superseded'))",[id])?,
        };
        Ok(())
    }
    fn parse_skip(value: &str) -> Result<(String, String)> {
        let value: Value = serde_json::from_str(value)?;
        Ok((
            value["operation_id"]
                .as_str()
                .context("Invalid skip operation")?
                .into(),
            value["reason"]
                .as_str()
                .context("Invalid skip reason")?
                .into(),
        ))
    }
    /// The requested skip of a `Global` queue, or the oldest one of a `PerResource` queue.
    pub fn requested_skip(&self) -> Result<Option<(String, String)>> {
        Ok(self.requested_skips()?.into_iter().next())
    }
    /// Every requested skip: at most one per lane.
    pub fn requested_skips(&self) -> Result<Vec<(String, String)>> {
        let values: Vec<String> = {
            let db = self.lock()?;
            let mut statement = db.prepare(match self.lanes {
                QueueLanes::Global => "SELECT value FROM settings WHERE key='skip'",
                QueueLanes::PerResource => {
                    "SELECT value FROM settings WHERE substr(key,1,5)='skip:' ORDER BY key"
                }
            })?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<rusqlite::Result<_>>()?
        };
        values.iter().map(|value| Self::parse_skip(value)).collect()
    }
    pub fn complete_skip(
        &self,
        id: &str,
        reason: &str,
        local_name: Option<&str>,
        branch: &str,
        baseline: Option<u64>,
    ) -> Result<()> {
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let resource: String = tx.query_row("SELECT resource FROM operations WHERE operation_id=?1 AND state NOT IN ('applied','skipped','superseded')",[id],|r|r.get(0))?;
        tx.execute("UPDATE operations SET state='skipped',error=?2,payload=x'6e756c6c',bytes=0,coalesce_key=NULL WHERE operation_id=?1",params![id,reason])?;
        tx.execute("UPDATE operations SET local_version=NULL,local_expected=NULL WHERE resource=?1 AND state NOT IN ('applied','skipped','superseded')",[&resource])?;
        tx.execute("UPDATE resources SET local_name=COALESCE(?2,local_name),branch=?3,local_base=?4,local_head=?4 WHERE resource=?1",params![resource,local_name,branch,baseline])?;
        tx.execute(
            "DELETE FROM settings WHERE key=?1",
            [self.skip_key(&resource)],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn local_expected(&self, id: &str) -> Result<Option<Value>> {
        let value: Option<Vec<u8>> = self.lock()?.query_row(
            "SELECT local_expected FROM operations WHERE operation_id=?1",
            [id],
            |r| r.get(0),
        )?;
        value
            .map(|v| serde_json::from_slice(&v).map_err(Into::into))
            .transpose()
    }
    pub fn prepare_local(&self, id: &str, expected: &Value) -> Result<()> {
        self.lock()?.execute("UPDATE operations SET local_expected=COALESCE(local_expected,?2) WHERE operation_id=?1",params![id,serde_json::to_vec(expected)?])?;
        Ok(())
    }

    /// Only adjacent, never-dispatched replacements may share a cloud precondition.
    /// The caller supplies a key only for a complete replacement or exact-key delete.
    pub fn enqueue(
        &self,
        resource: &str,
        payload: Value,
        coalesce_key: Option<&str>,
        now: i64,
    ) -> Result<QueuedOperation> {
        let bytes = serde_json::to_vec(&payload)?;
        ensure!(
            bytes.len() <= MAX_OPERATION_BYTES,
            "Offline operation exceeds 8 MiB; split the logical batch"
        );
        let limits = self.limits()?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        Self::authorized(&tx)?;
        type Previous = (u64, String, String, String, u32, Option<String>, u64, i64);
        let previous_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<Previous> {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
                r.get(7)?,
            ))
        };
        let previous: Option<Previous> = match self.lanes {
            QueueLanes::Global => tx.query_row("SELECT sequence,operation_id,resource,state,attempts,coalesce_key,bytes,first_queued_at FROM operations ORDER BY sequence DESC LIMIT 1", [], previous_row),
            QueueLanes::PerResource => tx.query_row("SELECT sequence,operation_id,resource,state,attempts,coalesce_key,bytes,first_queued_at FROM operations WHERE resource=?1 ORDER BY sequence DESC LIMIT 1", [resource], previous_row),
        }.optional()?;
        let replace = previous.filter(|p| {
            p.2 == resource
                && p.3 == "pending"
                && p.4 == 0
                && coalesce_key.is_some()
                && p.5.as_deref() == coalesce_key
        });
        let (count, used, mut oldest): (u64,u64,Option<i64>) = tx.query_row("SELECT COUNT(*),COALESCE(SUM(bytes),0),MIN(first_queued_at) FROM operations WHERE state NOT IN ('applied','skipped','superseded')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        if self.lanes == QueueLanes::PerResource {
            oldest = tx.query_row(
                &format!("SELECT MIN(first_queued_at) FROM operations WHERE resource=?1 AND {NON_TERMINAL}"),
                [resource],
                |r| r.get(0),
            )?;
        }
        ensure!(
            oldest.is_none_or(|oldest| now.saturating_sub(oldest) <= limits.max_age_seconds as i64),
            "Offline queue exceeded its age limit; resolve its oldest operation"
        );
        let reclaimed = replace.as_ref().map_or(0, |p| p.6);
        let first_queued_at = replace.as_ref().map_or(now, |p| p.7);
        ensure!(
            count - u64::from(replace.is_some()) < u64::from(limits.max_operations),
            "Offline queue operation limit reached"
        );
        let cost = bytes.len() as u64
            + resource.len() as u64
            + coalesce_key.map_or(0, |key| key.len() as u64)
            + RECORD_OVERHEAD;
        ensure!(
            used.saturating_sub(reclaimed).saturating_add(cost) <= limits.max_queue_bytes,
            "Offline queue byte limit reached"
        );
        if let Some(previous) = replace {
            tx.execute("UPDATE operations SET state='superseded',superseded_by=?2,payload=x'6e756c6c',bytes=0,coalesce_key=NULL WHERE sequence=?1", params![previous.0,id])?;
        }
        tx.execute("INSERT INTO operations(operation_id,resource,payload,bytes,state,created_at,coalesce_key,first_queued_at) VALUES(?1,?2,?3,?4,'pending',?5,?6,?7)", params![id,resource,bytes,cost,now,coalesce_key,first_queued_at])?;
        let seq = tx.last_insert_rowid() as u64;
        // Audit tombstones contain no payload and have bounded retention.
        tx.execute("DELETE FROM operations WHERE state IN ('applied','skipped','superseded') AND sequence < (SELECT COALESCE(MAX(sequence),0)-1024 FROM operations)", [])?;
        tx.commit()?;
        Ok(QueuedOperation {
            sequence: seq,
            operation_id: id,
            resource: resource.into(),
            payload,
            state: "pending".into(),
            attempts: 0,
            created_at: now,
            error: None,
            local_version: None,
        })
    }

    pub fn head_state(&self) -> Result<Option<String>> {
        Ok(self.lock()?.query_row("SELECT state FROM operations WHERE state NOT IN ('applied','skipped','superseded') ORDER BY sequence LIMIT 1", [], |row| row.get(0)).optional()?)
    }
    pub fn head(&self) -> Result<Option<QueuedOperation>> {
        Ok(self.lock()?.query_row(&format!("{SELECT_OPERATION} WHERE state NOT IN ('applied','skipped','superseded') ORDER BY sequence LIMIT 1"), [], operation).optional()?)
    }
    /// A non-terminal operation with its payload.
    pub fn operation(&self, id: &str) -> Result<Option<QueuedOperation>> {
        Ok(self
            .lock()?
            .query_row(
                &format!("{SELECT_OPERATION} WHERE operation_id=?1 AND {NON_TERMINAL}"),
                [id],
                operation,
            )
            .optional()?)
    }
    /// The next non-terminal operation of a lane after `sequence`.
    pub fn next_in_lane(&self, resource: &str, sequence: u64) -> Result<Option<QueuedOperation>> {
        Ok(self
            .lock()?
            .query_row(
                &format!("{SELECT_OPERATION} WHERE resource=?1 AND sequence>?2 AND {NON_TERMINAL} ORDER BY sequence LIMIT 1"),
                params![resource, sequence],
                operation,
            )
            .optional()?)
    }
    /// The next non-terminal operation of the queue after `sequence`.
    pub fn next_after(&self, sequence: u64) -> Result<Option<QueuedOperation>> {
        Ok(self
            .lock()?
            .query_row(
                &format!("{SELECT_OPERATION} WHERE sequence>?1 AND {NON_TERMINAL} ORDER BY sequence LIMIT 1"),
                [sequence],
                operation,
            )
            .optional()?)
    }
    /// Lane heads, oldest first, without payloads. With `Global` lanes only the queue head.
    pub fn lane_heads(&self) -> Result<Vec<OperationSummary>> {
        let db = self.lock()?;
        let heads = match self.lanes {
            QueueLanes::Global => GLOBAL_HEAD,
            QueueLanes::PerResource => LANE_HEADS,
        };
        let mut statement =
            db.prepare(&format!("{SELECT_SUMMARY} WHERE {heads} ORDER BY sequence"))?;
        Ok(statement
            .query_map([], summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Lane heads in blocked|conflict|outcome_unknown, oldest first, no payloads.
    pub fn blocked_heads(&self, limit: u32) -> Result<Vec<OperationSummary>> {
        ensure!(
            (1..=256).contains(&limit),
            "Invalid offline listing page size"
        );
        let db = self.lock()?;
        let heads = match self.lanes {
            QueueLanes::Global => GLOBAL_HEAD,
            QueueLanes::PerResource => LANE_HEADS,
        };
        let mut statement = db.prepare(&format!("{SELECT_SUMMARY} WHERE {heads} AND state IN ('blocked','conflict','outcome_unknown') ORDER BY sequence LIMIT ?1"))?;
        Ok(statement
            .query_map([limit], summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// Non-terminal operations ordered by sequence. Never returns payloads.
    pub fn list_operations(
        &self,
        after_sequence: Option<u64>,
        limit: u32,
    ) -> Result<Vec<OperationSummary>> {
        ensure!(
            (1..=256).contains(&limit),
            "Invalid offline listing page size"
        );
        let db = self.lock()?;
        let mut statement = db.prepare(&format!(
            "{SELECT_SUMMARY} WHERE {NON_TERMINAL} AND sequence>?1 ORDER BY sequence LIMIT ?2"
        ))?;
        Ok(statement
            .query_map(params![after_sequence.unwrap_or(0), limit], summary)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn operation_state(&self, operation_id: &str) -> Result<Option<OperationLookup>> {
        Ok(self
            .lock()?
            .query_row(
                "SELECT state,superseded_by,error,error_code FROM operations WHERE operation_id=?1",
                [operation_id],
                |row| {
                    Ok(OperationLookup {
                        state: row.get(0)?,
                        superseded_by: row.get(1)?,
                        error: row.get(2)?,
                        error_code: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }
    pub fn pending_for(&self, resource: &str) -> Result<Vec<QueuedOperation>> {
        let db = self.lock()?;
        let mut stmt = db.prepare(&format!("{SELECT_OPERATION} WHERE resource=?1 AND state NOT IN ('applied','skipped','superseded') ORDER BY sequence"))?;
        Ok(stmt
            .query_map([resource], operation)?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    /// States of the resource's non-terminal operations, oldest first.
    pub fn pending_states(&self, resource: &str) -> Result<Vec<String>> {
        let db = self.lock()?;
        let mut stmt = db.prepare(&format!(
            "SELECT state FROM operations WHERE resource=?1 AND {NON_TERMINAL} ORDER BY sequence"
        ))?;
        Ok(stmt
            .query_map([resource], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn next_unmaterialized(&self, resource: &str) -> Result<Option<QueuedOperation>> {
        Ok(self.lock()?.query_row(&format!("{SELECT_OPERATION} WHERE resource=?1 AND local_version IS NULL AND state NOT IN ('applied','skipped','superseded') ORDER BY sequence LIMIT 1"),[resource],operation).optional()?)
    }
    pub fn has_pending(&self, resource: &str) -> Result<bool> {
        Ok(self.lock()?.query_row("SELECT EXISTS(SELECT 1 FROM operations WHERE resource=?1 AND state NOT IN ('applied','skipped','superseded'))",[resource],|r| r.get(0))?)
    }
    pub fn latest_pending(&self, resource: &str) -> Result<Option<QueuedOperation>> {
        Ok(self.lock()?.query_row(&format!("{SELECT_OPERATION} WHERE resource=?1 AND state NOT IN ('applied','skipped','superseded') ORDER BY sequence DESC LIMIT 1"),[resource],operation).optional()?)
    }
    pub fn can_coalesce(&self, resource: &str, key: &str) -> Result<bool> {
        let sql = match self.lanes {
            QueueLanes::Global => {
                "SELECT resource=?1 AND state='pending' AND attempts=0 AND coalesce_key=?2 FROM operations ORDER BY sequence DESC LIMIT 1"
            }
            QueueLanes::PerResource => {
                "SELECT resource=?1 AND state='pending' AND attempts=0 AND coalesce_key=?2 FROM operations WHERE resource=?1 ORDER BY sequence DESC LIMIT 1"
            }
        };
        Ok(self
            .lock()?
            .query_row(sql, params![resource, key], |r| r.get::<_, Option<bool>>(0))
            .optional()?
            .flatten()
            .unwrap_or(false))
    }
    pub fn pending_file_resources(&self, after: Option<&str>, limit: u32) -> Result<Vec<String>> {
        ensure!(
            (1..=256).contains(&limit),
            "Invalid offline listing page size"
        );
        let db = self.lock()?;
        let mut stmt = db.prepare("SELECT DISTINCT resource FROM operations WHERE state NOT IN ('applied','skipped','superseded') AND json_extract(resource,'$.kind')='file' AND (?1 IS NULL OR resource > ?1) ORDER BY resource LIMIT ?2")?;
        Ok(stmt
            .query_map(params![after, limit], |r| r.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?)
    }
    pub fn replace_resource_revision_if_idle(
        &self,
        resource: &str,
        revision: &Value,
    ) -> Result<()> {
        self.lock()?.execute("UPDATE resources SET revision=?2 WHERE resource=?1 AND NOT EXISTS(SELECT 1 FROM operations WHERE resource=?1 AND state NOT IN ('applied','skipped','superseded'))",params![resource,serde_json::to_vec(revision)?])?;
        Ok(())
    }
    pub fn mark_local(&self, id: &str, version: u64) -> Result<()> {
        let mut db = self.lock()?;
        let tx = db.transaction()?;
        tx.execute(
            "UPDATE operations SET local_version=?2 WHERE operation_id=?1",
            params![id, version],
        )?;
        tx.execute("UPDATE resources SET local_head=?2 WHERE resource=(SELECT resource FROM operations WHERE operation_id=?1)", params![id,version])?;
        tx.commit()?;
        Ok(())
    }
    pub fn prepare_attempt(&self, id: &str, immutable_request: &Value) -> Result<()> {
        self.prepare_attempt_merged(id, immutable_request, &[])
    }
    /// `prepare_attempt` that also supersedes `merged`, the operations that directly follow
    /// the head in its lane and whose changes `immutable_request` already contains. They
    /// must still be pending, never attempted and locally materialized; the head takes the
    /// local version of the last one.
    pub fn prepare_attempt_merged(
        &self,
        id: &str,
        immutable_request: &Value,
        merged: &[String],
    ) -> Result<()> {
        let bytes = serde_json::to_vec(immutable_request)?;
        ensure!(
            bytes.len() <= MAX_OPERATION_BYTES,
            "Replay request exceeds queue operation limit"
        );
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        Self::authorized(&tx)?;
        let changed = match self.lanes {
            QueueLanes::Global => tx.execute("UPDATE operations SET state='attempting',attempts=attempts+1,payload=?2,error=NULL,error_code=NULL WHERE operation_id=?1 AND state='pending' AND local_version IS NOT NULL AND sequence=(SELECT MIN(sequence) FROM operations WHERE state NOT IN ('applied','skipped','superseded')) AND NOT EXISTS(SELECT 1 FROM settings WHERE key='skip' AND json_extract(value,'$.operation_id')=?1)", params![id,bytes])?,
            QueueLanes::PerResource => tx.execute("UPDATE operations SET state='attempting',attempts=attempts+1,payload=?2,error=NULL,error_code=NULL WHERE operation_id=?1 AND state='pending' AND local_version IS NOT NULL AND sequence=(SELECT MIN(o.sequence) FROM operations o WHERE o.resource=operations.resource AND o.state NOT IN ('applied','skipped','superseded')) AND NOT EXISTS(SELECT 1 FROM settings WHERE key='skip:'||operations.resource AND json_extract(value,'$.operation_id')=?1)", params![id,bytes])?,
        };
        ensure!(
            changed == 1,
            "Only the locally materialized queue head may start replay"
        );
        if !merged.is_empty() {
            let (resource, sequence): (String, u64) = tx.query_row(
                "SELECT resource,sequence FROM operations WHERE operation_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let followers = {
                let mut statement = tx.prepare(&format!("SELECT operation_id,state,attempts,local_version FROM operations WHERE resource=?1 AND sequence>?2 AND {NON_TERMINAL} ORDER BY sequence LIMIT ?3"))?;
                statement
                    .query_map(params![resource, sequence, merged.len()], |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, u32>(2)?,
                            r.get::<_, Option<u64>>(3)?,
                        ))
                    })?
                    .collect::<rusqlite::Result<Vec<_>>>()?
            };
            ensure!(
                followers.len() == merged.len()
                    && followers.iter().zip(merged).all(|(follower, id)| {
                        follower.0 == *id
                            && follower.1 == "pending"
                            && follower.2 == 0
                            && follower.3.is_some()
                    }),
                "Coalesced offline changes are no longer adjacent, unsent and materialized"
            );
            let last = followers.last().and_then(|follower| follower.3);
            for merged in merged {
                tx.execute("UPDATE operations SET state='superseded',superseded_by=?2,payload=x'6e756c6c',bytes=0,coalesce_key=NULL WHERE operation_id=?1", params![merged,id])?;
            }
            let cost = bytes.len() as u64 + resource.len() as u64 + RECORD_OVERHEAD;
            tx.execute(
                "UPDATE operations SET local_version=?2,bytes=?3,coalesce_key=NULL WHERE operation_id=?1",
                params![id, last, cost],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn block(&self, id: &str, state: &str, reason: &str) -> Result<()> {
        self.block_with_code(id, state, None, reason)
    }
    /// `block` plus an optional machine code.
    pub fn block_with_code(
        &self,
        id: &str,
        state: &str,
        code: Option<&str>,
        reason: &str,
    ) -> Result<()> {
        ensure!(
            matches!(
                state,
                "blocked" | "conflict" | "outcome_unknown" | "attempting"
            ),
            "Invalid queue failure state"
        );
        self.lock()?.execute("UPDATE operations SET state=?2,error=?3,error_code=?4 WHERE operation_id=?1 AND state NOT IN ('applied','skipped','superseded')", params![id,state,reason.chars().take(2048).collect::<String>(),code])?;
        Ok(())
    }
    /// A rejection that provably preceded any server-side claim: 'pending' or 'attempting'
    /// becomes 'blocked' with the attempt count reset. The payload is kept.
    pub fn block_unclaimed(&self, operation_id: &str, code: &str, reason: &str) -> Result<()> {
        let changed = self.lock()?.execute("UPDATE operations SET state='blocked',attempts=0,error=?2,error_code=?3 WHERE operation_id=?1 AND state IN ('pending','attempting')", params![operation_id,reason.chars().take(2048).collect::<String>(),code])?;
        ensure!(
            changed == 1,
            "Only a queued or sending change can be blocked"
        );
        Ok(())
    }
    pub fn acknowledge(&self, id: &str, revision: &Value, receipt: &Value) -> Result<()> {
        let mut db = self.lock()?;
        let tx = db.transaction()?;
        let (resource,local_version): (String,u64) = tx.query_row("SELECT resource,local_version FROM operations WHERE operation_id=?1 AND state IN ('attempting','outcome_unknown')", [id], |r| Ok((r.get(0)?,r.get(1)?)))?;
        tx.execute(
            "UPDATE resources SET revision=?2,local_base=?3 WHERE resource=?1",
            params![resource, serde_json::to_vec(revision)?, local_version],
        )?;
        tx.execute("UPDATE operations SET state='applied',receipt=?2,payload=x'6e756c6c',bytes=0,error=NULL,error_code=NULL,coalesce_key=NULL WHERE operation_id=?1", params![id,serde_json::to_vec(receipt)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn retry(&self, id: &str) -> Result<()> {
        let db = self.lock()?;
        Self::authorized(&db)?;
        // An attempted request stays immutable. The server receipt reconciles it.
        let changed = db.execute("UPDATE operations SET state=CASE WHEN attempts=0 THEN 'pending' ELSE 'attempting' END,error=NULL,error_code=NULL WHERE operation_id=?1 AND state IN ('blocked','conflict','outcome_unknown')", [id])?;
        if changed == 0 {
            // The worker can advance after the durable retry and before the
            // management journal records its reply. Repeating that control is safe.
            let state: Option<String> = db
                .query_row(
                    "SELECT state FROM operations WHERE operation_id=?1",
                    [id],
                    |row| row.get(0),
                )
                .optional()?;
            ensure!(
                matches!(state.as_deref(), Some("pending" | "attempting" | "applied")),
                "Operation cannot be retried"
            );
        }
        Ok(())
    }
    pub fn status(&self) -> Result<OutboxStatus> {
        status_of(&*self.lock()?, self.scope.clone())
    }
}

impl OutboxReader {
    pub fn status(&self) -> Result<OutboxStatus> {
        let scope: String =
            self.db
                .query_row("SELECT value FROM settings WHERE key='scope'", [], |r| {
                    r.get(0)
                })?;
        status_of(&self.db, scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn open(root: &Path) -> Outbox {
        Outbox::open(
            root,
            "placement",
            &"a".repeat(64),
            BufferingConfig::default(),
        )
        .unwrap()
    }
    #[test]
    fn adjacent_unsent_update_then_delete_compacts_and_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        let first = queue
            .enqueue("table", json!({"update": 1}), Some("id:1"), 100)
            .unwrap();
        queue.mark_local(&first.operation_id, 2).unwrap();
        let second = queue
            .enqueue("table", json!({"delete":1}), Some("id:1"), 101)
            .unwrap();
        assert_eq!(queue.status().unwrap().pending_count, 1);
        assert_eq!(queue.status().unwrap().oldest_at, Some(100));
        assert_eq!(second.created_at, 101);
        drop(queue);
        let queue = open(root.path());
        assert_eq!(
            queue.head().unwrap().unwrap().operation_id,
            second.operation_id
        );
        assert_eq!(queue.head().unwrap().unwrap().payload, json!({"delete":1}));
    }
    #[test]
    fn an_attempted_update_and_intervening_operations_are_never_compacted() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        let op = queue
            .enqueue("table", json!({"update":1}), Some("id:1"), 100)
            .unwrap();
        queue.mark_local(&op.operation_id, 2).unwrap();
        queue
            .prepare_attempt(&op.operation_id, &op.payload)
            .unwrap();
        queue
            .enqueue("table", json!({"delete":1}), Some("id:1"), 101)
            .unwrap();
        assert_eq!(queue.status().unwrap().pending_count, 2);
        queue.enqueue("other", json!(1), Some("id:2"), 102).unwrap();
        queue.enqueue("table", json!(1), Some("id:1"), 103).unwrap();
        assert_eq!(queue.status().unwrap().pending_count, 4);
    }
    #[test]
    fn denial_quarantines_without_discarding_payload_and_scope_isolation_survives_restart() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        queue
            .enqueue("table", json!({"sensitive":true}), None, 100)
            .unwrap();
        queue.quarantine("revoked").unwrap();
        assert!(queue.enqueue("table", json!(1), None, 101).is_err());
        assert!(queue.status().unwrap().head.unwrap().payload.is_null());
        drop(queue);
        let queue = open(root.path());
        assert!(queue.status().unwrap().quarantined);
        assert_eq!(
            queue.head().unwrap().unwrap().payload,
            json!({"sensitive":true})
        );
        let other = Outbox::open(
            root.path(),
            "placement",
            &"b".repeat(64),
            BufferingConfig::default(),
        )
        .unwrap();
        assert_eq!(other.status().unwrap().pending_count, 0);
    }
    #[test]
    fn full_and_over_age_queues_preserve_accepted_work() {
        let root = tempfile::tempdir().unwrap();
        let mut config = BufferingConfig::default();
        config.max_operations = 1;
        config.max_age_seconds = 60;
        let queue = Outbox::open(root.path(), "placement", &"a".repeat(64), config).unwrap();
        queue.enqueue("table", json!(1), None, 100).unwrap();
        assert!(queue.enqueue("other", json!(2), None, 101).is_err());
        assert!(queue.enqueue("table", json!(3), None, 200).is_err());
        assert_eq!(queue.head().unwrap().unwrap().payload, json!(1));
    }

    #[test]
    fn skip_request_fences_dispatch_across_queue_connections() {
        let root = tempfile::tempdir().unwrap();
        let worker = open(root.path());
        let controller = open(root.path());
        worker
            .initialize_resource("table", &json!({"version": 1}), Some(1))
            .unwrap();
        let first = worker
            .enqueue("table", json!({"insert": 1}), None, 100)
            .unwrap();
        worker.mark_local(&first.operation_id, 2).unwrap();
        // The worker may already have read this head before management asks to skip it.
        controller
            .request_skip(&first.operation_id, "discard unsent write", false)
            .unwrap();
        assert!(
            worker
                .prepare_attempt(&first.operation_id, &first.payload)
                .is_err()
        );
        assert_eq!(worker.head().unwrap().unwrap().attempts, 0);
        worker
            .complete_skip(
                &first.operation_id,
                "discard unsent write",
                None,
                "main",
                None,
            )
            .unwrap();

        let second = worker
            .enqueue("table", json!({"insert": 2}), None, 101)
            .unwrap();
        worker.mark_local(&second.operation_id, 3).unwrap();
        worker
            .prepare_attempt(&second.operation_id, &second.payload)
            .unwrap();
        assert!(
            controller
                .request_skip(&second.operation_id, "discard sent write", false)
                .is_err()
        );
        controller
            .request_skip(&second.operation_id, "discard sent write", true)
            .unwrap();
        assert_eq!(
            worker.requested_skip().unwrap(),
            Some((second.operation_id, "discard sent write".into()))
        );
    }

    #[test]
    fn stale_snapshot_cannot_replace_a_newly_acknowledged_revision() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        let base = json!({"version": 1, "fingerprint": "base"});
        queue.initialize_resource("table", &base, Some(1)).unwrap();
        let operation = queue
            .enqueue("table", json!({"insert": 1}), None, 100)
            .unwrap();
        queue.mark_local(&operation.operation_id, 2).unwrap();
        assert!(
            !queue
                .checkpoint("table", "snapshot", Some(1), &base, &base)
                .unwrap()
        );
        queue
            .prepare_attempt(&operation.operation_id, &operation.payload)
            .unwrap();
        let applied = json!({"version": 2, "fingerprint": "written"});
        queue
            .acknowledge(&operation.operation_id, &applied, &json!({"applied": true}))
            .unwrap();
        assert!(!queue.has_pending("table").unwrap());
        // A background copy started at version 1 and completed after this acknowledgement.
        assert!(
            !queue
                .checkpoint("table", "stale_snapshot", Some(1), &base, &base)
                .unwrap()
        );
        assert_eq!(
            queue.resource_revision("table").unwrap(),
            Some(applied.clone())
        );
        assert_eq!(queue.local_view("table").unwrap().2, Some(2));
        let refreshed = json!({"version": 3, "fingerprint": "newer"});
        assert!(
            queue
                .checkpoint("table", "fresh_snapshot", Some(1), &refreshed, &applied)
                .unwrap()
        );
        assert_eq!(
            queue.local_view("table").unwrap(),
            (Some("fresh_snapshot".into()), "main".into(), Some(1))
        );
    }

    #[test]
    fn management_status_is_read_only_and_excludes_payloads() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        let payload = json!({"rows": [{"secret": "sensitive-row-body"}]});
        let operation = queue.enqueue("table", payload.clone(), None, 100).unwrap();
        for _ in 0..2 {
            let status = queue.status().unwrap();
            assert!(status.head.as_ref().unwrap().payload.is_null());
            assert!(
                !serde_json::to_string(&status)
                    .unwrap()
                    .contains("sensitive-row-body")
            );
        }
        let retained = queue.head().unwrap().unwrap();
        assert_eq!(retained.operation_id, operation.operation_id);
        assert_eq!(retained.payload, payload);
        assert_eq!(retained.state, "pending");
        assert_eq!(retained.attempts, 0);
        queue
            .mirror_error("table-a", Some("Unavailable A"))
            .unwrap();
        queue
            .mirror_error("table-b", Some("Unavailable B"))
            .unwrap();
        queue.mirror_error("table-a", None).unwrap();
        assert_eq!(
            queue.status().unwrap().mirror_error.as_deref(),
            Some("Unavailable B")
        );
        queue.mirror_error("table-b", None).unwrap();
        assert!(queue.status().unwrap().mirror_error.is_none());
    }

    #[test]
    fn retry_after_restart_preserves_operation_id_and_frozen_request() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        let operation = queue
            .enqueue("table", json!({"rows": [1]}), Some("id:1"), 100)
            .unwrap();
        let frozen = json!({"operation_id": operation.operation_id, "expected": {"version": 9}, "rows": [1]});
        queue.mark_local(&operation.operation_id, 2).unwrap();
        queue
            .prepare_attempt(&operation.operation_id, &frozen)
            .unwrap();
        queue
            .block(
                &operation.operation_id,
                "outcome_unknown",
                "HTTP acknowledgement lost",
            )
            .unwrap();
        drop(queue);
        let queue = open(root.path());
        queue.retry(&operation.operation_id).unwrap();
        let retry = queue.head().unwrap().unwrap();
        assert_eq!(retry.operation_id, operation.operation_id);
        assert_eq!(retry.payload, frozen);
        assert_eq!(retry.attempts, 1);
        assert_eq!(retry.state, "attempting");
        assert!(
            queue
                .prepare_attempt(&retry.operation_id, &json!({"changed": true}))
                .is_err()
        );
        queue
            .enqueue("table", json!({"rows": [2]}), Some("id:1"), 101)
            .unwrap();
        assert_eq!(queue.status().unwrap().pending_count, 2);
        assert_eq!(
            queue.head().unwrap().unwrap().operation_id,
            retry.operation_id
        );
    }

    fn open_lanes(root: &Path, limits: BufferingConfig) -> Outbox {
        Outbox::open_with(
            root,
            "placement",
            &"a".repeat(64),
            limits,
            OutboxOptions {
                lanes: QueueLanes::PerResource,
            },
        )
        .unwrap()
    }

    fn listing(path: &Path) -> Vec<std::ffi::OsString> {
        let mut names = std::fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[test]
    fn open_read_only_creates_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("missing");
        assert!(Outbox::open_read_only(&missing).unwrap().is_none());
        assert!(!missing.exists());
        let queue = open(root.path());
        queue
            .enqueue(
                "table",
                json!({"rows": [{"secret": "row-body"}]}),
                None,
                100,
            )
            .unwrap();
        let scope_root = queue.root().to_path_buf();
        let open_files = listing(&scope_root);
        let reader = Outbox::open_read_only(&scope_root).unwrap().unwrap();
        let status = reader.status().unwrap();
        assert_eq!(status.pending_count, 1);
        assert_eq!(status.scope, "a".repeat(64));
        assert!(!serde_json::to_string(&status).unwrap().contains("row-body"));
        drop(reader);
        assert_eq!(listing(&scope_root), open_files);
        drop(queue);
        let closed_files = listing(&scope_root);
        let bytes = std::fs::read(scope_root.join("queue.sqlite")).unwrap();
        let reader = Outbox::open_read_only(&scope_root).unwrap().unwrap();
        assert_eq!(reader.status().unwrap().pending_count, 1);
        drop(reader);
        assert_eq!(listing(&scope_root), closed_files);
        assert_eq!(
            std::fs::read(scope_root.join("queue.sqlite")).unwrap(),
            bytes
        );
    }

    #[test]
    fn list_operations_never_returns_payloads_and_pages_by_sequence() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        let ids = (0..5)
            .map(|n| {
                queue
                    .enqueue(
                        "table",
                        json!({"mutation": {"kind": "table_insert", "rows": [{"secret": format!("row-{n}")}]}}),
                        None,
                        100 + n,
                    )
                    .unwrap()
                    .operation_id
            })
            .collect::<Vec<_>>();
        let first = queue.list_operations(None, 2).unwrap();
        assert_eq!(
            first
                .iter()
                .map(|o| o.operation_id.as_str())
                .collect::<Vec<_>>(),
            ids[..2].iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(first[0].mutation_kind.as_deref(), Some("table_insert"));
        assert!(first[0].bytes > 0);
        let rest = queue.list_operations(Some(first[1].sequence), 256).unwrap();
        assert_eq!(
            rest.iter()
                .map(|o| o.operation_id.as_str())
                .collect::<Vec<_>>(),
            ids[2..].iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert!(!serde_json::to_string(&rest).unwrap().contains("row-"));
        assert!(queue.list_operations(None, 0).is_err());
        assert!(queue.list_operations(None, 257).is_err());
        queue
            .block_with_code(&ids[0], "blocked", Some("forbidden"), "Access denied")
            .unwrap();
        assert_eq!(
            queue.list_operations(None, 1).unwrap()[0]
                .error_code
                .as_deref(),
            Some("forbidden")
        );
        assert_eq!(
            queue.operation_state(&ids[0]).unwrap().unwrap(),
            OperationLookup {
                state: "blocked".into(),
                superseded_by: None,
                error: Some("Access denied".into()),
                error_code: Some("forbidden".into()),
            }
        );
        queue.retry(&ids[0]).unwrap();
        assert_eq!(
            queue.operation_state(&ids[0]).unwrap().unwrap().error_code,
            None
        );
        assert!(queue.operation_state("missing").unwrap().is_none());
    }

    #[test]
    fn per_resource_lanes_isolate_blocked_heads() {
        let root = tempfile::tempdir().unwrap();
        let queue = open_lanes(root.path(), BufferingConfig::default());
        let first = queue.enqueue("a", json!(1), None, 100).unwrap();
        queue.mark_local(&first.operation_id, 2).unwrap();
        let second = queue.enqueue("a", json!(2), None, 101).unwrap();
        queue.mark_local(&second.operation_id, 3).unwrap();
        let other = queue.enqueue("b", json!(3), None, 102).unwrap();
        queue.mark_local(&other.operation_id, 2).unwrap();
        queue
            .prepare_attempt(&first.operation_id, &first.payload)
            .unwrap();
        queue
            .block(&first.operation_id, "conflict", "Foreign commit")
            .unwrap();
        assert!(
            queue
                .prepare_attempt(&second.operation_id, &second.payload)
                .is_err()
        );
        queue
            .prepare_attempt(&other.operation_id, &other.payload)
            .unwrap();
        assert_eq!(
            queue
                .blocked_heads(10)
                .unwrap()
                .into_iter()
                .map(|head| head.operation_id)
                .collect::<Vec<_>>(),
            vec![first.operation_id.clone()]
        );
        assert_eq!(
            queue
                .lane_heads()
                .unwrap()
                .into_iter()
                .map(|head| head.operation_id)
                .collect::<Vec<_>>(),
            vec![first.operation_id.clone(), other.operation_id.clone()]
        );
        assert_eq!(
            queue
                .request_skip(&second.operation_id, "not the head", false)
                .unwrap_err()
                .to_string(),
            "Only the oldest queued change of a resource may be skipped"
        );
        queue
            .request_skip(&first.operation_id, "discard a", true)
            .unwrap();
        queue
            .request_skip(&other.operation_id, "discard b", true)
            .unwrap();
        assert_eq!(queue.requested_skips().unwrap().len(), 2);
        queue
            .complete_skip(&first.operation_id, "discard a", None, "main", None)
            .unwrap();
        assert_eq!(
            queue.requested_skips().unwrap(),
            vec![(other.operation_id.clone(), "discard b".into())]
        );
        assert_eq!(
            queue.head().unwrap().unwrap().operation_id,
            second.operation_id
        );
    }

    #[test]
    fn lane_age_limit_is_per_resource() {
        let root = tempfile::tempdir().unwrap();
        let limits = BufferingConfig {
            max_age_seconds: 60,
            ..BufferingConfig::default()
        };
        let lanes = open_lanes(root.path(), limits.clone());
        lanes.enqueue("a", json!(1), None, 100).unwrap();
        lanes.enqueue("b", json!(2), None, 200).unwrap();
        assert_eq!(
            lanes
                .enqueue("a", json!(3), None, 200)
                .unwrap_err()
                .to_string(),
            "Offline queue exceeded its age limit; resolve its oldest operation"
        );
        let other = tempfile::tempdir().unwrap();
        let global = Outbox::open(other.path(), "placement", &"a".repeat(64), limits).unwrap();
        global.enqueue("a", json!(1), None, 100).unwrap();
        assert!(global.enqueue("b", json!(2), None, 200).is_err());
    }

    #[test]
    fn set_limits_apply_to_later_enqueues() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        queue.enqueue("table", json!(1), None, 100).unwrap();
        queue
            .set_limits(BufferingConfig {
                max_operations: 1,
                ..BufferingConfig::default()
            })
            .unwrap();
        assert_eq!(
            queue
                .enqueue("table", json!(2), None, 101)
                .unwrap_err()
                .to_string(),
            "Offline queue operation limit reached"
        );
        assert!(
            queue
                .set_limits(BufferingConfig {
                    max_operations: 0,
                    ..BufferingConfig::default()
                })
                .is_err()
        );
        queue.set_limits(BufferingConfig::default()).unwrap();
        queue.enqueue("table", json!(2), None, 101).unwrap();
    }

    #[test]
    fn unclaimed_rejections_reset_attempts_and_closed_queues_fail() {
        let root = tempfile::tempdir().unwrap();
        let queue = open(root.path());
        let operation = queue.enqueue("table", json!(1), None, 100).unwrap();
        queue.mark_local(&operation.operation_id, 2).unwrap();
        queue
            .prepare_attempt(&operation.operation_id, &operation.payload)
            .unwrap();
        queue
            .block_unclaimed(&operation.operation_id, "hub_limit", "Too large")
            .unwrap();
        let head = queue.head().unwrap().unwrap();
        assert_eq!((head.state.as_str(), head.attempts), ("blocked", 0));
        assert!(
            queue
                .block_unclaimed(&operation.operation_id, "hub_limit", "Too large")
                .is_err()
        );
        queue
            .request_skip(&operation.operation_id, "no acknowledgement needed", false)
            .unwrap();
        queue.close().unwrap();
        assert_eq!(queue.status().unwrap_err().to_string(), CLOSED);
        drop(queue);
        let reopened = open(root.path());
        assert_eq!(
            reopened
                .operation_state(&operation.operation_id)
                .unwrap()
                .unwrap()
                .error_code
                .as_deref(),
            Some("hub_limit")
        );
    }
}
