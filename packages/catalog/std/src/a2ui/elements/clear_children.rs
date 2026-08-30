use super::element_utils::extract_element_id;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Clears all children from a container element.
#[crate::register_node]
#[derive(Default)]
pub struct ClearChildren;

impl ClearChildren {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ClearChildren {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_clear_children",
            "Clear Children",
            "Removes all children from a container element",
            "UI/Elements/Containers",
        );
        node.set_flowscript_name("ui", "clearChildren");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "container_ref",
            "Container",
            "Reference to the container element (ID or element object)",
            VariableType::Generic,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let container_value: Value = context.evaluate_pin("container_ref").await?;
        let container_id = extract_element_id(&container_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid container reference"))?;

        let update_value = json!({
            "type": "clearChildren"
        });

        context.upsert_element(&container_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
