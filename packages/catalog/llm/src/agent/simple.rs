use crate::generative::agent::{Agent, ContextManagementMode};
/// # Simple Agent Node
/// This is an LLM-controlled while loop over an arbitrary number of flow-leafes with back-propagation of leaf outputs into the agent.
/// Uses Rig's agent system with dynamic tools for executing Flow-Like subcontexts.
/// Recursive agent calls until no more tool calls are made or recursion limit hit.
/// Effectively, this node allows the LLM to control it's own execution until further human input required.
use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
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
pub struct SimpleAgentNode {}

impl SimpleAgentNode {
    pub fn new() -> Self {
        SimpleAgentNode {}
    }

    #[cfg(feature = "execute")]
    async fn run_internal(
        &self,
        context: &mut ExecutionContext,
        stream_state: &AgentStreamState,
    ) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_done").await?;

        let max_iterations: u64 = context.evaluate_pin("max_iter").await?;
        let model_bit = context.evaluate_pin::<Bit>("model").await?;
        let history = context.evaluate_pin::<History>("history").await?;
        let infinite_context: bool = context.evaluate_pin("infinite_context").await?;
        let max_context_tokens: u64 = context.evaluate_pin("max_context_tokens").await?;
        let context_mode: String = context.evaluate_pin("context_mode").await?;

        // Parse context management mode
        let mode = match context_mode.to_lowercase().as_str() {
            "summarize" | "summary" => ContextManagementMode::Summarize,
            _ => ContextManagementMode::Truncate,
        };

        // Get referenced functions and store as function refs
        let referenced_functions = context.get_referenced_functions().await?;

        let mut tool_name_to_node = HashMap::with_capacity(referenced_functions.len());
        let mut agent = Agent::new(model_bit.clone(), max_iterations);

        // Set model display name from bit metadata
        if let Some(meta) = model_bit.meta.get("en") {
            agent.model_display_name = Some(meta.name.clone());
        }

        // Enable infinite context mode if requested
        if infinite_context {
            agent.enable_infinite_context(Some(max_context_tokens as u32));
            agent.set_context_management_mode(mode);
        }

        // Store function references - tools will be generated at execution time
        for referenced_node in referenced_functions {
            let node_guard = referenced_node.node.lock().await;
            let node_id = node_guard.id.clone();
            let node_name = node_guard.name.clone();
            let friendly_tool_name = node_guard
                .friendly_name
                .to_lowercase()
                .replace([' ', '-'], "_");
            drop(node_guard);

            agent.add_function_ref(node_id, node_name.clone());

            // Allow calling tools by both internal name and friendly snake_case variant.
            tool_name_to_node.insert(node_name, referenced_node.clone());
            tool_name_to_node.insert(friendly_tool_name, referenced_node);
        }

        let result =
            execute_agent_streaming(context, &agent, history, tool_name_to_node, stream_state)
                .await?;

        context
            .set_pin_value("history_out", json::json!(result.history))
            .await?;

        context
            .set_pin_value("response", json::json!(result.response))
            .await?;

        context
            .set_pin_value("stats", json::json!(result.stats))
            .await?;

        context.activate_exec_pin("exec_done").await?;

        Ok(())
    }
}

#[async_trait]
impl NodeLogic for SimpleAgentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "simple_agent",
            "Simple Agent",
            "LLM-driven control loop that repeatedly calls referenced Flow functions as tools until it decides to stop",
            "AI/Agents",
        );
        node.add_icon("/flow/icons/for-each.svg");
        node.set_can_reference_fns(true);
        node.set_version(5);

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
            "Execution trigger for starting the agent loop",
            VariableType::Execution,
        );

        node.add_input_pin(
            "model",
            "Model",
            "Bit describing the LLM that powers the agent",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "history",
            "History",
            "Conversation history shared with the agent (used for reasoning context)",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "max_iter",
            "Iter",
            "Maximum number of internal iterations/tool calls before failing",
            VariableType::Integer,
        )
        .set_default_value(Some(json::json!(15)));

        node.add_input_pin(
            "infinite_context",
            "Infinite Context",
            "Enable automatic context window management to prevent overflow",
            VariableType::Boolean,
        )
        .set_default_value(Some(json::json!(false)));

        node.add_input_pin(
            "max_context_tokens",
            "Max Tokens",
            "Maximum tokens to retain when truncating (default: 32000)",
            VariableType::Integer,
        )
        .set_default_value(Some(json::json!(32000)));

        node.add_input_pin(
            "context_mode",
            "Context Mode",
            "How to handle context overflow: 'truncate' (drop old messages) or 'summarize' (LLM compresses history)",
            VariableType::String,
        )
        .set_default_value(Some(json::json!("truncate")));

        node.add_output_pin(
            "on_stream",
            "On Stream",
            "Triggers whenever the agent streams its final response",
            VariableType::Execution,
        );

        node.add_output_pin(
            "chunk",
            "Chunk",
            "Latest streamed agent chunk (final response)",
            VariableType::Struct,
        )
        .set_schema::<ResponseChunk>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_done",
            "Done",
            "Fires when the agent stops (successfully or due to error)",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response",
            "Response",
            "Final assistant response produced when the agent halts",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "history_out",
            "History Out",
            "Conversation history enriched with all agent/tool turns",
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
        let stream_state = AgentStreamState::new(context).await?;
        let run_result = self.run_internal(context, &stream_state).await;
        let finalize_result = stream_state.finalize(context).await;

        if let Err(finalize_err) = finalize_result
            && run_result.is_ok()
        {
            return Err(finalize_err);
        }

        run_result
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "LLM processing requires the 'execute' feature"
        ))
    }
}
