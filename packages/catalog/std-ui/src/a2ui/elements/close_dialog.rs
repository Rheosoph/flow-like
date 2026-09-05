use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct CloseDialog;

impl CloseDialog {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for CloseDialog {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_close_dialog",
            "Close Dialog",
            "Closes an open dialog. If no dialog ID is specified, closes the topmost dialog.",
            "UI/Navigation",
        );
        node.set_flowscript_name("ui", "closeDialog");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "dialog_id",
            "Dialog ID",
            "Optional ID of the specific dialog to close. If empty, closes the topmost dialog.",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let dialog_id: Option<String> = context
            .evaluate_pin("dialog_id")
            .await
            .ok()
            .filter(|s: &String| !s.is_empty());

        context.close_dialog(dialog_id).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
