/// # Chunk From String Node
/// Transform custom input strings into chunk objects that can be pushed as continuous, intermediate results to frontend.
/// Useful when intermediate steps are not LLM tokens but action steps performed by tools etc.
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response_chunk::{Delta, ResponseChunk, ResponseChunkChoice};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ChunkFromStringNode {}

impl ChunkFromStringNode {
    pub fn new() -> Self {
        ChunkFromStringNode {}
    }
}

#[async_trait]
impl NodeLogic for ChunkFromStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_llm_chunk_from_string",
            "Chunk From String",
            "Wraps an arbitrary string in a synthetic streaming chunk",
            "AI/Generative/Response",
        );
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
            "content",
            "Content",
            "Plain text that should stream to clients",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "chunk",
            "Chunk",
            "Response chunk built from the provided text",
            VariableType::Struct,
        )
        .set_schema::<ResponseChunk>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // fetch inputs
        let message: String = context.evaluate_pin("content").await?;

        // make chunk
        let mut chunk = ResponseChunk::default();
        chunk.choices.push(ResponseChunkChoice {
            finish_reason: None,
            index: 0,
            logprobs: None,
            delta: Some(Delta {
                role: Some("assistant".to_string()),
                content: Some(message),
                content_parts: None,
                tool_calls: None,
                refusal: None,
                reasoning: None,
            }),
        });

        // set outputs
        context.set_pin_value("chunk", json!(chunk)).await?;
        Ok(())
    }
}
