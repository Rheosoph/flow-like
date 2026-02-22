use flow_like::credentials::SharedCredentials;
use flow_like::flow::execution::UserExecutionContext;
use flow_like::flow::variable::Variable;
use flow_like_types::OAuthTokenInput;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use flow_like_types::dispatch::{DispatchPayload, WasmPackageRef};

/// Board version as a tuple (major, minor, patch)
pub type BoardVersion = (u32, u32, u32);

/// Request to execute a flow
/// The API is responsible for resolving events to board_id + board_version before dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    /// Credentials for storage access (meta, content, logs buckets)
    pub credentials: SharedCredentials,
    /// Application ID
    pub app_id: String,
    /// Board ID to execute (required)
    pub board_id: String,
    /// Board version as tuple (major, minor, patch) - pre-resolved by API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_version: Option<BoardVersion>,
    /// Node ID to start execution from
    pub node_id: String,
    /// Serialized Event struct when executing via event trigger (optional)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_json: Option<String>,
    /// Input payload for the execution
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    /// JWT containing callback_url and run metadata
    pub executor_jwt: String,
    /// OAuth tokens keyed by provider name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_tokens: Option<HashMap<String, OAuthTokenInput>>,
    /// Whether to stream node state updates (true for interactive boards, false for events/background)
    #[serde(default)]
    pub stream_state: bool,
    /// Runtime-configured variables to override board variables
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_variables: Option<HashMap<String, Variable>>,
    /// User execution context (role, permissions, attributes)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_context: Option<UserExecutionContext>,
    /// User profile (bits, hubs, settings) - pre-filtered for cloud deployments
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<serde_json::Value>,
    /// Pre-resolved WASM packages needed for this execution.
    /// Map of package_id → (version, wasm_hash) pre-resolved by the API from AppPackage records.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_packages: Option<HashMap<String, WasmPackageRef>>,
}

/// Result of an execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionResult {
    /// Run ID from the JWT
    pub run_id: String,
    /// Final status
    pub status: ExecutionStatus,
    /// Output payload (if any)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<serde_json::Value>,
    /// Error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Execution duration in milliseconds
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

/// Event emitted during execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionEvent {
    /// Unique event ID
    pub id: String,
    /// Run ID this event belongs to
    pub run_id: String,
    /// Sequence number for ordering
    pub sequence: i32,
    /// Event type (log, progress, output, error, chunk, etc.)
    pub event_type: EventType,
    /// Event payload
    pub payload: serde_json::Value,
    /// Timestamp when event was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EventType {
    Log,
    Progress,
    Output,
    Error,
    Chunk,
    NodeStart,
    NodeEnd,
    Custom(String),
}

impl TryFrom<DispatchPayload> for ExecutionRequest {
    type Error = serde_json::Error;

    fn try_from(p: DispatchPayload) -> Result<Self, Self::Error> {
        let credentials: SharedCredentials = serde_json::from_value(p.credentials)?;
        let runtime_variables = p
            .runtime_variables
            .map(serde_json::from_value)
            .transpose()?;
        let user_context = p.user_context.map(serde_json::from_value).transpose()?;

        Ok(Self {
            credentials,
            app_id: p.app_id,
            board_id: p.board_id,
            board_version: p.board_version,
            node_id: p.node_id,
            event_json: p.event_json,
            payload: p.payload,
            executor_jwt: p.executor_jwt,
            oauth_tokens: p.oauth_tokens,
            stream_state: p.stream_state,
            runtime_variables,
            user_context,
            profile: p.profile,
            wasm_packages: p.wasm_packages,
        })
    }
}
