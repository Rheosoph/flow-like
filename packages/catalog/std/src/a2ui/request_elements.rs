use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::async_trait;

#[crate::register_node]
#[derive(Default)]
pub struct RequestElements;

impl RequestElements {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RequestElements {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_request_elements",
            "Request Elements",
            "Requests element values from the frontend before processing",
            "UI/Data",
        );
        node.set_flowscript_name("ui", "requestElements");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ids",
            "Element IDs",
            "Array of element IDs to request (e.g., ['main/input-field', 'main/checkbox'])",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_ids: Vec<String> = context.evaluate_pin("element_ids").await?;

        context.request_elements(element_ids).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
