/// # Register Thinking Tool Node
/// Enables Rig's built-in Thinking tool on an Agent object.
/// The thinking tool allows the agent to reason about complex problems before responding.
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    json::{self, json},
};

use crate::generative::agent::Agent;
#[crate::register_node]
#[derive(Default)]
pub struct SetAgentSystemPromptNode {}

impl SetAgentSystemPromptNode {
    pub fn new() -> Self {
        SetAgentSystemPromptNode {}
    }
}

#[async_trait]
impl NodeLogic for SetAgentSystemPromptNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_set_system_prompt",
            "Set Agent System Prompt",
            "Sets the system prompt for an Agent to guide its behavior",
            "AI/Agents/Builder",
        );
        node.set_flowscript_name("agent", "setSystemPrompt");
        node.set_receiver("agent_in");
        node.set_version(1);
        node.add_icon("/flow/icons/bot-invoke.svg");

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

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent object to enable thinking on",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "system_prompt",
            "System Prompt",
            "System prompt string to set for the agent",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent object with thinking tool enabled",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut agent: Agent = context.evaluate_pin("agent_in").await?;
        let system_prompt: String = context.evaluate_pin("system_prompt").await?;

        agent.set_system_prompt(system_prompt);

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }
}
