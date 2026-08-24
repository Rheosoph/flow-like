use super::element_utils::extract_element_id;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RemoveChildAtIndex;

impl RemoveChildAtIndex {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for RemoveChildAtIndex {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_remove_child_at_index",
            "Remove Child At Index",
            "Removes a child element at a specific index from a container",
            "UI/Elements/Containers",
        );
        node.set_flowscript_name("ui", "removeChildAtIndex");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "container_ref",
            "Container",
            "Reference to the container element (ID or element object)",
            VariableType::Generic,
        );

        node.add_input_pin(
            "index",
            "Index",
            "The index of the child to remove (0-based)",
            VariableType::Integer,
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

        let index: i64 = context.evaluate_pin("index").await?;

        let update_value = json!({
            "type": "removeChildAt",
            "index": index
        });

        context.upsert_element(&container_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
