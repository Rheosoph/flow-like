use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::{ChatMainWidget, ChatMainWidgetDisplay, ChatWidget};

#[crate::register_node]
#[derive(Default)]
pub struct SetMainWidgetNode {}

impl SetMainWidgetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SetMainWidgetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_set_main_widget",
            "Set Main Widget",
            "Shows or replaces the chat's main widget beside the messages. Connect an Instantiate Widget reference; element nodes can update the displayed widget.",
            "Events/Chat",
        );
        node.set_flowscript_name("chat", "setMainWidget");
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
            "Widget instance to display (from Instantiate Widget)",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::ElementRef>();
        node.add_input_pin(
            "title",
            "Title",
            "Title shown above the main widget",
            VariableType::String,
        )
        .set_default_value(Some(json!("Main widget")));
        node.add_input_pin(
            "display",
            "Display",
            "Open beside the chat, collapse the pane, or maximize it",
            VariableType::String,
        )
        .set_default_value(Some(json!("open")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "open".to_string(),
                    "collapsed".to_string(),
                    "maximized".to_string(),
                ])
                .build(),
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
        let element_ref: Value = context.evaluate_pin("element_ref").await?;
        let title: String = context.evaluate_pin("title").await?;
        let display: ChatMainWidgetDisplay = context.evaluate_pin("display").await?;
        let mut widget = ChatWidget::from_element_ref(&element_ref)?;
        let (update_log, truncated) = context.get_a2ui_update_log().await;
        if truncated {
            context.log_message(
                "The a2ui update log hit its cap; earlier element updates may be missing from the main widget.",
                flow_like::flow::execution::LogLevel::Warn,
            );
        }
        widget.attach_update_log(&update_log);

        ChatMainWidget {
            command_id: None,
            widget: Some(widget),
            title,
            display: Some(display),
        }
        .push(context)
        .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
