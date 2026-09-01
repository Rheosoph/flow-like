use flow_like::credentials::SharedCredentials;
use flow_like::flow::execution::{ExecutionMode, RunStatus, UserExecutionContext};
use flow_like::flow::variable::Variable;
use flow_like_types::OAuthTokenInput;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub use flow_like_types::channel::ChannelGrant;
pub use flow_like_types::dispatch::{DispatchPayload, WasmPackageRef};

/// Board version as a tuple (major, minor, patch)
pub type BoardVersion = (u32, u32, u32);

/// Request to execute a flow
/// The API is responsible for resolving events to board_id + board_version before dispatch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionRequest {
    /// Broker job identifier. Queue runtimes bind their ownership lease to
    /// this value; direct runtimes may leave it empty.
    #[serde(default)]
    pub job_id: String,
    /// Credentials for storage access (meta, content, logs buckets)
    pub credentials: SharedCredentials,
    /// Application ID
    pub app_id: String,
    /// Board ID to execute (required)
    pub board_id: String,
    /// Board version as tuple (major, minor, patch) - pre-resolved by API
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_version: Option<BoardVersion>,
    /// Exact source object ETag for a floating Latest board.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_etag: Option<String>,
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
    /// User/PAT/API token injected by the API for workflow nodes to call back into the hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// OAuth tokens keyed by provider name
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_tokens: Option<HashMap<String, OAuthTokenInput>>,
    /// Whether to stream node state updates (true for interactive boards, false for events/background)
    #[serde(default)]
    pub stream_state: bool,
    /// Execution mode hint from the API dispatch path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<ExecutionMode>,
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
    /// Channel credentials minted by the API: how this run waits for client replies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelGrant>,
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

impl ExecutionStatus {
    /// Converts the core run status after `InternalRun::execute` has finished.
    /// A still-running status at that boundary is invalid and must fail closed.
    pub fn from_final_run_status(status: &RunStatus) -> Self {
        match status {
            RunStatus::Success => Self::Completed,
            RunStatus::Failed => Self::Failed,
            RunStatus::Stopped => Self::Cancelled,
            RunStatus::Running => Self::Failed,
        }
    }
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

fn decode_board_selector(
    wire_version: Option<BoardVersion>,
    board_etag: Option<String>,
) -> flow_like_types::Result<(Option<BoardVersion>, Option<String>)> {
    use flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL;

    let board_etag = board_etag
        .map(|etag| etag.trim().to_string())
        .filter(|etag| !etag.is_empty());
    match (wire_version, board_etag) {
        (Some(version), Some(etag)) if version == ETAG_BOUND_LATEST_VERSION_SENTINEL => {
            Ok((None, Some(etag)))
        }
        (Some(version), None) if version == ETAG_BOUND_LATEST_VERSION_SENTINEL => Err(
            flow_like_types::anyhow!("ETag-bound Latest dispatch is missing board_etag"),
        ),
        (version, Some(_)) => Err(flow_like_types::anyhow!(
            "board_etag requires the ETag-bound Latest wire selector, got {version:?}"
        )),
        (version, None) => Ok((version, None)),
    }
}

impl TryFrom<DispatchPayload> for ExecutionRequest {
    type Error = flow_like_types::Error;

