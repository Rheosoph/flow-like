use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::StoragePurpose;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};

const MAX_OPERATION_BYTES: usize = flow_like_device_protocol::MAX_OFFLINE_REPLAY_HTTP_BYTES;
const RECORD_OVERHEAD: u64 = 4096;

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

pub(crate) fn relative_path(value: &str, empty: bool) -> bool {
    (empty && value.is_empty())
        || (!value.is_empty()
            && value.len() <= 1024
            && !value.contains(['\\', '%', '\0'])
            && value
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."))
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
        let mut seen = std::collections::HashSet::new();
        for table in &self.tables {
            ensure!(
                matches!(
                    table.purpose,
                    StoragePurpose::Storage | StoragePurpose::User
                ),
                "Only project and user databases support offline tables"
            );
            ensure!(
                table.database == "db"
                    && relative_path(&table.table, false)
                    && !table.table.contains('/'),
                "Offline tables must use the project or user db directory"
            );
            flow_like_device_protocol::validate_instance_identifier(&table.table)?;
            ensure!(
                !table.table.contains('.'),
                "Buffered table identifiers cannot contain dots"
            );
            ensure!(
                !table.primary_key.is_empty()
                    && table.primary_key.len() <= 128
                    && table
                        .primary_key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "Buffered tables require a primary-key column"
            );
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

pub struct Outbox {
    db: Mutex<Connection>,
    root: PathBuf,
    scope: String,
    limits: BufferingConfig,
}

pub(crate) fn private_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) =>
        {
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                std::fs::File::open(parent)?.sync_all()?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "Outbox directory must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.mode() & 0o077 == 0 && meta.uid() == unsafe { libc::geteuid() },
            "Outbox directory must be private and owned by this user"
        );
    }
    Ok(())
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

impl Outbox {
    pub fn for_placement(
        state_dir: &Path,
        config: &crate::config::PlacementConfig,
    ) -> Result<Vec<Self>> {
        let limits = config.offline_writes.clone().unwrap_or_default();
        let parent = state_dir
            .join("placement-data")
            .join(&config.id)
            .join("current")
            .join("store");
        let root = parent.join(".standalone-outbox").join(&config.id);
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut result = Vec::new();
        for entry in entries {
            let entry = entry?;
            ensure!(
                entry.file_type()?.is_dir(),
                "Unexpected file in offline authorization scopes"
            );
            ensure!(
                result.len() < 64,
                "Offline scope inventory exceeds its management limit"
            );
            let scope = entry.file_name().to_string_lossy().into_owned();
            ensure!(
                entry.path().join("queue.sqlite").is_file(),
                "Offline scope has no queue database"
            );
            result.push(Self::open(&parent, &config.id, &scope, limits.clone())?);
        }
        Ok(result)
    }
    pub fn open(
        parent: &Path,
        placement: &str,
        scope: &str,
        limits: BufferingConfig,
    ) -> Result<Self> {
        crate::config::validate_id("placement", placement)?;
        ensure!(
            scope.len() == 64
                && scope
                    .bytes()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "Invalid outbox authorization scope"
        );
        let mut root = parent.to_path_buf();
        for part in [".standalone-outbox", placement, scope] {
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
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        drop(options.open(&path)?);
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(10))?;
        db.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA secure_delete=ON; PRAGMA auto_vacuum=INCREMENTAL; PRAGMA journal_size_limit=16777216; PRAGMA wal_autocheckpoint=64;
            CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
            CREATE TABLE IF NOT EXISTS resources(resource TEXT PRIMARY KEY,revision BLOB NOT NULL,local_base INTEGER,local_head INTEGER,branch TEXT,local_name TEXT);
            CREATE TABLE IF NOT EXISTS operations(sequence INTEGER PRIMARY KEY AUTOINCREMENT,operation_id TEXT NOT NULL UNIQUE,resource TEXT NOT NULL,payload BLOB NOT NULL,bytes INTEGER NOT NULL,state TEXT NOT NULL,attempts INTEGER NOT NULL DEFAULT 0,created_at INTEGER NOT NULL,first_queued_at INTEGER NOT NULL,error TEXT,coalesce_key TEXT,local_version INTEGER,local_expected BLOB,superseded_by TEXT,receipt BLOB);
            CREATE INDEX IF NOT EXISTS operations_pending ON operations(state,sequence);")?;
        ensure!(
            limits.max_queue_bytes <= 64 * 1024 * 1024 * 1024,
            "Outbox byte budget exceeds 64 GiB"
        );
        let page_size: u64 = db.query_row("PRAGMA page_size", [], |row| row.get(0))?;
        let existing_pages: u64 = db.query_row("PRAGMA page_count", [], |row| row.get(0))?;
        // Keep fixed headroom for bounded receipts and one SQLite transaction.
        // A lower configuration still permits draining an existing larger queue.
        let pages =
            ((limits.max_queue_bytes + 16 * 1024 * 1024).div_ceil(page_size)).max(existing_pages);
        db.execute_batch(&format!("PRAGMA max_page_count={pages}"))?;
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
            db: Mutex::new(db),
            root,
            scope: scope.into(),
            limits,
        })
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
            Self::open(parent,placement,&scope,self.limits.clone())?.quarantine("Another authorization scope replaced this queue; its payloads remain on this device")?;
        }
        Ok(())
    }
    pub fn scope(&self) -> &str {
        &self.scope
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.db
            .lock()
            .map_err(|_| anyhow::anyhow!("Outbox mutex poisoned"))
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
    pub fn mirror_error(&self, resource: &str, error: Option<&str>) -> Result<()> {
        let key = format!("mirror_error:{resource}");
        let db = self.lock()?;
        match error {
            Some(error) => {
                db.execute(
                    "INSERT OR REPLACE INTO settings(key,value) VALUES(?1,?2)",
                    params![key, error.chars().take(1024).collect::<String>()],
                )?;
            }
            None => {
                db.execute("DELETE FROM settings WHERE key=?1", [key])?;
            }
        }
        Ok(())
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
        entries
            .map(|entry| {
                let (resource, name) = entry?;
                Ok(name.unwrap_or_else(|| {
                    format!("table_{}", blake3::hash(resource.as_bytes()).to_hex())
                }))
            })
            .collect::<rusqlite::Result<_>>()
            .map_err(Into::into)
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
    pub fn request_skip(&self, id: &str, reason: &str, acknowledge_uncertain: bool) -> Result<()> {
        ensure!(
            !reason.trim().is_empty() && reason.len() <= 1024,
            "Skipping requires a bounded operator reason"
        );
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        Self::authorized(&tx)?;
        let previous: Option<(String, Option<String>)> = tx
            .query_row(
                "SELECT state,error FROM operations WHERE operation_id=?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((state, previous_reason)) = previous {
            if state == "skipped" {
                ensure!(
                    previous_reason.as_deref() == Some(reason),
                    "Skipped operation belongs to another request"
                );
                return Ok(());
            }
        }
        let head = tx.query_row(&format!("{SELECT_OPERATION} WHERE state NOT IN ('applied','skipped','superseded') ORDER BY sequence LIMIT 1"),[],operation).optional()?.context("Offline queue is empty")?;
        ensure!(
            head.operation_id == id,
            "Only the queue head may be skipped"
        );
        ensure!(
            head.attempts == 0 || acknowledge_uncertain,
            "This write may already have reached the cloud; acknowledge that skipping cannot undo a cloud effect"
        );
        tx.execute(
            "INSERT OR REPLACE INTO settings(key,value) VALUES('skip',?1)",
            [serde_json::to_string(
                &serde_json::json!({"operation_id":id,"reason":reason}),
            )?],
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn clear_finished_skip(&self, id: &str) -> Result<()> {
        self.lock()?.execute("DELETE FROM settings WHERE key='skip' AND json_extract(value,'$.operation_id')=?1 AND EXISTS(SELECT 1 FROM operations WHERE operation_id=?1 AND state IN ('applied','skipped','superseded'))",[id])?;
        Ok(())
    }
    pub fn requested_skip(&self) -> Result<Option<(String, String)>> {
        let value: Option<String> = self
            .lock()?
            .query_row("SELECT value FROM settings WHERE key='skip'", [], |r| {
                r.get(0)
            })
            .optional()?;
        value
            .map(|value| {
                let value: Value = serde_json::from_str(&value)?;
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
            })
            .transpose()
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
        tx.execute("DELETE FROM settings WHERE key='skip'", [])?;
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
        let id = uuid::Uuid::new_v4().to_string();
        let mut db = self.lock()?;
        let tx = db.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        Self::authorized(&tx)?;
        let previous: Option<(u64, String, String, String, u32, Option<String>, u64, i64)> = tx.query_row("SELECT sequence,operation_id,resource,state,attempts,coalesce_key,bytes,first_queued_at FROM operations ORDER BY sequence DESC LIMIT 1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).optional()?;
        let replace = previous.filter(|p| {
            p.2 == resource
                && p.3 == "pending"
                && p.4 == 0
                && coalesce_key.is_some()
                && p.5.as_deref() == coalesce_key
        });
        let (count, used, oldest): (u64,u64,Option<i64>) = tx.query_row("SELECT COUNT(*),COALESCE(SUM(bytes),0),MIN(first_queued_at) FROM operations WHERE state NOT IN ('applied','skipped','superseded')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        ensure!(
            oldest.is_none_or(
                |oldest| now.saturating_sub(oldest) <= self.limits.max_age_seconds as i64
            ),
            "Offline queue exceeded its age limit; resolve its oldest operation"
        );
        let reclaimed = replace.as_ref().map_or(0, |p| p.6);
        let first_queued_at = replace.as_ref().map_or(now, |p| p.7);
        ensure!(
            count - u64::from(replace.is_some()) < u64::from(self.limits.max_operations),
            "Offline queue operation limit reached"
        );
        let cost = bytes.len() as u64
            + resource.len() as u64
            + coalesce_key.map_or(0, |key| key.len() as u64)
            + RECORD_OVERHEAD;
        ensure!(
            used.saturating_sub(reclaimed).saturating_add(cost) <= self.limits.max_queue_bytes,
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
    pub fn pending_for(&self, resource: &str) -> Result<Vec<QueuedOperation>> {
        let db = self.lock()?;
        let mut stmt = db.prepare(&format!("{SELECT_OPERATION} WHERE resource=?1 AND state NOT IN ('applied','skipped','superseded') ORDER BY sequence"))?;
        Ok(stmt
            .query_map([resource], operation)?
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
        Ok(self.lock()?.query_row("SELECT resource=?1 AND state='pending' AND attempts=0 AND coalesce_key=?2 FROM operations ORDER BY sequence DESC LIMIT 1",params![resource,key],|r|r.get::<_,Option<bool>>(0)).optional()?.flatten().unwrap_or(false))
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
        let bytes = serde_json::to_vec(immutable_request)?;
        ensure!(
            bytes.len() <= MAX_OPERATION_BYTES,
            "Replay request exceeds queue operation limit"
        );
        let db = self.lock()?;
        Self::authorized(&db)?;
        let changed = db.execute("UPDATE operations SET state='attempting',attempts=attempts+1,payload=?2,error=NULL WHERE operation_id=?1 AND state='pending' AND local_version IS NOT NULL AND sequence=(SELECT MIN(sequence) FROM operations WHERE state NOT IN ('applied','skipped','superseded')) AND NOT EXISTS(SELECT 1 FROM settings WHERE key='skip' AND json_extract(value,'$.operation_id')=?1)", params![id,bytes])?;
        ensure!(
            changed == 1,
            "Only the locally materialized queue head may start replay"
        );
        Ok(())
    }
    pub fn block(&self, id: &str, state: &str, reason: &str) -> Result<()> {
        ensure!(
            matches!(
                state,
                "blocked" | "conflict" | "outcome_unknown" | "attempting"
            ),
            "Invalid queue failure state"
        );
        self.lock()?.execute("UPDATE operations SET state=?2,error=?3 WHERE operation_id=?1 AND state NOT IN ('applied','skipped','superseded')", params![id,state,reason.chars().take(2048).collect::<String>()])?;
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
        tx.execute("UPDATE operations SET state='applied',receipt=?2,payload=x'6e756c6c',bytes=0,error=NULL,coalesce_key=NULL WHERE operation_id=?1", params![id,serde_json::to_vec(receipt)?])?;
        tx.commit()?;
        Ok(())
    }
    pub fn retry(&self, id: &str) -> Result<()> {
        let db = self.lock()?;
        Self::authorized(&db)?;
        // An attempted request stays immutable. The server receipt reconciles it.
        let changed = db.execute("UPDATE operations SET state=CASE WHEN attempts=0 THEN 'pending' ELSE 'attempting' END,error=NULL WHERE operation_id=?1 AND state IN ('blocked','conflict','outcome_unknown')", [id])?;
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
        let db = self.lock()?;
        let (pending_count,pending_bytes,oldest_at) = db.query_row("SELECT COUNT(*),COALESCE(SUM(bytes),0),MIN(first_queued_at) FROM operations WHERE state NOT IN ('applied','skipped','superseded')", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        let quarantined = Self::authorized(&db).is_err();
        let mirror_error = db
            .query_row(
                "SELECT value FROM settings WHERE key LIKE 'mirror_error:%' ORDER BY key LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        drop(db);
        let mut head = self.head()?;
        // Management status never includes user rows or file bodies.
        if let Some(head) = &mut head {
            head.payload = Value::Null;
        }
        Ok(OutboxStatus {
            scope: self.scope.clone(),
            quarantined,
            pending_count,
            pending_bytes,
            oldest_at,
            head,
            mirror_error,
        })
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
}
