//! AWS EventBridge Scheduler implementation
//!
//! Uses AWS EventBridge Scheduler to create and manage cron schedules.
//! Each schedule triggers a Lambda function that calls the central API.

use super::{ScheduleInfo, SchedulerBackend, SchedulerError, SchedulerResult};
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

    /// Convert standard cron to AWS cron format
    ///
    /// AWS cron: `cron(minutes hours day-of-month month day-of-week year)`
    /// Standard: `minutes hours day-of-month month day-of-week`
    fn to_aws_cron(&self, cron_expr: &str) -> String {
        let parts: Vec<&str> = cron_expr.split_whitespace().collect();
        if parts.len() == 5 {
            // Standard 5-part cron, add year wildcard
            format!("cron({} *)", cron_expr)
        } else if parts.len() == 6 {
            // Already has year
            format!("cron({})", cron_expr)
        } else {
            // Invalid, return as-is and let AWS validate
            format!("cron({})", cron_expr)
        }
    }

    fn schedule_name(&self, event_id: &str) -> String {
        format!("flow-like-cron-{}", event_id.replace(['/', ':'], "-"))
    }

    fn schedule_description(event_id: &str) -> String {
        format!("Flow-Like cron schedule for event {}", event_id)
    }

    fn build_target(&self, event_id: &str) -> SchedulerResult<aws_sdk_scheduler::types::Target> {
        use aws_sdk_scheduler::types::{DeadLetterConfig, Target};

        let mut builder = Target::builder()
            .arn(&self.config.target_arn)
            .role_arn(&self.config.role_arn)
            .input(serde_json::json!({ "event_id": event_id }).to_string());

        if let Some(dlq_arn) = &self.config.dlq_arn {
            builder = builder.dead_letter_config(
                DeadLetterConfig::builder().arn(dlq_arn).build(),
            );
        }

        builder
            .build()
            .map_err(|e| SchedulerError::ProviderError(format!("Failed to build target: {}", e)))
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

    fn to_aws_cron(&self, cron_expr: &str) -> String {
        let parts: Vec<&str> = cron_expr.split_whitespace().collect();
        if parts.len() == 5 {
            format!("cron({} *)", cron_expr)
        } else if parts.len() == 6 {
            format!("cron({})", cron_expr)
        } else {
            format!("cron({})", cron_expr)
        }
    }

    fn schedule_name(&self, event_id: &str) -> String {
        format!("flow-like-cron-{}", event_id.replace(['/', ':'], "-"))
    }
}

