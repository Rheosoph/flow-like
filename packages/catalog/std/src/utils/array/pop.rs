use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, bail, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct PopArrayNode {}

impl PopArrayNode {
    pub fn new() -> Self {
        PopArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for PopArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_pop",
            "Pop",
            "Removes and returns the last element of an array",
            "Utils/Array",
        );
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Adjusted Array",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("value", "Value", "Popped Value", VariableType::Generic);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let mut array_out = array_in.clone();
        let popped_value = array_out.pop();
        let success = popped_value.is_some();

        context.set_pin_value("array_out", json!(array_out)).await?;
        if let Some(value) = popped_value {
            context.set_pin_value("value", json!(value)).await?;
        }

        if !success {
            bail!("Array is empty");
        }
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("value", board, Some(ValueType::Normal), None);
        node.harmonize_type(vec!["array_in", "array_out", "value"], true);
    }
}
