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
pub struct BatchPushArrayNode {}

impl BatchPushArrayNode {
    pub fn new() -> Self {
        BatchPushArrayNode {}
    }
}

#[async_trait]
impl NodeLogic for BatchPushArrayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "array_batch_push",
            "Batch Push",
            "Push multiple items into an array in one operation. More efficient than multiple single pushes.",
            "Utils/Array/Batch",
        );
        node.set_flowscript_name("array", "batchPush");
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

        node.add_input_pin(
            "items",
            "Items",
            "Array of items to push",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Out", "", VariableType::Execution);

        node.add_output_pin(
            "array_out",
            "Array",
            "Array with all items pushed",
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

        let mut array_in: Vec<Value> = context.evaluate_pin("array_in").await?;
        let items: Vec<Value> = context.evaluate_pin("items").await?;

        array_in.extend(items);

        context.set_pin_value("array_out", json!(array_in)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        let _ = node.match_type("array_out", board, Some(ValueType::Array), None);
        let _ = node.match_type("array_in", board, Some(ValueType::Array), None);
        let _ = node.match_type("items", board, Some(ValueType::Array), None);
        node.harmonize_type(vec!["array_in", "array_out", "items"], true);
    }
}
