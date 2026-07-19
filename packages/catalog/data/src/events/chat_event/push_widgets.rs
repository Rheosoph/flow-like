use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::{CachedChatResponse, ChatStreamingResponse, ChatWidget};

#[crate::register_node]
#[derive(Default)]
pub struct PushWidgetsNode {}

impl PushWidgetsNode {
    pub fn new() -> Self {
        PushWidgetsNode {}
    }
}

#[async_trait]
impl NodeLogic for PushWidgetsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_push_widgets",
            "Push Widgets",
            "Embeds multiple a2ui widget instances into the chat message. Add an Element Ref pin for each Instantiate Widget node.",
            "Events/Chat",
        );
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
            "Widget instance to embed (from Instantiate Widget). Add more pins for multiple widgets.",
            VariableType::Struct,
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

        let pins = context.get_pins_by_name("element_ref").await?;
        let (update_log, truncated) = context.get_a2ui_update_log().await;
        if truncated {
            context.log_message(
                "The a2ui update log hit its cap; element updates streamed before this push may be missing from the widgets.",
                flow_like::flow::execution::LogLevel::Warn,
            );
        }
        let mut widgets = Vec::with_capacity(pins.len());
        for pin in pins {
            let value: Value = context.evaluate_pin_ref(pin).await?;
            if value.is_null() {
                continue;
            }
            let mut widget = ChatWidget::from_element_ref(&value)?;
            widget.attach_update_log(&update_log);
            widgets.push(widget);
        }

        if widgets.is_empty() {
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let cached_response = CachedChatResponse::load(context).await?;
        {
            let mut mutable_response = cached_response.response.lock().await;
            mutable_response.widgets.extend(widgets.clone());
        }

        let streaming_response = ChatStreamingResponse {
            actions: vec![],
            attachments: vec![],
            chunk: None,
            plan: None,
            widgets,
        };

        context
            .stream_response("chat_stream_partial", streaming_response)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
