//! Event database operations
//!
//! Provides functions to sync events between the bucket (source of truth for versions)
//! and the database (fast lookup mirror).
//!
//! For older events not yet in the database, fallback functions load from the bucket
//! and sync to the database for future fast lookups.

use std::collections::HashMap;

use crate::db::{
    DEFAULT_WRITE_CHUNK, DbDialect, RetryPolicy, delete_in_batches, retry_transaction,
};
use crate::entity::{event, event_remote_auth, event_remote_registration};
use flow_like::app::App;
use flow_like::flow::event::{
    CanaryEvent, Event as CoreEvent, EventExecutionMode, EventExposure, EventInput, EventVariant,
    ReleaseNotes,
};
use flow_like_types::anyhow;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, ConnectionTrait,
    DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
};
use serde_json::json;
use std::sync::Arc;

// Shared with the desktop and any future restore/variant path — the
// implementations moved to core so both crates blank and restore secrets
// identically. Re-exported so every existing `super::db::` caller keeps
// compiling.
pub use flow_like::flow::event::{filter_event_secrets, preserve_event_secrets};

/// Splice the LIVE event's HTTP auth token into a restored config for
/// api/http/webhook events. `sync_sink` mirrors the sink's `auth_token`
/// column from the config bytes — clearing the token clears the column — so
/// replaying an archived config would silently rotate every caller onto the
/// snapshot's stale key, or resurrect a revoked one. As with
/// `preserve_event_secrets`, the live value wins in both directions.
pub fn preserve_event_config_secrets(incoming: &mut CoreEvent, existing: &CoreEvent) {
    if !matches!(incoming.event_type.as_str(), "api" | "http" | "webhook") {
        return;
    }
    let live_token = extract_http_auth_token(&existing.config);
    if live_token == extract_http_auth_token(&incoming.config) {
        return;
    }
    let mut config: serde_json::Value = if incoming.config.is_empty() {
        json!({})
    } else {
        // A config that does not parse as JSON carries no auth_token to fix.
        match serde_json::from_slice(&incoming.config) {
            Ok(value) => value,
            Err(_) => return,
        }
    };
    let Some(object) = config.as_object_mut() else {
        return;
    };
    match live_token {
        Some(token) => {
            object.insert("auth_token".to_string(), json!(token));
        }
        None => {
            object.remove("auth_token");
        }
    }
    if let Ok(bytes) = serde_json::to_vec(&config) {
        incoming.config = bytes;
    }
}

pub fn filter_event_list_execution(mut event: CoreEvent) -> CoreEvent {
    event.canary = None;
    event.variants = Vec::new();
    event.variables = HashMap::new();
    event.notes = None;
    event
}

/// Remove the direct Board selector from a Page Event returned to a caller
/// that cannot use the direct Board path. The Event and Page IDs remain
/// sufficient for bootstrap and invoke-event requests.
pub fn redact_page_event_board_metadata(mut event: CoreEvent) -> CoreEvent {
    if event.default_page_id.is_some() {
        event.board_id.clear();
        event.board_version = None;
        event.node_id.clear();
    }
    event
}

const USER_FACING_EVENT_TYPES: &[&str] = &["simple_chat", "generic_form", "quick_action"];

/// Event types backing generated machinery rather than an author-managed event.
/// They are never part of the listed event set.
const HIDDEN_EVENT_TYPES: &[&str] = &["ontology_action"];

/// The route list is the same rows as the event list seen through a different
/// column, so both endpoints must hide the same events. When they disagree, a
/// caller diffing the two reads the surplus as orphaned and deletes live data.
pub fn is_listed_event_type(event_type: &str) -> bool {
    !HIDDEN_EVENT_TYPES.contains(&event_type)
}

pub fn is_user_facing_event_parts(default_page_id: Option<&str>, event_type: &str) -> bool {
    default_page_id.is_some() || USER_FACING_EVENT_TYPES.contains(&event_type)
}

pub fn is_user_facing_event(event: &CoreEvent) -> bool {
    is_user_facing_event_parts(event.default_page_id.as_deref(), &event.event_type)
}

