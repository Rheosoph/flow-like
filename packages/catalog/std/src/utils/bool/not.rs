use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BoolNot {}

impl BoolNot {
    pub fn new() -> Self {
        BoolNot {}
    }
}

#[async_trait]
impl NodeLogic for BoolNot {
    fn get_node(&self) -> Node {
        let mut node = Node::new("bool_not", "! (Bool)", "Boolean NOT", "Utils/Bool");
        node.add_icon("/flow/icons/bool.svg");

        node.add_input_pin("boolean", "Boolean", "Input Boolean", VariableType::Boolean)
            .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "result",
            "Result",
            "NOT operation on the input",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let input_value: bool = context.evaluate_pin("boolean").await?;
        let output_value = !input_value;

        context.set_pin_value("result", json!(output_value)).await?;

        return Ok(());
    }
}
