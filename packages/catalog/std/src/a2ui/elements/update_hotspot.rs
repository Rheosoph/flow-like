use super::element_utils::extract_element_id;
use super::update_schemas::{Hotspot, ImageSource};
use flow_like::a2ui::components::ImageHotspotProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unified ImageHotspot update node.
///
/// Manage hotspots on an ImageHotspot element with a single node.
/// The input pins change dynamically based on the selected operation.
///
/// **Operations:**
/// - Add: Add a single hotspot (id, x, y, size, label, description, color, action)
/// - Remove: Remove a hotspot by ID
/// - Set All: Replace all hotspots with an array
/// - Clear: Remove all hotspots
/// - Get All: Retrieve all hotspots
/// - Set Image: Set the background image
#[crate::register_node]
#[derive(Default)]
pub struct UpdateHotspot;

impl UpdateHotspot {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateHotspot {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_hotspot",
            "Update Hotspot",
            "Add, remove, or manage hotspots on an ImageHotspot element",
            "UI/Elements/Hotspot",
        );
        node.set_flowscript_name("ui", "updateHotspot");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Hotspot Image",
            "Reference to the ImageHotspot element",
            VariableType::Struct,
        )
        .set_schema::<ImageHotspotProps>()
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
                    "Set All".to_string(),
                    "Clear".to_string(),
                    "Get All".to_string(),
                    "Set Image".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Add")));

        // Default: Add operation pins
        node.add_input_pin("hotspot", "Hotspot", "Hotspot to add", VariableType::Struct)
            .set_schema::<Hotspot>();

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
                let hotspot: Hotspot = context.evaluate_pin("hotspot").await?;
                let update = json!({
                    "type": "addHotspot",
                    "hotspot": hotspot
                });
                context.upsert_element(&element_id, update).await?;
                context
                    .set_pin_value("hotspot_id", json!(hotspot.id))
                    .await?;
            }
            "Remove" => {
                let hotspot_id: String = context.evaluate_pin("hotspot_id").await?;
                let update = json!({
                    "type": "removeHotspot",
                    "hotspotId": hotspot_id
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Set All" => {
                let hotspots: Vec<Hotspot> = context.evaluate_pin("hotspots").await?;
                let update = json!({
                    "type": "setHotspots",
                    "hotspots": hotspots
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Clear" => {
                let update = json!({
                    "type": "setHotspots",
                    "hotspots": []
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Get All" => {
                let element = context.read_element(&element_id).await?;
                let hotspots = element
                    .as_ref()
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("hotspots"))
                    .cloned()
                    .unwrap_or(json!([]));
                let count = hotspots.as_array().map(|a| a.len()).unwrap_or(0);
                context.set_pin_value("hotspots", hotspots).await?;
                context.set_pin_value("count", json!(count)).await?;
            }
            "Set Image" => {
                let image: ImageSource = context.evaluate_pin("image").await?;
                let mut update = json!({
                    "type": "setHotspotImage",
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
        let pins_to_check = ["hotspot", "hotspot_id", "hotspots", "count", "image"];
        for pin_name in pins_to_check {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match operation.as_str() {
            "Add" => {
                node.add_input_pin("hotspot", "Hotspot", "Hotspot to add", VariableType::Struct)
                    .set_schema::<Hotspot>();
                node.add_output_pin(
                    "hotspot_id",
                    "Hotspot ID",
                    "ID of the added hotspot",
                    VariableType::String,
                );
            }
            "Remove" => {
                node.add_input_pin(
                    "hotspot_id",
                    "Hotspot ID",
                    "ID of hotspot to remove",
                    VariableType::String,
                );
            }
            "Set All" => {
                node.add_input_pin(
                    "hotspots",
                    "Hotspots",
                    "Array of all hotspots",
                    VariableType::Struct,
                )
                .set_value_type(ValueType::Array)
                .set_schema::<Hotspot>();
            }
            "Clear" => {
                // No additional pins needed
            }
            "Get All" => {
                node.add_output_pin(
                    "hotspots",
                    "Hotspots",
                    "Array of all hotspots",
                    VariableType::Struct,
                )
                .set_value_type(ValueType::Array)
                .set_schema::<Hotspot>();
                node.add_output_pin(
                    "count",
                    "Count",
                    "Number of hotspots",
                    VariableType::Integer,
                );
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