/// Convert a core Event to database Event model
pub fn event_to_db_model(app_id: &str, event: &CoreEvent) -> event::ActiveModel {
    let board_version = event
        .board_version
        .map(|(major, minor, patch)| format!("{}.{}.{}", major, minor, patch));

    let event_version = format!(
        "{}.{}.{}",
        event.event_version.0, event.event_version.1, event.event_version.2
    );

    let variables = if event.variables.is_empty() {
        None
    } else {
        serde_json::to_value(&event.variables).ok()
    };

    let config = if event.config.is_empty() {
        None
    } else {
        // Store config as base64 in JSON
        Some(
            json!({ "base64": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &event.config) }),
        )
    };

    let inputs = if event.inputs.is_empty() {
        None
    } else {
        serde_json::to_value(&event.inputs).ok()
    };

    let notes = event
        .notes
        .as_ref()
        .and_then(|n| serde_json::to_value(n).ok());
    let canary = event
        .canary
        .as_ref()
        .and_then(|c| serde_json::to_value(c).ok());
    let variants = if event.variants.is_empty() {
        None
    } else {
        serde_json::to_value(&event.variants).ok()
    };

    event::ActiveModel {
        id: Set(event.id.clone()),
        app_id: Set(app_id.to_string()),
        name: Set(event.name.clone()),
        description: Set(if event.description.is_empty() {
            None
        } else {
            Some(event.description.clone())
        }),
        event_type: Set(event.event_type.clone()),
        active: Set(event.active),
        priority: Set(event.priority as i32),
        board_id: Set(Some(event.board_id.clone())),
        board_version: Set(board_version),
        node_id: Set(Some(event.node_id.clone())),
        page_id: Set(event.default_page_id.clone()),
        route: Set(event.route.clone()),
        is_default: Set(event.is_default),
        event_version: Set(event_version),
        execution_mode: Set(event.execution_mode.as_str().to_string()),
        exposure: Set(event.exposure.as_str().to_string()),
        variables: Set(variables),
        config: Set(config),
        inputs: Set(inputs),
        notes: Set(notes),
        canary: Set(canary),
        variants: Set(variants),
        correlation_mappings: Set(event
            .correlation_mappings
            .as_ref()
            .filter(|mappings| !mappings.is_empty())
            .and_then(|mappings| serde_json::to_value(mappings).ok())),
        created_at: Set(chrono::DateTime::from_timestamp(
            event
                .created_at
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64,
            0,
        )
        .unwrap_or_default()
        .naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
        // Setup tracking fields are only written by the remote-setup endpoint
        // (see routes::app::events::setup_event). Preserve existing values on
        // event upserts by leaving them NotSet here.
        setup_status: sea_orm::ActiveValue::NotSet,
        last_setup_at: sea_orm::ActiveValue::NotSet,
        last_setup_version: sea_orm::ActiveValue::NotSet,
        last_setup_error: sea_orm::ActiveValue::NotSet,
    }
}

/// Convert a database Event model to core Event
pub fn db_model_to_event(model: event::Model) -> flow_like_types::Result<CoreEvent> {
    let board_version = model.board_version.and_then(|v| {
        let parts: Vec<&str> = v.split('.').collect();
        if parts.len() == 3 {
            Some((
                parts[0].parse().ok()?,
                parts[1].parse().ok()?,
                parts[2].parse().ok()?,
            ))
        } else {
            None
        }
    });

    let event_version = {
        let parts: Vec<&str> = model.event_version.split('.').collect();
        if parts.len() == 3 {
            (
                parts[0].parse().unwrap_or(0),
                parts[1].parse().unwrap_or(0),
                parts[2].parse().unwrap_or(0),
            )
        } else {
            (0, 0, 0)
        }
    };

    let variables = model
        .variables
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let config = model
        .config
        .and_then(|c| {
            if let Some(b64) = c.get("base64").and_then(|v| v.as_str()) {
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64).ok()
            } else {
                None
            }
        })
        .unwrap_or_default();

    let inputs: Vec<EventInput> = model
        .inputs
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let notes: Option<ReleaseNotes> = model.notes.and_then(|v| serde_json::from_value(v).ok());

    let canary: Option<CanaryEvent> = model.canary.and_then(|v| serde_json::from_value(v).ok());

    let variants: Vec<EventVariant> = model
        .variants
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    let created_at = std::time::UNIX_EPOCH
        + std::time::Duration::from_secs(model.created_at.and_utc().timestamp() as u64);
    let updated_at = std::time::UNIX_EPOCH
        + std::time::Duration::from_secs(model.updated_at.and_utc().timestamp() as u64);

    Ok(CoreEvent {
        id: model.id,
        name: model.name,
        description: model.description.unwrap_or_default(),
        board_id: model.board_id.unwrap_or_default(),
        board_version,
        node_id: model.node_id.unwrap_or_default(),
        variables,
        config,
        active: model.active,
        canary,
        variants,
        priority: model.priority as u32,
        event_type: model.event_type,
        notes,
        event_version,
        created_at,
        updated_at,
        default_page_id: model.page_id,
        inputs,
        route: model.route,
        is_default: model.is_default,
        execution_mode: EventExecutionMode::parse(&model.execution_mode),
        exposure: EventExposure::parse(&model.exposure),
        correlation_mappings: model
            .correlation_mappings
            .and_then(|value| serde_json::from_value(value).ok()),
    })
}

