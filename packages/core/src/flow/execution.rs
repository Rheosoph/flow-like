use super::board::{ExecutionStage, LayerType};
use super::event::Event;
use super::oauth::OAuthToken;
use super::{board::Board, node::NodeState, variable::Variable};
use crate::app::AppVisibility;
use crate::credentials::SharedCredentials;
use crate::flow::execution::internal_node::ExecutionTarget;
use crate::profile::Profile;
use crate::state::FlowLikeState;
use ahash::{AHashMap, AHashSet, AHasher};
use context::ExecutionContext;
use flow_like_storage::arrow_array::{RecordBatch, RecordBatchIterator, RecordBatchReader};
use flow_like_storage::arrow_schema::{FieldRef, SchemaRef};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::index::scalar::BitmapIndexBuilder;
use flow_like_storage::serde_arrow::schema::{SchemaLike, TracingOptions};
use flow_like_storage::{Path, serde_arrow};
use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
use flow_like_types::intercom::InterComCallback;
use flow_like_types::json::to_vec;
use flow_like_types::sync::{Mutex, RwLock};
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::utils::ptr_key;
use flow_like_types::{Cacheable, anyhow, create_id};
use flow_like_types::{Context, Value};
use futures::StreamExt;
use futures::future::BoxFuture;
use internal_node::InternalNode;
use internal_pin::InternalPin;
use log::LogMessage;
use num_cpus;
use once_cell::sync::Lazy;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::hash::Hasher;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};
use std::{
    sync::{Arc, Weak},
    time::SystemTime,
};
use trace::Trace;

pub mod context;
pub mod internal_node;
pub mod internal_pin;
pub mod log;
pub mod trace;
pub mod user_context;

pub use user_context::{RoleContext, UserExecutionContext};

const USE_DEPENDENCY_GRAPH: bool = false;
const RUN_LOCK_TIMEOUT: Duration = Duration::from_secs(3);
pub const DEFAULT_RUN_LOG_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD: usize = 500;
static STORED_META_FIELDS: Lazy<Vec<FieldRef>> = Lazy::new(|| {
    Vec::<FieldRef>::from_type::<StoredLogMeta>(
        TracingOptions::default()
            .allow_null_fields(true)
            .strings_as_large_utf8(false),
    )
    .expect("derive FieldRef for StoredLogMeta")
});

async fn wait_for_flush_tick_or_cancel(
    interval: &mut flow_like_types::tokio::time::Interval,
    cancel: &CancellationToken,
) -> bool {
    flow_like_types::tokio::select! {
        _ = cancel.cancelled() => false,
        _ = interval.tick() => true,
    }
}

pub(super) async fn lock_with_timeout<'a, T>(
    mutex: &'a Mutex<T>,
    label: &str,
) -> flow_like_types::Result<flow_like_types::tokio::sync::MutexGuard<'a, T>> {
    flow_like_types::tokio::time::timeout(RUN_LOCK_TIMEOUT, mutex.lock())
        .await
        .map_err(|_| anyhow!("Timeout acquiring {}", label))
}

#[derive(
    Serialize, Deserialize, JsonSchema, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
    Fatal = 4,
}

impl LogLevel {
    pub fn from_u32(value: u32) -> Self {
        match value {
            0 => LogLevel::Debug,
            1 => LogLevel::Info,
            2 => LogLevel::Warn,
            3 => LogLevel::Error,
            4 => LogLevel::Fatal,
            _ => LogLevel::Debug,
        }
    }

    pub fn from_u8(value: u8) -> Self {
        match value {
            0 => LogLevel::Debug,
            1 => LogLevel::Info,
            2 => LogLevel::Warn,
            3 => LogLevel::Error,
            4 => LogLevel::Fatal,
            _ => LogLevel::Debug,
        }
    }

    pub fn to_u32(self) -> u32 {
        match self {
            LogLevel::Debug => 0,
            LogLevel::Info => 1,
            LogLevel::Warn => 2,
            LogLevel::Error => 3,
            LogLevel::Fatal => 4,
        }
    }

    pub fn to_u8(self) -> u8 {
        match self {
            LogLevel::Debug => 0,
            LogLevel::Info => 1,
            LogLevel::Warn => 2,
            LogLevel::Error => 3,
            LogLevel::Fatal => 4,
        }
    }
}

#[derive(
    Serialize, Deserialize, JsonSchema, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionEnvironment {
    Local,
    Desktop,
    Mobile,
    BrowserSandbox,
    Server,
}

impl Default for ExecutionEnvironment {
    fn default() -> Self {
        Self::Local
    }
}

impl ExecutionEnvironment {
    pub const ENV_VAR: &'static str = "FLOW_LIKE_EXECUTION_ENVIRONMENT";

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Desktop => "desktop",
            Self::Mobile => "mobile",
            Self::BrowserSandbox => "browser_sandbox",
            Self::Server => "server",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "local" => Some(Self::Local),
            "desktop" => Some(Self::Desktop),
            "mobile" => Some(Self::Mobile),
            "browser_sandbox" | "browser-sandbox" | "browser" | "sandbox" => {
                Some(Self::BrowserSandbox)
            }
            "server" | "remote" => Some(Self::Server),
            _ => None,
        }
    }

    pub fn is_local(self) -> bool {
        !matches!(self, Self::Server)
    }

    pub fn from_env() -> Option<Self> {
        match std::env::var(Self::ENV_VAR) {
            Ok(value) => match Self::parse(&value) {
                Some(environment) => Some(environment),
                None => {
                    tracing::warn!(
                        env_var = Self::ENV_VAR,
                        value = %value,
                        "Ignoring invalid execution environment override"
                    );
                    None
                }
            },
            Err(std::env::VarError::NotPresent) => None,
            Err(err) => {
                tracing::warn!(
                    env_var = Self::ENV_VAR,
                    error = %err,
                    "Unable to read execution environment override"
                );
                None
            }
        }
    }
}

#[derive(
    Serialize, Deserialize, JsonSchema, Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord,
)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    Sync,
    Async,
    Event,
    Scheduled,
}

impl Default for ExecutionMode {
    fn default() -> Self {
        Self::Sync
    }
}

impl ExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
            Self::Event => "event",
            Self::Scheduled => "scheduled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sync" | "synchronous" => Some(Self::Sync),
            "async" | "asynchronous" | "queue" | "queued" => Some(Self::Async),
            "event" => Some(Self::Event),
            "scheduled" | "schedule" | "cron" => Some(Self::Scheduled),
            _ => None,
        }
    }

    pub fn from_event(event: Option<&Event>) -> Self {
        match event {
            Some(event) if event.event_type.eq_ignore_ascii_case("cron") => Self::Scheduled,
            Some(_) => Self::Event,
            None => Self::Sync,
        }
    }
}

/// Storage struct for LanceDB - excludes runtime-only fields like is_remote
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct StoredLogMeta {
    pub app_id: String,
    pub run_id: String,
    pub board_id: String,
    pub start: u64,
    pub end: u64,
    pub log_level: u8,
    pub version: String,
    pub nodes: Option<Vec<(String, u8)>>,
    pub logs: Option<u64>,
    pub node_id: String,
    pub event_version: Option<String>,
    pub event_id: String,
    pub payload: Vec<u8>,
}

impl From<&LogMeta> for StoredLogMeta {
    fn from(meta: &LogMeta) -> Self {
        StoredLogMeta {
            app_id: meta.app_id.clone(),
            run_id: meta.run_id.clone(),
            board_id: meta.board_id.clone(),
            start: meta.start,
            end: meta.end,
            log_level: meta.log_level,
            version: meta.version.clone(),
            nodes: meta.nodes.clone(),
            logs: meta.logs,
            node_id: meta.node_id.clone(),
            event_version: meta.event_version.clone(),
            event_id: meta.event_id.clone(),
            payload: meta.payload.clone(),
        }
    }
}

