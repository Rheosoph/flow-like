use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_storage::blake3;
use flow_like_types::{
    async_trait,
    json::{json, to_vec},
};

#[crate::register_node]
#[derive(Default)]
pub struct Blake3Node {}

impl Blake3Node {
    pub fn new() -> Self {
        Blake3Node {}
    }
}

#[async_trait]
impl NodeLogic for Blake3Node {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_hash_blake3",
            "Blake3 Hash",
            "Computes the Blake3 hash of the input",
            "Utils/Hash",
        );
        node.set_flowscript_name("hash", "blake3");
        node.set_receiver("input");
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);

        node.add_input_pin(
            "input",
            "Input",
            "Input data to hash",
            VariableType::Generic,
        );

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution output pin",
            VariableType::Execution,
        );
        node.add_output_pin(
            "hash",
            "Hash (hex)",
            "Blake3 hash of the input",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let value = context.evaluate_pin_to_ref("input").await?;
        let reference_value = to_vec(&value)?;
        let mut hasher = blake3::Hasher::new();
        let hash = hasher.update(&reference_value).finalize();
        let hash_hex = hash.to_hex().to_string();
        context.set_pin_value("hash", json!(hash_hex)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
