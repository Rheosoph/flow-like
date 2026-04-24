//! AWS EventBridge Scheduler implementation
//!
//! Uses AWS EventBridge Scheduler to create and manage cron and one-time schedules.
//! Each schedule invokes a Lambda function that calls the central API's
//! `/sink/trigger/async` endpoint.
//!
//! References:
//! - Schedule types: <https://docs.aws.amazon.com/scheduler/latest/UserGuide/schedule-types.html>
//! - UpdateSchedule (replace-all semantics):
//!   <https://docs.aws.amazon.com/scheduler/latest/APIReference/API_UpdateSchedule.html>
//! - Quotas (name length 64, client-token idempotency window):
//!   <https://docs.aws.amazon.com/scheduler/latest/UserGuide/scheduler-quotas.html>

use super::{
    ScheduleInfo, SchedulerBackend, SchedulerError, SchedulerResult, standard_to_aws_cron,
};
use crate::CronSinkConfig;

/// AWS EventBridge Scheduler configuration
#[derive(Debug, Clone)]
pub struct AwsEventBridgeConfig {
    /// ARN of the Lambda function to invoke
    pub target_arn: String,
    /// ARN of the IAM role for the scheduler
    pub role_arn: String,
    /// Schedule group name (optional, defaults to "flow-like")
    pub group_name: String,
    /// ARN of the SQS dead-letter queue for failed schedule invocations
    pub dlq_arn: Option<String>,
    /// Maximum retry attempts for the Lambda target (default: 3).
    /// AWS default is 185 over 24h which silently duplicates executions when
    /// the API is transiently unhealthy.
    pub max_retry_attempts: i32,
    /// Maximum event age in seconds for the Lambda target (default: 300 = 5 min).
    /// Beyond this the invocation is dropped (or sent to DLQ when configured).
    pub max_event_age_seconds: i32,
}

