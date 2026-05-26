use std::{collections::HashMap, time::SystemTime};

use flow_like::flow::oauth::OAuthToken;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::event_sink::cron::CronSink;
use crate::event_sink::daemon::DaemonSink;
use crate::event_sink::deeplink::DeeplinkSink;
use crate::event_sink::discord::DiscordSink;
use crate::event_sink::email::EmailSink;
use crate::event_sink::file::FileSink;
use crate::event_sink::geolocation::GeoLocationSink;
use crate::event_sink::github::GitHubSink;
use crate::event_sink::http::HttpSink;
use crate::event_sink::mcp::MCPSink;
use crate::event_sink::mqtt::MQTTSink;
use crate::event_sink::nfc::NFCSink;
use crate::event_sink::notion::NotionSink;
use crate::event_sink::rss::RSSSink;
use crate::event_sink::shortcut::ShortcutSink;
use crate::event_sink::slack::SlackSink;
use crate::event_sink::telegram::TelegramSink;
use crate::event_sink::web_watcher::WebWatcherSink;
use crate::event_sink::webhook::WebhookSink;

pub mod cron;
pub mod daemon;
pub mod deeplink;
pub mod discord;
pub mod email;
pub mod file;
pub mod geolocation;
pub mod github;
pub mod http;
pub mod manager;
pub mod mcp;
pub mod mqtt;
pub mod nfc;
pub mod notion;
pub mod rss;
pub mod shortcut;
pub mod slack;
pub mod storage;
pub mod stubs; // Stub implementations for not-yet-complete sinks
pub mod telegram;
pub mod web_watcher;
pub mod webhook;

use manager::DbConnection;

// Re-export manager for convenience
pub use manager::EventSinkManager;

#[async_trait::async_trait]
pub trait EventSink: Send + Sync {
    /// Initialize sink infrastructure (e.g., start HTTP server, Discord client, cron worker)
    /// This is called once when the sink type is first used
    async fn start(&self, app_handle: &AppHandle, db: DbConnection) -> anyhow::Result<()>;

    /// Shutdown sink infrastructure and cleanup resources
    async fn stop(&self, app_handle: &AppHandle, db: DbConnection) -> anyhow::Result<()>;

    /// Called when a new event registration is added for this sink
    /// The sink should set up the specific trigger (e.g., add route to HTTP server, register Discord handler)
    async fn on_register(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> anyhow::Result<()>;

    /// Called when an event registration is removed
    /// The sink should clean up the specific trigger
    async fn on_unregister(
        &self,
        app_handle: &AppHandle,
        registration: &EventRegistration,
        db: DbConnection,
    ) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "sink_type")]
pub enum EventConfig {
    #[serde(rename = "discord")]
    Discord(DiscordSink),
    #[serde(rename = "email")]
    Email(EmailSink),

    #[serde(rename = "slack")]
    Slack(SlackSink),
    #[serde(rename = "telegram")]
    Telegram(TelegramSink),

    // Check for the state of a website and trigger the event when it changes
    #[serde(rename = "web_watcher")]
    WebWatcher(WebWatcherSink),
    #[serde(rename = "rss")]
    Rss(RSSSink),

    #[serde(rename = "deeplink")]
    Deeplink(DeeplinkSink),
    #[serde(rename = "http")]
    Http(HttpSink),
    #[serde(rename = "webhook")]
    Webhook(WebhookSink),
    #[serde(rename = "mqtt")]
    Mqtt(MQTTSink),
    #[serde(rename = "mcp")]
    Mcp(MCPSink),
    #[serde(rename = "file")]
    File(FileSink),
    #[serde(rename = "github")]
    GitHub(GitHubSink),

    #[serde(rename = "nfc")]
    Nfc(NFCSink),
    #[serde(rename = "geolocation")]
    GeoLocation(GeoLocationSink),
    #[serde(rename = "notion")]
    Notion(NotionSink),
    #[serde(rename = "shortcut")]
    Shortcut(ShortcutSink),
    #[serde(rename = "cron")]
    Cron(CronSink),
    #[serde(rename = "daemon")]
    Daemon(DaemonSink),
}

impl EventConfig {
    /// Returns the `sink_execution` target if the underlying sink supports it.
    /// Values: `"LOCAL"`, `"REMOTE"`, `"HYBRID"`, or `None` (default = local).
    pub fn sink_execution(&self) -> Option<&str> {
        match self {
            EventConfig::Http(s) => s.sink_execution.as_deref(),
            EventConfig::Cron(s) => s.sink_execution.as_deref(),
            _ => None,
        }
    }

    /// Returns `true` if this config is explicitly set to remote-only execution,
    /// meaning the local desktop should NOT register the sink.
    pub fn is_remote_only(&self) -> bool {
        self.sink_execution()
            .is_some_and(|v| v.eq_ignore_ascii_case("REMOTE"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventRegistration {
    pub event_id: String, // Primary key - each event can only be attached to one sink
    pub name: String,
    pub r#type: String,
    pub updated_at: SystemTime,
    pub created_at: SystemTime,
    pub config: EventConfig,
    pub offline: bool,
    pub app_id: String,
    pub default_payload: Option<flow_like_types::Value>,
    pub personal_access_token: Option<String>,
    /// OAuth tokens for third-party services, keyed by provider ID
    #[serde(default)]
    pub oauth_tokens: HashMap<String, OAuthToken>,
}