/// Sync an event to the database (upsert)
pub async fn sync_event_to_db<C>(
    db: &C,
    app_id: &str,
    event: &CoreEvent,
) -> flow_like_types::Result<()>
where
    C: ConnectionTrait,
{
    let model = event_to_db_model(app_id, event);

    // Try to find existing
    let existing = event::Entity::find_by_id(&event.id).one(db).await?;

    if let Some(existing) = existing {
        if existing.app_id != app_id {
            tracing::error!(
                event_id = %event.id,
                requested_app_id = %app_id,
                existing_app_id = %existing.app_id,
                "Refusing to reassign an event database row across apps"
            );
            return Err(anyhow!("Event ID collision while synchronizing event"));
        }
        model.update(db).await?;
    } else {
        model.insert(db).await?;
    }

    Ok(())
}

/// Sync an event and its sink to the database
///
/// This is the main entry point for event creation/updates.
/// It syncs the event to the database and also creates/updates the associated sink,
/// including any external scheduler for cron events.
#[allow(dead_code)]
pub async fn sync_event_with_sink(
    db: &DatabaseConnection,
    state: &crate::state::AppState,
    app_id: &str,
    event: &CoreEvent,
) -> flow_like_types::Result<()> {
    sync_event_with_sink_tokens(db, state, app_id, event, None, None, None).await
}

pub async fn validate_event_schedule(
    state: &crate::state::AppState,
    event: &CoreEvent,
) -> flow_like_sinks::SchedulerResult<()> {
    if event.event_type != "cron" {
        return Ok(());
    }

    let Some(scheduler) = state.sink_scheduler.as_ref() else {
        return Ok(());
    };

    let cron_expression = extract_cron_expression(&event.config).unwrap_or_default();
    let config = flow_like_sinks::CronSinkConfig {
        expression: cron_expression.clone(),
        timezone: extract_cron_timezone(&event.config).unwrap_or_else(|| "UTC".to_string()),
        scheduled_for: extract_scheduled_for(&event.config),
        active: event.active,
    };

    scheduler.validate_schedule(&cron_expression, &config).await
}

/// Sync an event and its sink to the database, with optional PAT and OAuth tokens
///
/// This is the main entry point for event creation/updates when tokens are provided.
/// It syncs the event to the database and also creates/updates the associated sink,
/// including any external scheduler for cron events.
///
/// If `pat` or `oauth_tokens` are provided, they will be encrypted and stored with the sink.
/// This enables triggered flows to access models and personal files.
///
/// If `profile_json` is provided, it will be stored with the sink so triggered flows
/// can use the last updater's profile (bits, hubs) for model resolution.
pub async fn sync_event_with_sink_tokens(
    db: &DatabaseConnection,
    state: &crate::state::AppState,
    app_id: &str,
    event: &CoreEvent,
    pat: Option<&str>,
    oauth_tokens: Option<&std::collections::HashMap<String, serde_json::Value>>,
    profile_json: Option<serde_json::Value>,
) -> flow_like_types::Result<()> {
    use crate::routes::sink::service::{SinkConfig, sink_type_from_event_type, sync_sink};

    validate_event_schedule(state, event)
        .await
        .map_err(|error| anyhow!("Invalid cron schedule for event {}: {}", event.id, error))?;

    // First sync the event
    sync_event_to_db(db, app_id, event).await?;

    // Derive sink configuration from event
    let sink_type = sink_type_from_event_type(&event.event_type);

    // Extract cron expression from event config if it's a cron event
    let cron_expression = if event.event_type == "cron" {
        extract_cron_expression(&event.config)
    } else {
        None
    };

    let cron_timezone = if event.event_type == "cron" {
        extract_cron_timezone(&event.config)
    } else {
        None
    };

    let cron_scheduled_for = if event.event_type == "cron" {
        extract_scheduled_for(&event.config)
    } else {
        None
    };

    // Determine the sink path:
    // - For HTTP/API events, extract from event config (path field)
    // - For other events, use the UI route
    let sink_path = if matches!(event.event_type.as_str(), "api" | "http" | "webhook") {
        extract_http_path(&event.config).or_else(|| event.route.clone())
    } else {
        event.route.clone()
    };

    // Extract auth token from HTTP event config
    let config_auth_token = if matches!(event.event_type.as_str(), "api" | "http" | "webhook") {
        extract_http_auth_token(&event.config)
    } else {
        None
    };

    // Extract HTTP method from event config (default to POST — the most
    // common trigger method — if the event config doesn't specify one).
    let sink_method = if matches!(event.event_type.as_str(), "api" | "http" | "webhook") {
        Some(extract_http_method(&event.config).unwrap_or_else(|| "POST".to_string()))
    } else {
        None
    };

    // Encrypt PAT if provided
    let pat_encrypted = pat.map(|p| encrypt_token(p, &state.encryption_key));

    // Encrypt OAuth tokens if provided
    let oauth_tokens_encrypted = oauth_tokens.and_then(|tokens| {
        serde_json::to_string(tokens)
            .ok()
            .map(|json| encrypt_token(&json, &state.encryption_key))
    });

    // Sync the sink (creates if not exists, updates if exists)
    sync_sink(
        db,
        state,
        SinkConfig {
            event_id: event.id.clone(),
            app_id: app_id.to_string(),
            sink_type: sink_type.to_string(),
            path: sink_path,
            method: sink_method,
            auth_token: config_auth_token,
            webhook_secret: None, // Webhook secret is set separately
            cron_expression,
            cron_timezone,
            cron_scheduled_for,
            pat_encrypted,
            oauth_tokens_encrypted,
            profile_json,
            active: Some(event.active),
        },
    )
    .await?;

    Ok(())
}