impl AwsEventBridgeConfig {
    /// Create configuration from environment variables
    pub fn from_env() -> Result<Self, SchedulerError> {
        Ok(Self {
            target_arn: std::env::var("EVENTBRIDGE_TARGET_ARN").map_err(|_| {
                SchedulerError::ConfigError("EVENTBRIDGE_TARGET_ARN not set".into())
            })?,
            role_arn: std::env::var("EVENTBRIDGE_ROLE_ARN")
                .map_err(|_| SchedulerError::ConfigError("EVENTBRIDGE_ROLE_ARN not set".into()))?,
            group_name: std::env::var("EVENTBRIDGE_GROUP_NAME")
                .unwrap_or_else(|_| "flow-like".to_string()),
            dlq_arn: std::env::var("EVENTBRIDGE_DLQ_ARN").ok(),
            max_retry_attempts: std::env::var("EVENTBRIDGE_MAX_RETRY_ATTEMPTS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(3),
            max_event_age_seconds: std::env::var("EVENTBRIDGE_MAX_EVENT_AGE_SECONDS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(300),
        })
    }
}

/// AWS EventBridge Scheduler implementation
#[cfg(feature = "aws")]
pub struct AwsEventBridgeScheduler {
    config: AwsEventBridgeConfig,
    client: aws_sdk_scheduler::Client,
}

#[cfg(not(feature = "aws"))]
pub struct AwsEventBridgeScheduler {
    config: AwsEventBridgeConfig,
}

/// AWS schedule-name allowed characters: `[0-9a-zA-Z-_.]`, max length 64.
fn sanitize_schedule_name(event_id: &str) -> String {
    const PREFIX: &str = "flow-like-cron-";
    const MAX: usize = 64;

    let sanitized: String = event_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();

    let budget = MAX.saturating_sub(PREFIX.len());
    let sanitized = if sanitized.len() > budget {
        // Deterministically truncate by prefix + short hash suffix so names stay unique.
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::hash::Hash::hash(event_id, &mut hasher);
        let suffix = format!("-{:x}", std::hash::Hasher::finish(&hasher));
        let head_len = budget.saturating_sub(suffix.len());
        format!("{}{}", &sanitized[..head_len], suffix)
    } else {
        sanitized
    };

    format!("{}{}", PREFIX, sanitized)
}

#[cfg(feature = "aws")]
impl AwsEventBridgeScheduler {
    /// Create a new scheduler from environment variables
    pub async fn from_env() -> Self {
        let config = AwsEventBridgeConfig::from_env()
            .expect("Failed to load EventBridge config from environment");
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = aws_sdk_scheduler::Client::new(&aws_config);
        Self { config, client }
    }

    /// Create a new scheduler with explicit configuration
    pub async fn new(config: AwsEventBridgeConfig) -> Self {
        let aws_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = aws_sdk_scheduler::Client::new(&aws_config);
        Self { config, client }
    }

    fn schedule_name(&self, event_id: &str) -> String {
        sanitize_schedule_name(event_id)
    }

    fn schedule_description(event_id: &str) -> String {
        format!("Flow-Like cron schedule for event {}", event_id)
    }

    fn build_target(&self, event_id: &str) -> SchedulerResult<aws_sdk_scheduler::types::Target> {
        use aws_sdk_scheduler::types::{DeadLetterConfig, RetryPolicy, Target};

        let retry_policy = RetryPolicy::builder()
            .maximum_retry_attempts(self.config.max_retry_attempts)
            .maximum_event_age_in_seconds(self.config.max_event_age_seconds)
            .build();

        let mut builder = Target::builder()
            .arn(&self.config.target_arn)
            .role_arn(&self.config.role_arn)
            .input(serde_json::json!({ "event_id": event_id }).to_string())
            .retry_policy(retry_policy);

        if let Some(dlq_arn) = &self.config.dlq_arn {
            builder = builder.dead_letter_config(DeadLetterConfig::builder().arn(dlq_arn).build());
        }

        builder
            .build()
            .map_err(|e| SchedulerError::ProviderError(format!("Failed to build target: {}", e)))
    }

    /// Validate and build the schedule expression. For one-time `at(...)`
    /// schedules the resolved UTC instant is checked to be in the future.
    fn build_schedule_expression(
        config: &CronSinkConfig,
        cron_expr: &str,
    ) -> SchedulerResult<(String, bool)> {
        if config.is_one_time() {
            let at_expr = config.effective_expression().ok_or_else(|| {
                SchedulerError::InvalidCronExpression(
                    "Missing scheduled_for for one-time schedule".into(),
                )
            })?;

            // Reject past one-time schedules — AWS accepts them silently and
            // the user never gets a firing.
            if let Some(instant) = config.scheduled_instant_utc()
                && instant <= chrono::Utc::now()
            {
                return Err(SchedulerError::InvalidCronExpression(format!(
                    "scheduled time {} is in the past (timezone: {})",
                    at_expr, config.timezone
                )));
            }

            Ok((at_expr, true))
        } else {
            let aws_cron = standard_to_aws_cron(cron_expr)?;
            Ok((aws_cron, false))
        }
    }

    /// Classify an AWS SDK error string. SDK errors are stringified because the
    /// concrete error types differ per operation; matching on substrings keeps
    /// call sites uniform.
    fn not_found(err: &str) -> bool {
        err.contains("ResourceNotFoundException")
    }

    fn already_exists(err: &str) -> bool {
        err.contains("ConflictException")
    }

    fn state_for(active: bool) -> aws_sdk_scheduler::types::ScheduleState {
        if active {
            aws_sdk_scheduler::types::ScheduleState::Enabled
        } else {
            aws_sdk_scheduler::types::ScheduleState::Disabled
        }
    }
}

#[cfg(not(feature = "aws"))]
impl AwsEventBridgeScheduler {
    /// Create a new scheduler (stub without AWS SDK)
    pub fn from_env() -> Self {
        let config = AwsEventBridgeConfig::from_env()
            .expect("Failed to load EventBridge config from environment");
        Self { config }
    }

    /// Create a new scheduler with explicit configuration
    pub fn new(config: AwsEventBridgeConfig) -> Self {
        Self { config }
    }

    fn schedule_name(&self, event_id: &str) -> String {
        sanitize_schedule_name(event_id)
    }
}

#[cfg(feature = "aws")]
#[async_trait::async_trait]
impl SchedulerBackend for AwsEventBridgeScheduler {
    async fn validate_schedule(
        &self,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        Self::build_schedule_expression(config, cron_expr).map(|_| ())
    }

    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        use aws_sdk_scheduler::types::{
            ActionAfterCompletion, FlexibleTimeWindow, FlexibleTimeWindowMode,
        };

        let schedule_name = self.schedule_name(event_id);
        let target = self.build_target(event_id)?;

        let flexible_time_window = FlexibleTimeWindow::builder()
            .mode(FlexibleTimeWindowMode::Off)
            .build()
            .map_err(|e| {
                SchedulerError::ProviderError(format!("Failed to build time window: {}", e))
            })?;

        let (schedule_expression, is_one_time) =
            Self::build_schedule_expression(config, cron_expr)?;

        let mut req = self
            .client
            .create_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .description(Self::schedule_description(event_id))
            .schedule_expression(&schedule_expression)
            .schedule_expression_timezone(&config.timezone)
            .state(Self::state_for(config.active))
            .flexible_time_window(flexible_time_window)
            .target(target)
            // Fresh idempotency token per attempt: reusing the schedule name as
            // token (previous behavior) means retries within AWS's 10-min window
            // replay the cached response instead of re-creating after a delete.
            .client_token(flow_like_types::create_id());

        if is_one_time {
            req = req.action_after_completion(ActionAfterCompletion::Delete);
        }

        match req.send().await {
            Ok(_) => {
                tracing::info!(
                    event_id = %event_id,
                    schedule_name = %schedule_name,
                    expression = %schedule_expression,
                    tz = %config.timezone,
                    one_time = is_one_time,
                    active = config.active,
                    "Created EventBridge schedule"
                );
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if Self::already_exists(&msg) {
                    Err(SchedulerError::AlreadyExists(schedule_name))
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        msg
                    )))
                }
            }
        }
    }

    async fn update_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        use aws_sdk_scheduler::types::{
            ActionAfterCompletion, FlexibleTimeWindow, FlexibleTimeWindowMode,
        };

        let schedule_name = self.schedule_name(event_id);
        let target = self.build_target(event_id)?;

        let flexible_time_window = FlexibleTimeWindow::builder()
            .mode(FlexibleTimeWindowMode::Off)
            .build()
            .map_err(|e| {
                SchedulerError::ProviderError(format!("Failed to build time window: {}", e))
            })?;

        let (schedule_expression, is_one_time) =
            Self::build_schedule_expression(config, cron_expr)?;

        // UpdateSchedule uses replace-all semantics: every optional field we
        // omit is reset to its system default (e.g. State -> ENABLED). We set
        // every relevant field explicitly to preserve intent.
        let mut req = self
            .client
            .update_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .description(Self::schedule_description(event_id))
            .schedule_expression(&schedule_expression)
            .schedule_expression_timezone(&config.timezone)
            .state(Self::state_for(config.active))
            .flexible_time_window(flexible_time_window)
            .target(target);

        if is_one_time {
            req = req.action_after_completion(ActionAfterCompletion::Delete);
        }

        match req.send().await {
            Ok(_) => {
                tracing::info!(
                    event_id = %event_id,
                    schedule_name = %schedule_name,
                    expression = %schedule_expression,
                    tz = %config.timezone,
                    one_time = is_one_time,
                    active = config.active,
                    "Updated EventBridge schedule"
                );
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if Self::not_found(&msg) {
                    Err(SchedulerError::NotFound(schedule_name))
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        msg
                    )))
                }
            }
        }
    }

    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        let schedule_name = self.schedule_name(event_id);

        match self
            .client
            .delete_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            // Fresh token per attempt so a retry after a transient failure can
            // actually re-delete rather than replay a cached success.
            .client_token(flow_like_types::create_id())
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!(event_id = %event_id, "Deleted EventBridge schedule");
                Ok(())
            }
            Err(e) => {
                let err_str = e.to_string();
                if Self::not_found(&err_str) {
                    tracing::debug!(event_id = %event_id, "Schedule already deleted");
                    Ok(())
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        err_str
                    )))
                }
            }
        }
    }

    async fn enable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        self.set_state(event_id, true).await
    }

    async fn disable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        self.set_state(event_id, false).await
    }

    async fn schedule_exists(&self, event_id: &str) -> SchedulerResult<bool> {
        let schedule_name = self.schedule_name(event_id);

        match self
            .client
            .get_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .send()
            .await
        {
            Ok(_) => Ok(true),
            Err(e) => {
                let err_str = e.to_string();
                if Self::not_found(&err_str) {
                    Ok(false)
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        err_str
                    )))
                }
            }
        }
    }

    async fn get_schedule(&self, event_id: &str) -> SchedulerResult<Option<ScheduleInfo>> {
        use aws_sdk_scheduler::types::ScheduleState;

        let schedule_name = self.schedule_name(event_id);

        match self
            .client
            .get_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .send()
            .await
        {
            Ok(response) => {
                // Preserve the original shape: `cron(...)` and `at(...)` both
                // round-trip unchanged so callers can distinguish recurring
                // from one-time schedules.
                let cron_expr = response.schedule_expression.unwrap_or_default();
                let active = response.state == Some(ScheduleState::Enabled);

                Ok(Some(ScheduleInfo {
                    event_id: event_id.to_string(),
                    cron_expression: cron_expr,
                    active,
                    last_triggered: None,
                    next_trigger: None,
                }))
            }
            Err(e) => {
                let err_str = e.to_string();
                if Self::not_found(&err_str) {
                    Ok(None)
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        err_str
                    )))
                }
            }
        }
    }

    async fn list_schedules(
        &self,
        limit: Option<usize>,
        offset: Option<usize>,
    ) -> SchedulerResult<Vec<ScheduleInfo>> {
        // AWS caps max_results at 100 per page; clamp so callers who ask for
        // more don't hit a ValidationException.
        const AWS_MAX_PAGE: i32 = 100;

        let skip = offset.unwrap_or(0);
        let want = limit.map(|l| l.saturating_add(skip));
        let mut schedules = Vec::new();
        let mut next_token: Option<String> = None;

        loop {
            let mut request = self
                .client
                .list_schedules()
                .group_name(&self.config.group_name)
                .name_prefix("flow-like-cron-");

            if let Some(token) = &next_token {
                request = request.next_token(token);
            }

            let page_size = match want {
                Some(w) => (w.saturating_sub(schedules.len()) as i32).min(AWS_MAX_PAGE),
                None => AWS_MAX_PAGE,
            };
            if page_size > 0 {
                request = request.max_results(page_size);
            }

            let response = request
                .send()
                .await
                .map_err(|e| SchedulerError::ProviderError(format!("AWS SDK error: {}", e)))?;

            for schedule in response.schedules {
                let name = schedule.name.unwrap_or_default();
                let event_id = name.strip_prefix("flow-like-cron-").unwrap_or(&name);

                schedules.push(ScheduleInfo {
                    event_id: event_id.to_string(),
                    cron_expression: String::new(),
                    active: schedule.state
                        == Some(aws_sdk_scheduler::types::ScheduleState::Enabled),
                    last_triggered: None,
                    next_trigger: None,
                });
            }

            next_token = response.next_token;
            if next_token.is_none() {
                break;
            }
            if let Some(w) = want
                && schedules.len() >= w
            {
                break;
            }
        }

        if skip >= schedules.len() {
            return Ok(Vec::new());
        }
        let mut out: Vec<ScheduleInfo> = schedules.into_iter().skip(skip).collect();
        if let Some(l) = limit {
            out.truncate(l);
        }

        Ok(out)
    }
}

