use std::{collections::HashMap, time::SystemTime};

use flow_like::flow::event::{Event, EventExecutionMode};
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
pub mod failure_log;
pub mod file;
pub mod geolocation;
pub mod github;
pub mod http;
pub mod manager;
pub mod mcp;
pub mod mqtt;
pub mod nfc;
pub mod notion;
pub(crate) mod oauth;
pub mod rss;
pub mod shortcut;
pub mod slack;
pub mod storage;
pub mod stubs; // Stub implementations for not-yet-complete sinks
pub mod telegram;
pub mod trigger_token;
pub mod web_watcher;
pub mod webhook;

use manager::DbConnection;
use trigger_token::{TriggerAccount, TriggerProfile};

// Re-export manager for convenience
pub use manager::EventSinkManager;

/// Whether `pat` holds a token: present and not blank.
pub(crate) fn has_token(pat: Option<&str>) -> bool {
    pat.is_some_and(|value| !value.trim().is_empty())
}

/// Whether `pat` was verified to belong to `account` on the hub it signs in to.
fn token_owned_by(
    pat: Option<&str>,
    owner_sub: Option<&str>,
    hub_origin: Option<&str>,
    profile_binding: Option<&TriggerProfile>,
    account: Option<&TriggerAccount>,
) -> bool {
    has_token(pat)
        && account.is_some_and(|account| {
            owner_sub == Some(account.sub.as_str())
                && hub_origin == Some(account.hub_origin.as_str())
                && profile_binding.and_then(TriggerProfile::account).as_ref() == Some(account)
        })
}

/// Refuses the run a trigger of an online app starts with `token`, verified
/// for `owner`, unless `owner` is `account`: the account signed in now on the
/// hub the run talks to. A trigger never sends its PAT to another hub, nor
/// runs as an account that is no longer signed in.
pub(crate) fn ensure_signed_in_owner(
    event_id: &str,
    token: Option<&str>,
    owner: Option<&TriggerAccount>,
    account: Option<&TriggerAccount>,
) -> anyhow::Result<()> {
    let refusal = match (owner.filter(|_| has_token(token)), account) {
        (None, _) => String::from(
            "it holds no personal access token verified with the hub; add one in the event's settings",
        ),
        (Some(_), None) => String::from(
            "nobody is signed in to the hub of the current profile; sign in as the account its personal access token was verified for",
        ),
        (Some(owner), Some(account)) if owner.hub_origin != account.hub_origin => format!(
            "its personal access token was verified with {}, but the current profile uses {}; switch back or add a token for this hub in the event's settings",
            owner.hub_origin, account.hub_origin
        ),
        (Some(owner), Some(account)) if owner != account => String::from(
            "its personal access token belongs to another profile or account; enable the trigger for the intended profile again",
        ),
        _ => return Ok(()),
    };
    anyhow::bail!("The trigger of event {event_id} did not start a run: {refusal}")
}

#[async_trait::async_trait]
pub trait EventSink: Send + Sync {
    /// Initialize sink infrastructure (e.g., start HTTP server, Discord client, cron worker)
    /// This is called once when the sink type is first used
    async fn start(&self, app_handle: &AppHandle, db: DbConnection) -> anyhow::Result<()>;

    /// Shutdown sink infrastructure and cleanup resources
    #[allow(dead_code)] // lifecycle contract: implemented by every sink, no shutdown path invokes it yet
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
    /// Hub subject the PAT was verified to belong to. Rows saved before tokens
    /// were verified have none.
    #[serde(default)]
    pub pat_owner_sub: Option<String>,
    /// Origin of the hub the PAT was verified against.
    #[serde(default)]
    pub hub_origin: Option<String>,
    /// Profile and account that explicitly enabled this trigger. Legacy rows require re-enabling.
    #[serde(default)]
    pub profile_binding: Option<TriggerProfile>,
    /// The event runs on the hub; this device only forwards its trigger.
    #[serde(default)]
    pub remote: bool,
    /// OAuth tokens for third-party services, keyed by provider ID
    #[serde(default)]
    pub oauth_tokens: HashMap<String, OAuthToken>,
}

impl EventRegistration {
    /// Whether the trigger can start a run while `account` is signed in: an
    /// offline app needs no token, an online one a PAT verified for that
    /// account on the current profile's hub. Each run checks this again when
    /// it starts (see [`ensure_signed_in_owner`]).
    pub fn can_fire(&self, account: Option<&TriggerAccount>) -> bool {
        self.profile_binding.is_some() && (self.offline || self.owns_token(account))
    }

    /// Whether the stored PAT was verified with the hub, which recorded its owner.
    pub(crate) fn has_verified_token(&self) -> bool {
        self.token_owner().is_some()
    }

    /// Whom the stored PAT was verified for, and on which hub.
    pub(crate) fn token_owner(&self) -> Option<TriggerAccount> {
        if !has_token(self.personal_access_token.as_deref()) {
            return None;
        }
        let owner = self.profile_binding.as_ref()?.account()?;
        (self.pat_owner_sub.as_deref() == Some(owner.sub.as_str())
            && self.hub_origin.as_deref() == Some(owner.hub_origin.as_str()))
        .then_some(owner)
    }

    /// Drops the PAT and whom it was verified for.
    pub(crate) fn forget_token(&mut self) {
        self.personal_access_token = None;
        self.pat_owner_sub = None;
        self.hub_origin = None;
    }

    /// Whether this registration is `event`'s trigger: the same type, and
    /// forwarding to the hub exactly when the event runs there.
    pub(crate) fn is_trigger_of(&self, event: &Event) -> bool {
        self.r#type == event.event_type
            && self.remote == (event.execution_mode == EventExecutionMode::Remote)
    }

    /// Whether the stored PAT was verified to belong to `account`.
    pub fn owns_token(&self, account: Option<&TriggerAccount>) -> bool {
        token_owned_by(
            self.personal_access_token.as_deref(),
            self.pat_owner_sub.as_deref(),
            self.hub_origin.as_deref(),
            self.profile_binding.as_ref(),
            account,
        )
    }

    pub(crate) fn has_credentials(&self) -> bool {
        has_token(self.personal_access_token.as_deref()) || !self.oauth_tokens.is_empty()
    }

    /// Whether `other` starts the same trigger with the same credentials, so
    /// refreshing this registration with it would change nothing.
    pub(crate) fn same_trigger(&self, other: &EventRegistration) -> bool {
        self.trigger_key() == other.trigger_key()
            && self.profile_binding == other.profile_binding
            && same_json(&self.config, &other.config)
            && same_json(&self.oauth_tokens, &other.oauth_tokens)
    }

    fn trigger_key(
        &self,
    ) -> (
        &str,
        &str,
        bool,
        bool,
        Option<&str>,
        Option<&str>,
        Option<&str>,
    ) {
        (
            &self.app_id,
            &self.r#type,
            self.offline,
            self.remote,
            self.personal_access_token.as_deref(),
            self.pat_owner_sub.as_deref(),
            self.hub_origin.as_deref(),
        )
    }
}

fn same_json<T: Serialize>(left: &T, right: &T) -> bool {
    match (serde_json::to_value(left), serde_json::to_value(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}
