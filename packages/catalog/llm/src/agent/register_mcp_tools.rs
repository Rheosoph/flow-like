/// # Register MCP Tools Node
/// Adds Model Context Protocol (MCP) tools to an Agent object.
/// Supports two modes:
/// - Automatic: Uses all available tools from the MCP server
/// - Manual: Lets the user enable individual tools via dynamic boolean pins
use crate::generative::agent::Agent;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, PinType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json, json::from_slice};
#[cfg(feature = "execute")]
use rmcp::{
    ServiceExt,
    model::{ClientCapabilities, ClientInfo, Implementation, PaginatedRequestParam, Tool},
    transport::StreamableHttpClientTransport,
};
use std::{collections::HashSet, sync::Arc};

const TOOL_PIN_PREFIX: &str = "mcp_tool_";

#[crate::register_node]
#[derive(Default)]
pub struct RegisterMcpToolsNode {}

#[async_trait]
impl NodeLogic for RegisterMcpToolsNode {
    fn get_node(&self) -> Node {
        build_register_mcp_tools_node()
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut agent: Agent = context.evaluate_pin("agent_in").await?;
        let uri: String = context.evaluate_pin("uri").await?;
        let mode = context
            .evaluate_pin::<String>("mode")
            .await
            .unwrap_or_else(|_| "Automatic".to_string());

        let tool_filter = if is_manual_mode(&mode) {
            Some(collect_manual_tool_selection(context).await)
        } else {
            None
        };

        agent.add_mcp_server(super::McpServerConfig { uri, tool_filter });

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }

    #[cfg(feature = "execute")]
    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
        node.error = None;

        if !read_pin_string(node, "mode")
            .as_deref()
            .is_some_and(is_manual_mode)
        {
            cleanup_tool_pins(node, &HashSet::new());
            return;
        }

        let uri = match read_pin_string(node, "uri") {
            Some(uri) if !uri.is_empty() => uri,
            _ => {
                node.error = Some(
                    "MCP Server URI is required when configuring manual tool selection".into(),
                );
                cleanup_tool_pins(node, &HashSet::new());
                return;
            }
        };

        if let Err(error) = refresh_manual_tool_pins(node, &uri).await {
            node.error = Some(error);
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn on_update(&self, node: &mut Node, _board: Arc<Board>) {
        node.error = None;
        cleanup_tool_pins(node, &HashSet::new());
    }
}

fn read_pin_string(node: &Node, pin_name: &str) -> Option<String> {
    let pin = node.get_pin_by_name(pin_name)?;
    let raw = pin.default_value.as_ref()?;
    from_slice::<String>(raw).ok()
}

fn sanitize_pin_identifier(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            sanitized.push(ch.to_ascii_lowercase());
        } else {
            sanitized.push('_');
        }
    }
    let sanitized = sanitized.trim_matches('_').to_string();
    if sanitized.is_empty() {
        "tool".to_string()
    } else {
        sanitized
    }
}

fn cleanup_tool_pins(node: &mut Node, keep: &HashSet<String>) {
    node.pins.retain(|_, pin| {
        if pin.pin_type == PinType::Input && pin.name.starts_with(TOOL_PIN_PREFIX) {
            keep.contains(&pin.name)
        } else {
            true
        }
    });
}

fn build_register_mcp_tools_node() -> Node {
    let mut node = base_node();
    add_agent_pin(&mut node);
    add_uri_pin(&mut node);
    add_mode_pin(&mut node);
    add_agent_out_pin(&mut node);
    node
}

fn base_node() -> Node {
    let mut node = Node::new(
        "agent_register_mcp_tools",
        "Register MCP Tools",
        "Adds Model Context Protocol (MCP) server tools to an Agent",
        "AI/Agents/Builder",
    );
    node.add_icon("/flow/icons/bot-invoke.svg");
    node.set_scores(
        NodeScores::new()
            .set_privacy(5)
            .set_security(5)
            .set_performance(7)
            .set_governance(6)
            .set_reliability(7)
            .set_cost(3)
            .build(),
    );
    node
}

