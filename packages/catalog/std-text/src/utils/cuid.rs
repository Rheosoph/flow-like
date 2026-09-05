use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct CuidNode {}

impl CuidNode {
    pub fn new() -> Self {
        CuidNode {}
    }
}

#[async_trait]
impl NodeLogic for CuidNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "cuid",
            "CUID v2",
            "Generates a Collision Resistant Unique Identifier",
            "Utils",
        );
        node.set_flowscript_name("random", "cuid");
        node.add_icon("/flow/icons/random.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);

        node.add_output_pin("exec_out", "Output", "", VariableType::Execution);

        node.add_output_pin("cuid", "Cuid", "Generated CUID", VariableType::String);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let cuid = flow_like_types::create_id();
        context.set_pin_value("cuid", json!(cuid)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