#[cfg(feature = "aws")]
impl AwsEventBridgeScheduler {
    /// Toggle a schedule's enabled state. UpdateSchedule resets omitted optional
    /// fields, so we GET the current schedule, mutate only `state`, and re-send
    /// all preserved fields.
    async fn set_state(&self, event_id: &str, active: bool) -> SchedulerResult<()> {
        use aws_sdk_scheduler::types::ActionAfterCompletion;

        let schedule_name = self.schedule_name(event_id);

        let current = match self
            .client
            .get_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .send()
            .await
        {
            Ok(c) => c,
            Err(e) => {
                let msg = e.to_string();
                if Self::not_found(&msg) {
                    return Err(SchedulerError::NotFound(schedule_name));
                }
                return Err(SchedulerError::ProviderError(format!(
                    "AWS SDK error: {}",
                    msg
                )));
            }
        };

        let target = current
            .target
            .ok_or_else(|| SchedulerError::ProviderError("Schedule has no target".to_string()))?;
        let flexible_time_window = current.flexible_time_window.ok_or_else(|| {
            SchedulerError::ProviderError("Schedule has no time window".to_string())
        })?;
        let schedule_expression = current.schedule_expression.ok_or_else(|| {
            SchedulerError::ProviderError("Schedule has no expression".to_string())
        })?;
        let timezone = current
            .schedule_expression_timezone
            .unwrap_or_else(|| "UTC".to_string());

        let mut update = self
            .client
            .update_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .schedule_expression(&schedule_expression)
            .schedule_expression_timezone(&timezone)
            .state(Self::state_for(active))
            .flexible_time_window(flexible_time_window)
            .target(target);

        if let Some(desc) = current.description {
            update = update.description(desc);
        }
        if let Some(action) = current.action_after_completion {
            // Preserve Delete-on-completion for one-time schedules; otherwise
            // the replace-all semantics would reset it to NONE.
            if action == ActionAfterCompletion::Delete {
                update = update.action_after_completion(action);
            }
        }

        match update.send().await {
            Ok(_) => {
                tracing::info!(event_id = %event_id, active, "Toggled EventBridge schedule state");
                Ok(())
            }
            Err(e) => {
                let msg = e.to_string();
                if Self::not_found(&msg) {
                    Err(SchedulerError::NotFound(schedule_name))
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        msg
                    )))
                }
            }
        }
    }
}

