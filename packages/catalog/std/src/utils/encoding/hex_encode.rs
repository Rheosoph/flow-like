use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct HexEncodeNode {}

impl HexEncodeNode {
    pub fn new() -> Self {
        HexEncodeNode {}
    }
}

#[async_trait]
impl NodeLogic for HexEncodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_hex_encode",
            "Hex Encode",
            "Encodes a string's bytes to a hexadecimal string",
            "Utils/Encoding",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("input", "Input", "String to encode", VariableType::String);
        node.add_output_pin(
            "output",
            "Encoded",
            "Hex-encoded string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input: String = context.evaluate_pin("input").await?;
        let encoded = hex_encode(input.as_bytes());
        context.set_pin_value("output", json!(encoded)).await?;
        Ok(())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
