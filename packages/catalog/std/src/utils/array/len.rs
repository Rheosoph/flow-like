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
pub struct ArrayLengthNode {}

impl ArrayLengthNode {
    pub fn new() -> Self {
        ArrayLengthNode {}
    }
}

#[async_trait]
impl NodeLogic for ArrayLengthNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_length",
            "Array Length",
            "Gets the length of an array",
            "Utils/Array",
        );
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("array", "Array", "Input Array", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_output_pin(
            "length",
            "Length",
            "Length of the array",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array = context.evaluate_pin_to_ref("array").await?;

        if let Some(array) = array.as_array() {
            context.set_pin_value("length", json!(array.len())).await?;
            return Ok(());
        }

        context.set_pin_value("length", json!(0)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array", board, Some(ValueType::Array), None);
    }
}
