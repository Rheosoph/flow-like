use flow_like::a2ui::components::ButtonProps;
use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::element_utils::extract_element_id;

/// Sets the label text of a button element.
#[crate::register_node]
#[derive(Default)]
pub struct SetButtonLabel;

impl SetButtonLabel {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetButtonLabel {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_button_label",
            "Set Button Label",
            "Sets the label text of a button element",
            "UI/Elements/Button",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Button",
            "Element ID string or element object from Get Element",
            VariableType::Struct,
        )
        .set_schema::<ButtonProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin("label", "Label", "The new label text", VariableType::String);

        node.add_output_pin("exec_out", "▶", "Execution output", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.activate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value).ok_or_else(|| {
            flow_like_types::anyhow!(
                "Invalid element reference - expected string ID or element object"
            )
        })?;
        let label: String = context.evaluate_pin("label").await?;

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setText",
                    "text": label
                }),
            )
            .await?;

        context.log_message(
            &format!("Set button label: {} = {}", element_id, label),
            LogLevel::Debug,
        );

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
    }
}
