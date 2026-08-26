use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait};

use super::element_utils::extract_element_id;

/// Removes an element from the page.
#[crate::register_node]
#[derive(Default)]
pub struct RemoveElement;

impl RemoveElement {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RemoveElement {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_remove_element",
            "Remove Element",
            "Removes an element from the page",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "removeElement");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "surface_id",
            "Surface ID",
            "The surface containing the element",
            VariableType::String,
        );

        node.add_input_pin(
            "element_id",
            "Element ID",
            "Element ID string or element object from Get Element",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let surface_id: String = context.evaluate_pin("surface_id").await?;
        let element_value: Value = context.evaluate_pin("element_id").await?;
        let element_id = extract_element_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected string ID or element object"
            )
        })?;

        context.remove_element(&surface_id, &element_id).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
