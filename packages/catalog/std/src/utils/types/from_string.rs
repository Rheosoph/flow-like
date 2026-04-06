use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::from_str};
use std::sync::Arc;

use crate::utils::types::normalize_json_value;

#[crate::register_node]
#[derive(Default)]
pub struct FromStringNode {}

impl FromStringNode {
    pub fn new() -> Self {
        FromStringNode {}
    }
}

#[async_trait]
impl NodeLogic for FromStringNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "val_from_string",
            "From String",
            "Convert String to Struct",
            "Utils/Conversions",
        );
        node.add_icon("/flow/icons/convert.svg");

        node.add_input_pin(
            "string",
            "String",
            "String to convert",
            VariableType::String,
        );

        node.add_output_pin(
            "value_ref",
            "Value",
            "Value of the Generic",
            VariableType::Generic,
        );
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let string: String = context.evaluate_pin("string").await?;
        let value: Value = from_str(&string)?;
        let normalized_value = normalize_json_value(value);
        context.set_pin_value("value_ref", normalized_value).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value_ref", board, None, None);
    }
}
