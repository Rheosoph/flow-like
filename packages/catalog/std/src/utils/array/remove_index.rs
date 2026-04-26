use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, bail, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RemoveArrayIndexNode {}

impl RemoveArrayIndexNode {
    pub fn new() -> Self {
        RemoveArrayIndexNode {}
    }
}

#[async_trait]
impl NodeLogic for RemoveArrayIndexNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_remove_index",
            "Remove Index",
            "Removes an element from an array at a specific index",
            "Utils/Array",
        );
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_input_pin("index", "Index", "Index to remove", VariableType::Integer);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Adjusted Array",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let index: usize = context.evaluate_pin("index").await?;

        let mut array_out = array_in.clone();
        let success = index < array_out.len();

        if success {
            array_out.remove(index);
        } else {
            context.log_message(
                &format!(
                    "Index {} is out of bounds for array of length {}",
                    index,
                    array_out.len()
                ),
                LogLevel::Warn,
            );
        }

        context.set_pin_value("array_out", json!(array_out)).await?;

        if !success {
            bail!("Index out of bounds");
        }
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        node.harmonize_type(vec!["array_in", "array_out"], true);
    }
}
