use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct FallbackNode {}

impl FallbackNode {
    pub fn new() -> Self {
        FallbackNode {}
    }
}

#[async_trait]
impl NodeLogic for FallbackNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "utils_types_fallback",
            "Fallback",
            "Returns the input value if valid, otherwise returns the fallback default. Useful for handling optional values or error recovery.",
            "Utils/Types",
        );
        node.add_icon("/flow/icons/shield.svg");

        node.add_input_pin(
            "value",
            "Value",
            "The primary value to use if available and valid",
            VariableType::Generic,
        );

        node.add_input_pin(
            "default",
            "Default",
            "Fallback value used when the primary value is null, missing, or invalid",
            VariableType::Generic,
        );

        node.add_output_pin(
            "result",
            "Result",
            "The resolved value (primary if valid, otherwise default)",
            VariableType::Generic,
        );

        node.add_output_pin(
            "used_fallback",
            "Used Fallback",
            "True if the fallback value was used",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let value_result: Result<Value, _> = context.evaluate_pin("value").await;
        let default_value: Value = context.evaluate_pin("default").await?;

        let (result, used_fallback) = match value_result {
            Ok(value) if !value.is_null() => (value, false),
            _ => (default_value, true),
        };

        context.set_pin_value("result", result).await?;
        context
            .set_pin_value("used_fallback", Value::Bool(used_fallback))
            .await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("value", board, None, None);
        let _ = node.match_type("default", board, None, None);
        let _ = node.match_type("result", board, None, None);
        node.harmonize_type(vec!["value", "default", "result"], true);
    }
}
