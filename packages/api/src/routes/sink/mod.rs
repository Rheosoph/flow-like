//! Sink routes module
//!
//! Provides endpoints for:
//! - Sink activation/deactivation (user can only toggle active state)
//! - HTTP sink triggers (/sink/trigger/http/{app_id}/{path})
//! - Telegram webhook triggers (/sink/trigger/telegram/{event_id})
//! - Discord interactions webhook triggers (/sink/trigger/discord/{event_id})
//! - Service-to-service triggers (/sink/trigger/async) - for cron, discord bot, telegram bot
//! - Listing all active sinks for user's apps
//!
//! Note: Sink config comes from the Event itself. We only store sink-specific
//! data like path and auth token. Sinks are auto-created when events are created.

use crate::state::AppState;
use axum::{
    Router,
    routing::{delete, get, head, options, patch, post, put},
};

pub mod management;
pub mod service;
pub mod trigger;

pub fn routes() -> Router<AppState> {
    Router::new()
        // List all active sinks for apps user has access to
        .route("/", get(management::list_sinks))
        // List sinks for a specific app
        .route("/app/{app_id}", get(management::list_app_sinks))
        // Get/update/toggle a specific sink
        .route("/{event_id}", get(management::get_sink))
        .route("/{event_id}", patch(management::update_sink))
        .route("/{event_id}/toggle", post(management::toggle_sink))
        // Service-to-service trigger (for internal sink services: cron, discord bot, telegram bot)
        .route("/trigger/async", post(trigger::trigger_service))
        // List cron schedules (for docker-compose sink service to sync)
        .route("/schedules", get(trigger::get_cron_sinks))
        // List sink configs by type (for discord/telegram bots to sync)
        .route("/configs", get(trigger::get_sink_configs))
        // HTTP sink trigger - matches any method and path after app_id
        .route("/trigger/http/{app_id}/{*path}", get(trigger::trigger_http))
        .route(
            "/trigger/http/{app_id}/{*path}",
            post(trigger::trigger_http),
        )
        .route("/trigger/http/{app_id}/{*path}", put(trigger::trigger_http))
        .route(
            "/trigger/http/{app_id}/{*path}",
            patch(trigger::trigger_http),
        )
        .route(
            "/trigger/http/{app_id}/{*path}",
            delete(trigger::trigger_http),
        )
        .route(
            "/trigger/http/{app_id}/{*path}",
            head(trigger::trigger_http),
        )
        .route(
            "/trigger/http/{app_id}/{*path}",
            options(trigger::trigger_http),
        )
        // Telegram webhook trigger - async execution with secret token verification
        .route(
            "/trigger/telegram/{event_id}",
            post(trigger::trigger_telegram),
        )
        // Discord interactions webhook trigger - async execution with Ed25519 signature verification
        .route(
            "/trigger/discord/{event_id}",
            post(trigger::trigger_discord),
        )
}
