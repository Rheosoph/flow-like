use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};
use std::sync::Arc;

use crate::utils::types::normalize_json_value;

#[crate::register_node]
#[derive(Default)]
pub struct FromBytesNode {}

impl FromBytesNode {
    pub fn new() -> Self {
        FromBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for FromBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "val_from_bytes",
            "From Bytes",
            "Convert String to Bytes",
            "Utils/Conversions",
        );
        node.add_icon("/flow/icons/convert.svg");

        node.add_input_pin("bytes", "Bytes", "Bytes to convert", VariableType::Byte)
            .set_value_type(ValueType::Array);

        node.add_output_pin("value", "Value", "Parsed Value", VariableType::Generic);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let bytes: Vec<u8> = context.evaluate_pin("bytes").await?;
        let value: Value = flow_like_types::json::from_slice(&bytes)?;
        let normalized_value = normalize_json_value(value);
        context.set_pin_value("value", normalized_value).await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
    }
}
