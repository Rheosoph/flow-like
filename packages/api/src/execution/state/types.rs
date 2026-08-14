//! Types for execution state store abstraction

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::fmt::Debug;

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

/// Execution run record
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionRunRecord {
    pub id: String,
    pub board_id: String,
    pub version: Option<String>,
    pub event_id: Option<String>,
    pub status: RunStatus,
    pub mode: RunMode,
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

    /// Mark events as delivered
    async fn mark_events_delivered(&self, event_ids: &[String]) -> Result<(), StateStoreError>;

    /// Delete expired events (for cleanup jobs)
    async fn delete_expired_events(&self) -> Result<i64, StateStoreError>;
}
