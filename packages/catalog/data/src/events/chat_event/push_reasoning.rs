use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

use super::{CachedChatResponse, ChatStreamingResponse};

#[crate::register_node]
#[derive(Default)]
pub struct PushReasoningNode {}

impl PushReasoningNode {
    pub fn new() -> Self {
        PushReasoningNode {}
    }
}

#[async_trait]
impl NodeLogic for PushReasoningNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_reasoning",
            "Push Reasoning",
            "Pushes reasoning tokens to the current step",
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
            "reasoning",
            "Reasoning",
            "Reasoning text to append to current step",
            VariableType::String,
        );

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
        let reasoning: String = context.evaluate_pin("reasoning").await?;

        let cached_response = CachedChatResponse::load(context).await?;
        {
            let mut mutable_reasoning = cached_response.reasoning.lock().await;
            if !mutable_reasoning.current_message.is_empty() {
                mutable_reasoning.current_message.push('\n');
            }
            mutable_reasoning.current_message.push_str(&reasoning);
        }

        let reasoning_ref = cached_response.reasoning.lock().await;
        let streaming_response = ChatStreamingResponse {
            actions: vec![],
            attachments: vec![],
            chunk: None,
            plan: Some(reasoning_ref.clone()),
            widgets: vec![],
        };
        drop(reasoning_ref);

        context
            .stream_response("chat_stream_partial", streaming_response)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}
