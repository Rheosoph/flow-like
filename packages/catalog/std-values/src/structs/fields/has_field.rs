use super::path_utils::has_value_by_path;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct HasStructFieldNode {}

impl HasStructFieldNode {
    pub fn new() -> Self {
        HasStructFieldNode {}
    }
}

#[async_trait]
impl NodeLogic for HasStructFieldNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "struct_has",
            "Has Field",
            "Checks if a field exists in a struct (supports dot notation and array access)",
            "Structs/Fields",
        );
        node.set_flowscript_name("struct", "has");
        node.set_receiver("struct");
        node.add_icon("/flow/icons/struct.svg");

        node.add_output_pin(
            "found",
            "Found?",
            "Indicates if the value was found",
            VariableType::Boolean,
        );

        node.add_input_pin("struct", "Struct", "Struct Output", VariableType::Struct)
            .set_open_schema();

        node.add_input_pin(
            "field",
            "Field",
            "Field selector (e.g., 'message.content' or 'items[0].name')",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let struct_value = context
            .evaluate_pin::<flow_like_types::Value>("struct")
            .await?;
        let field = context.evaluate_pin::<String>("field").await?;

        let found = has_value_by_path(&struct_value, &field);
        context
            .set_pin_value("found", flow_like_types::json::json!(found))
            .await?;

        return Ok(());
    }
}
