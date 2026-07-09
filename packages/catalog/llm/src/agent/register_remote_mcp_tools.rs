/// # Register Remote MCP Tools Node
/// Adds the MCP server of a connected app's MCP event as agent tools, using a
/// short-lived app-to-app token for authentication. The connected app must
/// have granted this app a role that allows executing events.
use crate::generative::agent::Agent;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};

const PIN_REMOTE_APP_ID: &str = "_flow_remote_app_id";
const PIN_REMOTE_EVENT: &str = "_flow_remote_event";
const PIN_REMOTE_EVENT_META: &str = "_flow_remote_event_meta";
#[cfg(feature = "execute")]
const PROXY_EVENT_AUTHORIZATION_HEADER: &str = "x-flow-like-event-authorization";

#[crate::register_node]
#[derive(Default)]
pub struct RegisterRemoteMcpToolsNode {}

#[async_trait]
impl NodeLogic for RegisterRemoteMcpToolsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_register_remote_mcp_tools",
            "Register Remote MCP Tools",
            "Adds a connected app's MCP event as agent tools. Uses a short-lived app-to-app token (valid ~15 minutes) that is refreshed on every run.",
            "AI/Agents/Builder",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(2);
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(6)
                .set_cost(3)
                .build(),
        );

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent object to add the remote MCP tools to",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            PIN_REMOTE_APP_ID,
            "Project",
            "Connected project that hosts the MCP event",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_input_pin(
            PIN_REMOTE_EVENT,
            "Event",
            "MCP event of the selected project",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_input_pin(
            PIN_REMOTE_EVENT_META,
            "Event Details",
            "Auto-filled by the editor when an event is selected",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("")));

        node.add_input_pin(
            "tool_filter",
            "Tool Filter",
            "Optional list of tool names to include. Empty = all tools.",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "headers",
            "Auth Headers",
            "Static registration authentication headers (for example Authorization or x-api-key). HMAC auth is not supported because each MCP request requires a fresh signature.",
            VariableType::Struct,
        )
        .set_schema::<std::collections::HashMap<String, String>>();

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent object with the remote MCP tools registered",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut agent: Agent = context.evaluate_pin("agent_in").await?;
        let remote_app_id: String = context.evaluate_pin(PIN_REMOTE_APP_ID).await?;
        let event_id: String = context.evaluate_pin(PIN_REMOTE_EVENT).await?;
        let tool_filter: Vec<String> = context
            .evaluate_pin("tool_filter")
            .await
            .unwrap_or_default();
        let registration_headers: HashMap<String, String> =
            context.evaluate_pin("headers").await.unwrap_or_default();

        let remote_app_id = remote_app_id.trim();
        let event_id = event_id.trim();
        if remote_app_id.is_empty() || event_id.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Both a project and an MCP event must be selected"
            ));
        }

        let (token, base_url) = mint_app_connection_token(context, remote_app_id).await?;
        let uri = format!(
            "{}/apps/{}/events/{}/mcp",
            base_url, remote_app_id, event_id
        );

        let tool_filter = if tool_filter.is_empty() {
            None
        } else {
            Some(tool_filter.into_iter().collect::<HashSet<String>>())
        };
        let custom_headers = registration_headers
            .into_iter()
            .map(|(name, value)| {
                if name.eq_ignore_ascii_case("authorization") {
                    (PROXY_EVENT_AUTHORIZATION_HEADER.to_string(), value)
                } else {
                    (name, value)
                }
            })
            .collect();

        agent.add_mcp_server(super::McpServerConfig {
            uri,
            tool_filter,
            auth_header: Some(token),
            custom_headers,
        });

        context
            .set_pin_value("agent_out", flow_like_types::json::json!(agent))
            .await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}

/// Resolves the API base url from the runtime profile, appending `/api/v1`.
#[cfg(feature = "execute")]
fn api_base_url(hub: &str, secure: bool) -> Option<String> {
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

/// Exchanges the runtime token for a short-lived app-to-app token bound to the
/// origin app and the connected target app.
#[cfg(feature = "execute")]
async fn mint_app_connection_token(
    context: &ExecutionContext,
    target_app_id: &str,
) -> flow_like_types::Result<(String, String)> {
    let context_cache = context
        .execution_cache
        .clone()
        .ok_or_else(|| flow_like_types::anyhow!("No execution cache found"))?;
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
        .ok_or_else(|| {
            flow_like_types::anyhow!(
                "Working with a connected app requires a connected session (no auth token available)"
            )
        })?;
    let base_url = api_base_url(&context.profile.hub, context.profile.secure).ok_or_else(|| {
        flow_like_types::anyhow!("No hub URL configured on the execution profile")
    })?;

    let url = format!(
        "{}/apps/{}/connections/{}/token",
        base_url, origin_app_id, target_app_id
    );
    let response = flow_like_types::reqwest::Client::new()
        .post(&url)
        .bearer_auth(token.trim())
        .json(&flow_like_types::json::json!({ "ttl_seconds": 900 }))
        .send()
        .await
        .map_err(|err| {
            flow_like_types::anyhow!("Failed to request app connection token: {}", err)
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        let body: String = body.chars().take(500).collect();
        return Err(flow_like_types::anyhow!(
            "App connection token request failed with status {}: {}",
            status,
            body
        ));
    }

    let value: flow_like_types::Value = response.json().await?;
    let token = value
        .get("token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| flow_like_types::anyhow!("App connection token response missing token"))?
        .to_string();

    Ok((token, base_url))
}
