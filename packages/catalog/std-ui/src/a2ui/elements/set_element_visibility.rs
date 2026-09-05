use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::element_utils::extract_element_id;

/// Sets the visibility of an element.
///
/// Streams a ui_update event to the frontend to show/hide the element.
#[crate::register_node]
#[derive(Default)]
pub struct SetElementVisibility;

impl SetElementVisibility {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetElementVisibility {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_element_visibility",
            "Set Element Visibility",
            "Shows or hides an element",
            "UI/Elements",
        );
        node.set_flowscript_name("ui", "setElementVisibility");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Element ID string or element object from Get Element",
            VariableType::Generic,
        );

        node.add_input_pin(
            "visible",
            "Visible",
            "Whether the element should be visible",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected string ID or element object"
            )
        })?;
        let visible: bool = context.evaluate_pin("visible").await?;

        let update_value = json!({
            "type": "setVisibility",
            "visible": visible
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
