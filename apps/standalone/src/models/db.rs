use super::stats::LatencyHistogram;
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::{
    DigestAlgorithm, ModelAssetDescriptor, ModelAssetDigest, ModelAssetState, ModelBackend,
    ModelConsumer, ModelRuntime, ModelSettings, ModelSpec, Residency,
};
use rusqlite::{Connection, OptionalExtension, Row, Transaction, TransactionBehavior, params};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x464c_4d53;
const OWNER_ID_MAX_LEN: usize = 256;

/// Applied in order inside one transaction; `PRAGMA user_version` counts the applied
/// entries. Append new migrations and never edit a released one.
pub const MIGRATIONS: &[&str] = &[
    "
CREATE TABLE assets (
    digest TEXT PRIMARY KEY
        CHECK (length(digest) = 71 AND (digest GLOB 'sha256/*' OR digest GLOB 'blake3/*')),
    size INTEGER NOT NULL CHECK (size > 0),
    file_name TEXT NOT NULL,
    present_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL
);
CREATE TABLE asset_refs (
    digest TEXT NOT NULL,
    owner_kind TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    PRIMARY KEY (digest, owner_kind, owner_id)
);
CREATE INDEX asset_refs_by_owner ON asset_refs (owner_kind, owner_id);
CREATE TABLE jobs (
    digest TEXT PRIMARY KEY,
    job_id TEXT NOT NULL UNIQUE,
    size INTEGER NOT NULL CHECK (size > 0),
    file_name TEXT NOT NULL,
    sources TEXT NOT NULL,
    state TEXT NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0 CHECK (attempts >= 0),
    operation_ids TEXT NOT NULL DEFAULT '[]',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at INTEGER NOT NULL
);
",
    "
CREATE TABLE hosted_models (
    id TEXT PRIMARY KEY,
    spec TEXT NOT NULL,
    settings TEXT NOT NULL,
    residency TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK (revision > 0),
    origin TEXT NOT NULL CHECK (origin IN ('user', 'placement')),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_used_at INTEGER NOT NULL
);
CREATE TABLE runtimes (
    runtime TEXT NOT NULL,
    backend TEXT NOT NULL,
    build TEXT NOT NULL,
    entrypoint TEXT NOT NULL,
    size INTEGER NOT NULL CHECK (size >= 0),
    needs_fallback INTEGER NOT NULL CHECK (needs_fallback IN (0, 1)),
    installed_at INTEGER NOT NULL,
    PRIMARY KEY (runtime, backend)
);
CREATE TABLE stats_requests (
    id INTEGER PRIMARY KEY,
    at INTEGER NOT NULL,
    model_id TEXT NOT NULL,
    principal_kind TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    status INTEGER NOT NULL,
    prompt_tokens INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    cached_tokens INTEGER NOT NULL,
    ttft_ms INTEGER,
    duration_ms INTEGER NOT NULL,
    decode_ms INTEGER NOT NULL,
    queue_ms INTEGER NOT NULL,
    request_bytes INTEGER NOT NULL,
    response_bytes INTEGER NOT NULL
);
CREATE INDEX stats_requests_by_time ON stats_requests (at);
CREATE TABLE stats_rollups (
    step INTEGER NOT NULL CHECK (step IN (60, 3600)),
    bucket INTEGER NOT NULL,
    model_id TEXT NOT NULL,
    principal_kind TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    requests INTEGER NOT NULL,
    errors INTEGER NOT NULL,
    prompt_tokens INTEGER NOT NULL,
    completion_tokens INTEGER NOT NULL,
    cached_tokens INTEGER NOT NULL,
    decode_ms INTEGER NOT NULL,
    ttft TEXT NOT NULL,
    queue TEXT NOT NULL,
    PRIMARY KEY (step, bucket, model_id, principal_kind, principal_id)
);
",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OwnerKind {
    Placement,
    HostedModel,
    User,
    /// A runtime pack archive while it is installed.
    Runtime,
    /// A deploy whose placement has not started yet; it lapses after the store's grace period.
    Deploy,
}

impl OwnerKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Placement => "placement",
            Self::HostedModel => "hosted_model",
            Self::User => "user",
            Self::Runtime => "runtime",
            Self::Deploy => "deploy",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "placement" => Self::Placement,
            "hosted_model" => Self::HostedModel,
            "user" => Self::User,
            "runtime" => Self::Runtime,
            "deploy" => Self::Deploy,
            other => bail!("Unknown model asset owner kind {other}"),
        })
    }
}

/// Keeps a blob out of garbage collection while it exists.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetOwner {
    pub kind: OwnerKind,
    pub id: String,
}

