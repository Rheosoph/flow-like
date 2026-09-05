use flow_like::{
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic},
        variable::VariableType,
    },
    utils::json::parse_malformed_json,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct RepairParseNode {}

impl RepairParseNode {
    pub fn new() -> Self {
        RepairParseNode {}
    }
}

#[async_trait]
impl NodeLogic for RepairParseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "repair_parse",
            "Repair Parse JSON",
            "Attempts to repair and parse potentially malformed JSON",
            "Utils/JSON",
        );
        node.set_flowscript_name("json", "repairParse");

        node.add_icon("/flow/icons/repair.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "json_string",
            "JSON String",
            "String containing potentially malformed JSON",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Execution continues if parsing succeeds",
            VariableType::Execution,
        );

        node.add_output_pin(
            "result",
            "Result",
            "The parsed JSON structure",
            VariableType::Struct,
        )
        .set_open_schema();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let json_string: String = context.evaluate_pin("json_string").await?;

        let json = parse_malformed_json(&json_string)?;
        context.set_pin_value("result", json).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
