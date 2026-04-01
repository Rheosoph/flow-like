use crate::generative::agent::Agent;
/// # Register Thinking Tool Node
/// Enables Rig's built-in Thinking tool on an Agent object.
/// The thinking tool allows the agent to reason about complex problems before responding.
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json};
#[crate::register_node]
#[derive(Default)]
pub struct RegisterThinkingToolNode {}

impl RegisterThinkingToolNode {
    pub fn new() -> Self {
        RegisterThinkingToolNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterThinkingToolNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_register_thinking",
            "Register Thinking Tool",
            "Enables Rig's built-in Thinking tool for reasoning capabilities",
            "AI/Agents/Builder",
        );
        node.set_version(2);
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
            "agent_in",
            "Agent",
            "Agent object to enable thinking on",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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

        agent.enable_thinking();

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }
}
