use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct MakeArrayNode {}

impl MakeArrayNode {
    pub fn new() -> Self {
        MakeArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for MakeArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "make_array",
            "Make Array",
            "Creates an empty array",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "make");

        node.add_icon("/flow/icons/grip.svg");

        node.add_output_pin(
            "array_out",
            "Array",
            "The created array",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array_out: Vec<flow_like_types::Value> = Vec::new(); // Create an empty array
        context.set_pin_value("array_out", json!(array_out)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
    }
}
