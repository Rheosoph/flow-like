//! Sink service operations
//!
//! Handles the creation, update, and deletion of sinks in external schedulers
//! (AWS EventBridge, Kubernetes CronJobs) when event sinks are modified.

use crate::entity::event_sink;
use crate::state::AppState;
use flow_like_types::anyhow;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};

/// Sink type constants
pub mod sink_types {
    pub const HTTP: &str = "http";
    pub const CRON: &str = "cron";
    pub const DISCORD: &str = "discord";
    pub const TELEGRAM: &str = "telegram";
    pub const EMAIL: &str = "email";
    pub const CHAT: &str = "chat";
}

/// Configuration for creating/updating a sink
#[derive(Debug, Clone, Default)]
pub struct SinkConfig {
    pub event_id: String,
    pub app_id: String,
    pub sink_type: String,
    pub path: Option<String>,
    /// HTTP method for HTTP sinks (GET/POST/...). Only meaningful when
    /// `sink_type` is "http"; stored so different methods on the same path
    /// route to different events.
    pub method: Option<String>,
    pub auth_token: Option<String>,
    pub webhook_secret: Option<String>,
    pub cron_expression: Option<String>,
    pub cron_timezone: Option<String>,
    pub cron_scheduled_for: Option<flow_like_sinks::ScheduledLocal>,
    /// Encrypted PAT for execution (optional - enables model/file access)
    pub pat_encrypted: Option<String>,
    /// Encrypted OAuth tokens JSON (optional - for provider-specific access)
    pub oauth_tokens_encrypted: Option<String>,
    /// Snapshot of the last updater's profile (bits, hubs) for trigger execution
    pub profile_json: Option<serde_json::Value>,
    /// Whether the sink should be active (synced from event.active)
    pub active: Option<bool>,
}

