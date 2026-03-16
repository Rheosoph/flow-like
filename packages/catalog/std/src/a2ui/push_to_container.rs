use super::elements::element_utils::extract_element_id;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Push To Container - Dynamically adds a widget or component to a container.
///
/// This node allows workflows to dynamically insert elements into container
/// components (rows, columns, stacks, etc.) at runtime.
#[crate::register_node]
#[derive(Default)]
pub struct PushToContainer;

impl PushToContainer {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PushToContainer {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_push_to_container",
            "Push To Container",
            "Dynamically adds an element to a container's children list",
            "UI/Container",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "container_ref",
            "Container",
            "Reference to the container element (ID or element object)",
            VariableType::Struct,
        );

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the element to add (e.g. from Instantiate Widget)",
            VariableType::Struct,
        );

        node.add_input_pin(
            "position",
            "Position",
            "Position to insert: -1 for end, 0 for start, or specific index",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.add_output_pin(
            "success",
            "Success",
            "Whether the element was successfully added",
            VariableType::Boolean,
        );

        node.set_long_running(true);
        node.set_version(2);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let container_value: Value = context.evaluate_pin("container_ref").await?;
        let container_id = extract_element_id(&container_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid container reference"))?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let child_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let position: i64 = context.evaluate_pin("position").await?;

        let update_value = if position < 0 {
            json!({
                "type": "pushChild",
                "childId": child_id
            })
        } else {
            json!({
                "type": "insertChildAt",
                "childId": child_id,
                "index": position
            })
        };

        context.upsert_element(&container_id, update_value).await?;

        context
            .get_pin_by_name("success")
            .await?
            .set_value(json!(true))
            .await;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
