use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct HexEncodeBytesNode {}

impl HexEncodeBytesNode {
    pub fn new() -> Self {
        HexEncodeBytesNode {}
    }
}

#[async_trait]
impl NodeLogic for HexEncodeBytesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_encoding_hex_encode_bytes",
            "Hex Encode Bytes",
            "Encodes raw bytes to a hexadecimal string",
            "Utils/Encoding",
        );
        node.set_flowscript_name("bytes", "toHex");
        node.set_receiver("input");
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("input", "Input", "Raw bytes to encode", VariableType::Byte)
            .set_value_type(ValueType::Array);
        node.add_output_pin(
            "output",
            "Encoded",
            "Hex-encoded string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input = context.evaluate_pin_bytes("input").await?;
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut s = String::with_capacity(input.len() * 2);
        for &b in &input {
            s.push(HEX[usize::from(b >> 4)] as char);
            s.push(HEX[usize::from(b & 0x0f)] as char);
        }
        context.set_pin_value("output", json!(s)).await?;
        Ok(())
    }
}
