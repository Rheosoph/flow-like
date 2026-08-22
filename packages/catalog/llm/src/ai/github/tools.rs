//! Copilot Tool Configuration Nodes
//!
//! Nodes for configuring agent tools.

use super::CopilotToolConfig;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json};

#[crate::register_node]
#[derive(Default)]
pub struct CopilotToolConfigNode {}

#[async_trait]
impl NodeLogic for CopilotToolConfigNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_tool_config",
            "Tool Config",
            "Configures an agent tool with parameters",
            "AI/GitHub/Copilot/Tools",
        );
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(9)
                .set_security(8)
                .set_performance(10)
                .set_governance(9)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node.add_input_pin("name", "Name", "Tool name", VariableType::String);

        node.add_input_pin(
            "description",
            "Description",
            "Tool description",
            VariableType::String,
        );

        node.add_input_pin(
            "schema",
            "Schema",
            "Tool parameters JSON schema",
            VariableType::Struct,
        )
        .set_default_value(Some(json::json!({})))
        .set_open_schema();

        node.add_output_pin("tool", "Tool", "Configured tool", VariableType::Struct)
            .set_schema::<CopilotToolConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let name: String = context.evaluate_pin("name").await?;
        let description: String = context.evaluate_pin("description").await?;
        let schema: flow_like_types::Value = context
            .evaluate_pin("schema")
            .await
            .unwrap_or(json::json!({}));

        let tool = CopilotToolConfig {
            name,
            description,
            schema,
        };

        context.set_pin_value("tool", json::json!(tool)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CopilotToolListNode {}

#[async_trait]
impl NodeLogic for CopilotToolListNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "copilot_tool_list",
            "Tool List Builder",
            "Combines multiple tools into a list for session configuration",
            "AI/GitHub/Copilot/Tools",
        );
        node.add_icon("/flow/icons/github.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(10)
                .set_governance(10)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );

        for i in 1..=8 {
            node.add_input_pin(
                &format!("tool_{}", i),
                &format!("Tool {}", i),
                "Optional tool configuration",
                VariableType::Struct,
            )
            .set_schema::<CopilotToolConfig>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        }

        node.add_output_pin(
            "tools",
            "Tools",
            "List of configured tools",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<CopilotToolConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut tools: Vec<CopilotToolConfig> = Vec::new();

        for i in 1..=8 {
            if let Ok(tool) = context
                .evaluate_pin::<CopilotToolConfig>(&format!("tool_{}", i))
                .await
            {
                tools.push(tool);
            }
        }

        context.set_pin_value("tools", json::json!(tools)).await?;
        Ok(())
    }
}
