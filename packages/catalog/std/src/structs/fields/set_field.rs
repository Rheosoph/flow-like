use super::path_utils::set_value_by_path;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct SetStructFieldNode {}

impl SetStructFieldNode {
    pub fn new() -> Self {
        SetStructFieldNode {}
    }
}

#[async_trait]
impl NodeLogic for SetStructFieldNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_set",
            "Set Field",
            "Sets a field in a struct (supports dot notation and array access)",
            "Structs/Fields",
        );
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
        node.add_input_pin("struct_in", "Struct", "Struct In", VariableType::Struct)
        .set_open_schema();

        node.add_input_pin(
            "field",
            "Field",
            "Field selector (e.g., 'message.content' or 'items[0].name')",
            VariableType::String,
        );

        node.add_input_pin("value", "Value", "Value to set", VariableType::Generic);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let mut old_struct = context
            .evaluate_pin::<flow_like_types::Value>("struct_in")
            .await?;
        let field = context.evaluate_pin::<String>("field").await?;
        let value = context
            .evaluate_pin::<flow_like_types::Value>("value")
            .await?;

        set_value_by_path(&mut old_struct, &field, value)?;

        context.set_pin_value("struct_out", old_struct).await?;
        context.activate_exec_pin("exec_out").await?;
        return Ok(());
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
    }
}
