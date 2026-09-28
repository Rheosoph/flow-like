mod auth;
mod cards;
mod config;
pub mod management;
mod provision;
pub mod runtime;
mod store;

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
