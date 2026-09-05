use super::element_utils::extract_element_id;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Appends a child element to a container.
///
/// Works with row, column, stack, grid, and other container types.
#[crate::register_node]
#[derive(Default)]
pub struct PushChild;

impl PushChild {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PushChild {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_push_child",
            "Push Child",
            "Appends a child element to a container",
            "UI/Elements/Containers",
        );
        node.set_flowscript_name("ui", "pushChild");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "container_ref",
            "Container",
            "Reference to the container element (ID or element object)",
            VariableType::Generic,
        );

        node.add_input_pin(
            "child_ref",
            "Child",
            "Reference to the child element to append",
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

        let child_value: Value = context.evaluate_pin("child_ref").await?;
        let child_id = extract_element_id(&child_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid child reference"))?;

        let update_value = json!({
            "type": "pushChild",
            "childId": child_id
        });

        context.upsert_element(&container_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
