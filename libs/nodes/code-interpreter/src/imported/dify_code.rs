use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct DifyCodeNode {}

#[async_trait]
impl NodeLogic for DifyCodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "dify_code",
            "Dify Code",
            "Execute Python code imported from a Dify workflow.\n\
             Expects a `def main(**kwargs)` function.\n\
             Input/output pins are auto-generated from the schema.",
            "Code/Dify",
        );
        node.set_long_running(true);
        node.add_icon("/flow/icons/code.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "code",
            "Code",
            "Python source (def main(**kwargs) → dict)",
            VariableType::String,
        )
        .set_default_value(Some(json!("def main():\n    return {}\n")));

        node.add_input_pin(
            "packages",
            "Packages",
            "micropip packages to install before execution",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "input_schema",
            "Input Schema",
            "JSON mapping input names to types, e.g. {\"text\": \"string\"}",
            VariableType::String,
        )
        .set_default_value(Some(json!("{}")));

        node.add_input_pin(
            "output_schema",
            "Output Schema",
            "JSON mapping output names to types, e.g. {\"result\": \"string\"}",
            VariableType::String,
        )
        .set_default_value(Some(json!("{}")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Activated on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_error",
            "Error",
            "Activated on failure",
            VariableType::Execution,
        );
        node.add_output_pin(
            "stdout",
            "Stdout",
            "Captured standard output",
            VariableType::String,
        );
        node.add_output_pin(
            "stderr",
            "Stderr",
            "Captured standard error",
            VariableType::String,
        );
        node.add_output_pin(
            "error_msg",
            "Error Message",
            "Error traceback on failure",
            VariableType::String,
        );
        node.add_output_pin(
            "success",
            "Success",
            "True if code completed without error",
            VariableType::Boolean,
        );

        node
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        super::update_dynamic_pins(node);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        flow_like_catalog_core::run_with_execute_gate!(context, {
            super::execute_imported_code(context, true).await
        })
    }
}
