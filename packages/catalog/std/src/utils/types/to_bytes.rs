use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use crate::utils::types::normalize_json_value;

#[crate::register_node]
#[derive(Default)]
pub struct ToBytesNode {}

impl ToBytesNode {
    pub fn new() -> Self {
        ToBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for ToBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "val_to_bytes",
            "To Bytes",
            "Convert Struct to Bytes",
            "Utils/Conversions",
        );
        node.set_flowscript_name("json", "toBytes");
        node.add_icon("/flow/icons/convert.svg");

        node.add_input_pin("value", "Value", "Input Value", VariableType::Generic);
        node.add_input_pin(
            "pretty",
            "Pretty?",
            "Should the struct be pretty printed?",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("bytes", "Bytes", "Output Bytes", VariableType::Byte)
            .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value: Value = context.evaluate_pin("value").await?;
        let pretty = context.evaluate_pin::<bool>("pretty").await?;

        // Normalize the value to ensure consistent key ordering
        let normalized_value = normalize_json_value(value);

        let bytes: Vec<u8> = if pretty {
            flow_like_types::json::to_vec_pretty(&normalized_value)?
        } else {
            flow_like_types::json::to_vec(&normalized_value)?
        };
        context
            .set_pin_value("bytes", flow_like_types::json::json!(bytes))
            .await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
    }
}
