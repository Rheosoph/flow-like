use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::element_utils::extract_element_id;

/// Sets the style properties of an element.
///
/// Streams a ui_update event to the frontend to update the element's style.
#[crate::register_node]
#[derive(Default)]
pub struct SetElementStyle;

impl SetElementStyle {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetElementStyle {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_element_style",
            "Set Element Style",
            "Sets style properties of an element",
            "UI/Elements",
        );
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Element ID string or element object from Get Element",
            VariableType::Generic,
        );

        node.add_input_pin(
            "style",
            "Style",
            "Style properties to set (JSON object)",
            VariableType::Struct,
        )
        .set_schema::<flow_like::a2ui::Style>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
        let style: Value = context.evaluate_pin("style").await?;

        let update_value = json!({
            "type": "setStyle",
            "style": style
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