/// Encrypt a token using AES-256-GCM
/// Returns base64-encoded ciphertext with prepended nonce
pub fn encrypt_token(token: &str, key: &[u8; 32]) -> String {
    crate::utils::crypto::encrypt_secret(token, key)
}

/// Decrypt a token using AES-256-GCM
/// Expects base64-encoded ciphertext with prepended nonce
pub fn decrypt_token(encrypted: &str, key: &[u8; 32]) -> Option<String> {
    crate::utils::crypto::decrypt_secret(encrypted, key)
}

/// Extract cron expression from event config bytes
pub(super) fn extract_cron_expression(config: &[u8]) -> Option<String> {
    if config.is_empty() {
        return None;
    }

    // Try to parse as JSON
    let value: serde_json::Value = serde_json::from_slice(config).ok()?;

    // Look for common cron expression field names
    value
        .get("cron_expression")
        .or_else(|| value.get("cronExpression"))
        .or_else(|| value.get("cron"))
        .or_else(|| value.get("schedule"))
        .or_else(|| value.get("expression"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract cron timezone from event config bytes
fn extract_cron_timezone(config: &[u8]) -> Option<String> {
    if config.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_slice(config).ok()?;

    value
        .get("timezone")
        .or_else(|| value.get("tz"))
        .or_else(|| value.get("cron_timezone"))
        .or_else(|| value.get("cronTimezone"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract scheduled_for from event config bytes (for one-time cron events)
fn extract_scheduled_for(config: &[u8]) -> Option<flow_like_sinks::ScheduledLocal> {
    if config.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_slice(config).ok()?;
    let sf = value
        .get("scheduled_for")
        .or_else(|| value.get("scheduledFor"))?;

    let date = sf.get("date").and_then(|v| v.as_str())?.to_string();
    let time = sf.get("time").and_then(|v| v.as_str())?.to_string();

    Some(flow_like_sinks::ScheduledLocal { date, time })
}

/// Extract HTTP path from event config bytes (for api/http/webhook events)
fn extract_http_path(config: &[u8]) -> Option<String> {
    if config.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_slice(config).ok()?;

    value
        .get("path")
        .or_else(|| value.get("path_suffix"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extract HTTP method from event config bytes (for api/http/webhook events)
fn extract_http_method(config: &[u8]) -> Option<String> {
    if config.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_slice(config).ok()?;

    value
        .get("method")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_ascii_uppercase())
        .filter(|s| !s.is_empty())
}

fn normalize_http_auth_token(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some((scheme, token)) = trimmed.split_once(' ')
        && scheme.eq_ignore_ascii_case("Bearer")
    {
        return token.trim();
    }
    trimmed
}

/// Extract HTTP auth token from event config bytes (for api/http/webhook events)
fn extract_http_auth_token(config: &[u8]) -> Option<String> {
    if config.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_slice(config).ok()?;

    value
        .get("auth_token")
        .and_then(|v| v.as_str())
        .map(normalize_http_auth_token)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

/// Delete an event from the database.
///
/// The inbound rows are drained in bounded batches first — registrations
/// before the auth rows they point at — so the event delete itself only
/// cascades to the handful of alias and setup rows.
pub async fn delete_event_from_db(
    db: &DatabaseConnection,
    dialect: DbDialect,
    event_id: &str,
) -> flow_like_types::Result<()> {
    delete_in_batches::<event_remote_registration::Entity>(
        db,
        dialect,
        Condition::all().add(event_remote_registration::Column::EventId.eq(event_id)),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await?;
    delete_in_batches::<event_remote_auth::Entity>(
        db,
        dialect,
        Condition::all().add(event_remote_auth::Column::EventId.eq(event_id)),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await?;
    event::Entity::delete_by_id(event_id).exec(db).await?;
    Ok(())
}

/// Delete an event and its sink from the database (and external scheduler)
pub async fn delete_event_with_sink(
    db: &DatabaseConnection,
    state: &crate::state::AppState,
    event_id: &str,
) -> flow_like_types::Result<()> {
    use crate::routes::sink::service::delete_sink;

    // Delete sink first (handles external scheduler cleanup)
    delete_sink(db, state, event_id).await?;

    // Then delete the event
    delete_event_from_db(db, state.db_dialect, event_id).await?;

    Ok(())
}

/// Get an event from the database by ID, validating it belongs to the given app.
/// This prevents cross-app event access.
pub async fn get_event_from_db(
    db: &DatabaseConnection,
    event_id: &str,
    app_id: &str,
) -> flow_like_types::Result<CoreEvent> {
    let model = event::Entity::find_by_id(event_id)
        .filter(event::Column::AppId.eq(app_id))
        .one(db)
        .await?
        .ok_or_else(|| anyhow!("Event not found: {}", event_id))?;

    db_model_to_event(model)
}

/// Get an event from the database by ID, validating it belongs to the given app.
/// Returns None if not found.
pub async fn get_event_from_db_opt(
    db: &DatabaseConnection,
    event_id: &str,
    app_id: &str,
) -> flow_like_types::Result<Option<CoreEvent>> {
    let model = event::Entity::find_by_id(event_id)
        .filter(event::Column::AppId.eq(app_id))
        .one(db)
        .await?;

    match model {
        Some(m) => Ok(Some(db_model_to_event(m)?)),
        None => Ok(None),
    }
}

/// Get all events for an app from the database
pub async fn get_events_for_app(
    db: &DatabaseConnection,
    app_id: &str,
) -> flow_like_types::Result<Vec<CoreEvent>> {
    let models = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .order_by_desc(event::Column::Priority)
        .order_by_asc(event::Column::Name)
        .all(db)
        .await?;

    models.into_iter().map(db_model_to_event).collect()
}

/// Get all active events for an app from the database
#[allow(dead_code)]
pub async fn get_active_events_for_app(
    db: &DatabaseConnection,
    app_id: &str,
) -> flow_like_types::Result<Vec<CoreEvent>> {
    let models = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::Active.eq(true))
        .order_by_desc(event::Column::Priority)
        .order_by_asc(event::Column::Name)
        .all(db)
        .await?;

    models.into_iter().map(db_model_to_event).collect()
}

/// Get events by type for an app
#[allow(dead_code)]
pub async fn get_events_by_type(
    db: &DatabaseConnection,
    app_id: &str,
    event_type: &str,
) -> flow_like_types::Result<Vec<CoreEvent>> {
    let models = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::EventType.eq(event_type))
        .filter(event::Column::Active.eq(true))
        .order_by_desc(event::Column::Priority)
        .all(db)
        .await?;

    models.into_iter().map(db_model_to_event).collect()
}

/// Get the event that links to a specific board
#[allow(dead_code)]
pub async fn get_event_by_board(
    db: &DatabaseConnection,
    app_id: &str,
    board_id: &str,
) -> flow_like_types::Result<Option<CoreEvent>> {
    let model = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::BoardId.eq(board_id))
        .one(db)
        .await?;

    match model {
        Some(m) => Ok(Some(db_model_to_event(m)?)),
        None => Ok(None),
    }
}

/// Get the event that links to a specific page (A2UI events)
#[allow(dead_code)]
pub async fn get_event_by_page(
    db: &DatabaseConnection,
    app_id: &str,
    page_id: &str,
) -> flow_like_types::Result<Option<CoreEvent>> {
    let model = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::PageId.eq(page_id))
        .one(db)
        .await?;

    match model {
        Some(m) => Ok(Some(db_model_to_event(m)?)),
        None => Ok(None),
    }
}

/// Get the event for a specific route path
#[allow(dead_code)]
pub async fn get_event_by_route(
    db: &DatabaseConnection,
    app_id: &str,
    route: &str,
) -> flow_like_types::Result<Option<CoreEvent>> {
    let model = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::Route.eq(route))
        .one(db)
        .await?;

    match model {
        Some(m) => Ok(Some(db_model_to_event(m)?)),
        None => Ok(None),
    }
}

/// Get the default event for an app (the one shown at "/" or when no route matches)
#[allow(dead_code)]
pub async fn get_default_event(
    db: &DatabaseConnection,
    app_id: &str,
) -> flow_like_types::Result<Option<CoreEvent>> {
    let model = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::IsDefault.eq(true))
        .one(db)
        .await?;

    match model {
        Some(m) => Ok(Some(db_model_to_event(m)?)),
        None => Ok(None),
    }
}

// ============================================================================
// Fallback functions - load from bucket if not in DB, then sync to DB
// ============================================================================

/// Get an event by ID with fallback to bucket if not in DB
/// If found in bucket, syncs to DB for future fast lookups
#[allow(dead_code)]
pub async fn get_event_with_fallback(
    db: &DatabaseConnection,
    app: &App,
    event_id: &str,
) -> flow_like_types::Result<CoreEvent> {
    // Try DB first
    if let Some(event) = get_event_from_db_opt(db, event_id, &app.id).await? {
        return Ok(event);
    }

    // Fallback to bucket
    let event = app.get_event(event_id, None).await?;

    // Sync to DB for future lookups
    if let Err(e) = sync_event_to_db(db, &app.id, &event).await {
        tracing::warn!("Failed to sync event {} to DB: {}", event_id, e);
    }

    Ok(event)
}

/// Get an event by ID with fallback, returning None if not found anywhere
pub async fn get_event_with_fallback_opt(
    db: &DatabaseConnection,
    app: &App,
    event_id: &str,
) -> flow_like_types::Result<Option<CoreEvent>> {
    // Try DB first
    if let Some(event) = get_event_from_db_opt(db, event_id, &app.id).await? {
        return Ok(Some(event));
    }

    // Fallback to bucket
    match app.get_event(event_id, None).await {
        Ok(event) => {
            // Sync to DB for future lookups
            if let Err(e) = sync_event_to_db(db, &app.id, &event).await {
                tracing::warn!("Failed to sync event {} to DB: {}", event_id, e);
            }
            Ok(Some(event))
        }
        Err(_) => Ok(None),
    }
}

/// Get all events for an app with fallback to bucket
/// If bucket has events not in DB, syncs them
pub async fn get_events_with_fallback(
    db: &DatabaseConnection,
    dialect: DbDialect,
    app: &App,
) -> flow_like_types::Result<Vec<CoreEvent>> {
    // Try DB first
    let db_events = get_events_for_app(db, &app.id).await?;

    if !db_events.is_empty() {
        return Ok(db_events);
    }

    // Load and validate the complete artifact set before changing the mirror.
    // This prevents one unreadable artifact from leaving a partial DB snapshot
    // that subsequent reads would incorrectly treat as authoritative.
    let mut bucket_events = Vec::with_capacity(app.events.len());
    for event_id in &app.events {
        let event = app.get_event(event_id, None).await?;
        if event.id != *event_id {
            tracing::error!(
                expected_event_id = %event_id,
                artifact_event_id = %event.id,
                app_id = %app.id,
                "Event artifact ID does not match its manifest entry"
            );
            return Err(anyhow!("Event artifact ID mismatch"));
        }
        bucket_events.push(event);
    }

    // Commit the mirror backfill atomically. The complete bucket result is
    // still safe to serve when a transient DB write fails; rollback keeps the
    // next request eligible to retry the repair. Every row is an upsert, so
    // the body may be re-run after a lost commit race.
    let bucket_events = Arc::new(bucket_events);
    let app_id = app.id.clone();
    let backfill =
        retry_transaction::<_, (), DbErr>(db, dialect, None, &RetryPolicy::idempotent(), |txn| {
            let bucket_events = bucket_events.clone();
            let app_id = app_id.clone();
            Box::pin(async move {
                for event in bucket_events.iter() {
                    sync_event_to_db(txn, &app_id, event)
                        .await
                        .map_err(|error| match error.downcast::<DbErr>() {
                            Ok(db_error) => db_error,
                            Err(other) => DbErr::Custom(other.to_string()),
                        })?;
                }
                Ok(())
            })
        })
        .await;
    if let Err(error) = backfill {
        tracing::warn!(
            app_id = %app.id,
            %error,
            "Failed to backfill event database mirror; transaction rolled back"
        );
    }

    Ok(Arc::try_unwrap(bucket_events).unwrap_or_else(|events| events.as_ref().clone()))
}

/// Get event by route with fallback - searches bucket events if not in DB
#[allow(dead_code)]
pub async fn get_event_by_route_with_fallback(
    db: &DatabaseConnection,
    app: &App,
    route: &str,
) -> flow_like_types::Result<Option<CoreEvent>> {
    // Try DB first
    if let Some(event) = get_event_by_route(db, &app.id, route).await? {
        return Ok(Some(event));
    }

    // Fallback: load all events from bucket using IDs and find by route
    for event_id in &app.events {
        match app.get_event(event_id, None).await {
            Ok(event) => {
                // Sync to DB
                if let Err(e) = sync_event_to_db(db, &app.id, &event).await {
                    tracing::warn!("Failed to sync event {} to DB: {}", event.id, e);
                }

                if event.route.as_deref() == Some(route) {
                    return Ok(Some(event));
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load event {} from bucket: {}", event_id, e);
            }
        }
    }

    Ok(None)
}

/// Get default event with fallback - searches bucket events if not in DB
#[allow(dead_code)]
pub async fn get_default_event_with_fallback(
    db: &DatabaseConnection,
    app: &App,
) -> flow_like_types::Result<Option<CoreEvent>> {
    // Try DB first
    if let Some(event) = get_default_event(db, &app.id).await? {
        return Ok(Some(event));
    }

    // Fallback: load all events from bucket using IDs and find default
    for event_id in &app.events {
        match app.get_event(event_id, None).await {
            Ok(event) => {
                // Sync to DB
                if let Err(e) = sync_event_to_db(db, &app.id, &event).await {
                    tracing::warn!("Failed to sync event {} to DB: {}", event.id, e);
                }

                if event.is_default {
                    return Ok(Some(event));
                }
            }
            Err(e) => {
                tracing::warn!("Failed to load event {} from bucket: {}", event_id, e);
            }
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::pin::ValueType;
    use flow_like::flow::variable::{Variable, VariableType};

    fn secret(name: &str, value: Option<&str>) -> Variable {
        let mut variable = Variable::new(name, VariableType::String, ValueType::Normal);
        variable.secret = true;
        variable.default_value = value.map(|v| v.as_bytes().to_vec());
        variable
    }

    fn event_with(variables: HashMap<String, Variable>) -> CoreEvent {
        CoreEvent {
            id: "evt-1".to_string(),
            name: "Nightly sync".to_string(),
            description: String::new(),
            board_id: "board-1".to_string(),
            board_version: None,
            node_id: "node-1".to_string(),
            variables,
            config: Vec::new(),
            active: true,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: "cron".to_string(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
            default_page_id: None,
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: EventExecutionMode::default(),
            exposure: EventExposure::default(),
            correlation_mappings: None,
        }
    }

    fn value_of(event: &CoreEvent, id: &str) -> Option<String> {
        event.variables[id]
            .default_value
            .as_ref()
            .map(|bytes| String::from_utf8(bytes.clone()).unwrap())
    }

    /// The round trip that used to erase secrets: read blanks the value, so the
    /// client sends it back empty and the save wrote that emptiness through.
    #[test]
    fn blank_secret_from_a_round_trip_keeps_the_stored_value() {
        let existing = event_with(HashMap::from([(
            "v1".to_string(),
            secret("TOKEN", Some("stored")),
        )]));
        let mut incoming = filter_event_secrets(existing.clone());
        assert_eq!(value_of(&incoming, "v1"), None);

        preserve_event_secrets(&mut incoming, &existing);

        assert_eq!(value_of(&incoming, "v1"), Some("stored".to_string()));
    }

    #[test]
    fn a_secret_the_client_sends_is_a_deliberate_change() {
        let existing = event_with(HashMap::from([(
            "v1".to_string(),
            secret("TOKEN", Some("stored")),
        )]));
        let mut incoming = event_with(HashMap::from([(
            "v1".to_string(),
            secret("TOKEN", Some("rotated")),
        )]));

        preserve_event_secrets(&mut incoming, &existing);

        assert_eq!(value_of(&incoming, "v1"), Some("rotated".to_string()));
    }

    #[test]
    fn non_secret_variables_are_left_alone() {
        let mut plain = Variable::new("LIMIT", VariableType::String, ValueType::Normal);
        plain.default_value = Some(b"10".to_vec());
        let existing = event_with(HashMap::from([("v1".to_string(), plain.clone())]));

        let mut cleared = plain.clone();
        cleared.default_value = None;
        let mut incoming = event_with(HashMap::from([("v1".to_string(), cleared)]));

        preserve_event_secrets(&mut incoming, &existing);

        assert_eq!(value_of(&incoming, "v1"), None);
    }

    #[test]
    fn a_newly_added_secret_has_nothing_to_restore() {
        let existing = event_with(HashMap::new());
        let mut incoming = event_with(HashMap::from([("v1".to_string(), secret("TOKEN", None))]));

        preserve_event_secrets(&mut incoming, &existing);

        assert_eq!(value_of(&incoming, "v1"), None);
    }

    #[test]
    fn canary_secrets_are_preserved_too() {
        let canary = |value: Option<&str>| CanaryEvent {
            weight: 0.5,
            variables: HashMap::from([("v1".to_string(), secret("TOKEN", value))]),
            board_id: "board-1".to_string(),
            board_version: None,
            node_id: "node-1".to_string(),
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        };

        let mut existing = event_with(HashMap::new());
        existing.canary = Some(canary(Some("stored")));
        let mut incoming = event_with(HashMap::new());
        incoming.canary = Some(canary(None));

        preserve_event_secrets(&mut incoming, &existing);

        let restored = incoming.canary.unwrap().variables["v1"]
            .default_value
            .clone();
        assert_eq!(restored, Some(b"stored".to_vec()));
    }

    #[test]
    fn variants_round_trip_through_the_db_model_and_an_empty_set_stores_null() {
        use flow_like::flow::event::{EventVariant, EventVariantMode};

        let mut event = event_with(HashMap::new());
        let model = event_to_db_model("app-1", &event);
        assert!(matches!(model.variants, Set(None)));

        event.variants = vec![EventVariant {
            name: "canary".to_string(),
            board_id: "board-2".to_string(),
            board_version: Some((1, 2, 3)),
            node_id: "node-2".to_string(),
            variables: HashMap::new(),
            default_page_id: None,
            mode: EventVariantMode::Live { weight: 0.25 },
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        }];
        let model = event_to_db_model("app-1", &event);
        let Set(Some(variants_json)) = model.variants else {
            panic!("a non-empty variant set must be stored as JSON");
        };

        let row = event::Model {
            id: "evt-1".to_string(),
            app_id: "app-1".to_string(),
            name: "Nightly sync".to_string(),
            description: None,
            event_type: "cron".to_string(),
            active: true,
            priority: 0,
            board_id: Some("board-1".to_string()),
            board_version: None,
            node_id: Some("node-1".to_string()),
            page_id: None,
            route: None,
            is_default: false,
            event_version: "1.0.0".to_string(),
            variables: None,
            config: None,
            inputs: None,
            notes: None,
            canary: None,
            variants: Some(variants_json),
            created_at: Default::default(),
            updated_at: Default::default(),
            execution_mode: "Local".to_string(),
            last_setup_at: None,
            last_setup_error: None,
            last_setup_version: None,
            setup_status: None,
            correlation_mappings: None,
            exposure: "PUBLIC".to_string(),
        };
        let restored = db_model_to_event(row).unwrap();

        assert_eq!(restored.variants.len(), 1);
        let variant = &restored.variants[0];
        assert_eq!(variant.name, "canary");
        assert_eq!(variant.board_id, "board-2");
        assert_eq!(variant.board_version, Some((1, 2, 3)));
        assert_eq!(variant.node_id, "node-2");
        assert_eq!(variant.mode, EventVariantMode::Live { weight: 0.25 });
    }

    #[test]
    fn execution_list_filter_clears_variants_alongside_the_canary() {
        use flow_like::flow::event::{EventVariant, EventVariantMode};

        let mut event = event_with(HashMap::new());
        event.variants = vec![EventVariant {
            name: "canary".to_string(),
            board_id: "board-2".to_string(),
            board_version: None,
            node_id: "node-2".to_string(),
            variables: HashMap::new(),
            default_page_id: None,
            mode: EventVariantMode::Live { weight: 0.25 },
            created_at: std::time::SystemTime::UNIX_EPOCH,
            updated_at: std::time::SystemTime::UNIX_EPOCH,
        }];

        let filtered = filter_event_list_execution(event);

        assert!(filtered.canary.is_none());
        assert!(filtered.variants.is_empty());
    }

    fn http_event(config: &str) -> CoreEvent {
        let mut event = event_with(HashMap::new());
        event.event_type = "http".to_string();
        event.config = config.as_bytes().to_vec();
        event
    }

    #[test]
    fn restored_http_config_keeps_the_live_auth_token() {
        let live = http_event(r#"{"path":"/hook","auth_token":"rotated"}"#);
        let mut restored = http_event(r#"{"path":"/hook","auth_token":"stale"}"#);

        preserve_event_config_secrets(&mut restored, &live);

        assert_eq!(
            extract_http_auth_token(&restored.config),
            Some("rotated".to_string())
        );
        let value: serde_json::Value = serde_json::from_slice(&restored.config).unwrap();
        assert_eq!(value["path"], "/hook");
    }

    #[test]
    fn a_token_cleared_live_stays_cleared_on_restore() {
        let live = http_event(r#"{"path":"/hook"}"#);
        let mut restored = http_event(r#"{"path":"/hook","auth_token":"revoked"}"#);

        preserve_event_config_secrets(&mut restored, &live);

        assert_eq!(extract_http_auth_token(&restored.config), None);
    }

    #[test]
    fn non_http_event_configs_are_left_alone() {
        let mut live = event_with(HashMap::new());
        live.config = br#"{"auth_token":"live"}"#.to_vec();
        let mut restored = event_with(HashMap::new());
        restored.config = br#"{"cron":"* * * * *"}"#.to_vec();
        let before = restored.config.clone();

        preserve_event_config_secrets(&mut restored, &live);

        assert_eq!(restored.config, before);
    }

    #[test]
    fn runtime_page_event_redaction_hides_only_the_direct_board_selector() {
        let ordinary = event_with(HashMap::new());
        assert_eq!(
            redact_page_event_board_metadata(ordinary.clone()).board_id,
            ordinary.board_id
        );

        let mut page = ordinary;
        page.default_page_id = Some("page-1".to_string());
        page.board_version = Some((1, 2, 3));
        let redacted = redact_page_event_board_metadata(page);

        assert_eq!(redacted.default_page_id.as_deref(), Some("page-1"));
        assert!(redacted.board_id.is_empty());
        assert!(redacted.board_version.is_none());
        assert!(redacted.node_id.is_empty());
    }
}
