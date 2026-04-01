use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    json::{json, to_vec},
};

#[crate::register_node]
#[derive(Default)]
pub struct AHashNode {}

impl AHashNode {
    pub fn new() -> Self {
        AHashNode {}
    }
}

#[async_trait]
impl NodeLogic for AHashNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_hash_ahash",
            "AHash",
            "Computes the AHash of the input",
            "Utils/Hash",
        );
        node.add_icon("/flow/icons/hash.svg");

        node.add_input_pin("exec_in", "Execute", "", VariableType::Execution);

        node.add_input_pin(
            "input",
            "Input",
            "Input data to hash",
            VariableType::Generic,
        );

        node.add_input_pin(
            "consistent",
            "Consistent",
            "Use consistent hashing",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "seed",
            "Seed",
            "Seed value for consistent hashing",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1234)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Execution output pin",
            VariableType::Execution,
        );

        node.add_output_pin("hash", "Hash", "AHash of the input", VariableType::Integer);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let value = context.evaluate_pin_to_ref("input").await?;
        let consistent = context.evaluate_pin::<bool>("consistent").await?;
        let seed = context.evaluate_pin::<i64>("seed").await?;
        let reference_value = to_vec(&value)?;
        let hasher = if consistent {
            ahash::RandomState::with_seed(seed as usize)
        } else {
            ahash::RandomState::default()
        };
        let hash = hasher.hash_one(&reference_value);
        context.set_pin_value("hash", json!(hash)).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