impl AssetOwner {
    pub fn new(kind: OwnerKind, id: impl Into<String>) -> Result<Self> {
        let id = id.into();
        ensure!(
            !id.is_empty() && id.len() <= OWNER_ID_MAX_LEN && !id.chars().any(char::is_control),
            "Invalid {} owner id for a model asset reference",
            kind.as_str()
        );
        Ok(Self { kind, id })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetRecord {
    pub digest: ModelAssetDigest,
    pub size: u64,
    pub file_name: String,
    pub present_at: i64,
    pub last_used_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobRecord {
    pub job_id: String,
    pub asset: ModelAssetDescriptor,
    pub state: ModelAssetState,
    pub attempts: u32,
    pub operation_ids: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub fn digest_from_key(key: &str) -> Result<ModelAssetDigest> {
    let (algorithm, hex) = key
        .split_once('/')
        .with_context(|| format!("Model asset digest key {key} has no algorithm"))?;
    let algorithm = match algorithm {
        "sha256" => DigestAlgorithm::Sha256,
        "blake3" => DigestAlgorithm::Blake3,
        other => bail!("Unknown model asset digest algorithm {other}"),
    };
    let digest = ModelAssetDigest {
        algorithm,
        hex: hex.to_owned(),
    };
    digest
        .validate()
        .with_context(|| format!("Invalid model asset digest key {key}"))?;
    Ok(digest)
}

fn size_column(size: u64) -> Result<i64> {
    i64::try_from(size)
        .with_context(|| format!("Model asset size {size} exceeds the database range"))
}

type AssetRow = (String, i64, String, i64, i64);
type JobRow = (
    String,
    String,
    i64,
    String,
    String,
    String,
    i64,
    String,
    i64,
    i64,
);

fn asset_from_row(row: &Row) -> rusqlite::Result<AssetRow> {
    AssetRow::try_from(row)
}

fn job_from_row(row: &Row) -> rusqlite::Result<JobRow> {
    JobRow::try_from(row)
}

fn asset_record(row: AssetRow) -> Result<AssetRecord> {
    let (digest, size, file_name, present_at, last_used_at) = row;
    Ok(AssetRecord {
        digest: digest_from_key(&digest)?,
        size: u64::try_from(size)
            .with_context(|| format!("Stored size of {digest} is negative"))?,
        file_name,
        present_at,
        last_used_at,
    })
}

fn job_record(row: JobRow) -> Result<JobRecord> {
    let (digest, job_id, size, file_name, sources, state, attempts, operations, created, updated) =
        row;
    let context = || format!("Read model asset job {digest}");
    Ok(JobRecord {
        job_id,
        asset: ModelAssetDescriptor {
            digest: digest_from_key(&digest)?,
            size: u64::try_from(size).with_context(context)?,
            file_name,
            sources: serde_json::from_str(&sources).with_context(context)?,
        },
        state: serde_json::from_str(&state).with_context(context)?,
        attempts: u32::try_from(attempts).with_context(context)?,
        operation_ids: serde_json::from_str(&operations).with_context(context)?,
        created_at: created,
        updated_at: updated,
    })
}

const JOB_COLUMNS: &str = "digest, job_id, size, file_name, sources, state, attempts, operation_ids, created_at, updated_at";
const ASSET_COLUMNS: &str = "digest, size, file_name, present_at, last_used_at";

/// How many migrations the database already carries; refuses foreign or newer databases.
fn applied_migrations(transaction: &Transaction, path: &Path) -> Result<usize> {
    let version: i64 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let application_id: i64 =
        transaction.pragma_query_value(None, "application_id", |row| row.get(0))?;
    ensure!(
        application_id == APPLICATION_ID || (application_id == 0 && version == 0),
        "Model database {} belongs to another application (id {application_id:#x})",
        path.display()
    );
    usize::try_from(version)
        .ok()
        .filter(|applied| *applied <= MIGRATIONS.len())
        .with_context(|| {
            format!(
                "Model database {} has schema {version}, newer than the {} this agent knows",
                path.display(),
                MIGRATIONS.len()
            )
        })
}

fn migrate(connection: &mut Connection, path: &Path) -> Result<()> {
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let applied = applied_migrations(&transaction, path)?;
    for (index, migration) in MIGRATIONS.iter().enumerate().skip(applied) {
        transaction
            .execute_batch(migration)
            .with_context(|| format!("Apply model database migration {}", index + 1))?;
    }
    transaction.execute_batch(&format!(
        "PRAGMA user_version = {}; PRAGMA application_id = {APPLICATION_ID};",
        MIGRATIONS.len()
    ))?;
    transaction
        .commit()
        .with_context(|| format!("Commit the schema of model database {}", path.display()))
}

/// `models.sqlite`, separate from the management database.
pub struct ModelsDb {
    connection: Connection,
}

impl ModelsDb {
    pub fn open(path: &Path) -> Result<Self> {
        let mut connection = Connection::open(path)
            .with_context(|| format!("Open model database {}", path.display()))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        migrate(&mut connection, path)?;
        connection.pragma_update(None, "journal_mode", "WAL")?;
        connection.pragma_update(None, "synchronous", "FULL")?;
        Ok(Self { connection })
    }

    pub fn asset(&self, digest: &ModelAssetDigest) -> Result<Option<AssetRecord>> {
        self.connection
            .query_row(
                &format!("SELECT {ASSET_COLUMNS} FROM assets WHERE digest = ?1"),
                [digest.store_key()],
                asset_from_row,
            )
            .optional()?
            .map(asset_record)
            .transpose()
    }

    pub fn assets(&self) -> Result<Vec<AssetRecord>> {
        self.select_assets(
            &format!("SELECT {ASSET_COLUMNS} FROM assets ORDER BY digest"),
            [],
        )
    }

    /// Assets without a reference whose last use is at or before `unused_since`, oldest first.
    pub fn unreferenced_assets(&self, unused_since: i64) -> Result<Vec<AssetRecord>> {
        self.select_assets(
            &format!(
                "SELECT {ASSET_COLUMNS} FROM assets WHERE last_used_at <= ?1 AND NOT EXISTS \
                 (SELECT 1 FROM asset_refs WHERE asset_refs.digest = assets.digest) \
                 ORDER BY last_used_at, digest"
            ),
            [unused_since],
        )
    }

    fn select_assets(&self, sql: &str, params: impl rusqlite::Params) -> Result<Vec<AssetRecord>> {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params, asset_from_row)?;
        rows.map(|row| asset_record(row?)).collect()
    }

    pub fn put_asset(&self, record: &AssetRecord) -> Result<()> {
        self.connection.execute(
            &format!("INSERT OR REPLACE INTO assets ({ASSET_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5)"),
            params![
                record.digest.store_key(),
                size_column(record.size)?,
                record.file_name,
                record.present_at,
                record.last_used_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_asset(&self, digest: &ModelAssetDigest) -> Result<()> {
        self.connection
            .execute("DELETE FROM assets WHERE digest = ?1", [digest.store_key()])?;
        Ok(())
    }

    pub fn touch_asset(&self, digest: &ModelAssetDigest, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE assets SET last_used_at = max(last_used_at, ?2) WHERE digest = ?1",
            params![digest.store_key(), now],
        )?;
        Ok(())
    }

    pub fn stored_bytes(&self) -> Result<u64> {
        let bytes: i64 =
            self.connection
                .query_row("SELECT COALESCE(SUM(size), 0) FROM assets", [], |row| {
                    row.get(0)
                })?;
        u64::try_from(bytes).context("Stored model asset bytes are negative")
    }

    pub fn add_ref(&self, digest: &ModelAssetDigest, owner: &AssetOwner, now: i64) -> Result<()> {
        self.connection.execute(
            "INSERT OR IGNORE INTO asset_refs (digest, owner_kind, owner_id, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![digest.store_key(), owner.kind.as_str(), owner.id, now],
        )?;
        Ok(())
    }

    /// Adds the reference, or moves the creation time of the existing one to `now`.
    pub fn renew_ref(&self, digest: &ModelAssetDigest, owner: &AssetOwner, now: i64) -> Result<()> {
        self.connection.execute(
            "INSERT INTO asset_refs (digest, owner_kind, owner_id, created_at) \
             VALUES (?1, ?2, ?3, ?4) ON CONFLICT (digest, owner_kind, owner_id) \
             DO UPDATE SET created_at = excluded.created_at",
            params![digest.store_key(), owner.kind.as_str(), owner.id, now],
        )?;
        Ok(())
    }

    pub fn remove_ref(&self, digest: &ModelAssetDigest, owner: &AssetOwner) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM asset_refs WHERE digest = ?1 AND owner_kind = ?2 AND owner_id = ?3",
            params![digest.store_key(), owner.kind.as_str(), owner.id],
        )? > 0)
    }

    /// Drops the references of `kind` created at or before `cutoff`; answers how many went.
    pub fn expire_refs(&self, kind: OwnerKind, cutoff: i64) -> Result<usize> {
        Ok(self.connection.execute(
            "DELETE FROM asset_refs WHERE owner_kind = ?1 AND created_at <= ?2",
            params![kind.as_str(), cutoff],
        )?)
    }

    pub fn refs(&self, digest: &ModelAssetDigest) -> Result<Vec<AssetOwner>> {
        let mut statement = self.connection.prepare(
            "SELECT owner_kind, owner_id FROM asset_refs WHERE digest = ?1 \
             ORDER BY owner_kind, owner_id",
        )?;
        let rows = statement.query_map([digest.store_key()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (kind, id) = row?;
            AssetOwner::new(OwnerKind::parse(&kind)?, id)
        })
        .collect()
    }

    /// Every reference one kind of owner holds, ordered by owner.
    pub fn refs_of_kind(&self, kind: OwnerKind) -> Result<Vec<(ModelAssetDigest, AssetOwner)>> {
        let mut statement = self.connection.prepare(
            "SELECT digest, owner_id FROM asset_refs WHERE owner_kind = ?1 \
             ORDER BY owner_id, digest",
        )?;
        let rows = statement.query_map([kind.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (digest, id) = row?;
            Ok((digest_from_key(&digest)?, AssetOwner::new(kind, id)?))
        })
        .collect()
    }

    pub fn job(&self, digest: &ModelAssetDigest) -> Result<Option<JobRecord>> {
        self.connection
            .query_row(
                &format!("SELECT {JOB_COLUMNS} FROM jobs WHERE digest = ?1"),
                [digest.store_key()],
                job_from_row,
            )
            .optional()?
            .map(job_record)
            .transpose()
    }

    pub fn jobs(&self) -> Result<Vec<JobRecord>> {
        let mut statement = self.connection.prepare(&format!(
            "SELECT {JOB_COLUMNS} FROM jobs ORDER BY created_at, digest"
        ))?;
        let rows = statement.query_map([], job_from_row)?;
        rows.map(|row| job_record(row?)).collect()
    }

    pub fn put_job(&self, record: &JobRecord) -> Result<()> {
        self.connection.execute(
            &format!(
                "INSERT OR REPLACE INTO jobs ({JOB_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)"
            ),
            params![
                record.asset.digest.store_key(),
                record.job_id,
                size_column(record.asset.size)?,
                record.asset.file_name,
                serde_json::to_string(&record.asset.sources)?,
                serde_json::to_string(&record.state)?,
                record.attempts,
                serde_json::to_string(&record.operation_ids)?,
                record.created_at,
                record.updated_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_job(&self, digest: &ModelAssetDigest) -> Result<()> {
        self.connection
            .execute("DELETE FROM jobs WHERE digest = ?1", [digest.store_key()])?;
        Ok(())
    }

    pub fn setting(&self, key: &str) -> Result<Option<Value>> {
        self.connection
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
                row.get::<_, String>(0)
            })
            .optional()?
            .map(|value| {
                serde_json::from_str(&value).with_context(|| format!("Read model setting {key}"))
            })
            .transpose()
    }

    pub fn set_setting(&self, key: &str, value: &Value, now: i64) -> Result<()> {
        self.connection.execute(
            "INSERT OR REPLACE INTO settings (key, value, updated_at) VALUES (?1, ?2, ?3)",
            params![key, serde_json::to_string(value)?, now],
        )?;
        Ok(())
    }
}

/// Who created a hosted model: the Models tab, or a placement that needed a Bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelOrigin {
    User,
    Placement,
}

impl ModelOrigin {
    fn as_str(&self) -> &str {
        match self {
            Self::User => "user",
            Self::Placement => "placement",
        }
    }

    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "user" => Self::User,
            "placement" => Self::Placement,
            other => bail!("Unknown hosted model origin {other}"),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostedModelRecord {
    pub id: String,
    pub spec: ModelSpec,
    pub settings: ModelSettings,
    pub residency: Residency,
    pub revision: u64,
    pub origin: ModelOrigin,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_used_at: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeRecord {
    pub runtime: ModelRuntime,
    pub backend: ModelBackend,
    pub build: String,
    /// Relative to the install directory.
    pub entrypoint: String,
    pub size: u64,
    /// The entrypoint loads only with the pack's `fallback/` libraries on the loader path.
    pub needs_fallback: bool,
    pub installed_at: i64,
}

/// One gateway request; counts and timings only, never prompt or response text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestRecord {
    pub at: i64,
    pub model_id: String,
    pub consumer: ModelConsumer,
    pub status: u16,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub ttft_ms: Option<u32>,
    pub duration_ms: u32,
    pub decode_ms: u32,
    pub queue_ms: u32,
    pub request_bytes: u64,
    pub response_bytes: u64,
}

/// Requests of one model and consumer within `[bucket, bucket + step)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollupRecord {
    pub step: i64,
    pub bucket: i64,
    pub model_id: String,
    pub consumer: ModelConsumer,
    pub requests: u64,
    pub errors: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub decode_ms: u64,
    pub ttft: LatencyHistogram,
    pub queue: LatencyHistogram,
}

fn wire_name<T: Serialize>(value: &T) -> Result<String> {
    match serde_json::to_value(value)? {
        Value::String(name) => Ok(name),
        other => bail!("Expected a string wire name, not {other}"),
    }
}

fn from_wire<T: DeserializeOwned>(name: &str) -> Result<T> {
    serde_json::from_value(Value::String(name.to_owned()))
        .with_context(|| format!("Unknown stored value {name}"))
}

fn counter(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn consumer_columns(consumer: &ModelConsumer) -> (&str, &str) {
    match consumer {
        ModelConsumer::Owner => ("owner", ""),
        ModelConsumer::Grant { grant_id } => ("grant", grant_id),
        ModelConsumer::Placement { placement_id } => ("placement", placement_id),
    }
}

fn consumer_from(kind: &str, id: String) -> Result<ModelConsumer> {
    Ok(match kind {
        "owner" => ModelConsumer::Owner,
        "grant" => ModelConsumer::Grant { grant_id: id },
        "placement" => ModelConsumer::Placement { placement_id: id },
        other => bail!("Unknown model consumer kind {other}"),
    })
}

const HOSTED_MODEL_COLUMNS: &str =
    "id, spec, settings, residency, revision, origin, created_at, updated_at, last_used_at";
const RUNTIME_COLUMNS: &str =
    "runtime, backend, build, entrypoint, size, needs_fallback, installed_at";
const REQUEST_COLUMNS: &str = "at, model_id, principal_kind, principal_id, status, prompt_tokens, \
     completion_tokens, cached_tokens, ttft_ms, duration_ms, decode_ms, queue_ms, request_bytes, \
     response_bytes";
const ROLLUP_COLUMNS: &str = "step, bucket, model_id, principal_kind, principal_id, requests, \
     errors, prompt_tokens, completion_tokens, cached_tokens, decode_ms, ttft, queue";

type HostedModelRow = (String, String, String, String, i64, String, i64, i64, i64);
type RuntimeRow = (String, String, String, String, i64, bool, i64);
type RequestRow = (
    i64,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    Option<i64>,
    i64,
    i64,
    i64,
    i64,
    i64,
);
type RollupRow = (
    i64,
    i64,
    String,
    String,
    String,
    i64,
    i64,
    i64,
    i64,
    i64,
    i64,
    String,
    String,
);

fn millis(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

fn hosted_model_from_row(row: &Row) -> Result<HostedModelRecord> {
    let (id, spec, settings, residency, revision, origin, created_at, updated_at, last_used_at) =
        HostedModelRow::try_from(row)?;
    let context = || format!("Read hosted model {id}");
    Ok(HostedModelRecord {
        spec: serde_json::from_str(&spec).with_context(context)?,
        settings: serde_json::from_str(&settings).with_context(context)?,
        residency: serde_json::from_str(&residency).with_context(context)?,
        revision: count(revision),
        origin: ModelOrigin::parse(&origin)?,
        created_at,
        updated_at,
        last_used_at,
        id,
    })
}

fn runtime_from_row(row: &Row) -> Result<RuntimeRecord> {
    let (runtime, backend, build, entrypoint, size, needs_fallback, installed_at) =
        RuntimeRow::try_from(row)?;
    Ok(RuntimeRecord {
        runtime: from_wire(&runtime)?,
        backend: from_wire(&backend)?,
        build,
        entrypoint,
        size: count(size),
        needs_fallback,
        installed_at,
    })
}

fn request_from_row(row: &Row) -> Result<RequestRecord> {
    let row = RequestRow::try_from(row)?;
    Ok(RequestRecord {
        at: row.0,
        model_id: row.1,
        consumer: consumer_from(&row.2, row.3)?,
        status: u16::try_from(row.4).unwrap_or(0),
        prompt_tokens: count(row.5),
        completion_tokens: count(row.6),
        cached_tokens: count(row.7),
        ttft_ms: row.8.map(millis),
        duration_ms: millis(row.9),
        decode_ms: millis(row.10),
        queue_ms: millis(row.11),
        request_bytes: count(row.12),
        response_bytes: count(row.13),
    })
}

fn rollup_from_row(row: &Row) -> Result<RollupRecord> {
    let row = RollupRow::try_from(row)?;
    Ok(RollupRecord {
        step: row.0,
        bucket: row.1,
        model_id: row.2,
        consumer: consumer_from(&row.3, row.4)?,
        requests: count(row.5),
        errors: count(row.6),
        prompt_tokens: count(row.7),
        completion_tokens: count(row.8),
        cached_tokens: count(row.9),
        decode_ms: count(row.10),
        ttft: serde_json::from_str(&row.11)?,
        queue: serde_json::from_str(&row.12)?,
    })
}

impl ModelsDb {
    fn select<T>(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        read: impl Fn(&Row) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut statement = self.connection.prepare(sql)?;
        let mut rows = statement.query(params)?;
        let mut values = Vec::new();
        while let Some(row) = rows.next()? {
            values.push(read(row)?);
        }
        Ok(values)
    }

    pub fn hosted_model(&self, id: &str) -> Result<Option<HostedModelRecord>> {
        Ok(self
            .select(
                &format!("SELECT {HOSTED_MODEL_COLUMNS} FROM hosted_models WHERE id = ?1"),
                [id],
                hosted_model_from_row,
            )?
            .pop())
    }

    /// Ordered by id.
    pub fn hosted_models(&self) -> Result<Vec<HostedModelRecord>> {
        self.select(
            &format!("SELECT {HOSTED_MODEL_COLUMNS} FROM hosted_models ORDER BY id"),
            [],
            hosted_model_from_row,
        )
    }

    pub fn put_hosted_model(&self, record: &HostedModelRecord) -> Result<()> {
        self.connection.execute(
            &format!(
                "INSERT OR REPLACE INTO hosted_models ({HOSTED_MODEL_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ),
            params![
                record.id,
                serde_json::to_string(&record.spec)?,
                serde_json::to_string(&record.settings)?,
                serde_json::to_string(&record.residency)?,
                counter(record.revision),
                record.origin.as_str(),
                record.created_at,
                record.updated_at,
                record.last_used_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_hosted_model(&self, id: &str) -> Result<bool> {
        Ok(self
            .connection
            .execute("DELETE FROM hosted_models WHERE id = ?1", [id])?
            > 0)
    }

    pub fn touch_hosted_model(&self, id: &str, now: i64) -> Result<()> {
        self.connection.execute(
            "UPDATE hosted_models SET last_used_at = max(last_used_at, ?2) WHERE id = ?1",
            params![id, now],
        )?;
        Ok(())
    }

    pub fn runtime(
        &self,
        runtime: ModelRuntime,
        backend: ModelBackend,
    ) -> Result<Option<RuntimeRecord>> {
        Ok(self
            .select(
                &format!(
                    "SELECT {RUNTIME_COLUMNS} FROM runtimes WHERE runtime = ?1 AND backend = ?2"
                ),
                params![wire_name(&runtime)?, wire_name(&backend)?],
                runtime_from_row,
            )?
            .pop())
    }

    pub fn runtimes(&self) -> Result<Vec<RuntimeRecord>> {
        self.select(
            &format!("SELECT {RUNTIME_COLUMNS} FROM runtimes ORDER BY runtime, backend"),
            [],
            runtime_from_row,
        )
    }

    pub fn put_runtime(&self, record: &RuntimeRecord) -> Result<()> {
        self.connection.execute(
            &format!(
                "INSERT OR REPLACE INTO runtimes ({RUNTIME_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
            ),
            params![
                wire_name(&record.runtime)?,
                wire_name(&record.backend)?,
                record.build,
                record.entrypoint,
                counter(record.size),
                record.needs_fallback,
                record.installed_at
            ],
        )?;
        Ok(())
    }

    pub fn delete_runtime(&self, runtime: ModelRuntime, backend: ModelBackend) -> Result<bool> {
        Ok(self.connection.execute(
            "DELETE FROM runtimes WHERE runtime = ?1 AND backend = ?2",
            params![wire_name(&runtime)?, wire_name(&backend)?],
        )? > 0)
    }

    /// One transaction for the whole batch.
    pub fn insert_requests(&self, rows: &[RequestRecord]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        {
            let mut statement = transaction.prepare_cached(&format!(
                "INSERT INTO stats_requests ({REQUEST_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)"
            ))?;
            for row in rows {
                let (kind, id) = consumer_columns(&row.consumer);
                statement.execute(params![
                    row.at,
                    row.model_id,
                    kind,
                    id,
                    row.status,
                    counter(row.prompt_tokens),
                    counter(row.completion_tokens),
                    counter(row.cached_tokens),
                    row.ttft_ms,
                    row.duration_ms,
                    row.decode_ms,
                    row.queue_ms,
                    counter(row.request_bytes),
                    counter(row.response_bytes)
                ])?;
            }
        }
        transaction
            .commit()
            .context("Commit model gateway request statistics")
    }

    /// Requests started within `[from, to)`, of one model or all.
    pub fn requests_between(
        &self,
        from: i64,
        to: i64,
        model_id: Option<&str>,
    ) -> Result<Vec<RequestRecord>> {
        self.select(
            &format!(
                "SELECT {REQUEST_COLUMNS} FROM stats_requests \
                 WHERE at >= ?1 AND at < ?2 AND (?3 IS NULL OR model_id = ?3) ORDER BY at, id"
            ),
            params![from, to, model_id],
            request_from_row,
        )
    }

    /// Replaces the rollups of the same step, bucket, model and consumer.
    pub fn put_rollups(&self, rows: &[RollupRecord]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        {
            let mut statement = transaction.prepare_cached(&format!(
                "INSERT OR REPLACE INTO stats_rollups ({ROLLUP_COLUMNS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
            ))?;
            for row in rows {
                let (kind, id) = consumer_columns(&row.consumer);
                statement.execute(params![
                    row.step,
                    row.bucket,
                    row.model_id,
                    kind,
                    id,
                    counter(row.requests),
                    counter(row.errors),
                    counter(row.prompt_tokens),
                    counter(row.completion_tokens),
                    counter(row.cached_tokens),
                    counter(row.decode_ms),
                    serde_json::to_string(&row.ttft)?,
                    serde_json::to_string(&row.queue)?
                ])?;
            }
        }
        transaction
            .commit()
            .context("Commit model gateway statistics rollups")
    }

    /// Rollups of `step` whose bucket starts within `[from, to)`, of one model or all.
    pub fn rollups_between(
        &self,
        step: i64,
        from: i64,
        to: i64,
        model_id: Option<&str>,
    ) -> Result<Vec<RollupRecord>> {
        self.select(
            &format!(
                "SELECT {ROLLUP_COLUMNS} FROM stats_rollups WHERE step = ?1 AND bucket >= ?2 \
                 AND bucket < ?3 AND (?4 IS NULL OR model_id = ?4) ORDER BY bucket"
            ),
            params![step, from, to, model_id],
            rollup_from_row,
        )
    }

    /// Drops requests started before `requests_before` and rollups of each step whose
    /// bucket starts before its cutoff.
    pub fn prune_stats(&self, requests_before: i64, rollups_before: &[(i64, i64)]) -> Result<()> {
        let transaction = self.connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM stats_requests WHERE at < ?1",
            [requests_before],
        )?;
        for (step, before) in rollups_before {
            transaction.execute(
                "DELETE FROM stats_rollups WHERE step = ?1 AND bucket < ?2",
                [step, before],
            )?;
        }
        transaction
            .commit()
            .context("Prune model gateway statistics")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn digest(fill: char) -> ModelAssetDigest {
        ModelAssetDigest {
            algorithm: DigestAlgorithm::Blake3,
            hex: fill.to_string().repeat(64),
        }
    }

    fn job(digest: ModelAssetDigest) -> JobRecord {
        JobRecord {
            job_id: uuid::Uuid::new_v4().to_string(),
            asset: ModelAssetDescriptor {
                digest,
                size: 42,
                file_name: "model.gguf".into(),
                sources: vec!["https://cdn.flow-like.com/bits/a".into()],
            },
            state: ModelAssetState::Fetching {
                source_index: 0,
                bytes: 7,
            },
            attempts: 2,
            operation_ids: vec!["op-1".into()],
            created_at: 10,
            updated_at: 11,
        }
    }

    #[test]
    fn migrations_apply_once_and_refuse_newer_or_foreign_databases() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("models.sqlite");
        drop(ModelsDb::open(&path)?);
        let reopened = ModelsDb::open(&path)?;
        let version: i64 = reopened
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))?;
        assert_eq!(version, MIGRATIONS.len() as i64);
        reopened
            .connection
            .pragma_update(None, "user_version", version + 1)?;
        drop(reopened);
        assert!(ModelsDb::open(&path).is_err());

        let foreign = directory.path().join("foreign.sqlite");
        Connection::open(&foreign)?.pragma_update(None, "application_id", 7)?;
        assert!(ModelsDb::open(&foreign).is_err());
        Ok(())
    }

    fn database() -> (tempfile::TempDir, ModelsDb) {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let db = ModelsDb::open(&directory.path().join("models.sqlite")).expect("a database");
        (directory, db)
    }

    fn stored_asset(db: &ModelsDb) -> AssetRecord {
        let asset = AssetRecord {
            digest: digest('b'),
            size: 5,
            file_name: "weights.gguf".into(),
            present_at: 100,
            last_used_at: 100,
        };
        db.put_asset(&asset).expect("a stored asset");
        asset
    }

    #[test]
    fn jobs_round_trip() -> Result<()> {
        let (_directory, db) = database();
        let record = job(digest('a'));
        db.put_job(&record)?;
        assert_eq!(db.job(&record.asset.digest)?, Some(record.clone()));
        assert_eq!(db.jobs()?, vec![record.clone()]);
        db.delete_job(&record.asset.digest)?;
        assert!(db.jobs()?.is_empty());
        Ok(())
    }

    #[test]
    fn assets_round_trip_and_keep_their_latest_use() -> Result<()> {
        let (_directory, db) = database();
        let asset = stored_asset(&db);
        db.touch_asset(&asset.digest, 90)?;
        assert_eq!(db.asset(&asset.digest)?, Some(asset.clone()));
        assert_eq!(db.stored_bytes()?, 5);
        Ok(())
    }

    #[test]
    fn referenced_assets_are_not_collectable() -> Result<()> {
        let (_directory, db) = database();
        let asset = stored_asset(&db);
        let collectable = |since| db.unreferenced_assets(since).expect("collectable assets");
        let owner = AssetOwner::new(OwnerKind::Placement, "placement-1")?;
        db.add_ref(&asset.digest, &owner, 100)?;
        db.add_ref(&asset.digest, &owner, 101)?;
        assert_eq!(db.refs(&asset.digest)?, vec![owner.clone()]);
        assert!(collectable(i64::MAX).is_empty());
        assert!(db.remove_ref(&asset.digest, &owner)?);
        assert_eq!(collectable(100), vec![asset.clone()]);
        assert!(collectable(99).is_empty());
        Ok(())
    }

    #[test]
    fn references_list_by_owner_kind() -> Result<()> {
        let (_directory, db) = database();
        let placement = |id| AssetOwner::new(OwnerKind::Placement, id).expect("an owner");
        let model = AssetOwner::new(OwnerKind::HostedModel, "qwen")?;
        db.add_ref(&digest('b'), &placement("worker"), 1)?;
        db.add_ref(&digest('a'), &placement("api"), 1)?;
        db.add_ref(&digest('c'), &model, 1)?;
        assert_eq!(
            db.refs_of_kind(OwnerKind::Placement)?,
            vec![
                (digest('a'), placement("api")),
                (digest('b'), placement("worker"))
            ]
        );
        assert_eq!(
            db.refs_of_kind(OwnerKind::HostedModel)?,
            vec![(digest('c'), model)]
        );
        assert!(db.refs_of_kind(OwnerKind::User)?.is_empty());
        Ok(())
    }

    #[test]
    fn renewed_references_expire_from_their_last_renewal() -> Result<()> {
        let (_directory, db) = database();
        let digest = stored_asset(&db).digest;
        let deploy = AssetOwner::new(OwnerKind::Deploy, "project")?;
        let placement = AssetOwner::new(OwnerKind::Placement, "placement-1")?;
        for (owner, at) in [(&deploy, 100), (&deploy, 200), (&placement, 100)] {
            db.renew_ref(&digest, owner, at).expect("a reference");
        }
        assert_eq!(db.expire_refs(OwnerKind::Deploy, 199)?, 0);
        assert_eq!(db.expire_refs(OwnerKind::Deploy, 200)?, 1);
        assert_eq!(db.refs(&digest)?, vec![placement]);
        Ok(())
    }

    #[test]
    fn settings_round_trip() -> Result<()> {
        let (_directory, db) = database();
        db.set_setting("budget", &serde_json::json!({"bytes": 1}), 1)?;
        assert_eq!(db.setting("budget")?, Some(serde_json::json!({"bytes": 1})));
        assert_eq!(db.setting("missing")?, None);
        Ok(())
    }

    #[test]
    fn keys_and_owners_are_validated() {
        assert!(digest_from_key(&format!("sha256/{}", "0".repeat(64))).is_ok());
        assert!(digest_from_key(&format!("md5/{}", "0".repeat(64))).is_err());
        assert!(digest_from_key("blake3/abc").is_err());
        assert!(AssetOwner::new(OwnerKind::User, "").is_err());
        assert!(AssetOwner::new(OwnerKind::User, "a\nb").is_err());
        assert!(AssetOwner::new(OwnerKind::User, "x".repeat(257)).is_err());
    }
}
