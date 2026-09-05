//! Types for execution state store abstraction

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

/// Event payloads whose serialized form exceeds this go to the content store,
/// leaving only a reference on the record. Every backend shares the boundary so
/// that it does not move when a deployment changes cloud.
pub const PAYLOAD_OFFLOAD_BYTES: usize = 100 * 1024;

/// How long an event row lives, set where events are pushed
/// (`routes/execution/progress.rs`). A staged object outlives its row by at
/// most this long before the TTL sweep drains both.
pub const EVENT_TTL_SECS: u64 = 24 * 60 * 60;

/// How old a staged object must be before an age-based sweep may delete it
/// without consulting a row. Two event lifetimes: an object younger than this
/// can still belong to a live row, to a row the TTL sweep has not reached, or
/// to an insert that is still in flight.
pub const STAGED_PAYLOAD_MIN_AGE_SECS: u64 = 2 * EVENT_TTL_SECS;

/// `EXECUTION_STAGED_PAYLOAD_MIN_AGE_SECS`, floored at one event lifetime so a
/// misconfiguration cannot delete objects out from under live rows.
pub fn staged_payload_min_age_secs() -> u64 {
    std::env::var("EXECUTION_STAGED_PAYLOAD_MIN_AGE_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(STAGED_PAYLOAD_MIN_AGE_SECS)
        .max(EVENT_TTL_SECS)
}

/// What one age-based sweep of the staged-payload prefix achieved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StagedPayloadSweep {
    /// Objects the listing walked.
    pub scanned: u64,
    /// Objects old enough to have lost their row, and removed.
    pub deleted: u64,
    /// The scan budget ran out while objects were still listed; call again.
    pub stopped_early: bool,
}

/// Run status enum (matches Prisma schema)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Timeout,
}

impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Cancelled | Self::Timeout
        )
    }
}

/// Run mode enum (matches Prisma schema)
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunMode {
    Local,
    Http,
    Lambda,
    KubernetesIsolated,
    KubernetesPool,
    Function,
    Queue,
}

/// Run variant enum (matches Prisma schema). Records serialized before this
/// field existed deserialize as `Primary` via the `Default` impl.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RunVariant {
    #[default]
    Primary,
    Canary,
    Shadow,
    Regression,
}

/// Execution run record
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionRunRecord {
    pub id: String,
    pub board_id: String,
    pub version: Option<String>,
    pub event_id: Option<String>,
    pub status: RunStatus,
    pub mode: RunMode,
    #[serde(default)]
    pub run_variant: RunVariant,
    pub variant_name: Option<String>,
    pub shadow_of_run_id: Option<String>,
    pub regression_run_id: Option<String>,
    pub input_payload_len: i64,
    pub output_payload_len: i64,
    pub error_message: Option<String>,
    pub progress: i32,
    pub current_step: Option<String>,
    pub started_at: Option<i64>,   // Unix timestamp ms
    pub completed_at: Option<i64>, // Unix timestamp ms
    pub expires_at: Option<i64>,   // Unix timestamp ms
    pub user_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub app_id: String,
    pub created_at: i64, // Unix timestamp ms
    pub updated_at: i64, // Unix timestamp ms
}

/// Input for creating a new run
#[derive(Clone, Debug)]
pub struct CreateRunInput {
    pub id: String,
    pub board_id: String,
    pub version: Option<String>,
    pub event_id: Option<String>,
    pub mode: RunMode,
    pub run_variant: RunVariant,
    pub variant_name: Option<String>,
    pub shadow_of_run_id: Option<String>,
    pub regression_run_id: Option<String>,
    pub input_payload_len: i64,
    pub user_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub app_id: String,
    pub expires_at: Option<i64>,
}

/// Input for updating run progress
#[derive(Clone, Debug, Default)]
pub struct UpdateRunInput {
    pub progress: Option<i32>,
    pub current_step: Option<String>,
    pub status: Option<RunStatus>,
    pub output_payload_len: Option<i64>,
    pub error_message: Option<String>,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
}

/// Execution event record
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionEventRecord {
    pub id: String,
    pub run_id: String,
    pub sequence: i32,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub delivered: bool,
    pub expires_at: i64, // Unix timestamp ms
    pub created_at: i64, // Unix timestamp ms
}

/// Input for creating events
#[derive(Clone, Debug)]
pub struct CreateEventInput {
    pub id: String,
    pub run_id: String,
    pub sequence: i32,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub expires_at: i64,
}

/// The id an executor derives for `(run_id, sequence)`. Mirrors
/// `routes::execution::progress::execution_event_id`, which mints it.
pub fn canonical_execution_event_id(run_id: &str, sequence: i32) -> String {
    let digest = blake3::hash(format!("{run_id}:{sequence}").as_bytes());
    format!("evt-{}", digest.to_hex())
}

/// Whether a retry of this event would carry the same id, which is what makes
/// "already stored" a safe answer to "should I write it again".
pub fn has_canonical_identity(event: &CreateEventInput) -> bool {
    event.id == canonical_execution_event_id(&event.run_id, event.sequence)
}

/// Guard for cold imports from the canonical SQL run store: an expired source
/// row must never be re-imported into a live store, otherwise delete/import
/// cycles resurrect it (and TTL-based backends re-import it on every poll).
/// Terminal-but-unexpired rows still import — queue redelivery convergence
/// depends on reading the terminal state.
pub fn source_run_expired(record: &ExecutionRunRecord, now_ms: i64) -> bool {
    record
        .expires_at
        .is_some_and(|expires_at| expires_at <= now_ms)
}