fn add_agent_pin(node: &mut Node) {
    node.set_version(2);
    node.add_input_pin(
        "agent_in",
        "Agent",
        "Agent object to add MCP tools to",
        VariableType::Struct,
    )
    .set_schema::<Agent>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_uri_pin(node: &mut Node) {
    node.add_input_pin(
        "uri",
        "MCP Server URI",
        "URI of the MCP server to connect to",
        VariableType::String,
    );
}

fn add_mode_pin(node: &mut Node) {
    node.add_input_pin(
        "mode",
        "Mode",
        "How to select MCP tools (Automatic = all tools, Manual = pick specific tools)",
        VariableType::String,
    )
    .set_options(
        PinOptions::new()
            .set_valid_values(vec!["Automatic".to_string(), "Manual".to_string()])
            .build(),
    )
    .set_default_value(Some(json::json!("Automatic")));
}

fn add_agent_out_pin(node: &mut Node) {
    node.add_output_pin(
        "agent_out",
        "Agent",
        "Agent object with registered MCP tools",
        VariableType::Struct,
    )
    .set_schema::<Agent>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn is_manual_mode(value: &str) -> bool {
    value.eq_ignore_ascii_case("Manual")
}

#[cfg(feature = "execute")]
async fn collect_manual_tool_selection(context: &mut ExecutionContext) -> HashSet<String> {
    let pin_info = {
        let node_guard = context.node.node.lock().await;
        node_guard
            .pins
            .values()
            .filter(|pin| pin.pin_type == PinType::Input && pin.name.starts_with(TOOL_PIN_PREFIX))
            .map(|pin| (pin.name.clone(), pin.friendly_name.clone()))
            .collect::<Vec<_>>()
    };

    let mut selected = HashSet::new();
    for (pin_name, tool_name) in pin_info {
        if let Ok(include) = context.evaluate_pin::<bool>(&pin_name).await
            && include
        {
            selected.insert(tool_name);
        }
    }

    selected
}

#[cfg(feature = "execute")]
async fn refresh_manual_tool_pins(node: &mut Node, uri: &str) -> Result<(), String> {
    match list_all_tools(uri).await {
        Ok(tools) if tools.is_empty() => {
            cleanup_tool_pins(node, &HashSet::new());
            Err("The MCP server reported no available tools".into())
        }
        Ok(tools) => {
            let keep = apply_tool_pins(node, tools);
            cleanup_tool_pins(node, &keep);
            Ok(())
        }
        Err(error) => {
            cleanup_tool_pins(node, &HashSet::new());
            Err(error)
        }
    }
}

#[cfg(feature = "execute")]
async fn list_all_tools(uri: &str) -> Result<Vec<Tool>, String> {
    let client_info = ClientInfo {
        protocol_version: Default::default(),
        capabilities: ClientCapabilities::default(),
        client_info: Implementation {
            name: "Flow-Like".to_string(),
            version: "alpha".to_string(),
            title: None,
            icons: None,
            website_url: Some("https://flow-like.com".to_string()),
        },
    };

    let transport = StreamableHttpClientTransport::from_uri(uri);
    let client = client_info
        .serve(transport)
        .await
        .map_err(|error| format!("Failed to connect to MCP server: {}", error))?;

    let mut tools = Vec::new();
    let mut cursor: Option<PaginatedRequestParam> = None;

    loop {
        let response = client
            .list_tools(cursor.clone())
            .await
            .map_err(|error| format!("Failed to fetch MCP tools: {}", error))?;

        tools.extend(response.tools);

        if let Some(next_cursor) = response.next_cursor {
            cursor = Some(PaginatedRequestParam {
                cursor: Some(next_cursor),
            });
        } else {
            break;
        }
    }

    Ok(tools)
}

#[cfg(feature = "execute")]
fn apply_tool_pins(node: &mut Node, tools: Vec<Tool>) -> HashSet<String> {
    let mut keep = HashSet::with_capacity(tools.len());

    for (index, tool) in tools.into_iter().enumerate() {
        let tool_name = tool.name.to_string();
        let pin_name = format!(
            "{}{}_{}",
            TOOL_PIN_PREFIX,
            index,
            sanitize_pin_identifier(&tool_name)
        );
        keep.insert(pin_name.clone());

        let description = tool
            .description
            .map(|desc| desc.into_owned())
            .unwrap_or_else(|| format!("Include the `{}` tool during MCP registration", tool_name));

        let pin = if let Some(existing) = node.get_pin_mut_by_name(&pin_name) {
            existing
        } else {
            node.add_input_pin(&pin_name, &tool_name, &description, VariableType::Boolean)
        };

        pin.friendly_name = tool_name.clone();
        pin.description = description;
        pin.data_type = VariableType::Boolean;
        if pin.default_value.is_none() {
            pin.set_default_value(Some(json::json!(true)));
        }
        pin.reset_schema();
        pin.options = None;
    }

    keep
}
