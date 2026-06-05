use super::element_utils::extract_element_id;
use flow_like::a2ui::components::BoundingBoxOverlayProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::BoundingBox;
use flow_like_types::{Value, async_trait, json::json};

/// Unified BoundingBoxOverlay update node.
///
/// Push detection results onto a `boundingBoxOverlay` element. The box pins are
/// strictly typed to the object-detection `BoundingBox` output
/// (`{x1, y1, x2, y2, score, class_idx, class_name}`), so detection nodes connect
/// directly. The frontend normalizes the corner-coordinate format for rendering.
///
/// **Operations:**
/// - Set All: Replace all boxes with an array
/// - Add: Append a single box
/// - Clear: Remove all boxes
#[crate::register_node]
#[derive(Default)]
pub struct UpdateOverlay;

impl UpdateOverlay {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateOverlay {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_overlay",
            "Set Bounding Boxes",
            "Set, push, or clear bounding boxes on a BoundingBoxOverlay element",
            "UI/Elements/Overlay",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Overlay",
            "Reference to the BoundingBoxOverlay element",
            VariableType::Struct,
        )
        .set_schema::<BoundingBoxOverlayProps>()
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
                    "Set All".to_string(),
                    "Add".to_string(),
                    "Clear".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Set All")));

        // Default: Set All pins
        node.add_input_pin(
            "boxes",
            "Boxes",
            "Array of detection bounding boxes",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<BoundingBox>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
            "Set All" => {
                let boxes: Vec<Value> = context.evaluate_pin("boxes").await?;
                let update = json!({
                    "type": "setOverlayBoxes",
                    "boxes": boxes
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Add" => {
                let bounding_box: Value = context.evaluate_pin("box").await?;
                let update = json!({
                    "type": "addOverlayBox",
                    "box": bounding_box
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Clear" => {
                let update = json!({
                    "type": "clearOverlayBoxes"
                });
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
            .unwrap_or_else(|| "Set All".to_string());

        for pin_name in ["boxes", "box"] {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match operation.as_str() {
            "Set All" => {
                node.add_input_pin(
                    "boxes",
                    "Boxes",
                    "Array of detection bounding boxes",
                    VariableType::Struct,
                )
                .set_value_type(ValueType::Array)
                .set_schema::<BoundingBox>()
                .set_options(PinOptions::new().set_enforce_schema(true).build());
            }
            "Add" => {
                node.add_input_pin(
                    "box",
                    "Box",
                    "Detection bounding box to append",
                    VariableType::Struct,
                )
                .set_schema::<BoundingBox>()
                .set_options(PinOptions::new().set_enforce_schema(true).build());
            }
            "Clear" => {}
            _ => {}
        }
    }
}