// Stub implementation when AWS feature is disabled
#[cfg(not(feature = "aws"))]
#[async_trait::async_trait]
impl SchedulerBackend for AwsEventBridgeScheduler {
    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        _config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        tracing::warn!(
            event_id = %event_id,
            cron = %cron_expr,
            "AWS feature not enabled - schedule not created"
        );
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled. Compile with --features aws".into(),
        ))
    }

    async fn update_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        _config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, cron = %cron_expr, "AWS feature not enabled");
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled".into(),
        ))
    }

    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, "AWS feature not enabled");
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled".into(),
        ))
    }

    async fn enable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, "AWS feature not enabled");
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled".into(),
        ))
    }

    async fn disable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        tracing::warn!(event_id = %event_id, "AWS feature not enabled");
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled".into(),
        ))
    }

    async fn schedule_exists(&self, _event_id: &str) -> SchedulerResult<bool> {
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled".into(),
        ))
    }

    async fn get_schedule(&self, _event_id: &str) -> SchedulerResult<Option<ScheduleInfo>> {
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled".into(),
        ))
    }

    async fn list_schedules(
        &self,
        _limit: Option<usize>,
        _offset: Option<usize>,
    ) -> SchedulerResult<Vec<ScheduleInfo>> {
        Err(SchedulerError::ConfigError(
            "AWS feature not enabled".into(),
        ))
    }
}
