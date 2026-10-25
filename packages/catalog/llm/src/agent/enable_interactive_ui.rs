use crate::generative::agent::Agent;
/// # Enable Interactive UI Node
/// Lets an Agent answer with inline interactive UI (`a2ui` fences rendered by the chat).
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json};
#[crate::register_node]
#[derive(Default)]
pub struct EnableInteractiveUiNode {}

impl EnableInteractiveUiNode {
    pub fn new() -> Self {
        EnableInteractiveUiNode {}
    }
}

#[async_trait]
impl NodeLogic for EnableInteractiveUiNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "agent_enable_interactive_ui",
            "Enable Interactive UI",
            "Lets the agent answer with interactive UI: a reply can contain an `a2ui` code block that the chat renders inline as cards, forms, tables or charts, and its buttons can send a follow-up message on the user's behalf",
            "AI/Agents/Builder",
        );
        node.set_flowscript_name("agent", "enableInteractiveUi");
        node.set_receiver("agent_in");
        node.set_version(1);
        node.add_icon("/flow/icons/a2ui.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(9)
                .set_performance(8)
                .set_governance(9)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node.add_input_pin(
            "agent_in",
            "Agent",
            "Agent object to enable interactive UI replies on",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "agent_out",
            "Agent",
            "Agent object whose replies may contain inline interactive UI",
            VariableType::Struct,
        )
        .set_schema::<Agent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let mut agent: Agent = context.evaluate_pin("agent_in").await?;

        agent.enable_generative_ui();

        context
            .set_pin_value("agent_out", json::json!(agent))
            .await?;

        Ok(())
    }
}
