use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

#[crate::register_node]
#[derive(Default)]
pub struct UpsertElement;

impl UpsertElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpsertElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_upsert_element",
            "Upsert Element",
            "Updates or inserts an element value in the frontend",
            "UI/Data",
        );
        node.set_flowscript_name("ui", "upsertElement");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_id",
            "Element ID",
            "ID of the element to update (e.g., 'main/status-text')",
            VariableType::String,
        );

        node.add_input_pin(
            "value",
            "Value",
            "New value for the element",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_id: String = context.evaluate_pin("element_id").await?;
        let value: Value = context.evaluate_pin("value").await?;

        context.upsert_element(&element_id, value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
