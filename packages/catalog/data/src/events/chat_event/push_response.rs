use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_model_provider::response::Response;
use flow_like_types::async_trait;

use super::CachedChatResponse;

#[crate::register_node]
#[derive(Default)]
pub struct PushResponseNode {}

impl PushResponseNode {
    pub fn new() -> Self {
        PushResponseNode {}
    }
}

#[async_trait]
impl NodeLogic for PushResponseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_response",
            "Push Response",
            "Pushes a response to the chat",
            "Events/Chat",
        );
        node.set_flowscript_name("chat", "pushResponse");
        node.add_icon("/flow/icons/event.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "response",
            "Response",
            "Chat Response",
            VariableType::Struct,
        )
        .set_schema::<Response>()
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

        let response: Response = context.evaluate_pin("response").await?;
        let cached_response = CachedChatResponse::load(context).await?;
        let current = {
            let mut mutable_response = cached_response.response.lock().await;
            mutable_response.response = response;
            mutable_response.clone()
        };

        context.stream_response("chat_stream", current).await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}
