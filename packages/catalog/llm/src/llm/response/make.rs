use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response::Response;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct MakeResponseNode {}

impl MakeResponseNode {
    pub fn new() -> Self {
        MakeResponseNode {}
    }
}

#[async_trait]
impl NodeLogic for MakeResponseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_llm_response_make",
            "Make Response",
            "Creates an empty Response struct for manual composition",
            "AI/Generative/Response",
        );
        node.set_flowscript_name("ai.response", "make");
        node.add_icon("/flow/icons/history.svg");
        node.set_scores(
            NodeScores::new()
                .set_privacy(10)
                .set_security(10)
                .set_performance(9)
                .set_reliability(10)
                .set_governance(9)
                .set_cost(10)
                .build(),
        );

        node.add_output_pin(
            "response",
            "Response",
            "Empty Response ready to populate",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let response = Response::new();

        context.set_pin_value("response", json!(response)).await?;

        Ok(())
    }
}
