use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct ConstructArrayNode {}

impl ConstructArrayNode {
    pub fn new() -> Self {
        ConstructArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for ConstructArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "construct_array",
            "Construct Array",
            "Creates an array from individual elements. Add more input pins by connecting to the 'element' pins.",
            "Utils/Array",
        );

        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin(
            "element",
            "Element",
            "Element to include in the array",
            VariableType::Generic,
        );

        node.add_input_pin(
            "element",
            "Element",
            "Element to include in the array",
            VariableType::Generic,
        );

        node.add_output_pin(
            "array_out",
            "Array",
            "The constructed array",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let element_pins = context.get_pins_by_name("element").await?;
        let mut array_out: Vec<Value> = Vec::with_capacity(element_pins.len());

        for pin in element_pins {
            let value: Value = context.evaluate_pin_ref(pin).await?;
            array_out.push(value);
        }

        context.set_pin_value("array_out", json!(array_out)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("element", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["array_out", "element"], true);
    }
}