    fn try_from(p: DispatchPayload) -> Result<Self, Self::Error> {
        let credentials: SharedCredentials = serde_json::from_value(p.credentials)?;
        let runtime_variables = p
            .runtime_variables
            .map(serde_json::from_value)
            .transpose()?;
        let user_context = p.user_context.map(serde_json::from_value).transpose()?;
        let execution_mode = match p.execution_mode.as_deref() {
            Some(value) => Some(ExecutionMode::parse(value).ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Invalid execution_mode '{}'. Expected one of: sync, async, event, scheduled",
                    value
                )
            })?),
            None => None,
        };

        let (board_version, board_etag) = decode_board_selector(p.board_version, p.board_etag)?;

        Ok(Self {
            job_id: p.job_id,
            credentials,
            app_id: p.app_id,
            board_id: p.board_id,
            board_version,
            board_etag,
            node_id: p.node_id,
            event_json: p.event_json,
            payload: p.payload,
            executor_jwt: p.executor_jwt,
            token: p.token,
            oauth_tokens: p.oauth_tokens,
            stream_state: p.stream_state,
            execution_mode,
            runtime_variables,
            user_context,
            profile: p.profile,
            wasm_packages: p.wasm_packages,
            channel: p.channel,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_core_run_status_is_not_masked_as_completed() {
        assert_eq!(
            ExecutionStatus::from_final_run_status(&RunStatus::Success),
            ExecutionStatus::Completed
        );
        assert_eq!(
            ExecutionStatus::from_final_run_status(&RunStatus::Failed),
            ExecutionStatus::Failed
        );
        assert_eq!(
            ExecutionStatus::from_final_run_status(&RunStatus::Stopped),
            ExecutionStatus::Cancelled
        );
        assert_eq!(
            ExecutionStatus::from_final_run_status(&RunStatus::Running),
            ExecutionStatus::Failed
        );
    }

    /// The JSON body `build_executor_payload` sends over every direct
    /// transport (HTTP, SSE, Lambda stream) for an ETag-bound Latest run.
    fn etag_latest_wire_json() -> serde_json::Value {
        serde_json::json!({
            "job_id": "job-1",
            "run_id": "run-1",
            "app_id": "app-1",
            "board_id": "board-1",
            "board_version": [u32::MAX, u32::MAX, u32::MAX],
            "board_etag": "etag-a",
            "node_id": "node-1",
            "user_id": "user-1",
            "credentials": {
                "Aws": {
                    "access_key_id": "AKIAIOSFODNN7EXAMPLE",
                    "secret_access_key": "secret",
                    "session_token": null,
                    "meta_bucket": "meta",
                    "content_bucket": "content",
                    "logs_bucket": "logs",
                    "region": "us-east-1",
                    "expiration": null
                }
            },
            "executor_jwt": "jwt",
            "callback_url": "https://api.example",
            "stream_state": true
        })
    }

    #[test]
    fn dispatch_wire_payload_reaches_validation_with_the_sentinel_decoded() {
        use flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL;

        let payload: DispatchPayload = serde_json::from_value(etag_latest_wire_json())
            .expect("wire body deserializes as DispatchPayload");
        let request = ExecutionRequest::try_from(payload).expect("selector decodes");
        assert_eq!(request.board_version, None);
        assert_eq!(request.board_etag.as_deref(), Some("etag-a"));

        // Deserializing the same body directly as ExecutionRequest (the old
        // router path) leaves the raw sentinel in place, which validation
        // rejects — the router must decode through DispatchPayload.
        let raw: ExecutionRequest = serde_json::from_value(etag_latest_wire_json())
            .expect("serde ignores the DispatchPayload-only fields");
        assert_eq!(raw.board_version, Some(ETAG_BOUND_LATEST_VERSION_SENTINEL));
        assert_eq!(raw.board_etag.as_deref(), Some("etag-a"));
    }

    #[test]
    fn etag_latest_wire_selector_is_explicit_and_fail_closed() {
        use flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL;

        assert_eq!(
            decode_board_selector(
                Some(ETAG_BOUND_LATEST_VERSION_SENTINEL),
                Some(" etag-a ".into())
            )
            .unwrap(),
            (None, Some("etag-a".into()))
        );
        assert!(decode_board_selector(Some(ETAG_BOUND_LATEST_VERSION_SENTINEL), None).is_err());
        assert!(decode_board_selector(None, Some("etag-a".into())).is_err());
        assert_eq!(
            decode_board_selector(Some((1, 2, 3)), None).unwrap(),
            (Some((1, 2, 3)), None)
        );
    }
}
