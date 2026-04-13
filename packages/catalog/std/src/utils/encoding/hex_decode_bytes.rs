use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct HexDecodeBytesNode {}

impl HexDecodeBytesNode {
    pub fn new() -> Self {
        HexDecodeBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for HexDecodeBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_hex_decode_bytes",
            "Hex Decode to Bytes",
            "Decodes a hexadecimal string to raw bytes",
            "Utils/Encoding",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin(
            "input",
            "Input",
            "Hex-encoded string",
            VariableType::String,
        );
        node.add_output_pin(
            "output",
            "Decoded",
            "Decoded raw bytes",
            VariableType::Byte,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        if input.len() % 2 != 0 {
            return Err(flow_like_types::anyhow!("Hex string has odd length"));
        }
        let bytes: Vec<u8> = (0..input.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&input[i..i + 2], 16)
                    .map_err(|e| flow_like_types::anyhow!("Invalid hex at position {i}: {e}"))
            })
            .collect::<flow_like_types::Result<Vec<u8>>>()?;
        context.set_pin_value("output", json!(bytes)).await?;
        Ok(())
    }
}
