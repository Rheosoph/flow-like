use crate::generative::agent::Agent;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
/// # Stream Invoke Agent Node
/// Executes an Agent with streaming support, emitting chunks as they are generated.
/// Similar to simple agent's streaming behavior but using the built Agent object.
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{
    history::History,
    response::{LLMUsageStats, Response},
    response_chunk::ResponseChunk,
};
use flow_like_types::{async_trait, json};
#[cfg(feature = "execute")]
use std::collections::HashMap;

#[cfg(feature = "execute")]
use super::helpers::{AgentStreamState, StreamHandler, execute_agent_streaming};

#[crate::register_node]
#[derive(Default)]
pub struct StreamInvokeAgentNode {}

impl StreamInvokeAgentNode {
    pub fn new() -> Self {
        StreamInvokeAgentNode {}
    }
}

#[async_trait]
impl NodeLogic for StreamInvokeAgentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_stream_invoke",
            "Stream Invoke Agent",
            "Executes an Agent with streaming, emitting chunks in real-time",
            "AI/Agents",
        );
        node.add_icon("/flow/icons/bot-invoke.svg");
        node.set_version(4);

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
            "on_stream",
            "On Stream",
            "Triggers whenever the agent streams a chunk",
            VariableType::Execution,
        );

        node.add_output_pin(
            "chunk",
            "Chunk",
            "Latest streamed chunk from agent response",
            VariableType::Struct,
        )
        .set_schema::<ResponseChunk>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_done",
            "Done",
            "Fires when agent completes execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response",
            "Response",
            "Final complete agent response",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "history_out",
            "History Out",
            "Updated conversation history with all agent turns",
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
        context.deactivate_exec_pin("exec_done").await?;

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

        let stream_state = AgentStreamState::new(context).await?;

        let run_result =
            execute_agent_streaming(context, &agent, history, tool_name_to_node, &stream_state)
                .await;

        let finalize_result = stream_state.finalize(context).await;

        if let Err(finalize_err) = finalize_result
            && run_result.is_ok()
        {
            return Err(finalize_err);
        }

        let result = run_result?;

        context
            .set_pin_value("response", json::json!(result.response))
            .await?;
        context
            .set_pin_value("history_out", json::json!(result.history))
            .await?;
        context
            .set_pin_value("stats", json::json!(result.stats))
            .await?;

        context.activate_exec_pin("exec_done").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}
