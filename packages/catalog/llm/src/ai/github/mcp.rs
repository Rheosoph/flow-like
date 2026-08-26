//! Copilot MCP Server Configuration Nodes
//!
//! Nodes for configuring MCP servers.

use super::{McpHttpServerConfig, McpLocalServerConfig};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json};
use std::collections::HashMap;

#[crate::register_node]
#[derive(Default)]
pub struct CopilotMcpLocalServerNode {}

#[async_trait]
impl NodeLogic for CopilotMcpLocalServerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_mcp_local_server",
            "MCP Local Server",
            "Configures a local/stdio MCP server for tool integration",
            "AI/GitHub/Copilot/MCP",
        );
        node.set_flowscript_name("github.copilot", "mcpLocalServer");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(7)
                .set_performance(9)
                .set_governance(8)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin(
            "command",
            "Command",
            "Command to execute (e.g., npx, python)",
            VariableType::String,
        );

        node.add_input_pin(
            "args",
            "Arguments",
            "Command arguments",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json::json!([])));

        node.add_input_pin(
            "tools",
            "Tools",
            "Tool filter (use [\"*\"] for all tools)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json::json!(["*"])));

        node.add_input_pin(
            "timeout",
            "Timeout",
            "Server timeout in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json::json!(30000)));

        node.add_output_pin(
            "config",
            "Config",
            "MCP server configuration",
            VariableType::Struct,
        )
        .set_schema::<McpLocalServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let command: String = context.evaluate_pin("command").await?;
        let args: Vec<String> = context.evaluate_pin("args").await.unwrap_or_default();
        let tools: Vec<String> = context
            .evaluate_pin("tools")
            .await
            .unwrap_or_else(|_| vec!["*".to_string()]);
        let timeout: i64 = context.evaluate_pin("timeout").await.unwrap_or(30000);

        let config = McpLocalServerConfig {
            server_type: "local".to_string(),
            command,
            args,
            env: HashMap::new(),
            cwd: None,
            tools,
            timeout: Some(timeout as i32),
        };

        context.set_pin_value("config", json::json!(config)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotMcpHttpServerNode {}

#[async_trait]
impl NodeLogic for CopilotMcpHttpServerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_mcp_http_server",
            "MCP HTTP Server",
            "Configures an HTTP/SSE MCP server for remote tool integration",
            "AI/GitHub/Copilot/MCP",
        );
        node.set_flowscript_name("github.copilot", "mcpHttpServer");
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );

        node.add_input_pin("url", "URL", "HTTP endpoint URL", VariableType::String);

        node.add_input_pin(
            "tools",
            "Tools",
            "Tool filter (use [\"*\"] for all tools)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json::json!(["*"])));

        node.add_input_pin(
            "timeout",
            "Timeout",
            "Server timeout in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json::json!(30000)));

        node.add_output_pin(
            "config",
            "Config",
            "MCP server configuration",
            VariableType::Struct,
        )
        .set_schema::<McpHttpServerConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let url: String = context.evaluate_pin("url").await?;
        let tools: Vec<String> = context
            .evaluate_pin("tools")
            .await
            .unwrap_or_else(|_| vec!["*".to_string()]);
        let timeout: i64 = context.evaluate_pin("timeout").await.unwrap_or(30000);

        let config = McpHttpServerConfig {
            server_type: "http".to_string(),
            url,
            headers: HashMap::new(),
            tools,
            timeout: Some(timeout as i32),
        };

        context.set_pin_value("config", json::json!(config)).await?;
        Ok(())
    }
}
