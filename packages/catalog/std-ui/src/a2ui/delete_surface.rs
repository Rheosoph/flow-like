use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DeleteSurface;

impl DeleteSurface {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DeleteSurface {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_delete_surface",
            "Delete Surface",
            "Removes a surface from the frontend",
            "UI/Surface",
        );
        node.set_flowscript_name("ui", "deleteSurface");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "ID of the surface to delete",
            VariableType::String,
        )
        .set_default_value(Some(json!("main")));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface_id: String = context.evaluate_pin("surface_id").await?;

        context.stream_a2ui_delete_surface(&surface_id).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