#[cfg(feature = "aws")]
#[async_trait::async_trait]
impl SchedulerBackend for AwsEventBridgeScheduler {
    async fn create_schedule(
        &self,
        event_id: &str,
        cron_expr: &str,
        config: &CronSinkConfig,
    ) -> SchedulerResult<()> {
        use aws_sdk_scheduler::types::{
            ActionAfterCompletion, FlexibleTimeWindow, FlexibleTimeWindowMode, ScheduleState,
        };

        let schedule_name = self.schedule_name(event_id);
        let target = self.build_target(event_id)?;

        let flexible_time_window = FlexibleTimeWindow::builder()
            .mode(FlexibleTimeWindowMode::Off)
            .build()
            .map_err(|e| {
                SchedulerError::ProviderError(format!("Failed to build time window: {}", e))
            })?;

        let (schedule_expression, action_after) = if config.is_one_time() {
            let at_expr = config.effective_expression().ok_or_else(|| {
                SchedulerError::InvalidCronExpression("Missing scheduled_for for one-time schedule".into())
            })?;
            (at_expr, Some(ActionAfterCompletion::Delete))
        } else {
            (self.to_aws_cron(cron_expr), None)
        };

        let mut req = self.client
            .create_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .description(Self::schedule_description(event_id))
            .schedule_expression(&schedule_expression)
            .schedule_expression_timezone(&config.timezone)
            .state(ScheduleState::Enabled)
            .flexible_time_window(flexible_time_window)
            .target(target)
            .client_token(&schedule_name);

        if let Some(action) = action_after {
            req = req.action_after_completion(action);
        }

        req.send()
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("AWS SDK error: {}", e)))?;

        tracing::info!(
            event_id = %event_id,
            schedule_name = %schedule_name,
            expression = %schedule_expression,
            tz = %config.timezone,
            one_time = config.is_one_time(),
            "Created EventBridge schedule"
        );

        Ok(())
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

        let (schedule_expression, action_after) = if config.is_one_time() {
            let at_expr = config.effective_expression().ok_or_else(|| {
                SchedulerError::InvalidCronExpression("Missing scheduled_for for one-time schedule".into())
            })?;
            (at_expr, Some(ActionAfterCompletion::Delete))
        } else {
            (self.to_aws_cron(cron_expr), None)
        };

        let mut req = self.client
            .update_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .description(Self::schedule_description(event_id))
            .schedule_expression(&schedule_expression)
            .schedule_expression_timezone(&config.timezone)
            .flexible_time_window(flexible_time_window)
            .target(target);

        if let Some(action) = action_after {
            req = req.action_after_completion(action);
        }

        req.send()
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("AWS SDK error: {}", e)))?;

        tracing::info!(
            event_id = %event_id,
            schedule_name = %schedule_name,
            expression = %schedule_expression,
            tz = %config.timezone,
            one_time = config.is_one_time(),
            "Updated EventBridge schedule"
        );

        Ok(())
    }

    async fn delete_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        let schedule_name = self.schedule_name(event_id);

        match self
            .client
            .delete_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .send()
            .await
        {
            Ok(_) => {
                tracing::info!(event_id = %event_id, "Deleted EventBridge schedule");
                Ok(())
            }
            Err(e) => {
                // Ignore "not found" errors during delete
                let err_str = e.to_string();
                if err_str.contains("ResourceNotFoundException") {
                    tracing::debug!(event_id = %event_id, "Schedule already deleted");
                    Ok(())
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        e
                    )))
                }
            }
        }
    }

    async fn enable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        use aws_sdk_scheduler::types::ScheduleState;

        let schedule_name = self.schedule_name(event_id);

        // Get current schedule to preserve ALL settings (AWS resets omitted fields)
        let current = self
            .client
            .get_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .send()
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("AWS SDK error: {}", e)))?;

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
            .state(ScheduleState::Enabled)
            .flexible_time_window(flexible_time_window)
            .target(target);

        if let Some(desc) = current.description {
            update = update.description(desc);
        }

        update
            .send()
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("AWS SDK error: {}", e)))?;

        tracing::info!(event_id = %event_id, "Enabled EventBridge schedule");

        Ok(())
    }

    async fn disable_schedule(&self, event_id: &str) -> SchedulerResult<()> {
        use aws_sdk_scheduler::types::ScheduleState;

        let schedule_name = self.schedule_name(event_id);

        // Get current schedule to preserve ALL settings (AWS resets omitted fields)
        let current = self
            .client
            .get_schedule()
            .name(&schedule_name)
            .group_name(&self.config.group_name)
            .send()
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("AWS SDK error: {}", e)))?;

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
            .state(ScheduleState::Disabled)
            .flexible_time_window(flexible_time_window)
            .target(target);

        if let Some(desc) = current.description {
            update = update.description(desc);
        }

        update
            .send()
            .await
            .map_err(|e| SchedulerError::ProviderError(format!("AWS SDK error: {}", e)))?;

        tracing::info!(event_id = %event_id, "Disabled EventBridge schedule");

        Ok(())
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
                if err_str.contains("ResourceNotFoundException") {
                    Ok(false)
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        e
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
                let cron_expr = response
                    .schedule_expression
                    .unwrap_or_default()
                    .replace("cron(", "")
                    .replace(')', "");

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
                if err_str.contains("ResourceNotFoundException") {
                    Ok(None)
                } else {
                    Err(SchedulerError::ProviderError(format!(
                        "AWS SDK error: {}",
                        e
                    )))
                }
            }
        }
    }

    async fn list_schedules(
        &self,
        limit: Option<usize>,
        _offset: Option<usize>,
    ) -> SchedulerResult<Vec<ScheduleInfo>> {
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

            if let Some(l) = limit {
                request = request.max_results(l as i32);
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
                    cron_expression: String::new(), // Would need get_schedule for full details
                    active: schedule.state
                        == Some(aws_sdk_scheduler::types::ScheduleState::Enabled),
                    last_triggered: None,
                    next_trigger: None,
                });
            }

            next_token = response.next_token;
            if next_token.is_none() || (limit.is_some() && schedules.len() >= limit.unwrap()) {
                break;
            }
        }

        Ok(schedules)
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
