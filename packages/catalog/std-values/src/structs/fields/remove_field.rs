use super::path_utils::remove_value_by_path;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct RemoveStructFieldNode {}

impl RemoveStructFieldNode {
    pub fn new() -> Self {
        RemoveStructFieldNode {}
    }
}

#[async_trait]
impl NodeLogic for RemoveStructFieldNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_remove",
            "Remove Field",
            "Removes a field from a struct (supports dot notation and array access)",
            "Structs/Fields",
        );
        node.set_flowscript_name("struct", "remove");
        node.set_receiver("struct_in");
        node.add_icon("/flow/icons/struct.svg");

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
        node.add_output_pin("struct_out", "Struct", "Struct Out", VariableType::Struct)
            .set_open_schema();
        node.add_output_pin(
            "removed_value",
            "Removed Value",
            "The value that was removed (null if field didn't exist)",
            VariableType::Generic,
        );
        node.add_input_pin("struct_in", "Struct", "Struct In", VariableType::Struct)
            .set_open_schema();

        node.add_input_pin(
            "field",
            "Field",
            "Field selector to remove (e.g., 'message.content' or 'items[0]')",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut old_struct = context
            .evaluate_pin::<flow_like_types::Value>("struct_in")
            .await?;
        let field = context.evaluate_pin::<String>("field").await?;

        let removed = remove_value_by_path(&mut old_struct, &field)?;

        context.set_pin_value("struct_out", old_struct).await?;
        context
            .set_pin_value(
                "removed_value",
                removed.unwrap_or(flow_like_types::Value::Null),
            )
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
