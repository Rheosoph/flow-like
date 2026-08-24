use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ExtendArrayNode {}

impl ExtendArrayNode {
    pub fn new() -> Self {
        ExtendArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for ExtendArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_extend",
            "Extend",
            "Append an Array to another Array",
            "Utils/Array",
        );
        node.set_flowscript_name("array", "extend");
        node.set_receiver("array_in");
        node.add_icon("/flow/icons/grip.svg");

        node.add_input_pin("exec_in", "In", "", VariableType::Execution);

        node.add_input_pin("array_in", "Array", "Your Array", VariableType::Generic)
            .set_value_type(ValueType::Array)
            .set_options(
                PinOptions::new()
                    .set_enforce_generic_value_type(true)
                    .build(),
            );

        node.add_input_pin("values", "Values", "Value to push", VariableType::Generic)
            .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Adjusted Array",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array)
        .set_options(
            PinOptions::new()
                .set_enforce_generic_value_type(true)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let value: Vec<Value> = context.evaluate_pin("values").await?;
        let mut array_out = array_in.clone();
        array_out.extend(value);
        context.set_pin_value("array_out", json!(array_out)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("values", board, Some(ValueType::Array), None);

        node.harmonize_type(vec!["array_in", "array_out", "values"], true);
    }
}
