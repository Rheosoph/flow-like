use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

use super::ChatMainWidget;

#[crate::register_node]
#[derive(Default)]
pub struct ClearMainWidgetNode {}

impl ClearMainWidgetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClearMainWidgetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "events_chat_clear_main_widget",
            "Clear Main Widget",
            "Removes the chat's main widget and returns the space to the messages.",
            "Events/Chat",
        );
        node.set_flowscript_name("chat", "clearMainWidget");
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_event_callback(true);
        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
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
        ChatMainWidget::default().push(context).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
