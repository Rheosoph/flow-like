use crate::utils::pure_scores;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct IsNullNode {}

impl IsNullNode {
    pub fn new() -> Self {
        IsNullNode {}
    }
}

#[async_trait]
impl NodeLogic for IsNullNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_types_is_null",
            "Is Null",
            "Checks whether a value is null. Empty strings, arrays and structs return false.",
            "Utils/Types",
        );
        node.set_flowscript_name("types", "isNull");
        node.set_receiver("value");
        node.add_icon("/flow/icons/type.svg");
        node.set_scores(pure_scores());

        node.add_input_pin("value", "Value", "Value to inspect", VariableType::Generic);
        node.add_output_pin(
            "is_null",
            "Is Null",
            "True when the value is null",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value = context.evaluate_pin_raw("value").await?;
        context
            .set_pin_value("is_null", Value::Bool(value.is_null()))
            .await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
    }
}
