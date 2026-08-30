use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::{response::Response, response_chunk::ResponseChunk};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PushChunkNode {}

impl PushChunkNode {
    pub fn new() -> Self {
        PushChunkNode {}
    }
}

#[async_trait]
impl NodeLogic for PushChunkNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_generative_llm_response_push_chunk",
            "Push Chunk",
            "Appends a streaming chunk onto a response",
            "AI/Generative/Response",
        );
        node.set_flowscript_name("ai.response", "pushChunk");
        node.set_receiver("response");
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
            "exec_in",
            "Input",
            "Start execution before appending",
            VariableType::Execution,
        );

        node.add_input_pin(
            "response",
            "Response",
            "Response object that should receive the chunk",
            VariableType::Struct,
        )
        .set_schema::<Response>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("chunk", "Chunk", "Chunk to append", VariableType::Struct)
            .set_schema::<ResponseChunk>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Signals completion once appended",
            VariableType::Execution,
        );

        node.add_output_pin(
            "response_out",
            "Response",
            "Response including the appended chunk",
            VariableType::Struct,
        )
        .set_schema::<Response>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut response: Response = context.evaluate_pin("response").await?;
        let chunk: ResponseChunk = context.evaluate_pin("chunk").await?;

        response.push_chunk(chunk);

        context
            .set_pin_value("response_out", json!(response))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
