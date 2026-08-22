use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::json::json;
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct PushGlobalSessionNode {}

impl PushGlobalSessionNode {
    pub fn new() -> Self {
        PushGlobalSessionNode {}
    }
}

#[async_trait]
impl NodeLogic for PushGlobalSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_global_session",
            "Push Global Session",
            "Pushes a new global session to the chat. The session persists for all chat sessions.",
            "Events/Chat",
        );
        node.add_icon("/flow/icons/paperclip.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "global_session",
            "Global Session",
            "Generic Struct Type",
            VariableType::Struct,
        )
        .set_default_value(Some(json!({})))
        .set_open_schema();

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

        let session: Value = context
            .evaluate_pin("global_session")
            .await
            .unwrap_or(json!({}));

        context
            .stream_response("chat_global_session", session)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        return Ok(());
    }
}
