mod auth;
mod cards;
mod config;
mod context;
mod files;
pub mod lookup;
pub mod management;
mod microsoft;
mod provision;
pub mod runtime;
mod store;

pub use microsoft::TeamsMemberInfo;
pub(crate) use store::app_event_ids;
pub(crate) use store::sweep;

use crate::{error::ApiError, state::AppState};
use flow_like_types::tokio::{self, task::JoinHandle};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

const SWEEP_INTERVAL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
#[schema(as = TeamsAuthMode)]
pub enum AuthMode {
    FlowLikeManaged,
    CustomerTeams,
    CustomerAzure,
}

/// Read access the bot requests through resource-specific consent (RSC) in its Teams app.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TeamsPermission {
    ReadMessages,
    MeetingDetails,
    ConversationDetails,
}

impl TeamsPermission {
    /// The RSC permission names declared in the Teams app manifest.
    pub fn rsc(self) -> &'static [&'static str] {
        match self {
            Self::ReadMessages => &["ChannelMessage.Read.Group", "ChatMessage.Read.Chat"],
            Self::MeetingDetails => &[
                "OnlineMeeting.ReadBasic.Chat",
                "ChannelMeeting.ReadBasic.Group",
            ],
            Self::ConversationDetails => &[
                "TeamSettings.Read.Group",
                "ChannelSettings.Read.Group",
                "ChatSettings.Read.Chat",
            ],
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Connection {
    pub id: String,
    pub app_id: String,
    pub event_id: String,
    pub mode: AuthMode,
    pub name: String,
    pub description: String,
    pub customer_tenant_id: String,
    pub home_tenant_id: String,
    pub client_id: String,
    pub secret: String,
    pub graph_object_id: Option<String>,
    pub azure_resource_id: Option<String>,
    pub secret_key_id: Option<String>,
    pub secret_expires_at: Option<String>,
    #[serde(default)]
    pub pending_secret_key_ids: Vec<String>,
    pub status: String,
    pub allowed_responders: Vec<String>,
    #[serde(default)]
    pub permissions: Vec<TeamsPermission>,
    #[serde(default)]
    pub operation_until: i64,
}

impl Connection {
    /// Counts against the app's managed-bot quota until it is disconnected.
    pub fn holds_managed_slot(&self) -> bool {
        self.mode == AuthMode::FlowLikeManaged
            && !matches!(self.status.as_str(), "disconnected" | "not_configured")
    }
}

pub(super) fn now() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

/// Length-prefixed, so `["ab", "c"]` and `["a", "bc"]` never collide.
pub(super) fn hash(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for part in parts {
        h.update(&(part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}

pub(super) fn client() -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|_| ApiError::internal("Could not initialize Teams HTTP client"))
}

/// Microsoft responses worth retrying: throttling and server errors.
pub(super) fn transient(status: reqwest::StatusCode) -> bool {
    status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS
}

pub(super) fn guid(value: &str) -> Result<String, ApiError> {
    uuid::Uuid::parse_str(value.trim())
        .map(|id| id.to_string())
        .map_err(|_| ApiError::bad_request("Enter a valid Microsoft application or tenant ID"))
}

pub(super) fn ensure_enabled(state: &AppState) -> Result<(), ApiError> {
    if state
        .platform_config
        .supported_sinks
        .as_ref()
        .is_some_and(|sinks| sinks.teams)
    {
        return Ok(());
    }
    Err(ApiError::forbidden(
        "Teams bots are not enabled on this server. Ask your administrator to enable supported_sinks.teams.",
    ))
}

pub(super) async fn endpoint(state: &AppState, id: &str) -> Result<String, ApiError> {
    let fallback = std::env::var("API_BASE_URL").ok();
    let base = config::public_base(&state.secrets, fallback.as_deref()).await?;
    Ok(format!("{base}/sink/trigger/teams/{id}"))
}

/// Removes expired conversation, delivery and interaction state on long-running API servers.
/// Serverless deployments run the same sweep from the scheduled cache-cleanup maintenance job.
pub fn spawn_sweeper(state: AppState) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(SWEEP_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await;
        loop {
            ticker.tick().await;
            match sweep(&state).await {
                Ok(0) => {}
                Ok(deleted) => tracing::info!(deleted, "Teams state sweep removed expired rows"),
                Err(error) => tracing::error!(%error, "Teams state sweep failed"),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use TeamsPermission::*;
    use serde_json::json;

    #[test]
    fn permissions_use_snake_case_wire_names() {
        assert_eq!(
            serde_json::to_value([ReadMessages, MeetingDetails, ConversationDetails]).unwrap(),
            json!(["read_messages", "meeting_details", "conversation_details"])
        );
        assert_eq!(
            serde_json::from_value::<Vec<TeamsPermission>>(json!(["conversation_details"]))
                .unwrap(),
            vec![ConversationDetails]
        );
        assert!(serde_json::from_value::<TeamsPermission>(json!("ReadMessages")).is_err());
    }

    #[test]
    fn permissions_list_their_rsc_names_in_manifest_order() {
        assert_eq!(
            ReadMessages.rsc(),
            ["ChannelMessage.Read.Group", "ChatMessage.Read.Chat"]
        );
        assert_eq!(
            MeetingDetails.rsc(),
            [
                "OnlineMeeting.ReadBasic.Chat",
                "ChannelMeeting.ReadBasic.Group"
            ]
        );
        assert_eq!(
            ConversationDetails.rsc(),
            [
                "TeamSettings.Read.Group",
                "ChannelSettings.Read.Group",
                "ChatSettings.Read.Chat"
            ]
        );
    }

    #[test]
    fn connections_saved_before_permissions_existed_request_none() {
        let connection: Connection = serde_json::from_value(json!({
            "id":"c", "app_id":"app", "event_id":"event", "mode":"flow_like_managed",
            "name":"Bot", "description":"", "customer_tenant_id":"t", "home_tenant_id":"h",
            "client_id":"id", "secret":"s", "graph_object_id":null, "azure_resource_id":null,
            "secret_key_id":null, "secret_expires_at":null, "status":"ready", "allowed_responders":[]
        }))
        .unwrap();
        assert!(connection.permissions.is_empty());
    }

    #[test]
    fn hashes_are_length_prefixed() {
        assert_ne!(hash(&["ab", "c"]), hash(&["a", "bc"]));
        assert_eq!(hash(&["a", "b"]).len(), 64);
    }
}
