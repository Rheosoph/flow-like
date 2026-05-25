//! Remote server-config emission helpers.
//!
//! When the `remote` feature is enabled (and `local` is not), the REST and MCP
//! "server" nodes do not bind a socket — instead, during a setup run, they
//! serialize the composed server config and stream it back as a structured
//! `server_config` intercom event. The API layer listens for these events,
//! merges them, and persists the resulting registration so inbound requests
//! can be dispatched directly to the registered nodes.

use flow_like_types::Value;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Intercom event_type emitted by remote server nodes during a setup run.
pub const REMOTE_SERVER_CONFIG_EVENT_TYPE: &str = "server_config";

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RemoteServerKind {
    Rest,
    Mcp,
}

/// Payload of a `server_config` intercom event.
///
/// `node_id` identifies the server-node that emitted the config (multiple
/// server nodes can co-exist within one event — the API merges them).
/// `config` is the raw serialized `RestServerConfig` / `McpServerConfig`.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct RemoteServerConfigEvent {
    pub kind: RemoteServerKind,
    pub node_id: String,
    pub config: Value,
}

#[cfg(all(feature = "execute", feature = "remote", not(feature = "local")))]
pub(crate) async fn emit_remote_server_config<C>(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    kind: RemoteServerKind,
    config: &C,
) -> flow_like_types::Result<()>
where
    C: Serialize,
{
    use flow_like::flow::execution::LogLevel;

    let node_id = context.node.node.lock().await.id.clone();
    let config_value = match flow_like_types::json::to_value(config) {
        Ok(value) => value,
        Err(err) => {
            context.log_message(
                &format!("Failed to serialize remote server config: {}", err),
                LogLevel::Error,
            );
            return Err(flow_like_types::anyhow!(
                "Failed to serialize remote server config: {}",
                err
            ));
        }
    };

    let payload = RemoteServerConfigEvent {
        kind,
        node_id,
        config: config_value,
    };

    context
        .stream_response(REMOTE_SERVER_CONFIG_EVENT_TYPE, payload)
        .await
}
