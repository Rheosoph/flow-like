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
pub struct SetIndexArrayNode {}

impl SetIndexArrayNode {
    pub fn new() -> Self {
        SetIndexArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for SetIndexArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_set_index",
            "Set Index",
            "Sets an element at a specific index in an array",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "setIndex");
        node.set_receiver("array_in");
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_input_pin("index", "Index", "Index to set", VariableType::Integer);

        node.add_input_pin("value", "Value", "Value to set", VariableType::Generic);

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
        let value: Value = context.evaluate_pin("value").await?;

        let mut array_out = array_in.clone();
        let mut success = false;

        if index < array_out.len() {
            success = true;
            array_out[index] = value;
        }

        context.set_pin_value("array_out", json!(array_out)).await?;

        if success {
            bail!("Index out of bounds");
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
