//! Scheduler trait definitions

use crate::CronSinkConfig;

/// Result type for scheduler operations
pub type SchedulerResult<T> = Result<T, SchedulerError>;

/// Error type for scheduler operations
#[derive(Debug, thiserror::Error)]
pub enum SchedulerError {
    #[error("Schedule not found: {0}")]
    NotFound(String),

    #[error("Schedule already exists: {0}")]
    AlreadyExists(String),

    #[error("Invalid cron expression: {0}")]
    InvalidCronExpression(String),

    #[error("Provider error: {0}")]
    ProviderError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Internal error: {0}")]
    Internal(String),
}

/// Information about an existing schedule
#[derive(Debug, Clone)]
pub struct ScheduleInfo {
    /// The event ID this schedule triggers
    pub event_id: String,

    /// The cron expression
    pub cron_expression: String,

    /// Whether the schedule is currently active
    pub active: bool,

    /// Last time this schedule was triggered (if known)
    pub last_triggered: Option<chrono::DateTime<chrono::Utc>>,

    /// Next scheduled trigger time (if known)
    pub next_trigger: Option<chrono::DateTime<chrono::Utc>>,
}

/// Trait for scheduler backend implementations
///
/// Each implementation handles the platform-specific details of creating,
/// managing, and triggering scheduled events. All implementations ultimately
/// call the central API's `/api/v1/sink/trigger` endpoint.
#[async_trait::async_trait]
pub trait SchedulerBackend: Send + Sync {
    /// Validate a schedule configuration without mutating provider state.
    async fn validate_schedule(
        &self,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        let _ = (cron_expr, config);
        Ok(())
    }

    /// Create a new schedule for an event
    ///
    /// # Arguments
    /// * `event_id` - Unique identifier for the event
    /// * `cron_expr` - Cron expression for the schedule
    /// * `config` - Additional configuration for the cron sink
    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()>;

    /// Update an existing schedule
    ///
    /// This may delete and recreate the schedule on some platforms.
    async fn update_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()>;

    /// Delete a schedule
    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()>;

    /// Enable a previously disabled schedule
    async fn enable_schedule(&self, event_id: &str) -> SchedulerResult<()>;

    /// Disable a schedule without deleting it
    async fn disable_schedule(&self, event_id: &str) -> SchedulerResult<()>;

    /// Check if a schedule exists
    async fn schedule_exists(&self, event_id: &str) -> SchedulerResult<bool>;

    /// Get information about a schedule
    async fn get_schedule(&self, event_id: &str) -> SchedulerResult<Option<ScheduleInfo>>;

    /// List all schedules (with optional pagination)
    async fn list_schedules(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> SchedulerResult<Vec<ScheduleInfo>>;
}
