use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct HexDecodeNode {}

impl HexDecodeNode {
    pub fn new() -> Self {
        HexDecodeNode {}
    }
}

#[async_trait]
impl NodeLogic for HexDecodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_hex_decode",
            "Hex Decode",
            "Decodes a hexadecimal string back to a UTF-8 string",
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
            "Decoded UTF-8 string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let bytes = hex_decode(&input)?;
        let decoded = String::from_utf8(bytes)?;
        context.set_pin_value("output", json!(decoded)).await?;
        Ok(())
    }
}

fn hex_decode(s: &str) -> flow_like_types::Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(flow_like_types::anyhow!("Hex string has odd length"));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| flow_like_types::anyhow!("Invalid hex at position {i}: {e}"))
        })
        .collect()
}
