use super::element_utils::extract_element_id;
use super::update_schemas::{GeoMapMarker, GeoMapRoute, GeoMapViewport};
use flow_like::a2ui::components::GeoMapProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unified GeoMap update node.
///
/// Update any property of a map element with a single node.
/// The input pins change dynamically based on the selected property type.
///
/// **Properties:**
/// - Markers: Array of markers with id, coordinate, color, label, popup, draggable
/// - Routes: Array of routes with id, coordinates, color, width
/// - Viewport: Map view with latitude, longitude, zoom, bearing, pitch
#[crate::register_node]
#[derive(Default)]
pub struct UpdateGeoMap;

impl UpdateGeoMap {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateGeoMap {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_geomap",
            "Update GeoMap",
            "Update markers, routes, or viewport of a map",
            "UI/Elements/GeoMap",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "GeoMap",
            "Reference to the map element",
            VariableType::Struct,
        )
        .set_schema::<GeoMapProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "property",
            "Property",
            "Which property to update",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Markers".to_string(),
                    "Routes".to_string(),
                    "Viewport".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Markers")));

        // Default to Markers input (array)
        node.add_input_pin(
            "markers",
            "Markers",
            "Array of map markers",
            VariableType::Struct,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_schema::<GeoMapMarker>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let property: String = context.evaluate_pin("property").await?;

        let update = match property.as_str() {
            "Markers" => {
                let markers: Value = context.evaluate_pin("markers").await?;
                json!({
                    "type": "setGeoMapMarkers",
                    "markers": { "literalJson": flow_like_types::json::to_string(&markers)? }
                })
            }
            "Routes" => {
                let routes: Value = context.evaluate_pin("routes").await?;
                json!({
                    "type": "setGeoMapRoutes",
                    "routes": { "literalJson": flow_like_types::json::to_string(&routes)? }
                })
            }
            "Viewport" => {
                let viewport: GeoMapViewport = context.evaluate_pin("viewport").await?;
                let mut props = flow_like_types::json::Map::new();
                props.insert("latitude".to_string(), json!(viewport.latitude));
                props.insert("longitude".to_string(), json!(viewport.longitude));
                if let Some(z) = viewport.zoom {
                    props.insert("zoom".to_string(), json!(z));
                }
                if let Some(b) = viewport.bearing {
                    props.insert("bearing".to_string(), json!(b));
                }
                if let Some(p) = viewport.pitch {
                    props.insert("pitch".to_string(), json!(p));
                }
                json!({ "type": "setGeoMapViewport", "viewport": props })
            }
            _ => return Err(flow_like_types::anyhow!("Unknown property: {}", property)),
        };

        context.upsert_element(&element_id, update).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let property = node
            .get_pin_by_name("property")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Markers".to_string());

        let markers_pin = node.get_pin_by_name("markers").cloned();
        let routes_pin = node.get_pin_by_name("routes").cloned();
        let viewport_pin = node.get_pin_by_name("viewport").cloned();

        match property.as_str() {
            "Markers" => {
                remove_pin(node, routes_pin);
                remove_pin(node, viewport_pin);
                if markers_pin.is_none() {
                    node.add_input_pin(
                        "markers",
                        "Markers",
                        "Array of map markers",
                        VariableType::Struct,
                    )
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_schema::<GeoMapMarker>()
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
                }
            }
            "Routes" => {
                remove_pin(node, markers_pin);
                remove_pin(node, viewport_pin);
                if routes_pin.is_none() {
                    node.add_input_pin(
                        "routes",
                        "Routes",
                        "Array of map routes",
                        VariableType::Struct,
                    )
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_schema::<GeoMapRoute>()
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
                }
            }
            "Viewport" => {
                remove_pin(node, markers_pin);
                remove_pin(node, routes_pin);
                if viewport_pin.is_none() {
                    node.add_input_pin(
                        "viewport",
                        "Viewport",
                        "Map viewport configuration",
                        VariableType::Struct,
                    )
                    .set_schema::<GeoMapViewport>();
                }
            }
            _ => {}
        }
    }
}
