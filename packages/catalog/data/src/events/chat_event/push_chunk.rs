use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response_chunk::ResponseChunk;
use flow_like_types::async_trait;

use super::{CachedChatResponse, ChatStreamingResponse};

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
            "events_chat_push_response_chunk",
            "Push Chunk",
            "Pushes a response chunk to the chat",
            "Events/Chat",
        );
        node.add_icon("/flow/icons/event.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "chunk",
            "Chunk",
            "Generated Chat Chunk",
            VariableType::Struct,
        )
        .set_schema::<ResponseChunk>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let chunk: ResponseChunk = context.evaluate_pin("chunk").await?;
        let cached_response = CachedChatResponse::load(context).await?;
        {
            let mut mutable_response = cached_response.response.lock().await;
            mutable_response.response.push_chunk(chunk.clone());
        }

        // Extract reasoning from chunk delta and update the reasoning plan
        let plan = if let Some(reasoning) = chunk
            .choices
            .first()
            .and_then(|c| c.delta.as_ref())
            .and_then(|d| d.reasoning.as_ref())
        {
            let mut mutable_reasoning = cached_response.reasoning.lock().await;
            // Initialize a default "Thinking" step if no plan exists yet
            if mutable_reasoning.plan.is_empty() {
                mutable_reasoning.plan.push((0, "Thinking".to_string()));
            }
            // Append reasoning to current message
            if !mutable_reasoning.current_message.is_empty() {
                mutable_reasoning.current_message.push('\n');
            }
            mutable_reasoning.current_message.push_str(reasoning);
            Some(mutable_reasoning.clone())
        } else {
            None
        };

        let streaming_response = ChatStreamingResponse {
            actions: vec![],
            attachments: vec![],
            chunk: Some(chunk),
            plan,
            widgets: vec![],
        };

        context
            .stream_response("chat_stream_partial", streaming_response)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}
