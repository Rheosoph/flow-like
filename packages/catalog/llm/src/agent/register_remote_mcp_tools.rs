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
#[cfg(feature = "execute")]
use flow_like_types::PROXY_EVENT_AUTHORIZATION_HEADER;
use flow_like_types::async_trait;
#[cfg(feature = "execute")]
use std::collections::{HashMap, HashSet};

const PIN_REMOTE_APP_ID: &str = "_flow_remote_app_id";
const PIN_REMOTE_EVENT: &str = "_flow_remote_event";
const PIN_REMOTE_EVENT_META: &str = "_flow_remote_event_meta";

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

        let remote_app_id = flow_like_catalog_data::remote_util::validate_path_id(
            &remote_app_id,
            "remote project",
        )?;
        let event_id =
            flow_like_catalog_data::remote_util::validate_path_id(&event_id, "remote event")?;

        // Resolve once here to validate access and warm the run cache. The
        // bearer itself is deliberately not serialized into the Agent; it is
        // resolved again (normally a cache hit) immediately before transport
        // construction so a delayed invocation never freezes a near-expiry
        // token.
        let session = flow_like_catalog_data::remote_util::remote_app_session_for_mcp(
            context,
            &remote_app_id,
        )
        .await?;
        let uri = session.url(&format!("events/{event_id}/mcp"));

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
            auth_header: None,
            remote_app_id: Some(remote_app_id),
            remote_event_id: Some(event_id),
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
