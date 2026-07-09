//! Shared helpers for nodes that work with connected apps: they exchange the
//! runtime token for a short-lived app-to-app token and call the hub API on
//! behalf of the current app.

use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{json::json, reqwest};
use std::sync::OnceLock;

#[derive(Debug, flow_like_types::json::Deserialize)]
struct AppConnectionTokenResponse {
    token: String,
}

pub(crate) fn api_base_url(hub: &str, secure: bool) -> Option<String> {
    let hub = hub.trim().trim_end_matches('/');
    if hub.is_empty() {
        return None;
    }

    let origin = if hub.starts_with("http://") || hub.starts_with("https://") {
        hub.to_string()
    } else {
        let protocol = if secure { "https" } else { "http" };
        format!("{protocol}://{hub}")
    };

    if origin.ends_with("/api/v1") {
        return Some(origin);
    }

    Some(format!("{origin}/api/v1"))
}

pub(crate) fn http_client() -> reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new).clone()
}

pub(crate) async fn error_for_status(
    response: reqwest::Response,
    operation: &str,
) -> flow_like_types::Result<reqwest::Response> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    let body: String = body.chars().take(500).collect();
    Err(flow_like_types::anyhow!(
        "{} failed with status {}: {}",
        operation,
        status,
        body
    ))
}

/// Validates an id used as a URL path segment (app ids, event ids).
pub(crate) fn validate_path_id(value: &str, label: &str) -> flow_like_types::Result<String> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(flow_like_types::anyhow!("No {} selected", label));
    }
    if !value
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(flow_like_types::anyhow!("Invalid {} '{}'", label, value));
    }
    Ok(value)
}

/// A short-lived session against a connected app: the app-to-app token plus
/// the API base url. Mint one per remote operation — tokens expire quickly.
pub(crate) struct RemoteAppSession {
    pub token: String,
    pub base_url: String,
    pub target_app_id: String,
}

impl RemoteAppSession {
    pub async fn open(
        context: &ExecutionContext,
        target_app_id: &str,
    ) -> flow_like_types::Result<Self> {
        let context_cache = context
            .execution_cache
            .clone()
            .ok_or(flow_like_types::anyhow!("No execution cache found"))?;
        let origin_app_id = context_cache.app_id.clone();

        if origin_app_id == target_app_id {
            return Err(flow_like_types::anyhow!(
                "The remote project is the current project"
            ));
        }

        let token = context
            .token
            .clone()
            .filter(|token| !token.trim().is_empty())
            .ok_or(flow_like_types::anyhow!(
                "Working with a connected app requires a connected session (no auth token available)"
            ))?;
        let base_url = api_base_url(&context.profile.hub, context.profile.secure).ok_or(
            flow_like_types::anyhow!("No hub URL configured on the execution profile"),
        )?;

        let token_url = format!(
            "{}/apps/{}/connections/{}/token",
            base_url, origin_app_id, target_app_id
        );
        let response = http_client()
            .post(&token_url)
            .bearer_auth(token.trim())
            // The run id ties the minted token — and every run it triggers
            // downstream — into this run's process case, even when the bearer
            // is a user token instead of an executor JWT.
            .json(&json!({ "run_id": context.run_id() }))
            .send()
            .await
            .map_err(|err| {
                flow_like_types::anyhow!("Failed to request app connection token: {}", err)
            })?;
        let response = error_for_status(response, "App connection token request").await?;
        let token_response: AppConnectionTokenResponse = response.json().await?;

        Ok(Self {
            token: token_response.token,
            base_url,
            target_app_id: target_app_id.to_string(),
        })
    }

    pub fn url(&self, path: &str) -> String {
        format!(
            "{}/apps/{}/{}",
            self.base_url,
            self.target_app_id,
            path.trim_start_matches('/')
        )
    }
}

const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Parses an MCP HTTP response that may be either `application/json` or an SSE
/// `text/event-stream` (`data: {json}` frames). Returns the first decodable
/// JSON object and any `Mcp-Session-Id` echoed back.
async fn parse_mcp_response(
    response: reqwest::Response,
) -> flow_like_types::Result<(Option<flow_like_types::Value>, Option<String>)> {
    let session_id = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string());
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();

    let text = response.text().await?;
    if text.trim().is_empty() {
        return Ok((None, session_id));
    }

    if content_type.contains("text/event-stream") {
        for line in text.lines() {
            if let Some(data) = line.strip_prefix("data:")
                && let Ok(value) =
                    flow_like_types::json::from_str::<flow_like_types::Value>(data.trim())
            {
                return Ok((Some(value), session_id));
            }
        }
        return Ok((None, session_id));
    }

    let value = flow_like_types::json::from_str::<flow_like_types::Value>(text.trim())?;
    Ok((Some(value), session_id))
}

impl RemoteAppSession {
    async fn mcp_post(
        &self,
        event_id: &str,
        session_id: Option<&str>,
        body: &flow_like_types::Value,
    ) -> flow_like_types::Result<(Option<flow_like_types::Value>, Option<String>)> {
        let url = self.url(&format!("events/{}/mcp", event_id));
        let mut request = http_client()
            .post(&url)
            .bearer_auth(&self.token)
            .header(reqwest::header::ACCEPT, "application/json")
            .header("MCP-Protocol-Version", MCP_PROTOCOL_VERSION);
        if let Some(session_id) = session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        let response = request
            .json(body)
            .send()
            .await
            .map_err(|err| flow_like_types::anyhow!("Failed to call remote MCP: {}", err))?;
        let response = error_for_status(response, "Remote MCP request").await?;
        parse_mcp_response(response).await
    }

    /// Runs a single MCP JSON-RPC method against a connected app's MCP event:
    /// initializes a session, issues the call, and returns the JSON-RPC
    /// `result` (erroring on a JSON-RPC error).
    pub async fn mcp_request(
        &self,
        event_id: &str,
        method: &str,
        params: flow_like_types::Value,
    ) -> flow_like_types::Result<flow_like_types::Value> {
        let init = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "Flow-Like", "version": "alpha" }
            }
        });
        let (_, session_id) = self.mcp_post(event_id, None, &init).await?;

        let call = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": method,
            "params": params
        });
        let (response, _) = self
            .mcp_post(event_id, session_id.as_deref(), &call)
            .await?;

        let response = response
            .ok_or_else(|| flow_like_types::anyhow!("Empty MCP response for {}", method))?;
        if let Some(error) = response.get("error") {
            let message = error
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown error");
            return Err(flow_like_types::anyhow!(
                "MCP {} error: {}",
                method,
                message
            ));
        }
        Ok(response
            .get("result")
            .cloned()
            .unwrap_or(flow_like_types::Value::Null))
    }
}
