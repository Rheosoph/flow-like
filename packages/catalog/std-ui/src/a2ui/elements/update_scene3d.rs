use super::element_utils::extract_element_id;
use super::update_schemas::{Scene3dBackground, Scene3dCamera, Scene3dControls, Scene3dLighting};
use flow_like::a2ui::components::Scene3dProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unified Scene3D update node.
///
/// Update any property of a 3D scene element with a single node.
/// The input pins change dynamically based on the selected property type.
///
/// **Properties:**
/// - Camera: type, position, lookAt
/// - Background: color, environment preset
/// - Lighting: ambient intensity, directional intensity and position
/// - Controls: enabled, autoRotate, enableZoom, enablePan
#[crate::register_node]
#[derive(Default)]
pub struct UpdateScene3d;

impl UpdateScene3d {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateScene3d {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_scene3d",
            "Update Scene3D",
            "Update any property of a 3D scene",
            "UI/Elements/Game",
        );
        node.set_flowscript_name("ui", "updateScene3d");
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Scene3D",
            "Reference to the 3D scene element",
            VariableType::Struct,
        )
        .set_schema::<Scene3dProps>()
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
                    "Camera".to_string(),
                    "Background".to_string(),
                    "Lighting".to_string(),
                    "Controls".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Camera")));

        // Default to Camera input
        node.add_input_pin(
            "camera",
            "Camera",
            "Camera type, position, and target",
            VariableType::Struct,
        )
        .set_schema::<Scene3dCamera>();

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
            "Camera" => {
                let camera: Scene3dCamera = context.evaluate_pin("camera").await?;
                let mut props = flow_like_types::json::Map::new();
                if let Some(t) = camera.camera_type {
                    props.insert("cameraType".to_string(), json!(t));
                }
                if let Some(pos) = camera.position {
                    props.insert("cameraPosition".to_string(), json!([pos.x, pos.y, pos.z]));
                }
                if let Some(look) = camera.look_at {
                    props.insert("target".to_string(), json!([look.x, look.y, look.z]));
                }
                json!({ "type": "setProps", "props": props })
            }
            "Background" => {
                let bg: Scene3dBackground = context.evaluate_pin("background").await?;
                let mut props = flow_like_types::json::Map::new();
                if let Some(c) = bg.color {
                    props.insert("backgroundColor".to_string(), json!(c));
                }
                if let Some(e) = bg.environment {
                    props.insert("environment".to_string(), json!(e));
                }
                json!({ "type": "setProps", "props": props })
            }
            "Lighting" => {
                let light: Scene3dLighting = context.evaluate_pin("lighting").await?;
                let mut props = flow_like_types::json::Map::new();
                if let Some(a) = light.ambient_intensity {
                    props.insert("ambientLight".to_string(), json!(a));
                }
                if let Some(d) = light.directional_intensity {
                    props.insert("directionalLight".to_string(), json!(d));
                }
                if let Some(pos) = light.directional_position {
                    props.insert(
                        "directionalLightPosition".to_string(),
                        json!([pos.x, pos.y, pos.z]),
                    );
                }
                json!({ "type": "setProps", "props": props })
            }
            "Controls" => {
                let ctrl: Scene3dControls = context.evaluate_pin("controls").await?;
                let mut props = flow_like_types::json::Map::new();
                if let Some(e) = ctrl.enabled {
                    props.insert("enableControls".to_string(), json!(e));
                }
                if let Some(a) = ctrl.auto_rotate {
                    props.insert("autoRotate".to_string(), json!(a));
                }
                if let Some(z) = ctrl.enable_zoom {
                    props.insert("enableZoom".to_string(), json!(z));
                }
                if let Some(p) = ctrl.enable_pan {
                    props.insert("enablePan".to_string(), json!(p));
                }
                json!({ "type": "setProps", "props": props })
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
            .unwrap_or_else(|| "Camera".to_string());

        let camera_pin = node.get_pin_by_name("camera").cloned();
        let background_pin = node.get_pin_by_name("background").cloned();
        let lighting_pin = node.get_pin_by_name("lighting").cloned();
        let controls_pin = node.get_pin_by_name("controls").cloned();

        match property.as_str() {
            "Camera" => {
                remove_pin(node, background_pin);
                remove_pin(node, lighting_pin);
                remove_pin(node, controls_pin);
                if camera_pin.is_none() {
                    node.add_input_pin(
                        "camera",
                        "Camera",
                        "Camera type, position, and target",
                        VariableType::Struct,
                    )
                    .set_schema::<Scene3dCamera>();
                }
            }
            "Background" => {
                remove_pin(node, camera_pin);
                remove_pin(node, lighting_pin);
                remove_pin(node, controls_pin);
                if background_pin.is_none() {
                    node.add_input_pin(
                        "background",
                        "Background",
                        "Background color and environment",
                        VariableType::Struct,
                    )
                    .set_schema::<Scene3dBackground>();
                }
            }
            "Lighting" => {
                remove_pin(node, camera_pin);
                remove_pin(node, background_pin);
                remove_pin(node, controls_pin);
                if lighting_pin.is_none() {
                    node.add_input_pin(
                        "lighting",
                        "Lighting",
                        "Ambient and directional light settings",
                        VariableType::Struct,
                    )
                    .set_schema::<Scene3dLighting>();
                }
            }
            "Controls" => {
                remove_pin(node, camera_pin);
                remove_pin(node, background_pin);
                remove_pin(node, lighting_pin);
                if controls_pin.is_none() {
                    node.add_input_pin(
                        "controls",
                        "Controls",
                        "Control settings (zoom, pan, rotate)",
                        VariableType::Struct,
                    )
                    .set_schema::<Scene3dControls>();
                }
            }
            _ => {}
        }
    }
}
