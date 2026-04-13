use super::element_utils::{extract_element_id, find_element};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

/// Unified toggle (checkbox/switch) update node.
///
/// Set or toggle the checked state of checkbox or switch elements.
/// The input pins change dynamically based on the selected operation.
///
/// **Operations:**
/// - Set: Set checked state to a specific value
/// - Toggle: Flip the current checked state
/// - Get: Get the current checked state
#[crate::register_node]
#[derive(Default)]
pub struct UpdateToggle;

impl UpdateToggle {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateToggle {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_toggle",
            "Update Toggle",
            "Set or toggle checkbox/switch checked state",
            "UI/Elements/Checkbox",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to checkbox or switch element",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "operation",
            "Operation",
            "What operation to perform",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Set".to_string(),
                    "Toggle".to_string(),
                    "Get".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Set")));

        // Default: Set operation pins
        node.add_input_pin(
            "checked",
            "Checked",
            "New checked state",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let operation: String = context.evaluate_pin("operation").await?;

        match operation.as_str() {
            "Set" => {
                let checked: bool = context.evaluate_pin("checked").await?;
                let update = json!({
                    "type": "setChecked",
                    "checked": checked
                });
                context.upsert_element(&element_id, update).await?;
                context.set_pin_value("state", json!(checked)).await?;
            }
            "Toggle" => {
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let current = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("checked").or_else(|| c.get("value")))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let new_state = !current;
                let update = json!({
                    "type": "setChecked",
                    "checked": new_state
                });
                context.upsert_element(&element_id, update).await?;
                context.set_pin_value("state", json!(new_state)).await?;
            }
            "Get" => {
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let checked = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("checked").or_else(|| c.get("value")))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                context.set_pin_value("state", json!(checked)).await?;
            }
            _ => return Err(flow_like_types::anyhow!("Unknown operation: {}", operation)),
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let operation = node
            .get_pin_by_name("operation")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Set".to_string());

        // Remove dynamic pins
        let pins_to_check = ["checked", "state"];
        for pin_name in pins_to_check {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match operation.as_str() {
            "Set" => {
                node.add_input_pin(
                    "checked",
                    "Checked",
                    "New checked state",
                    VariableType::Boolean,
                )
                .set_default_value(Some(json!(false)));
                node.add_output_pin(
                    "state",
                    "State",
                    "The checked state after operation",
                    VariableType::Boolean,
                );
            }
            "Toggle" => {
                node.add_output_pin(
                    "state",
                    "New State",
                    "The new checked state after toggle",
                    VariableType::Boolean,
                );
            }
            "Get" => {
                node.add_output_pin(
                    "state",
                    "Checked",
                    "Current checked state",
                    VariableType::Boolean,
                );
            }
            _ => {}
        }
    }
}
