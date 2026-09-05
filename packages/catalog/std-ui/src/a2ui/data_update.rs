use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DataUpdate;

impl DataUpdate {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for DataUpdate {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_data_update",
            "Data Update",
            "Updates data in a surface's data model",
            "UI/Data",
        );
        node.set_flowscript_name("ui", "dataUpdate");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "ID of the surface to update",
            VariableType::String,
        )
        .set_default_value(Some(json!("main")));

        node.add_input_pin(
            "path",
            "Path",
            "Data path to update (e.g., 'user/name')",
            VariableType::String,
        );

        node.add_input_pin(
            "value",
            "Value",
            "New value to set at the path",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface_id: String = context.evaluate_pin("surface_id").await?;
        let path: Option<String> = context.evaluate_pin::<String>("path").await.ok();
        let value: Value = context.evaluate_pin("value").await?;

        context
            .stream_a2ui_data_update(&surface_id, path, value)
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
