use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use md5::{Digest, Md5};

#[crate::register_node]
#[derive(Default)]
pub struct Md5Node {}

impl Md5Node {
    pub fn new() -> Self {
        Md5Node {}
    }
}

#[async_trait]
impl NodeLogic for Md5Node {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_hash_md5",
            "MD5 Hash",
            "Computes the MD5 hash of the input string. Note: MD5 is not collision-resistant — use SHA-256 or Blake3 for security-sensitive hashing.",
            "Utils/Hash",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);
        node.add_input_pin("input", "Input", "String to hash", VariableType::String);

        node.add_output_pin("exec_out", "Done", "", VariableType::Execution);
        node.add_output_pin(
            "hash",
            "Hash (hex)",
            "MD5 hash as hex string",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let input: String = context.evaluate_pin("input").await?;
        let mut hasher = Md5::new();
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
