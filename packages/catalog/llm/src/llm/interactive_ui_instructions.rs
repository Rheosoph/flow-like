use crate::generative::agent::interactive_ui_instructions;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct InteractiveUiInstructionsNode {}

impl InteractiveUiInstructionsNode {
    pub fn new() -> Self {
        InteractiveUiInstructionsNode {}
    }
}

#[async_trait]
impl NodeLogic for InteractiveUiInstructionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_interactive_ui_instructions",
            "Interactive UI Instructions",
            "System prompt section that teaches a model to answer with interactive UI: an `a2ui` code block in its reply renders inline in the chat, and its buttons can send a follow-up message on the user's behalf. Append it to your own system prompt (Invoke Simple, Set System Message); agents use Enable Interactive UI instead",
            "AI/Generative",
        );
        node.set_flowscript_name("ai", "interactiveUiInstructions");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_version(1);
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(9)
                .set_performance(9)
                .set_reliability(10)
                .set_governance(9)
                .set_cost(8)
                .build(),
        );

        node.add_output_pin(
            "instructions",
            "Instructions",
            "Markdown prompt section with the `a2ui` fence rules, an example and the component reference",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context
            .set_pin_value("instructions", json!(interactive_ui_instructions()))
            .await?;
        Ok(())
    }
}
