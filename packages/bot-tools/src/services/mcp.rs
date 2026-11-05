//! The MCP client behind `connector_call` and the refresh of a connector's tool list: Streamable
//! HTTP only, no redirects (the token would follow them), one session per call.

use flow_like_types::Bytes;
use flow_like_types::bot_connector::ConnectorToolInfo;
use serde_json::{Map, Value};

use super::McpTool;
use crate::env::Environment;

/// Where an MCP call goes and how it signs in.
#[derive(Clone)]
pub struct McpEndpoint {
    pub url: String,
    /// The `Authorization` header value, if any.
    pub authorization: Option<String>,
    pub environment: Environment,
}

pub struct Binary {
    pub mime: String,
    pub data: Bytes,
}

pub struct McpOutcome {
    pub text: String,
    pub is_error: bool,
    pub binaries: Vec<Binary>,
}

/// blake3 of the schema's JSON, so a changed tool shows up as a changed hash.
pub fn schema_hash(schema: &Value) -> String {
    blake3::hash(schema.to_string().as_bytes()).to_hex()[..16].to_string()
}

fn flag(annotations: &Value, key: &str) -> Option<bool> {
    annotations.get(key).and_then(Value::as_bool)
}

fn tool_of(value: &Value) -> Option<McpTool> {
    let name = value.get("name")?.as_str()?.to_string();
    let schema = value.get("inputSchema").cloned().unwrap_or(Value::Null);
    let annotations = value.get("annotations").cloned().unwrap_or(Value::Null);
    Some(McpTool {
        info: ConnectorToolInfo {
            name,
            description: value
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            read_only_hint: flag(&annotations, "readOnlyHint"),
            destructive_hint: flag(&annotations, "destructiveHint"),
            schema_hash: schema_hash(&schema),
        },
        input_schema: schema,
    })
}

/// Text, saved binaries and the error flag of a `tools/call` result in its JSON form.
fn outcome_of(result: &Value) -> McpOutcome {
    let mut text = Vec::new();
    let mut binaries = Vec::new();
    for block in result
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        match block.get("type").and_then(Value::as_str) {
            Some("text") => {
                if let Some(part) = block.get("text").and_then(Value::as_str) {
                    text.push(part.to_string());
                }
            }
            Some("image" | "audio") => {
                let mime = block
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("application/octet-stream");
                let decoded = block.get("data").and_then(Value::as_str).and_then(|data| {
                    use flow_like_types::base64::{Engine, engine::general_purpose::STANDARD};
                    STANDARD.decode(data).ok()
                });
                match decoded {
                    Some(data) => binaries.push(Binary {
                        mime: mime.to_string(),
                        data: Bytes::from(data),
                    }),
                    None => text.push(format!("[{mime} content that could not be decoded]")),
                }
            }
            Some("resource") => {
                let resource = block.get("resource").unwrap_or(&Value::Null);
                if let Some(part) = resource.get("text").and_then(Value::as_str) {
                    text.push(part.to_string());
                } else if let Some(uri) = resource.get("uri").and_then(Value::as_str) {
                    text.push(format!("[binary resource {uri}]"));
                }
            }
            Some("resource_link") => {
                if let Some(uri) = block.get("uri").and_then(Value::as_str) {
                    text.push(format!("[resource {uri}]"));
                }
            }
            _ => {}
        }
    }
    if text.is_empty()
        && let Some(structured) = result.get("structuredContent")
    {
        text.push(structured.to_string());
    }
    McpOutcome {
        text: text.join("\n"),
        is_error: result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        binaries,
    }
}

#[cfg(feature = "mcp")]
mod client {
    use std::time::Duration;

    use rmcp::{
        ServiceExt,
        model::{
            CallToolRequestParams, ClientCapabilities, ClientInfo, Implementation,
            PaginatedRequestParams,
        },
        service::{RoleClient, RunningService},
        transport::{
            StreamableHttpClientTransport,
            streamable_http_client::StreamableHttpClientTransportConfig,
        },
    };

    use super::*;
    use crate::services::egress::resolve_reachable;

    const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
    const CALL_TIMEOUT: Duration = Duration::from_secs(60);
    const MAX_PAGES: usize = 20;

    type Session = RunningService<RoleClient, ClientInfo>;

