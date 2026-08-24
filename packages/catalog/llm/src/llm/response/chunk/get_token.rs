use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response_chunk::ResponseChunk;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct GetTokenNode {}

impl GetTokenNode {
    pub fn new() -> Self {
        GetTokenNode {}
    }
}

#[async_trait]
impl NodeLogic for GetTokenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_llm_response_chunk_get_token",
            "Get Token",
            "Extracts the latest streamed token from a response chunk",
            "AI/Generative/Response/Chunk",
        );
        node.set_flowscript_name("ai.response", "getToken");
        node.set_receiver("chunk");
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

        node.add_input_pin(
            "chunk",
            "Chunk",
            "Response chunk that carries streamed tokens",
            VariableType::Struct,
        )
        .set_schema::<ResponseChunk>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "token",
            "Token",
            "Most recent streamed token",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let chunk: ResponseChunk = context.evaluate_pin("chunk").await?;

        let token = chunk.get_streamed_token().unwrap_or_default();
        context.set_pin_value("token", json!(token)).await?;

        Ok(())
    }
}