/// Sync a sink to the database and external schedulers
///
/// This is called when an event is created/updated to ensure its sink exists
/// and is properly configured in external systems (EventBridge, K8s CronJobs).
pub async fn sync_sink(
    db: &DatabaseConnection,
    state: &AppState,
    config: SinkConfig,
) -> flow_like_types::Result<event_sink::Model> {
    let now = chrono::Utc::now().naive_utc();

    // Check if sink already exists
    let existing = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&config.event_id))
        .one(db)
        .await?;

    let sink = if let Some(existing) = existing {
        // Update existing sink
        let old_cron = existing.cron_expression.clone();
        let old_active = existing.active;
        let old_sink_type = existing.sink_type.clone();

        let mut active_model: event_sink::ActiveModel = existing.into();

        active_model.sink_type = Set(config.sink_type.clone());
        active_model.updated_at = Set(now);

        // Sync active state if provided
        let new_active = config.active.unwrap_or(old_active);
        if new_active != old_active {
            active_model.active = Set(new_active);
        }

        // Only update optional fields if provided
        if config.path.is_some() {
            active_model.path = Set(config.path.clone());
        }
        if config.method.is_some() {
            active_model.method = Set(config.method.clone());
        }
        if config.cron_expression.is_some() {
            active_model.cron_expression = Set(config.cron_expression.clone());
        }
        if config.cron_timezone.is_some() {
            active_model.cron_timezone = Set(config.cron_timezone.clone());
        }
        // Update PAT and OAuth tokens if provided
        if config.pat_encrypted.is_some() {
            active_model.pat_encrypted = Set(config.pat_encrypted.clone());
        }
        if config.oauth_tokens_encrypted.is_some() {
            active_model.oauth_tokens_encrypted = Set(config.oauth_tokens_encrypted.clone());
        }
        if config.profile_json.is_some() {
            active_model.profile_json = Set(config.profile_json.clone());
        }

        let updated = active_model.update(db).await?;

        // Handle scheduler updates for cron sinks
        if config.sink_type == sink_types::CRON {
            let cron_changed = old_cron != config.cron_expression;
            let type_changed = old_sink_type != config.sink_type;
            let has_schedule =
                config.cron_expression.is_some() || config.cron_scheduled_for.is_some();

            if (cron_changed || type_changed) && has_schedule {
                let cron_expr = config.cron_expression.as_deref().unwrap_or("");
                update_external_scheduler(
                    state,
                    &config.event_id,
                    cron_expr,
                    config.cron_timezone.as_deref(),
                    config.cron_scheduled_for.as_ref(),
                    old_active,
                )
                .await?;
            }

            // Sync active state with external scheduler
            if new_active != old_active {
                if new_active {
                    enable_external_schedule(state, &config.event_id).await?;
                } else {
                    disable_external_schedule(state, &config.event_id).await?;
                }
            }
        } else if old_sink_type == sink_types::CRON && config.sink_type != sink_types::CRON {
            // Type changed from cron to something else - delete the schedule
            delete_external_schedule(state, &config.event_id).await?;
        }

        updated
    } else {
        // Create new sink
        let sink_id = flow_like_types::create_id();
        let initial_active = config.active.unwrap_or(true);

        let active_model = event_sink::ActiveModel {
            id: Set(sink_id),
            event_id: Set(config.event_id.clone()),
            app_id: Set(config.app_id.clone()),
            sink_type: Set(config.sink_type.clone()),
            active: Set(initial_active),
            path: Set(config.path.clone()),
            method: Set(config.method.clone()),
            auth_token: Set(config.auth_token),
            webhook_secret: Set(config.webhook_secret),
            cron_expression: Set(config.cron_expression.clone()),
            cron_timezone: Set(config.cron_timezone.clone()),
            pat_encrypted: Set(config.pat_encrypted),
            oauth_tokens_encrypted: Set(config.oauth_tokens_encrypted),
            profile_json: Set(config.profile_json),
            created_at: Set(now),
            updated_at: Set(now),
        };

        let created = active_model.insert(db).await?;

        // Create schedule in external system for cron sinks
        if config.sink_type == sink_types::CRON
            && (config.cron_expression.is_some() || config.cron_scheduled_for.is_some())
        {
            let cron_expr = config.cron_expression.as_deref().unwrap_or("");
            create_external_schedule(
                state,
                &config.event_id,
                cron_expr,
                config.cron_timezone.as_deref(),
                config.cron_scheduled_for.as_ref(),
            )
            .await?;
            if !initial_active {
                disable_external_schedule(state, &config.event_id).await?;
            }
        }

        created
    };

    Ok(sink)
}

/// Delete a sink and its external scheduler
pub async fn delete_sink(
    db: &DatabaseConnection,
    state: &AppState,
    event_id: &str,
) -> flow_like_types::Result<()> {
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(db)
        .await?;

    if let Some(sink) = sink {
        // Delete from external scheduler if it's a cron sink (best-effort)
        if sink.sink_type == sink_types::CRON
            && let Err(e) = delete_external_schedule(state, event_id).await
        {
            tracing::warn!(
                event_id = %event_id,
                error = %e,
                "Failed to delete external schedule during sink cleanup, proceeding with DB deletion"
            );
        }

        // Delete from database
        event_sink::Entity::delete_by_id(&sink.id).exec(db).await?;
    }

    Ok(())
}

/// Toggle a sink's active state and update external scheduler
pub async fn toggle_sink_active(
    db: &DatabaseConnection,
    state: &AppState,
    event_id: &str,
) -> flow_like_types::Result<event_sink::Model> {
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(event_id))
        .one(db)
        .await?
        .ok_or_else(|| anyhow!("Sink not found for event: {}", event_id))?;

    let new_active = !sink.active;
    let sink_type = sink.sink_type.clone();

    let mut active_model: event_sink::ActiveModel = sink.into();
    active_model.active = Set(new_active);
    active_model.updated_at = Set(chrono::Utc::now().naive_utc());

    let updated = active_model.update(db).await?;

    // Update external scheduler for cron sinks
    if sink_type == sink_types::CRON {
        if new_active {
            enable_external_schedule(state, event_id).await?;
        } else {
            disable_external_schedule(state, event_id).await?;
        }
    }

    Ok(updated)
}

