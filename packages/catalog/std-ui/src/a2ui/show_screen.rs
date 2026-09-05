use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct ShowScreen;

impl ShowScreen {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ShowScreen {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_show_screen",
            "Show Screen",
            "Shows the current frontend screen while the workflow continues running",
            "UI/Surface",
        );
        node.set_flowscript_name("ui", "showScreen");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);
        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.show_screen().await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