impl From<StoredLogMeta> for LogMeta {
    fn from(stored: StoredLogMeta) -> Self {
        LogMeta {
            app_id: stored.app_id,
            run_id: stored.run_id,
            board_id: stored.board_id,
            start: stored.start,
            end: stored.end,
            log_level: stored.log_level,
            version: stored.version,
            nodes: stored.nodes,
            logs: stored.logs,
            node_id: stored.node_id,
            event_version: stored.event_version,
            event_id: stored.event_id,
            payload: stored.payload,
            is_remote: false,
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct LogMeta {
    pub app_id: String,
    pub run_id: String,
    pub board_id: String,
    pub start: u64,
    pub end: u64,
    pub log_level: u8,
    pub version: String,
    pub nodes: Option<Vec<(String, u8)>>,
    pub logs: Option<u64>,
    pub node_id: String,
    pub event_version: Option<String>,
    pub event_id: String,
    pub payload: Vec<u8>,
    /// Runtime-only field - not stored, set based on fetch source
    #[serde(default)]
    pub is_remote: bool,
}

impl LogMeta {
    fn into_arrow(&self) -> flow_like_types::Result<RecordBatch> {
        let fields = &*STORED_META_FIELDS;
        let stored: StoredLogMeta = self.into();
        let batch = serde_arrow::to_record_batch(fields, &vec![stored])?;
        Ok(batch)
    }

    pub fn into_duckdb_types() -> String {
        let fields = &*STORED_META_FIELDS;
        let mut types = vec![];

        for field in fields {
            let field_type = match field.data_type() {
                flow_like_storage::arrow_schema::DataType::Utf8 => "TEXT",
                flow_like_storage::arrow_schema::DataType::UInt64 => "INTEGER",
                flow_like_storage::arrow_schema::DataType::Int64 => "INTEGER",
                flow_like_storage::arrow_schema::DataType::Boolean => "BOOLEAN",
                _ => "TEXT",
            };
            types.push(format!("{} {}", field.name(), field_type));
        }

        types.join(", ")
    }

    pub async fn flush(
        &self,
        db: Connection,
        write_options: Option<&flow_like_storage::lancedb::table::WriteOptions>,
    ) -> flow_like_types::Result<()> {
        let arrow_batch = self.into_arrow()?;
        let schema = arrow_batch.schema();

        let make_iter = || -> Box<dyn RecordBatchReader + Send> {
            Box::new(RecordBatchIterator::new(
                vec![arrow_batch.clone()].into_iter().map(Ok),
                schema.clone(),
            ))
        };

        // Try to open and add to existing table first
        if let Ok(table) = db.open_table("runs").execute().await {
            let mut add = table.add(make_iter());
            if let Some(opts) = write_options {
                add = add.write_options(opts.clone());
            }
            if add.execute().await.is_ok() {
                return Ok(());
            }
        }

        // Table doesn't exist — try to create it with data
        let mut builder = db.create_table("runs", make_iter());
        if let Some(opts) = write_options {
            builder = builder.write_options(opts.clone());
        }
        match builder.execute().await {
            Ok(table) => {
                Self::create_runs_indexes(&table).await;
                return Ok(());
            }
            Err(_create_err) => {
                // Race: another flush created the table — fall back to open + add
                let table = db.open_table("runs").execute().await?;
                let mut add = table.add(make_iter());
                if let Some(opts) = write_options {
                    add = add.write_options(opts.clone());
                }
                add.execute().await?;
            }
        }

        Ok(())
    }

    async fn create_runs_indexes(table: &flow_like_storage::lancedb::Table) {
        let _ = table
            .create_index(
                &["event_id"],
                flow_like_storage::lancedb::index::Index::Bitmap(BitmapIndexBuilder {}),
            )
            .execute()
            .await;
        let _ = table
            .create_index(
                &["node_id"],
                flow_like_storage::lancedb::index::Index::Bitmap(BitmapIndexBuilder {}),
            )
            .execute()
            .await;
        let _ = table
            .create_index(
                &["log_level"],
                flow_like_storage::lancedb::index::Index::Bitmap(BitmapIndexBuilder {}),
            )
            .execute()
            .await;
        let _ = table
            .create_index(
                &["start"],
                flow_like_storage::lancedb::index::Index::BTree(
                    flow_like_storage::lancedb::index::scalar::BTreeIndexBuilder {},
                ),
            )
            .execute()
            .await;
    }
}

#[derive(Clone)]
pub struct Run {
    pub id: String,
    pub app_id: String,
    /// Server-backed app ID used for hosted-model usage attribution.
    /// Offline apps keep their local `app_id` for storage, but leave this unset.
    pub model_usage_app_id: Option<String>,
    pub traces: Vec<Trace>,
    pub status: RunStatus,
    pub start: SystemTime,
    pub end: SystemTime,
    pub board: Arc<Board>,
    pub log_level: LogLevel,
    pub payload: Arc<RunPayload>,
    pub sub: String,
    pub highest_log_level: LogLevel,
    pub log_initialized: bool,
    pub logs: u64,
    pub stream_state: bool,
    pub log_spill_threshold: usize,
    pub nodes_executed: Arc<AtomicU64>,

    pub event_id: Option<String>,
    pub event_version: Option<String>,

    pub visited_nodes: AHashMap<String, LogLevel>,
    pub log_store: Option<FlowLikeStore>,
    pub log_db: Option<
        Arc<dyn Fn(Path) -> flow_like_storage::lancedb::connection::ConnectBuilder + Send + Sync>,
    >,
    pub lance_write_options: Option<flow_like_storage::lancedb::table::WriteOptions>,
}

impl Run {
    pub(crate) fn push_trace(&mut self, trace: Trace) {
        let first_for_node = !self.visited_nodes.contains_key(trace.node_id.as_ref());
        if first_for_node {
            self.visited_nodes
                .insert(trace.node_id.to_string(), LogLevel::Debug);
        }

        // Non-empty traces carry user-visible diagnostics and are never
        // deduplicated. One empty trace per node is enough to retain visit
        // metadata and a target for cancellation logs.
        if !trace.logs.is_empty() || first_for_node {
            self.traces.push(trace);
        }
    }

    pub(crate) fn extend_traces(&mut self, traces: impl IntoIterator<Item = Trace>) {
        for trace in traces {
            self.push_trace(trace);
        }
    }

    pub(crate) fn prepare_flush(
        &mut self,
        finalize: bool,
    ) -> flow_like_types::Result<Option<PreparedFlush>> {
        let db_fn = match self.log_db.as_ref() {
            Some(db) => db.clone(),
            None => {
                tracing::debug!(
                    "No log database configured - logs will not be persisted to LanceDB"
                );
                return Ok(None);
            }
        };

        let base_path = Path::from("runs")
            .child(self.app_id.clone())
            .child(self.board.id.clone());
        tracing::debug!(path = %base_path, finalize, traces = self.traces.len(), "Preparing log flush");

        // 1) pre‑count total logs, reserve once, and find highest level in one pass
        let total = self.traces.iter().map(|t| t.logs.len()).sum();
        let mut logs = Vec::with_capacity(total);
        let mut highest = self.highest_log_level;
        for trace in self.traces.drain(..) {
            if !self.visited_nodes.contains_key(trace.node_id.as_ref()) {
                self.visited_nodes
                    .insert(trace.node_id.to_string(), LogLevel::Debug);
            }
            let node_level = self
                .visited_nodes
                .get_mut(trace.node_id.as_ref())
                .expect("trace node was registered above");

            for log in trace.logs {
                let lvl = log.log_level;

                if lvl > highest {
                    highest = lvl;
                }

                if lvl > *node_level {
                    *node_level = lvl;
                }

                logs.push(log);
            }
        }
        self.logs = self.logs.saturating_add(logs.len() as u64);
        self.highest_log_level = highest;

        // 2) build arrow batch in-memory
        let arrow_batch = LogMessage::into_arrow(logs)?;
        let schema = arrow_batch.schema();

        let meta = if finalize {
            let vs = &self.board.version;
            let version_string = format!("v{}-{}-{}", vs.0, vs.1, vs.2);
            let start_micros = self
                .start
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_micros()
                .try_into()
                .map_err(|_| anyhow!("start timestamp overflowed u64"))?;
            let end_micros = self
                .end
                .duration_since(SystemTime::UNIX_EPOCH)?
                .as_micros()
                .try_into()
                .map_err(|_| anyhow!("end timestamp overflowed u64"))?;
            let payload =
                to_vec(&self.payload.payload.clone().unwrap_or(Value::Null)).unwrap_or_default();
            let visited_nodes = self
                .visited_nodes
                .drain()
                .map(|(k, v)| (k, v.to_u8()))
                .collect::<Vec<(String, u8)>>();

            Some(LogMeta {
                app_id: self.app_id.clone(),
                run_id: self.id.clone(),
                board_id: self.board.id.clone(),
                start: start_micros,
                end: end_micros,
                log_level: self.highest_log_level.to_u8(),
                version: version_string,
                nodes: Some(visited_nodes),
                logs: Some(self.logs),
                node_id: self.payload.id.clone(),
                event_id: self.event_id.clone().unwrap_or("".to_string()),
                event_version: self.event_version.clone(),
                payload,
                is_remote: false,
            })
        } else {
            None
        };

        Ok(Some(PreparedFlush {
            db_fn,
            base_path,
            run_id: self.id.clone(),
            arrow_batch,
            schema,
            log_initialized: self.log_initialized,
            meta,
            write_options: self.lance_write_options.clone(),
        }))
    }

    pub async fn flush_logs(&mut self, finalize: bool) -> flow_like_types::Result<Option<LogMeta>> {
        let Some(prepared) = self.prepare_flush(finalize)? else {
            return Ok(None);
        };

        let result = prepared.write().await?;
        if result.created_table {
            self.log_initialized = true;
        }

        Ok(result.meta)
    }
}

pub(crate) struct PreparedFlush {
    db_fn:
        Arc<dyn Fn(Path) -> flow_like_storage::lancedb::connection::ConnectBuilder + Send + Sync>,
    base_path: Path,
    run_id: String,
    arrow_batch: RecordBatch,
    schema: SchemaRef,
    log_initialized: bool,
    meta: Option<LogMeta>,
    write_options: Option<flow_like_storage::lancedb::table::WriteOptions>,
}

pub(crate) struct FlushResult {
    pub created_table: bool,
    pub meta: Option<LogMeta>,
}

impl PreparedFlush {
    const MAX_RETRIES: u32 = 3;
    const INITIAL_BACKOFF_MS: u64 = 100;

    pub async fn write(self) -> flow_like_types::Result<FlushResult> {
        let mut last_err = None;

        for attempt in 0..Self::MAX_RETRIES {
            if attempt > 0 {
                let backoff = Self::INITIAL_BACKOFF_MS * (1 << (attempt - 1));
                flow_like_types::tokio::time::sleep(Duration::from_millis(backoff)).await;
            }

            match self.try_write().await {
                Ok(result) => return Ok(result),
                Err(err) => {
                    eprintln!(
                        "[Warn] log flush attempt {}/{} failed: {:?}",
                        attempt + 1,
                        Self::MAX_RETRIES,
                        err
                    );
                    last_err = Some(err);
                }
            }
        }

        Err(last_err.unwrap())
    }

    fn make_iter(&self) -> Box<dyn RecordBatchReader + Send> {
        Box::new(RecordBatchIterator::new(
            vec![self.arrow_batch.clone()].into_iter().map(Ok),
            self.schema.clone(),
        ))
    }

    async fn try_add(
        &self,
        table: &flow_like_storage::lancedb::Table,
    ) -> flow_like_types::Result<()> {
        let mut add = table.add(self.make_iter());
        if let Some(opts) = &self.write_options {
            add = add.write_options(opts.clone());
        }
        add.execute().await?;
        Ok(())
    }

    async fn try_write(&self) -> flow_like_types::Result<FlushResult> {
        let db = (self.db_fn)(self.base_path.clone()).execute().await?;

        // Fast path: table already exists, just append
        match db.open_table(&self.run_id).execute().await {
            Ok(table) => {
                self.try_add(&table).await?;
                return Ok(FlushResult {
                    created_table: false,
                    meta: self.meta.clone(),
                });
            }
            Err(open_err) => {
                tracing::debug!(run_id = %self.run_id, error = %open_err, "open_table failed, will create");
            }
        }

        // Table doesn't exist yet — try to create it with data
        let mut builder = db.create_table(&self.run_id, self.make_iter());
        if let Some(opts) = &self.write_options {
            builder = builder.write_options(opts.clone());
        }
        match builder.execute().await {
            Ok(_) => {
                return Ok(FlushResult {
                    created_table: !self.log_initialized,
                    meta: self.meta.clone(),
                });
            }
            Err(create_err) => {
                // Another concurrent flush likely created the table between our
                // open_table and create_table calls — fall back to open + add.
                tracing::debug!(run_id = %self.run_id, error = %create_err, "create_table failed, falling back to open+add");
                let table = db.open_table(&self.run_id).execute().await.map_err(|e| {
                    flow_like_types::anyhow!(
                        "create_table failed ({create_err}), then open_table also failed: {e}"
                    )
                })?;
                self.try_add(&table).await?;
            }
        }

        Ok(FlushResult {
            created_table: !self.log_initialized,
            meta: self.meta.clone(),
        })
    }
}

#[derive(Clone)]
struct RunStack {
    stack: Vec<ExecutionTarget>,
    deduplication: AHashSet<usize>,
    hash: u64,
}

impl RunStack {
    fn with_capacity(capacity: usize) -> Self {
        RunStack {
            stack: Vec::with_capacity(capacity),
            deduplication: AHashSet::with_capacity(capacity.saturating_mul(2)),
            hash: 0u64,
        }
    }

    fn push(&mut self, target: ExecutionTarget) {
        let nkey = ptr_key(&target.node);

        if !self.deduplication.insert(nkey) {
            return;
        }

        let mut h = AHasher::default();
        h.write_usize(nkey);
        self.hash ^= h.finish();

        self.stack.push(target);
    }

    #[inline]
    fn hash(&self) -> u64 {
        self.hash
    }
    #[inline]
    fn len(&self) -> usize {
        self.stack.len()
    }
}

pub type EventTrigger =
    Arc<dyn Fn(&InternalRun) -> BoxFuture<'_, flow_like_types::Result<()>> + Send + Sync>;

use std::sync::atomic::{AtomicBool, AtomicU64};

/// Cached immutable fields from Run to avoid locking during hot path execution
#[derive(Clone)]
pub struct RunMeta {
    pub run_id: String,
    pub app_id: String,
    pub model_usage_app_id: Option<String>,
    pub board_id: String,
    pub board_dir: Path,
    pub sub: String,
    pub stream_state: bool,
    pub environment: ExecutionEnvironment,
    pub execution_mode: ExecutionMode,
    pub log_spill_threshold: usize,
    pub log_flush_interval: Duration,
    pub nodes_executed: Arc<AtomicU64>,
}

impl RunMeta {
    pub fn increment_nodes_executed(&self) {
        self.nodes_executed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_nodes_executed(&self) -> u64 {
        self.nodes_executed
            .load(std::sync::atomic::Ordering::Relaxed)
    }
}

fn reset_execution_counters(
    nodes: &AHashMap<String, Arc<InternalNode>>,
    nodes_executed: &AtomicU64,
) {
    nodes_executed.store(0, Ordering::Relaxed);
    for node in nodes.values() {
        node.exec_calls.store(0, Ordering::Relaxed);
    }
}

fn stack_for_entry(
    nodes: &AHashMap<String, Arc<InternalNode>>,
    entry_node_id: &str,
) -> flow_like_types::Result<RunStack> {
    let node = nodes
        .get(entry_node_id)
        .cloned()
        .ok_or_else(|| anyhow!("Entry node {} not found", entry_node_id))?;
    let mut stack = RunStack::with_capacity(1);
    stack.push(ExecutionTarget {
        node,
        through_pins: Vec::new(),
    });
    Ok(stack)
}

fn model_usage_app_id_for_visibility(app_id: &str, visibility: &AppVisibility) -> Option<String> {
    match visibility {
        AppVisibility::Offline => None,
        _ => Some(app_id.to_string()),
    }
}

#[derive(Clone)]
pub struct InternalRun {
    pub run: Arc<Mutex<Run>>,
    pub nodes: Arc<AHashMap<String, Arc<InternalNode>>>,
    pub dependencies: AHashMap<String, Vec<Arc<InternalNode>>>,
    pub pins: AHashMap<String, Arc<InternalPin>>,
    pub variables: Arc<Mutex<AHashMap<String, Variable>>>,
    pub cache: Arc<RwLock<AHashMap<String, Arc<dyn Cacheable>>>>,
    pub profile: Arc<Profile>,
    pub callback: InterComCallback,
    pub credentials: Option<Arc<SharedCredentials>>,
    pub token: Option<String>,
    pub oauth_tokens: Arc<AHashMap<String, OAuthToken>>,
    /// User context for this execution
    pub user_context: Option<UserExecutionContext>,

    stack: Arc<RunStack>,
    concurrency_limit: u64,
    cpus: usize,
    log_level: LogLevel,
    completion_callbacks: Arc<RwLock<Vec<EventTrigger>>>,
    /// Set to true when any node execution fails
    has_node_errors: Arc<AtomicBool>,
    log_flush_interval: Duration,
    cancellation_token: Option<CancellationToken>,
    cancellation_log_level: LogLevel,
    cancellation_log_message: String,

    // Cached immutable fields from Run to avoid locking
    pub meta: RunMeta,
    pub board: Arc<Board>,
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct RunPayload {
    pub id: String,
    pub payload: Option<Value>,
    /// Runtime-configured variables and secrets (for local execution).
    /// These override board variable defaults when present.
    #[serde(default)]
    pub runtime_variables: Option<std::collections::HashMap<String, Variable>>,
    /// When true (default), secret variables from runtime_variables are ignored
    /// unless they are also marked as runtime_configured.
    /// Set to false only for trusted local (desktop) execution.
    #[serde(default)]
    pub filter_secrets: Option<bool>,
}

impl InternalRun {
    pub async fn new(
        app_id: &str,
        board: Arc<Board>,
        event: Option<Event>,
        handler: &Arc<FlowLikeState>,
        profile: &Profile,
        payload: &RunPayload,
        stream_state: bool,
        callback: InterComCallback,
        credentials: Option<SharedCredentials>,
        token: Option<String>,
        oauth_tokens: std::collections::HashMap<String, OAuthToken>,
    ) -> flow_like_types::Result<Self> {
        Self::new_with_run_id(
            app_id,
            board,
            event,
            handler,
            profile,
            payload,
            stream_state,
            callback,
            credentials,
            token,
            oauth_tokens,
            None,
        )
        .await
    }

    pub async fn new_with_run_id(
        app_id: &str,
        board: Arc<Board>,
        event: Option<Event>,
        handler: &Arc<FlowLikeState>,
        profile: &Profile,
        payload: &RunPayload,
        stream_state: bool,
        callback: InterComCallback,
        credentials: Option<SharedCredentials>,
        token: Option<String>,
        oauth_tokens: std::collections::HashMap<String, OAuthToken>,
        run_id: Option<String>,
    ) -> flow_like_types::Result<Self> {
        // Convert to AHashMap for internal use
        let oauth_tokens: AHashMap<String, OAuthToken> = oauth_tokens.into_iter().collect();

        let before = Instant::now();
        let run_id = run_id.unwrap_or_else(create_id);
        let execution_mode = ExecutionMode::from_event(event.as_ref());

        let (log_store, db, lance_write_options) = {
            let guard = handler.config.read().await;
            let log_store = guard.stores.log_store.clone();
            let db = guard.callbacks.build_logs_database.clone();
            let write_opts = guard.callbacks.lance_write_options.clone();
            tracing::debug!(
                has_log_store = log_store.is_some(),
                has_log_db = db.is_some(),
                "InternalRun: Reading log configuration from state"
            );
            (log_store, db, write_opts)
        };

        // derive sub from token (JWT) or default to "local"
        let sub_value = token
            .as_ref()
            .and_then(|t| extract_sub_from_jwt(t).ok())
            .unwrap_or_else(|| "local".to_string());

        let nodes_executed = Arc::new(AtomicU64::new(0));
        let run = Run {
            id: run_id.clone(),
            app_id: app_id.to_string(),
            model_usage_app_id: Some(app_id.to_string()),
            traces: vec![],
            status: RunStatus::Running,
            start: SystemTime::now(),
            end: SystemTime::now(),
            log_level: board.log_level,
            board: board.clone(),
            payload: Arc::new(payload.clone()),
            sub: sub_value.clone(),
            highest_log_level: LogLevel::Debug,
            log_initialized: false,
            logs: 0,
            stream_state,
            log_spill_threshold: DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD,
            nodes_executed: nodes_executed.clone(),

            event_id: event.as_ref().map(|e| e.id.clone()),
            event_version: event.as_ref().map(|e| {
                let (major, minor, patch) = e.event_version;
                format!("{}.{}.{}", major, minor, patch)
            }),

            visited_nodes: AHashMap::with_capacity(board.nodes.len()),
            log_store,
            log_db: db,
            lance_write_options,
        };

        let run = Arc::new(Mutex::new(run));

        let mut dependencies = AHashMap::with_capacity(board.nodes.len());

        let event_variables = event
            .as_ref()
            .map(|e| e.variables.clone())
            .unwrap_or_default();

        // Extract runtime_variables from payload
        let runtime_variables = payload.runtime_variables.clone().unwrap_or_default();
        let filter_secrets = payload.filter_secrets.unwrap_or(true);

        let variables = Arc::new(Mutex::new({
            let mut map = AHashMap::with_capacity(board.variables.len());
            for (variable_id, board_variable) in &board.variables {
                // Priority: runtime_configured/secret vars > event vars (for exposed) > board vars
                // When filter_secrets is true, only runtime_configured vars may be overridden;
                // secrets from untrusted callers are ignored to prevent injection.
                let allow_runtime_override =
                    board_variable.runtime_configured || (board_variable.secret && !filter_secrets);
                let variable = if allow_runtime_override {
                    runtime_variables.get(variable_id).unwrap_or(board_variable)
                } else if board_variable.exposed {
                    event_variables.get(variable_id).unwrap_or(board_variable)
                } else {
                    board_variable
                };

                let value = match &variable.default_value {
                    Some(bytes) => {
                        flow_like_types::json::from_slice::<Value>(bytes).unwrap_or(Value::Null)
                    }
                    None => Value::Null,
                };

                let mut var = variable.clone();
                var.value = Arc::new(Mutex::new(value));
                map.insert(variable_id.clone(), var);
            }
            map
        }));

        let mut pin_to_node = AHashMap::with_capacity(board.nodes.len() * 3);
        let mut pins: AHashMap<String, Arc<InternalPin>> =
            AHashMap::with_capacity(board.nodes.len() * 3);

        // Phase 1: Create all pins without connections
        for (node_id, node) in &board.nodes {
            for (pin_id, pin) in &node.pins {
                let internal_pin = InternalPin::new(pin, false);
                pin_to_node.insert(pin_id, (node_id, node.is_pure()));
                pins.insert(pin.id.clone(), Arc::new(internal_pin));
            }
        }

        // Also create pins for nodes inside function layers
        for layer in board.layers.values() {
            if !matches!(layer.r#type, LayerType::Function) {
                continue;
            }
            for (node_id, node) in &layer.nodes {
                for (pin_id, pin) in &node.pins {
                    let internal_pin = InternalPin::new(pin, false);
                    pin_to_node.insert(pin_id, (node_id, node.is_pure()));
                    pins.insert(pin.id.clone(), Arc::new(internal_pin));
                }
            }
        }

        for layer in board.layers.values() {
            for (pin_id, pin) in &layer.pins {
                if pins.contains_key(pin_id) {
                    // this is the old layer format, where we just relayed the connected pin to the layers pin.
                    continue;
                }

                let internal_pin = InternalPin::new(pin, true);
                pins.insert(pin.id.clone(), Arc::new(internal_pin));
            }
        }

        // Phase 2: Wire up connections using OnceLock init methods
        for node in board.nodes.values() {
            for pin in node.pins.values() {
                if let Some(internal_pin) = pins.get(&pin.id) {
                    // Build connected_to list
                    let connected: Vec<Weak<InternalPin>> = pin
                        .connected_to
                        .iter()
                        .filter_map(|id| pins.get(id).map(Arc::downgrade))
                        .collect();
                    internal_pin.init_connected_to(connected);

                    // Build depends_on list
                    let depends: Vec<Weak<InternalPin>> = pin
                        .depends_on
                        .iter()
                        .filter_map(|id| pins.get(id).map(Arc::downgrade))
                        .collect();
                    internal_pin.init_depends_on(depends);
                }
            }
        }

        // Wire connections for function layer nodes
        for layer in board.layers.values() {
            if !matches!(layer.r#type, LayerType::Function) {
                continue;
            }
            for node in layer.nodes.values() {
                for pin in node.pins.values() {
                    if let Some(internal_pin) = pins.get(&pin.id) {
                        let connected: Vec<Weak<InternalPin>> = pin
                            .connected_to
                            .iter()
                            .filter_map(|id| pins.get(id).map(Arc::downgrade))
                            .collect();
                        internal_pin.init_connected_to(connected);

                        let depends: Vec<Weak<InternalPin>> = pin
                            .depends_on
                            .iter()
                            .filter_map(|id| pins.get(id).map(Arc::downgrade))
                            .collect();
                        internal_pin.init_depends_on(depends);
                    }
                }
            }
        }

        // Also wire connections for layer pins
        for layer in board.layers.values() {
            for pin in layer.pins.values() {
                if let Some(internal_pin) = pins.get(&pin.id) {
                    let connected: Vec<Weak<InternalPin>> = pin
                        .connected_to
                        .iter()
                        .filter_map(|id| pins.get(id).map(Arc::downgrade))
                        .collect();
                    internal_pin.init_connected_to(connected);

                    let depends: Vec<Weak<InternalPin>> = pin
                        .depends_on
                        .iter()
                        .filter_map(|id| pins.get(id).map(Arc::downgrade))
                        .collect();
                    internal_pin.init_depends_on(depends);
                }
            }
        }

        let mut dependency_map = AHashMap::with_capacity(board.nodes.len());
        let mut nodes = AHashMap::with_capacity(board.nodes.len());
        let mut stack = RunStack::with_capacity(1);

        let registry = handler.node_registry.read().await.node_registry.clone();
        for (node_id, node) in &board.nodes {
            let logic = registry.instantiate(node)?;
            let mut node_pins = AHashMap::new();
            let mut pin_cache = AHashMap::new();

            for pin in node.pins.values() {
                if let Some(internal_pin) = pins.get(&pin.id) {
                    node_pins.insert(pin.id.clone(), internal_pin.clone());
                    let cached_array = pin_cache.entry(pin.name.clone()).or_insert(vec![]);
                    cached_array.push(internal_pin.clone());
                }

                if USE_DEPENDENCY_GRAPH {
                    for dependency_pin_id in &pin.depends_on {
                        if let Some((dependency_node_id, is_pure)) =
                            pin_to_node.get(dependency_pin_id)
                        {
                            dependency_map
                                .entry(node_id)
                                .or_insert(vec![])
                                .push((*dependency_node_id, is_pure));
                        }
                    }
                }
            }

            let internal_node = Arc::new(InternalNode::new(
                node.clone(),
                node_pins.clone(),
                logic,
                pin_cache.clone(),
            ));

            // Set node reference on all pins using OnceLock init method
            for internal_pin in node_pins.values() {
                internal_pin.init_node(Arc::downgrade(&internal_node));
            }

            if payload.id == node.id {
                let target = ExecutionTarget {
                    node: internal_node.clone(),
                    through_pins: vec![],
                };
                stack.push(target);
            }

            nodes.insert(node_id.clone(), internal_node);
        }

        // Instantiate nodes inside function layers so they are available during execution
        for layer in board.layers.values() {
            if !matches!(layer.r#type, LayerType::Function) {
                continue;
            }
            for (node_id, node) in &layer.nodes {
                let logic = registry.instantiate(node)?;
                let mut node_pins = AHashMap::new();
                let mut pin_cache = AHashMap::new();

                for pin in node.pins.values() {
                    if let Some(internal_pin) = pins.get(&pin.id) {
                        node_pins.insert(pin.id.clone(), internal_pin.clone());
                        let cached_array = pin_cache.entry(pin.name.clone()).or_insert(vec![]);
                        cached_array.push(internal_pin.clone());
                    }
                }

                let internal_node = Arc::new(InternalNode::new(
                    node.clone(),
                    node_pins.clone(),
                    logic,
                    pin_cache.clone(),
                ));

                for internal_pin in node_pins.values() {
                    internal_pin.init_node(Arc::downgrade(&internal_node));
                }

                nodes.insert(node_id.clone(), internal_node);
            }
        }

        if USE_DEPENDENCY_GRAPH {
            let mut recursion_filter: AHashSet<String> = AHashSet::new();
            for node_id in board.nodes.keys() {
                let deps = recursive_get_deps(
                    node_id.to_string(),
                    &dependency_map,
                    &nodes,
                    &mut recursion_filter,
                );
                dependencies.insert(node_id.clone(), deps);
            }
        }

        tracing::debug!(
            elapsed = ?before.elapsed(),
            nodes = nodes.len(),
            pins = pins.len(),
            "InternalRun::new completed"
        );

        let cache = Arc::new(RwLock::new(AHashMap::new()));
        let temporary_store = {
            let config = handler.config.read().await;
            config.stores.temporary_store.clone()
        };
        if let Some(temporary_store) = temporary_store {
            let cacheable_store: Arc<dyn Cacheable> = Arc::new(temporary_store);
            cache
                .write()
                .await
                .insert(REQUEST_FILES_STORE_REF.to_string(), cacheable_store);
        }

        Ok(InternalRun {
            run,
            nodes: Arc::new(nodes),
            pins,
            variables,
            cache,
            stack: Arc::new(stack),
            concurrency_limit: 128_000,
            cpus: num_cpus::get().max(4) * 4,
            callback,
            credentials: credentials.map(Arc::new),
            token,
            oauth_tokens: Arc::new(oauth_tokens),
            dependencies,
            log_level: board.log_level,
            profile: Arc::new(profile.clone()),
            completion_callbacks: Arc::new(RwLock::new(vec![])),
            user_context: None,
            has_node_errors: Arc::new(AtomicBool::new(false)),
            log_flush_interval: DEFAULT_RUN_LOG_FLUSH_INTERVAL,
            cancellation_token: None,
            cancellation_log_level: LogLevel::Fatal,
            cancellation_log_message: "Run cancelled".to_string(),
            // Cached immutable fields from Run
            meta: RunMeta {
                run_id: run_id.clone(),
                app_id: app_id.to_string(),
                model_usage_app_id: Some(app_id.to_string()),
                board_id: board.id.clone(),
                board_dir: board.board_dir.clone(),
                sub: sub_value.clone(),
                stream_state,
                environment: ExecutionEnvironment::Local,
                execution_mode,
                log_spill_threshold: DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD,
                log_flush_interval: DEFAULT_RUN_LOG_FLUSH_INTERVAL,
                nodes_executed,
            },
            board: board.clone(),
        })
    }

    pub async fn set_log_flush_policy(
        &mut self,
        flush_interval: Duration,
        spill_threshold: usize,
    ) -> flow_like_types::Result<()> {
        self.log_flush_interval = if flush_interval.is_zero() {
            DEFAULT_RUN_LOG_FLUSH_INTERVAL
        } else {
            flush_interval
        };

        let spill_threshold = spill_threshold.max(1);
        self.meta.log_spill_threshold = spill_threshold;
        self.meta.log_flush_interval = self.log_flush_interval;

        let mut run = lock_with_timeout(self.run.as_ref(), "set_log_flush_policy").await?;
        run.log_spill_threshold = spill_threshold;

        Ok(())
    }

    pub fn set_cancellation_token(&mut self, token: CancellationToken) {
        self.cancellation_token = Some(token);
    }

    pub fn set_cancellation_log(&mut self, message: impl Into<String>, level: LogLevel) {
        self.cancellation_log_message = message.into();
        self.cancellation_log_level = level;
    }

    pub fn set_execution_environment(&mut self, environment: ExecutionEnvironment) {
        self.meta.environment = environment;
    }

    pub fn set_execution_mode(&mut self, mode: ExecutionMode) {
        self.meta.execution_mode = mode;
    }

    /// Configure hosted-model usage attribution from the app's persisted visibility.
    /// Offline app IDs only exist locally and must not be authorized as cloud app IDs.
    pub async fn set_usage_attribution_from_visibility(&mut self, visibility: &AppVisibility) {
        let app_id = model_usage_app_id_for_visibility(&self.meta.app_id, visibility);
        {
            let mut run = self.run.lock().await;
            run.model_usage_app_id = app_id.clone();
        }
        self.meta.model_usage_app_id = app_id;
    }

    /// Set the user execution context for this run
    pub fn set_user_context(&mut self, user_context: UserExecutionContext) {
        self.user_context = Some(user_context);
    }

    /// Override the execution subject after the transport token has been
    /// verified. This keeps PAT-backed runs tied to the executor JWT subject.
    pub async fn set_execution_sub(&mut self, sub: String) {
        {
            let mut run = self.run.lock().await;
            run.sub = sub.clone();
        }
        self.meta.sub = sub;
    }

    /// Set the user execution context for offline/local execution
    pub fn set_offline_user_context(&mut self) {
        self.user_context = Some(UserExecutionContext::offline());
    }

    /// Get the user execution context if available
    pub fn user_context(&self) -> Option<&UserExecutionContext> {
        self.user_context.as_ref()
    }

    // Reuse the same run, but reset the states
    pub async fn fork(&mut self) -> flow_like_types::Result<()> {
        if self.stack.len() != 0 {
            return Err(flow_like_types::anyhow!(
                "Cannot fork a run that is not finished"
            ));
        }

        let entry_node_id = {
            let run = lock_with_timeout(self.run.as_ref(), "run_fork_entry").await?;
            run.payload.id.clone()
        };
        let next_stack = stack_for_entry(&self.nodes, &entry_node_id)?;

        self.cache.write().await.clear();
        self.stack = Arc::new(next_stack);
        self.concurrency_limit = 128_000;
        self.has_node_errors.store(false, Ordering::Relaxed);
        reset_execution_counters(&self.nodes, &self.meta.nodes_executed);
        {
            let mut run = lock_with_timeout(self.run.as_ref(), "run_fork").await?;
            run.status = RunStatus::Running;
            run.traces.clear();
            run.visited_nodes.clear();
            run.logs = 0;
            run.highest_log_level = LogLevel::Debug;
            run.log_initialized = false;
            run.start = SystemTime::now();
            run.end = SystemTime::now();
        }
        for node in self.nodes.values() {
            for pin in node.pins.values() {
                // Reset is async but pin access is lock-free
                pin.reset().await;
            }
        }
        for variable in self.variables.lock().await.values_mut() {
            let default = variable.default_value.as_ref();
            let value = default.map_or(Value::Null, |v| {
                flow_like_types::json::from_slice(v).unwrap()
            });
            *variable.value.lock().await = value;
        }

        Ok(())
    }

    async fn step_parallel(
        &mut self,
        stack: Arc<RunStack>,
        handler: &Arc<FlowLikeState>,
        log_level: LogLevel,
        stage: ExecutionStage,
    ) {
        let variables = &self.variables;
        let cache = &self.cache;
        let dependencies = &self.dependencies;
        let run = self.run.clone();
        let profile = self.profile.clone();
        let concurrency_limit = self.concurrency_limit;
        let callback = self.callback.clone();
        let meta = self.meta.clone();
        let user_context = self.user_context.clone();
        let has_node_errors = self.has_node_errors.clone();
        let cancellation_token = self.cancellation_token.clone();
        let cancellation_log_level = self.cancellation_log_level;
        let cancellation_log_message = self.cancellation_log_message.clone();

        let new_stack = futures::stream::iter(stack.stack.clone())
            .map(|target| {
                let dependencies = dependencies;
                let handler = handler.clone();
                let run = run.clone();
                let meta = meta.clone();
                let profile = profile.clone();
                let callback = callback.clone();
                let stage = stage.clone();
                let completion_callbacks = self.completion_callbacks.clone();
                let credentials = self.credentials.clone();
                let token = self.token.clone();
                let nodes = self.nodes.clone();
                let oauth_tokens = self.oauth_tokens.clone();
                let user_context = user_context.clone();
                let has_node_errors = has_node_errors.clone();
                let cancellation_token = cancellation_token.clone();
                let cancellation_log_message = cancellation_log_message.clone();

                async move {
                    step_core(
                        nodes,
                        target,
                        concurrency_limit,
                        &handler,
                        &run,
                        &meta,
                        variables,
                        cache,
                        log_level,
                        stage,
                        dependencies,
                        &profile,
                        &callback,
                        &completion_callbacks,
                        credentials,
                        token,
                        oauth_tokens,
                        user_context,
                        cancellation_token,
                        cancellation_log_level,
                        cancellation_log_message,
                        &has_node_errors,
                    )
                    .await
                }
            })
            .buffer_unordered(self.cpus)
            .fold(
                (RunStack::with_capacity(stack.stack.len()), Vec::new()),
                |mut acc: (RunStack, Vec<Trace>), result| async move {
                    if let Ok((inner_iter, traces)) = result {
                        for node in inner_iter {
                            acc.0.push(node);
                        }
                        acc.1.extend(traces);
                    }
                    acc
                },
            )
            .await;

        // Merge all collected traces in one lock acquisition
        if !new_stack.1.is_empty()
            && let Ok(mut run_locked) =
                lock_with_timeout(self.run.as_ref(), "run_traces_batch_merge").await
        {
            run_locked.extend_traces(new_stack.1);
        }

        self.stack = Arc::new(new_stack.0);
    }

    async fn step_single(
        &mut self,
        stack: Arc<RunStack>,
        handler: &Arc<FlowLikeState>,
        log_level: LogLevel,
        stage: ExecutionStage,
    ) {
        let variables = &self.variables;
        let cache = &self.cache;
        let concurrency_limit = self.concurrency_limit;

        let target = stack.stack.first().cloned().unwrap();
        let connected_nodes = step_core(
            self.nodes.clone(),
            target,
            concurrency_limit,
            handler,
            &self.run,
            &self.meta,
            variables,
            cache,
            log_level,
            stage.clone(),
            &self.dependencies,
            &self.profile,
            &self.callback,
            &self.completion_callbacks,
            self.credentials.clone(),
            self.token.clone(),
            self.oauth_tokens.clone(),
            self.user_context.clone(),
            self.cancellation_token.clone(),
            self.cancellation_log_level,
            self.cancellation_log_message.clone(),
            &self.has_node_errors,
        )
        .await;

        let mut new_stack = RunStack::with_capacity(stack.len());
        if let Ok((nodes, traces)) = connected_nodes {
            for node in nodes {
                new_stack.push(node);
            }
            // Merge traces in one lock acquisition
            if !traces.is_empty()
                && let Ok(mut run_locked) =
                    lock_with_timeout(self.run.as_ref(), "run_traces_single_merge").await
            {
                run_locked.extend_traces(traces);
            }
        }

        self.stack = Arc::new(new_stack);
    }

    async fn step(&mut self, handler: Arc<FlowLikeState>) {
        let start = Instant::now();

        // Use cached values instead of locking Run
        let stage = self.board.stage.clone();
        let log_level = self.log_level;
        let stack = self.stack.clone();

        match stack.len() {
            1 => self.step_single(stack, &handler, log_level, stage).await,
            _ => self.step_parallel(stack, &handler, log_level, stage).await,
        };

        tracing::debug!(elapsed = ?start.elapsed(), "InternalRun::step completed");
    }

    pub async fn execute(&mut self, handler: Arc<FlowLikeState>) -> Option<LogMeta> {
        let start = Instant::now();
        let flush_interval = self.log_flush_interval;

        {
            match lock_with_timeout(self.run.as_ref(), "run_start").await {
                Ok(mut run) => {
                    run.start = SystemTime::now();
                }
                Err(err) => {
                    eprintln!("[Error] {}", err);
                }
            }
        }

        // Spawn background flush task for long-running nodes
        let run_clone = self.run.clone();
        let flush_cancel = CancellationToken::new();
        let flush_cancel_clone = flush_cancel.clone();
        let flush_task = flow_like_types::tokio::spawn(async move {
            let mut interval = flow_like_types::tokio::time::interval(flush_interval);
            interval.tick().await; // Skip first immediate tick

            while wait_for_flush_tick_or_cancel(&mut interval, &flush_cancel_clone).await {
                if flush_cancel_clone.is_cancelled() {
                    break;
                }
                let prepared: Option<PreparedFlush> =
                    match lock_with_timeout(run_clone.as_ref(), "run_flush_prepare").await {
                        Ok(mut run) => {
                            if run.traces.is_empty() {
                                None
                            } else {
                                match run.prepare_flush(false) {
                                    Ok(prepared) => prepared,
                                    Err(err) => {
                                        eprintln!(
                                            "[Error] preparing background log flush: {:?}",
                                            err
                                        );
                                        None
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!("[Error] {}", err);
                            None
                        }
                    };

                if let Some(prepared) = prepared {
                    match prepared.write().await {
                        Ok(result) => {
                            if result.created_table {
                                match lock_with_timeout(run_clone.as_ref(), "run_flush_finalize")
                                    .await
                                {
                                    Ok(mut run) => {
                                        run.log_initialized = true;
                                    }
                                    Err(err) => {
                                        eprintln!("[Error] {}", err);
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!("[Error] background log flush: {:?}", err);
                        }
                    }
                }
            }
        });

        let mut stack_hash = self.stack.hash();
        let mut current_stack_len = self.stack.len();
        let mut errored = false;

        while current_stack_len > 0 {
            self.step(handler.clone()).await;

            current_stack_len = self.stack.len();
            let new_stack_hash = self.stack.hash();
            if new_stack_hash == stack_hash {
                errored = true;
                tracing::warn!("Execution stopped because the stack did not change");
                break;
            }
            stack_hash = new_stack_hash;
        }

        // Check if any node reported an error during execution
        if self.has_node_errors.load(Ordering::Relaxed) {
            errored = true;
        }
        let cancelled = self
            .cancellation_token
            .as_ref()
            .is_some_and(|token| token.is_cancelled());
        let cancellation_log_level = self.cancellation_log_level;
        let cancellation_log_message = self.cancellation_log_message.clone();

        // Stop background flush task
        flush_cancel.cancel();
        let _ = flush_task.await;

        if self.trigger_completion_callbacks().await {
            // Completion callbacks persist buffered data and other final
            // side-effects. Reporting Success after one fails hides data loss
            // from callers, so include callback failures in final run status.
            errored = true;
        }
        self.drop_nodes().await;

        let meta = {
            let prepared: Option<PreparedFlush> =
                match lock_with_timeout(self.run.as_ref(), "run_finalize").await {
                    Ok(mut run) => {
                        run.end = SystemTime::now();
                        run.status = if cancelled {
                            RunStatus::Stopped
                        } else if errored {
                            RunStatus::Failed
                        } else {
                            RunStatus::Success
                        };
                        if cancelled {
                            if cancellation_log_level > run.highest_log_level {
                                run.highest_log_level = cancellation_log_level;
                            }
                            let cancel_log = LogMessage::new(
                                &cancellation_log_message,
                                cancellation_log_level,
                                None,
                            );
                            if let Some(trace) = run.traces.last_mut() {
                                trace.logs.push(cancel_log);
                            } else {
                                let mut system_trace = Trace::new("system");
                                system_trace.logs.push(cancel_log);
                                run.push_trace(system_trace);
                            }
                        }
                        match run.prepare_flush(true) {
                            Ok(prepared) => prepared,
                            Err(err) => {
                                eprintln!("[Error] preparing logs (final): {:?}", err);
                                None
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("[Error] {}", err);
                        None
                    }
                };

            if let Some(prepared) = prepared {
                match prepared.write().await {
                    Ok(result) => {
                        if result.created_table {
                            match lock_with_timeout(self.run.as_ref(), "run_finalize_mark").await {
                                Ok(mut run) => {
                                    run.log_initialized = true;
                                }
                                Err(err) => {
                                    eprintln!("[Error] {}", err);
                                }
                            }
                        }
                        result.meta
                    }
                    Err(err) => {
                        eprintln!("[Error] flushing logs (final): {:?}", err);
                        None
                    }
                }
            } else {
                None
            }
        };

        tracing::debug!(elapsed = ?start.elapsed(), "InternalRun::execute completed");

        meta
    }

    pub async fn debug_step(&mut self, handler: Arc<FlowLikeState>) -> bool {
        let stack_hash = self.stack.hash();
        if self.stack.len() == 0 {
            match lock_with_timeout(self.run.as_ref(), "run_debug_step_success").await {
                Ok(mut run) => {
                    run.end = SystemTime::now();
                    run.status = RunStatus::Success;
                }
                Err(err) => {
                    eprintln!("[Error] {}", err);
                }
            }
            return false;
        }

        self.step(handler.clone()).await;

        if self.stack.len() == 0 {
            match lock_with_timeout(self.run.as_ref(), "run_debug_step_success").await {
                Ok(mut run) => {
                    run.end = SystemTime::now();
                    run.status = RunStatus::Success;
                }
                Err(err) => {
                    eprintln!("[Error] {}", err);
                }
            }
            return false;
        }

        let new_stack_hash = self.stack.hash();
        if new_stack_hash == stack_hash {
            match lock_with_timeout(self.run.as_ref(), "run_debug_step_failed").await {
                Ok(mut run) => {
                    run.end = SystemTime::now();
                    run.status = RunStatus::Failed;
                }
                Err(err) => {
                    eprintln!("[Error] {}", err);
                }
            }
            return false;
        }

        true
    }

    pub async fn get_run(&self) -> Run {
        self.run.lock().await.clone()
    }

    pub async fn get_traces(&self) -> Vec<Trace> {
        self.run.lock().await.traces.clone()
    }

    pub async fn get_status(&self) -> RunStatus {
        self.run.lock().await.status.clone()
    }

    /// Runs every completion callback and reports whether any failed.
    ///
    /// Callbacks are cloned out of the registry before awaiting them so a
    /// callback cannot deadlock by interacting with the registry itself.
    async fn trigger_completion_callbacks(&self) -> bool {
        let callbacks = self.completion_callbacks.read().await.clone();
        let mut failed = false;
        for callback in callbacks {
            if let Err(err) = callback(self).await {
                failed = true;
                eprintln!("[Error] executing completion callback: {:?}", err);
            }
        }
        failed
    }

    async fn drop_nodes(&self) {
        let all_nodes = self.nodes.values();
        for node in all_nodes {
            node.logic.on_drop().await;
        }
    }

    // ONLY CALL THIS IF WE ARE BEING CANCELLED
    pub async fn flush_logs_cancelled(&mut self) -> flow_like_types::Result<Option<LogMeta>> {
        let prepared = {
            let mut run = lock_with_timeout(self.run.as_ref(), "run_cancel").await?;
            run.highest_log_level = LogLevel::Fatal;
            run.status = RunStatus::Stopped;
            run.end = SystemTime::now();

            let cancel_log = LogMessage::new("Run cancelled", LogLevel::Fatal, None);
            if let Some(trace) = run.traces.last_mut() {
                trace.logs.push(cancel_log);
            } else {
                // Create a system trace if no traces exist
                let mut system_trace = Trace::new(
                    run.visited_nodes
                        .keys()
                        .next()
                        .map(String::as_str)
                        .unwrap_or("system"),
                );
                system_trace.logs.push(cancel_log);
                run.push_trace(system_trace);
            }

            run.prepare_flush(true)?
        };

        if let Some(prepared) = prepared {
            let result = prepared.write().await?;
            if result.created_table {
                let mut run = lock_with_timeout(self.run.as_ref(), "run_cancel_mark").await?;
                run.log_initialized = true;
            }
            Ok(result.meta)
        } else {
            Ok(None)
        }
    }
}

fn recursive_get_deps(
    node_id: String,
    dependencies: &AHashMap<&String, Vec<(&String, &bool)>>,
    lookup: &AHashMap<String, Arc<InternalNode>>,
    recursion_filter: &mut AHashSet<String>,
) -> Vec<Arc<InternalNode>> {
    if recursion_filter.contains(&node_id) {
        return vec![];
    }

    recursion_filter.insert(node_id.clone());

    if !dependencies.contains_key(&node_id) {
        return vec![];
    }

    let deps = dependencies.get(&node_id).unwrap();
    let mut found_dependencies = Vec::with_capacity(deps.len());

    for (dep_id, is_pure) in deps {
        if !**is_pure {
            continue;
        }

        if let Some(dep) = lookup.get(*dep_id) {
            found_dependencies.push(dep.clone());
        }

        found_dependencies.extend(recursive_get_deps(
            dep_id.to_string(),
            dependencies,
            lookup,
            recursion_filter,
        ));
    }

    found_dependencies
}

#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub enum RunStatus {
    Running,
    Success,
    Failed,
    Stopped,
}

async fn step_core(
    nodes: Arc<AHashMap<String, Arc<InternalNode>>>,
    target: ExecutionTarget,
    concurrency_limit: u64,
    handler: &Arc<FlowLikeState>,
    run: &Arc<Mutex<Run>>,
    run_meta: &RunMeta,
    variables: &Arc<Mutex<AHashMap<String, Variable>>>,
    cache: &Arc<RwLock<AHashMap<String, Arc<dyn Cacheable>>>>,
    log_level: LogLevel,
    stage: ExecutionStage,
    dependencies: &AHashMap<String, Vec<Arc<InternalNode>>>,
    profile: &Arc<Profile>,
    callback: &InterComCallback,
    completion_callbacks: &Arc<RwLock<Vec<EventTrigger>>>,
    credentials: Option<Arc<SharedCredentials>>,
    token: Option<String>,
    oauth_tokens: Arc<AHashMap<String, OAuthToken>>,
    user_context: Option<UserExecutionContext>,
    cancellation_token: Option<CancellationToken>,
    cancellation_log_level: LogLevel,
    cancellation_log_message: String,
    has_node_errors: &Arc<AtomicBool>,
) -> flow_like_types::Result<(Vec<ExecutionTarget>, Vec<Trace>)> {
    // Check Node State and Validate Execution Count (to stop infinite loops)
    {
        let calls_before = target.node.exec_calls.fetch_add(1, Ordering::Relaxed);
        if calls_before >= concurrency_limit {
            return Err(anyhow!("Concurrency limit reached"));
        }
    }

    let weak_run = Arc::downgrade(run);
    // Use with_meta to avoid locking Run
    let mut context = ExecutionContext::with_meta(
        nodes,
        &weak_run,
        run_meta,
        handler,
        &target.node,
        variables,
        cache,
        log_level,
        stage.clone(),
        profile.clone(),
        callback.clone(),
        completion_callbacks.clone(),
        credentials,
        token,
        oauth_tokens,
    )
    .await;
    context.user_context = user_context;
    if let Some(token) = cancellation_token {
        context.set_cancellation_token(token);
    }
    context.started_by = if target.through_pins.is_empty() {
        None
    } else {
        Some(target.through_pins.clone())
    };

    if context.is_cancelled() {
        context.log_message(&cancellation_log_message, cancellation_log_level);
        has_node_errors.store(true, Ordering::Relaxed);
        let traces = context.take_traces();
        return Ok((Vec::new(), traces));
    }

    if USE_DEPENDENCY_GRAPH {
        if let Err(err) =
            InternalNode::trigger_with_dependencies(&mut context, &mut None, false, dependencies)
                .await
        {
            eprintln!("[Error] executing node: {:?}", err);
        }
    } else if let Err(err) = InternalNode::trigger(&mut context, &mut None, false).await {
        eprintln!("[Error] executing node: {:?}", err);
    }

    let traces = context.take_traces();

    let state = context.get_state();

    if state == NodeState::Success {
        let connected = target
            .node
            .get_connected_exec(true, &context)
            .await
            .unwrap();
        drop(context);
        let mut connected_nodes = Vec::with_capacity(connected.len());
        for connected_node in connected {
            connected_nodes.push(connected_node);
        }
        return Ok((connected_nodes, traces));
    }

    // Flag this run as having node errors so RunStatus reflects the failure
    has_node_errors.store(true, Ordering::Relaxed);

    // Return traces even on failure so error logs are not silently dropped
    Ok((Vec::new(), traces))
}

#[derive(Deserialize)]
struct Claims {
    sub: String,
}

pub fn extract_sub_from_jwt(token: &str) -> flow_like_types::Result<String> {
    // Accept "Bearer " case-insensitively and trim
    let raw = token.trim();
    let raw = raw
        .strip_prefix("Bearer ")
        .or_else(|| raw.strip_prefix("bearer "))
        .unwrap_or(raw);

    // Require exactly 3 segments: header.payload.signature
    let mut parts = raw.split('.');
    let _header_b64 = parts
        .next()
        .ok_or_else(|| anyhow!("invalid JWT: missing header segment"))?;
    let payload_b64 = parts
        .next()
        .ok_or_else(|| anyhow!("invalid JWT: missing payload segment"))?;
    let _sig_b64 = parts
        .next()
        .ok_or_else(|| anyhow!("invalid JWT: missing signature segment"))?;
    if parts.next().is_some() {
        return Err(anyhow!("invalid JWT: too many segments"));
    }

    // Decode payload (support both unpadded and padded base64url)
    let decoded = URL_SAFE_NO_PAD
        .decode(payload_b64.as_bytes())
        .or_else(|_| URL_SAFE.decode(payload_b64.as_bytes()))
        .context("failed to base64url-decode JWT payload")?;

    // Minimal, typed deserialize for clarity/perf
    let claims: Claims =
        flow_like_types::json::from_slice(&decoded).context("invalid JWT JSON payload")?;

    Ok(claims.sub)
}

pub async fn flush_run_cancelled(
    run: &Arc<Mutex<Run>>,
) -> flow_like_types::Result<Option<LogMeta>> {
    let prepared = {
        let mut run = flow_like_types::tokio::time::timeout(RUN_LOCK_TIMEOUT, run.lock())
            .await
            .map_err(|_| anyhow!("Timeout acquiring run lock for cancel flush"))?;
        run.highest_log_level = LogLevel::Fatal;
        run.status = RunStatus::Stopped;
        run.end = std::time::SystemTime::now();

        let cancel_log = LogMessage::new("Run cancelled", LogLevel::Fatal, None);
        if let Some(trace) = run.traces.last_mut() {
            trace.logs.push(cancel_log);
        } else {
            let mut system_trace = Trace::new(
                run.visited_nodes
                    .keys()
                    .next()
                    .map(String::as_str)
                    .unwrap_or("system"),
            );
            system_trace.logs.push(cancel_log);
            run.push_trace(system_trace);
        }

        run.prepare_flush(true)?
    };

    if let Some(prepared) = prepared {
        let result = prepared.write().await?;
        if result.created_table {
            let mut run = flow_like_types::tokio::time::timeout(RUN_LOCK_TIMEOUT, run.lock())
                .await
                .map_err(|_| anyhow!("Timeout acquiring run lock after cancel flush"))?;
            run.log_initialized = true;
        }
        Ok(result.meta)
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::node::{Node, NodeLogic};
    use flow_like_types::{async_trait, tokio};

    struct NoopLogic;

    #[async_trait]
    impl NodeLogic for NoopLogic {
        fn get_node(&self) -> Node {
            Node::new("noop", "Noop", "Noop", "Tests")
        }

        async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn background_flush_wait_is_interrupted_by_cancellation() {
        let cancel = CancellationToken::new();
        let task_cancel = cancel.clone();
        let task = flow_like_types::tokio::spawn(async move {
            let mut interval = flow_like_types::tokio::time::interval(Duration::from_secs(60));
            interval.tick().await;
            wait_for_flush_tick_or_cancel(&mut interval, &task_cancel).await
        });

        flow_like_types::tokio::task::yield_now().await;
        cancel.cancel();

        let ticked = flow_like_types::tokio::time::timeout(Duration::from_secs(1), task)
            .await
            .expect("flush wait should stop promptly")
            .expect("flush wait task should complete");
        assert!(!ticked);
    }

    #[test]
    fn fork_helpers_reseed_entry_and_clear_execution_counts() {
        let node_definition = Node::new("noop", "Noop", "Noop", "Tests");
        let node_id = node_definition.id.clone();
        let node = Arc::new(InternalNode::new(
            node_definition,
            AHashMap::new(),
            Arc::new(NoopLogic),
            AHashMap::new(),
        ));
        node.exec_calls.store(17, Ordering::Relaxed);
        let nodes = AHashMap::from([(node_id.clone(), node.clone())]);
        let nodes_executed = AtomicU64::new(23);

        reset_execution_counters(&nodes, &nodes_executed);
        let stack = stack_for_entry(&nodes, &node_id).expect("entry node should seed the stack");

        assert_eq!(node.exec_calls.load(Ordering::Relaxed), 0);
        assert_eq!(nodes_executed.load(Ordering::Relaxed), 0);
        assert_eq!(stack.len(), 1);
        assert_eq!(stack.stack[0].node.node_id(), node_id);
    }

    #[test]
    fn offline_apps_are_not_attributed_as_server_apps() {
        assert_eq!(
            model_usage_app_id_for_visibility("local-app", &AppVisibility::Offline),
            None
        );
    }

    #[test]
    fn server_backed_apps_keep_app_attribution() {
        for visibility in [
            AppVisibility::Public,
            AppVisibility::PublicRequestAccess,
            AppVisibility::Private,
            AppVisibility::Prototype,
        ] {
            assert_eq!(
                model_usage_app_id_for_visibility("cloud-app", &visibility),
                Some("cloud-app".to_string())
            );
        }
    }
}