    async fn connect(endpoint: &McpEndpoint) -> Result<Session, String> {
        let reachable = resolve_reachable(endpoint.environment, &endpoint.url).await?;
        let url = reachable.url.clone();
        let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string());
        config.auth_header = endpoint
            .authorization
            .as_deref()
            .map(|value| value.strip_prefix("Bearer ").unwrap_or(value).to_string());
        let mut builder = reqwest_mcp::Client::builder()
            .pool_max_idle_per_host(0)
            .redirect(reqwest_mcp::redirect::Policy::none());
        if !reachable.addrs.is_empty() {
            builder = builder.resolve_to_addrs(&reachable.host, &reachable.addrs);
        }
        let http = builder
            .build()
            .map_err(|error| format!("The MCP client could not be created: {error}"))?;
        let transport = StreamableHttpClientTransport::with_client(http, config);
        let info = ClientInfo::new(
            ClientCapabilities::default(),
            Implementation::new("Flow-Like", "alpha"),
        );
        tokio_timeout(CONNECT_TIMEOUT, info.serve(transport))
            .await?
            .map_err(|error| format!("Could not connect to the MCP server: {error}"))
    }

    async fn tokio_timeout<T>(
        limit: Duration,
        future: impl std::future::Future<Output = T>,
    ) -> Result<T, String> {
        flow_like_types::tokio::time::timeout(limit, future)
            .await
            .map_err(|_| {
                format!(
                    "The MCP server did not answer within {} seconds",
                    limit.as_secs()
                )
            })
    }

    pub async fn list(endpoint: &McpEndpoint) -> Result<Vec<McpTool>, String> {
        let session = connect(endpoint).await?;
        let mut tools = Vec::new();
        let mut cursor: Option<PaginatedRequestParams> = None;
        for _ in 0..MAX_PAGES {
            let page = tokio_timeout(CALL_TIMEOUT, session.list_tools(cursor.clone()))
                .await?
                .map_err(|error| format!("Listing the MCP tools failed: {error}"))?;
            for tool in page.tools {
                let value = serde_json::to_value(&tool).unwrap_or(Value::Null);
                tools.extend(tool_of(&value));
            }
            match page.next_cursor {
                Some(next) => {
                    cursor = Some(PaginatedRequestParams::default().with_cursor(Some(next)))
                }
                None => break,
            }
        }
        let _ = session.cancel().await;
        Ok(tools)
    }

    pub async fn call(
        endpoint: &McpEndpoint,
        tool: &str,
        arguments: Map<String, Value>,
    ) -> Result<McpOutcome, String> {
        let session = connect(endpoint).await?;
        let mut params = CallToolRequestParams::new(tool.to_string());
        params.arguments = Some(arguments);
        let result = tokio_timeout(CALL_TIMEOUT, session.call_tool(params))
            .await?
            .map_err(|error| format!("The MCP call failed: {error}"));
        let _ = session.cancel().await;
        let value = serde_json::to_value(result?).unwrap_or(Value::Null);
        Ok(outcome_of(&value))
    }
}

#[cfg(feature = "mcp")]
pub(super) async fn call(
    endpoint: &McpEndpoint,
    tool: &str,
    arguments: Map<String, Value>,
) -> Result<McpOutcome, String> {
    client::call(endpoint, tool, arguments).await
}

#[cfg(not(feature = "mcp"))]
pub(super) async fn call(
    _endpoint: &McpEndpoint,
    _tool: &str,
    _arguments: Map<String, Value>,
) -> Result<McpOutcome, String> {
    Err("This build has no MCP client (feature 'mcp')".to_string())
}

#[cfg(feature = "mcp")]
pub(super) async fn fetch_live(endpoint: &McpEndpoint) -> Result<Vec<McpTool>, String> {
    client::list(endpoint).await
}

#[cfg(not(feature = "mcp"))]
pub(super) async fn fetch_live(_endpoint: &McpEndpoint) -> Result<Vec<McpTool>, String> {
    Err("This build has no MCP client (feature 'mcp')".to_string())
}

/// The server's tools as the settings snapshot keeps them.
pub async fn fetch_tools(endpoint: &McpEndpoint) -> Result<Vec<ConnectorToolInfo>, String> {
    Ok(fetch_live(endpoint)
        .await?
        .into_iter()
        .map(|tool| tool.info)
        .collect())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_result_becomes_text_files_and_an_error_flag() {
        let result = json!({
            "content": [
                { "type": "text", "text": "first" },
                { "type": "image", "mimeType": "image/png", "data": "aGVsbG8=" },
                { "type": "resource", "resource": { "uri": "file:///a", "text": "inside" } },
                { "type": "resource_link", "uri": "https://x/y" },
            ],
            "isError": true,
        });
        let outcome = outcome_of(&result);
        assert!(outcome.is_error);
        assert_eq!(outcome.text, "first\ninside\n[resource https://x/y]");
        assert_eq!(outcome.binaries.len(), 1);
        assert_eq!(&outcome.binaries[0].data[..], b"hello");
    }

    #[test]
    fn hints_and_a_schema_hash_come_from_the_tool() {
        let a = tool_of(&json!({
            "name": "search",
            "description": "Find things",
            "inputSchema": { "type": "object" },
            "annotations": { "readOnlyHint": true, "destructiveHint": false },
        }))
        .unwrap();
        assert_eq!(a.info.read_only_hint, Some(true));
        assert_eq!(a.info.destructive_hint, Some(false));
        let b = tool_of(&json!({ "name": "search", "inputSchema": { "type": "string" } })).unwrap();
        assert_ne!(a.info.schema_hash, b.info.schema_hash);
        assert_eq!(b.info.read_only_hint, None);
    }
}
