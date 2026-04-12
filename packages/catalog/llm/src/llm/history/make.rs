use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::history::History;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct MakeHistoryNode {}

impl MakeHistoryNode {
    pub fn new() -> Self {
        MakeHistoryNode {}
    }
}

#[async_trait]
impl NodeLogic for MakeHistoryNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_make_history",
            "Make History",
            "Creates a ChatHistory struct",
            "AI/Generative/History",
        );
        node.add_icon("/flow/icons/history.svg");
        node.set_version(1);

        // Pure helper to allocate an empty History; fully local and cheap.
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_governance(9)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );
        node.add_input_pin(
            "model_name",
            "Model Name",
            "Model Name",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("history", "History", "ChatHistory", VariableType::Struct)
            .set_schema::<History>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let model_name: String = context.evaluate_pin("model_name").await?;
        let history = History::new(model_name, vec![]);

        context.set_pin_value("history", json!(history)).await?;

        Ok(())
    }
}
