use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::{CachedChatResponse, ChatStreamingResponse, ChatWidget};

#[crate::register_node]
#[derive(Default)]
pub struct PushWidgetNode {}

impl PushWidgetNode {
    pub fn new() -> Self {
        PushWidgetNode {}
    }
}

#[async_trait]
impl NodeLogic for PushWidgetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_widget",
            "Push Widget",
            "Embeds an a2ui widget instance into the chat message. Connect the Element Ref of an Instantiate Widget node.",
            "Events/Chat",
        );
        node.set_flowscript_name("chat", "pushWidget");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_event_callback(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "element_ref",
            "Widget",
            "Widget instance to embed (from Instantiate Widget)",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::ElementRef>();

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

        let element_ref: Value = context.evaluate_pin("element_ref").await?;
        let mut widget = ChatWidget::from_element_ref(&element_ref)?;
        let (update_log, truncated) = context.get_a2ui_update_log().await;
        if truncated {
            context.log_message(
                "The a2ui update log hit its cap; element updates streamed before this push may be missing from the widget.",
                flow_like::flow::execution::LogLevel::Warn,
            );
        }
        widget.attach_update_log(&update_log);

        let cached_response = CachedChatResponse::load(context).await?;
        {
            let mut mutable_response = cached_response.response.lock().await;
            mutable_response.widgets.push(widget.clone());
        }

        let streaming_response = ChatStreamingResponse {
            actions: vec![],
            attachments: vec![],
            chunk: None,
            plan: None,
            widgets: vec![widget],
        };

        context
            .stream_response("chat_stream_partial", streaming_response)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
