use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use sha2::{Digest, Sha512};

#[crate::register_node]
#[derive(Default)]
pub struct Sha512Node {}

impl Sha512Node {
    pub fn new() -> Self {
        Sha512Node {}
    }
}

#[async_trait]
impl NodeLogic for Sha512Node {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_hash_sha512",
            "SHA-512 Hash",
            "Computes the SHA-512 hash of the input string",
            "Utils/Hash",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
        node.add_input_pin(
            "input",
            "Input",
            "String to hash",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Done", "", VariableType::Execution);
        node.add_output_pin(
            "hash",
            "Hash (hex)",
            "SHA-512 hash as hex string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let input: String = context.evaluate_pin("input").await?;
        let mut hasher = Sha512::new();
        hasher.update(input.as_bytes());
        let result = hasher.finalize();
        let hex = hex_encode(&result);
        context.set_pin_value("hash", json!(hex)).await?;

        context.activate_exec_pin("exec_out").await?;
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
