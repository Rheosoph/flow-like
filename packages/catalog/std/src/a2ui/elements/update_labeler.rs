use super::element_utils::extract_element_id;
use super::update_schemas::{ImageSource, LabelerBox};
use flow_like::a2ui::components::ImageLabelerProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

use super::element_utils::find_element;

/// Unified ImageLabeler update node.
///
/// Manage bounding boxes on an ImageLabeler element with a single node.
/// The input pins change dynamically based on the selected operation.
///
/// **Operations:**
/// - Add: Add a single bounding box
/// - Remove: Remove a box by ID
/// - Update Label: Update the label of an existing box
/// - Set All: Replace all boxes with an array
/// - Clear: Remove all boxes
/// - Get All: Retrieve all boxes
/// - Set Image: Set the background image
#[crate::register_node]
#[derive(Default)]
pub struct UpdateLabeler;

impl UpdateLabeler {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateLabeler {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_labeler",
            "Update Labeler",
            "Add, remove, or manage bounding boxes on an ImageLabeler element",
            "UI/Elements/Labeler",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Labeler",
            "Reference to the ImageLabeler element",
            VariableType::Struct,
        )
        .set_schema::<ImageLabelerProps>()
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
                    "Add".to_string(),
                    "Remove".to_string(),
                    "Update Label".to_string(),
                    "Set All".to_string(),
                    "Clear".to_string(),
                    "Get All".to_string(),
                    "Set Image".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Add")));

        // Default: Add operation pins
        node.add_input_pin("box", "Box", "Bounding box to add", VariableType::Struct)
            .set_schema::<LabelerBox>();

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
            "Add" => {
                let labeler_box: LabelerBox = context.evaluate_pin("box").await?;
                let update = json!({
                    "type": "addLabelerBox",
                    "box": labeler_box
                });
                context.upsert_element(&element_id, update).await?;
                context
                    .set_pin_value("box_id", json!(labeler_box.id))
                    .await?;
            }
            "Remove" => {
                let box_id: String = context.evaluate_pin("box_id").await?;
                let update = json!({
                    "type": "removeLabelerBox",
                    "boxId": box_id
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Update Label" => {
                let box_id: String = context.evaluate_pin("box_id").await?;
                let label: String = context.evaluate_pin("label").await?;
                let update = json!({
                    "type": "updateLabelerBoxLabel",
                    "boxId": box_id,
                    "label": label
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Set All" => {
                let boxes: Vec<LabelerBox> = context.evaluate_pin("boxes").await?;
                let update = json!({
                    "type": "setLabelerBoxes",
                    "boxes": boxes
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Clear" => {
                let update = json!({
                    "type": "setLabelerBoxes",
                    "boxes": []
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Get All" => {
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let boxes = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("boxes"))
                    .cloned()
                    .unwrap_or(json!([]));
                let count = boxes.as_array().map(|a| a.len()).unwrap_or(0);
                context.set_pin_value("boxes", boxes).await?;
                context.set_pin_value("count", json!(count)).await?;
            }
            "Set Image" => {
                let image: ImageSource = context.evaluate_pin("image").await?;
                let mut update = json!({
                    "type": "setLabelerImage",
                    "src": image.src
                });
                if let Some(alt) = image.alt {
                    update["alt"] = json!(alt);
                }
                context.upsert_element(&element_id, update).await?;
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
            .unwrap_or_else(|| "Add".to_string());

        // Remove all dynamic pins first
        let pins_to_check = ["box", "box_id", "label", "boxes", "count", "image"];
        for pin_name in pins_to_check {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match operation.as_str() {
            "Add" => {
                node.add_input_pin("box", "Box", "Bounding box to add", VariableType::Struct)
                    .set_schema::<LabelerBox>();
                node.add_output_pin(
                    "box_id",
                    "Box ID",
                    "ID of the added box",
                    VariableType::String,
                );
            }
            "Remove" => {
                node.add_input_pin(
                    "box_id",
                    "Box ID",
                    "ID of box to remove",
                    VariableType::String,
                );
            }
            "Update Label" => {
                node.add_input_pin(
                    "box_id",
                    "Box ID",
                    "ID of box to update",
                    VariableType::String,
                );
                node.add_input_pin(
                    "label",
                    "Label",
                    "New label for the box",
                    VariableType::String,
                );
            }
            "Set All" => {
                node.add_input_pin(
                    "boxes",
                    "Boxes",
                    "Array of all bounding boxes",
                    VariableType::Struct,
                )
                .set_value_type(ValueType::Array)
                .set_schema::<LabelerBox>();
            }
            "Clear" => {
                // No additional pins needed
            }
            "Get All" => {
                node.add_output_pin(
                    "boxes",
                    "Boxes",
                    "Array of all bounding boxes",
                    VariableType::Struct,
                )
                .set_value_type(ValueType::Array)
                .set_schema::<LabelerBox>();
                node.add_output_pin("count", "Count", "Number of boxes", VariableType::Integer);
            }
            "Set Image" => {
                node.add_input_pin(
                    "image",
                    "Image",
                    "Background image source",
                    VariableType::Struct,
                )
                .set_schema::<ImageSource>();
            }
            _ => {}
        }
    }
}
