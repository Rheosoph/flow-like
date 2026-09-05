use crate::utils::pure_scores;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct BoolUnequalNode {}

impl BoolUnequalNode {
    pub fn new() -> Self {
        BoolUnequalNode {}
    }
}

#[async_trait]
impl NodeLogic for BoolUnequalNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "bool_unequal",
            "!= (Bool)",
            "Checks whether two booleans differ",
            "Utils/Bool",
        );
        node.set_flowscript_name("bool", "unequal");
        node.set_receiver("boolean1");
        node.add_icon("/flow/icons/bool.svg");
        node.set_scores(pure_scores());

        node.add_input_pin(
            "boolean1",
            "Boolean 1",
            "Input Boolean",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "boolean2",
            "Boolean 2",
            "Input Boolean",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "result",
            "Result",
            "True when the booleans differ",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let boolean1: bool = context.evaluate_pin("boolean1").await?;
        let boolean2: bool = context.evaluate_pin("boolean2").await?;
        context
            .set_pin_value("result", json!(boolean1 != boolean2))
            .await?;
        Ok(())
    }
}