/// Create a schedule in the external scheduler (AWS EventBridge, K8s CronJob)
async fn create_external_schedule(
    state: &AppState,
    event_id: &str,
    cron_expression: &str,
    timezone: Option<&str>,
    scheduled_for: Option<&flow_like_sinks::ScheduledLocal>,
) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        let config = flow_like_sinks::CronSinkConfig {
            expression: cron_expression.to_string(),
            timezone: timezone.unwrap_or("UTC").to_string(),
            scheduled_for: scheduled_for.cloned(),
        };

        scheduler.create_schedule(event_id, cron_expression, &config).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to create external schedule");
            anyhow!("Failed to create schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, cron = %cron_expression, tz = %config.timezone, one_time = config.is_one_time(), "Created external schedule");
    }

    Ok(())
}

/// Update a schedule in the external scheduler
async fn update_external_scheduler(
    state: &AppState,
    event_id: &str,
    cron_expression: &str,
    timezone: Option<&str>,
    scheduled_for: Option<&flow_like_sinks::ScheduledLocal>,
    was_active: bool,
) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        let config = flow_like_sinks::CronSinkConfig {
            expression: cron_expression.to_string(),
            timezone: timezone.unwrap_or("UTC").to_string(),
            scheduled_for: scheduled_for.cloned(),
        };

        // If the schedule exists, update it; otherwise create it
        let exists = scheduler.schedule_exists(event_id).await.unwrap_or(false);

        if exists {
            scheduler.update_schedule(event_id, cron_expression, &config).await.map_err(|e| {
                tracing::error!(event_id = %event_id, error = %e, "Failed to update external schedule");
                anyhow!("Failed to update schedule: {}", e)
            })?;

            // Restore active state
            if !was_active {
                let _ = scheduler.disable_schedule(event_id).await;
            }
        } else {
            scheduler.create_schedule(event_id, cron_expression, &config).await.map_err(|e| {
                tracing::error!(event_id = %event_id, error = %e, "Failed to create external schedule");
                anyhow!("Failed to create schedule: {}", e)
            })?;

            // Set initial active state
            if !was_active {
                let _ = scheduler.disable_schedule(event_id).await;
            }
        }

        tracing::info!(event_id = %event_id, cron = %cron_expression, "Updated external schedule");
    }

    Ok(())
}

/// Delete a schedule from the external scheduler
async fn delete_external_schedule(state: &AppState, event_id: &str) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        scheduler.delete_schedule(event_id).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to delete external schedule");
            anyhow!("Failed to delete schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, "Deleted external schedule");
    }

    Ok(())
}

/// Enable a schedule in the external scheduler
async fn enable_external_schedule(state: &AppState, event_id: &str) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        scheduler.enable_schedule(event_id).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to enable external schedule");
            anyhow!("Failed to enable schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, "Enabled external schedule");
    }

    Ok(())
}

/// Disable a schedule in the external scheduler
async fn disable_external_schedule(
    state: &AppState,
    event_id: &str,
) -> flow_like_types::Result<()> {
    if let Some(ref scheduler) = state.sink_scheduler {
        scheduler.disable_schedule(event_id).await.map_err(|e| {
            tracing::error!(event_id = %event_id, error = %e, "Failed to disable external schedule");
            anyhow!("Failed to disable schedule: {}", e)
        })?;

        tracing::info!(event_id = %event_id, "Disabled external schedule");
    }

    Ok(())
}

/// Derive sink type from event type
pub fn sink_type_from_event_type(event_type: &str) -> &str {
    match event_type {
        "cron" => sink_types::CRON,
        "discord" => sink_types::DISCORD,
        "telegram" => sink_types::TELEGRAM,
        "email" => sink_types::EMAIL,
        "chat" => sink_types::CHAT,
        "api" | "http" | "webhook" => sink_types::HTTP,
        // Default to HTTP for unknown types
        _ => sink_types::HTTP,
    }
}
