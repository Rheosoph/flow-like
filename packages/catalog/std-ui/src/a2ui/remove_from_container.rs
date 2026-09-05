use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Remove From Container - Dynamically removes an element from a container.
///
/// This node allows workflows to remove elements from container components
/// (rows, columns, stacks, etc.) at runtime.
#[crate::register_node]
#[derive(Default)]
pub struct RemoveFromContainer;

impl RemoveFromContainer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RemoveFromContainer {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_remove_from_container",
            "Remove From Container",
            "Removes an element from a container's children list",
            "UI/Container",
        );
        node.set_flowscript_name("ui", "removeFromContainer");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "container_id",
            "Container ID",
            "ID of the container element to remove from",
            VariableType::String,
        );

        node.add_input_pin(
            "element_id",
            "Element ID",
            "ID of the element to remove",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "success",
            "Success",
            "Whether the element was successfully removed",
            VariableType::Boolean,
        );

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let container_id: String = context.evaluate_pin("container_id").await?;
        let element_id: String = context.evaluate_pin("element_id").await?;

        // Build the command payload for the frontend
        let command = json!({
            "type": "remove_from_container",
            "container_id": container_id,
            "element_id": element_id
        });

        // Use upsert_element to send command to frontend
        context
            .upsert_element(&format!("_cmd/{}", container_id), command)
            .await?;

        context
            .get_pin_by_name("success")
            .await?
            .set_value(json!(true))
            .await;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
