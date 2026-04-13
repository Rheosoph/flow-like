use flow_like::flow::{
    board::Board,
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

use super::element_utils::extract_element_id;

/// Clears the value of an input element.
#[crate::register_node]
#[derive(Default)]
pub struct ClearInput;

impl ClearInput {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ClearInput {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_clear_input",
            "Clear Input",
            "Clears the value of an input element",
            "UI/Elements/Input",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "Execution input", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Input",
            "Element ID string or element object from Get Element",
            VariableType::Generic,
        );

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

        context
            .upsert_element(
                &element_id,
                json!({
                    "type": "setValue",
                    "value": ""
                }),
            )
            .await?;

        context.log_message(&format!("Cleared input: {}", element_id), LogLevel::Debug);

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        node.error = None;
    }
}
