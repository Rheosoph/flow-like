/// # Agent from Model Node
/// Creates an Agent object from a model Bit with basic configuration.
/// This is the starting point for building an agent in the flow.
use crate::generative::agent::{Agent, ContextManagementMode};
use flow_like::{
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json};

const DEFAULT_MAX_CONTEXT_TOKENS: u32 = 32000;

#[crate::register_node]
#[derive(Default)]
pub struct AgentFromModelNode {}

impl AgentFromModelNode {
    pub fn new() -> Self {
        AgentFromModelNode {}
    }
}

#[async_trait]
impl NodeLogic for AgentFromModelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_from_model",
            "Agent from Model",
            "Creates an Agent object from a model Bit with configuration",
            "AI/Agents/Builder",
        );
        node.set_version(5);
        node.add_icon("/flow/icons/bot-invoke.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(5)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(7)
                .set_cost(2)
                .build(),
        );

        node.add_input_pin(
            "model",
            "Model",
            "LLM model Bit that will power the agent",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "max_iter",
            "Max Iterations",
            "Maximum number of tool call iterations before stopping",
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
            "context_mode",
            "Context Mode",
            "Strategy: 'truncate' (fast, drops old messages) or 'summarize' (LLM compresses history, slower but preserves info)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new().set_valid_values(vec!["summarize".into(), "truncate".into()]).build()
        )
        .set_default_value(Some(json::json!("summarize")));

        node.add_input_pin(
            "max_context_tokens",
            "Max Context Tokens",
            "Maximum tokens to retain in context window (default: 32000)",
            VariableType::Integer,
        )
        .set_default_value(Some(json::json!(DEFAULT_MAX_CONTEXT_TOKENS)));

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Configured Agent object ready for tool registration and execution",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let model: Bit = context.evaluate_pin("model").await?;
        let max_iter: u64 = context.evaluate_pin("max_iter").await?;
        let infinite_context: bool = context
            .evaluate_pin("infinite_context")
            .await
            .unwrap_or(true);
        let context_mode: String = context
            .evaluate_pin("context_mode")
            .await
            .unwrap_or_else(|_| "summarize".to_string());
        let max_context_tokens: u64 = context
            .evaluate_pin("max_context_tokens")
            .await
            .unwrap_or(DEFAULT_MAX_CONTEXT_TOKENS as u64);

        let mut agent = Agent::new(model.clone(), max_iter);

        // Store model display name
        if let Some(meta) = model.meta.get("en") {
            agent.model_display_name = Some(meta.name.clone());
        }

        if infinite_context {
            agent.enable_infinite_context(Some(max_context_tokens as u32));

            let mode = match context_mode.to_lowercase().as_str() {
                "summarize" | "summary" => ContextManagementMode::Summarize,
                _ => ContextManagementMode::Truncate,
            };
            agent.set_context_management_mode(mode);
        }

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }
}
