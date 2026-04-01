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
pub struct GetArrayElementNode {}

impl GetArrayElementNode {
    pub fn new() -> Self {
        GetArrayElementNode {}
    }
}

#[async_trait]
impl NodeLogic for GetArrayElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_get",
            "Get Element",
            "Gets an element from an array by index",
            "Utils/Array",
        );

        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_input_pin(
            "index",
            "Index",
            "Index of the element to get",
            VariableType::Integer,
        );

        node.add_output_pin(
            "element",
            "Element",
            "Element at the specified index",
            VariableType::Generic,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Was the get successful?",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let array_in = context.evaluate_pin_to_ref("array_in").await?;
        let index: i64 = context.evaluate_pin("index").await?;

        let mut success = false;
        let mut element = Value::Null;

        if let Some(array) = array_in.as_array()
            && index >= 0
            && index < array.len() as i64
        {
            element = array[index as usize].clone();
            success = true;
        }

        context.set_pin_value("element", json!(element)).await?;
        context.set_pin_value("success", json!(success)).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: Arc<Board>) {
        let _ = node.match_type("array_in", board.clone(), Some(ValueType::Array), None);
        let _ = node.match_type("element", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["element", "array_in"], true);
    }
}