/// Query options for listing events
#[derive(Clone, Debug, Default)]
pub struct EventQuery {
    pub run_id: String,
    pub after_sequence: Option<i32>,
    pub only_undelivered: bool,
    pub limit: Option<i32>,
}

/// Error type for state store operations
#[derive(Debug, thiserror::Error)]
pub enum StateStoreError {
    #[error("Record not found")]
    NotFound,
    #[error("Database error: {0}")]
    Database(String),
    #[error("Serialization error: {0}")]
    Serialization(String),
    #[error("Connection error: {0}")]
    Connection(String),
    #[error("Configuration error: {0}")]
    Configuration(String),
    #[error("Execution lease conflict: {0}")]
    LeaseConflict(String),
}

/// Atomic result of claiming a queued run for one broker delivery.
#[derive(Clone, Debug)]
pub enum RunLeaseClaim {
    Acquired {
        run: ExecutionRunRecord,
        expires_at: i64,
    },
    Busy {
        run: ExecutionRunRecord,
        expires_at: i64,
    },
    Terminal {
        run: ExecutionRunRecord,
    },
}

/// Trait for execution state storage backends
#[async_trait]
pub trait ExecutionStateStore: Send + Sync + Debug {
    /// Get backend name for logging
    fn backend_name(&self) -> &'static str;

    // ========================================================================
    // Run operations
    // ========================================================================

    /// Create a new execution run
    async fn create_run(
        &self,
        input: CreateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError>;

    /// Get a run by ID
    async fn get_run(&self, run_id: &str) -> Result<Option<ExecutionRunRecord>, StateStoreError>;

    /// Get a run by ID, verifying it belongs to the given app
    async fn get_run_for_app(
        &self,
        run_id: &str,
        app_id: &str,
    ) -> Result<Option<ExecutionRunRecord>, StateStoreError>;

    /// Update run progress/status
    async fn update_run(
        &self,
        run_id: &str,
        input: UpdateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError>;

    /// Trusted control-plane cancellation, called only after the execution
    /// manager confirms termination and blocks subsequent launches for this run.
    /// This revokes delivery ownership without requiring the killed runner's token.
    async fn cancel_run_after_termination(
        &self,
        _run_id: &str,
        _app_id: &str,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        Err(StateStoreError::Configuration(
            "confirmed execution cancellation is not supported by this state backend".into(),
        ))
    }

    /// Atomically bind a queued run to `job_id` and grant or renew ownership
    /// for one delivery token. Backends without a conditional-write lease
    /// implementation fail closed.
    async fn claim_run_lease(
        &self,
        _run_id: &str,
        _app_id: &str,
        _job_id: &str,
        _lease_token: &str,
        _lease_duration_ms: i64,
    ) -> Result<RunLeaseClaim, StateStoreError> {
        Err(StateStoreError::Configuration(
            "execution leases are not supported by this state backend".to_string(),
        ))
    }

    /// Persist terminal state only while the caller still owns the run lease.
    async fn update_run_with_lease(
        &self,
        _run_id: &str,
        _app_id: &str,
        _job_id: &str,
        _lease_token: &str,
        _input: UpdateRunInput,
    ) -> Result<ExecutionRunRecord, StateStoreError> {
        Err(StateStoreError::Configuration(
            "execution leases are not supported by this state backend".to_string(),
        ))
    }

    /// Verify that a callback is from the current, unexpired delivery owner.
    async fn validate_run_lease(
        &self,
        _run_id: &str,
        _app_id: &str,
        _job_id: &str,
        _lease_token: &str,
    ) -> Result<(), StateStoreError> {
        Err(StateStoreError::Configuration(
            "execution leases are not supported by this state backend".to_string(),
        ))
    }

    /// List runs for an app (with pagination)
    async fn list_runs_for_app(
        &self,
        app_id: &str,
        limit: i32,
        cursor: Option<&str>,
    ) -> Result<Vec<ExecutionRunRecord>, StateStoreError>;

    /// Delete expired runs (for cleanup jobs)
    async fn delete_expired_runs(&self) -> Result<i64, StateStoreError>;

    // ========================================================================
    // Event operations
    // ========================================================================

    /// Push events for a run
    async fn push_events(&self, events: Vec<CreateEventInput>) -> Result<i32, StateStoreError>;

    /// Get events for a run
    async fn get_events(
        &self,
        query: EventQuery,
    ) -> Result<Vec<ExecutionEventRecord>, StateStoreError>;

    /// Get the max sequence number for a run
    async fn get_max_sequence(&self, run_id: &str) -> Result<i32, StateStoreError>;

    /// Mark events of one run as delivered. Every ID must belong to `run_id`;
    /// backends use it to scope the lookup to that run's events.
    async fn mark_events_delivered(
        &self,
        run_id: &str,
        event_ids: &[String],
    ) -> Result<(), StateStoreError>;

    /// Delete expired events (for cleanup jobs)
    async fn delete_expired_events(&self) -> Result<i64, StateStoreError>;

    /// Delete staged payload objects older than `min_age_secs`.
    ///
    /// The row is the only pointer to a staged object, so an object whose row
    /// was never committed — a write that failed after the object went up, a
    /// partially applied multi-chunk insert — is unreachable by any row-driven
    /// cleanup. Age is the one property such an object still carries. Backends
    /// whose store expires staged objects on its own do nothing here.
    async fn sweep_staged_payloads(
        &self,
        _min_age_secs: u64,
    ) -> Result<StagedPayloadSweep, StateStoreError> {
        Ok(StagedPayloadSweep::default())
    }
}
