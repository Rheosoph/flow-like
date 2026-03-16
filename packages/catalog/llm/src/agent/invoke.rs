#[cfg(feature = "execute")]
use super::helpers::execute_agent;
use crate::generative::agent::Agent;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
/// # Invoke Agent Node
/// Executes an Agent with conversation history and returns the response.
/// Non-streaming version - waits for complete response before continuing.
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{history::History, response::{LLMUsageStats, Response}};
use flow_like_types::{async_trait, json};
#[cfg(feature = "execute")]
use std::collections::HashMap;

#[crate::register_node]
#[derive(Default)]
pub struct InvokeAgentNode {}

impl InvokeAgentNode {
    pub fn new() -> Self {
        InvokeAgentNode {}
    }
}

#[async_trait]
impl NodeLogic for InvokeAgentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_invoke",
            "Invoke Agent",
            "Executes an Agent with history and returns the complete response",
            "AI/Agents",
        );
        node.set_version(2);
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(6)
                .set_governance(4)
                .set_reliability(5)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Input",
            "Execution trigger",
            VariableType::Execution,
        );

        node.add_input_pin(
            "agent",
            "Agent",
            "Configured Agent object with tools",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "history",
            "History",
            "Conversation history to provide context",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when agent completes execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response",
            "Response",
            "Final agent response",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "history_out",
            "History Out",
            "Updated conversation history with agent turns",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "stats",
            "Stats",
            "Token usage, cost, and model statistics",
            VariableType::Struct,
        )
        .set_schema::<LLMUsageStats>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let agent: Agent = context.evaluate_pin("agent").await?;
        let history: History = context.evaluate_pin("history").await?;

        // Build tool name to node mapping from function_refs stored in agent
        let mut tool_name_to_node = HashMap::new();

        for node_id in agent.function_refs.keys() {
            if let Some(internal_node) = context.nodes.get(node_id) {
                let node_guard = internal_node.node.lock().await;
                let tool_name = node_guard.name.clone();
                let friendly_tool_name = node_guard
                    .friendly_name
                    .to_lowercase()
                    .replace([' ', '-'], "_");
                drop(node_guard);
                tool_name_to_node.insert(tool_name, internal_node.clone());
                tool_name_to_node.insert(friendly_tool_name, internal_node.clone());
            } else {
                context.log_message(
                    &format!("Warning: Referenced node {} not found in board", node_id),
                    LogLevel::Warn,
                );
            }
        }

        let result = execute_agent(context, &agent, history, tool_name_to_node).await?;

        context
            .set_pin_value("response", json::json!(result.response))
            .await?;
        context
            .set_pin_value("history_out", json::json!(result.history))
            .await?;
        context
            .set_pin_value("stats", json::json!(result.stats))
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}
