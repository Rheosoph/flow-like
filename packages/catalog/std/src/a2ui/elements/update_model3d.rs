use super::element_utils::extract_element_id;
use super::update_schemas::{Model3dAnimation, Model3dTransform};
use flow_like::a2ui::components::Model3dProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unified Model3D update node.
///
/// Update any property of a 3D model element with a single node.
/// The input pins change dynamically based on the selected property type.
///
/// **Properties:**
/// - Source: URL string (GLTF/GLB file)
/// - Transform: position, rotation, scale
/// - Animation: name, playing, loop, speed
#[crate::register_node]
#[derive(Default)]
pub struct UpdateModel3d;

impl UpdateModel3d {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateModel3d {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_model3d",
            "Update Model3D",
            "Update any property of a 3D model",
            "UI/Elements/Game",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Model3D",
            "Reference to the 3D model element",
            VariableType::Struct,
        )
        .set_schema::<Model3dProps>()
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
                    "Source".to_string(),
                    "Transform".to_string(),
                    "Animation".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Source")));

        // Default to Source input
        node.add_input_pin(
            "src",
            "Source URL",
            "GLTF/GLB model URL",
            VariableType::String,
        );

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
            "Source" => {
                let src: String = context.evaluate_pin("src").await?;
                json!({ "type": "setProps", "props": { "src": src } })
            }
            "Transform" => {
                let transform: Model3dTransform = context.evaluate_pin("transform").await?;
                let mut props = flow_like_types::json::Map::new();
                if let Some(pos) = transform.position {
                    props.insert("position".to_string(), json!([pos.x, pos.y, pos.z]));
                }
                if let Some(rot) = transform.rotation {
                    props.insert("rotation".to_string(), json!([rot.x, rot.y, rot.z]));
                }
                if let Some(scale) = transform.scale {
                    props.insert("scale".to_string(), json!(scale));
                }
                json!({ "type": "setProps", "props": props })
            }
            "Animation" => {
                let anim: Model3dAnimation = context.evaluate_pin("animation").await?;
                let mut props = flow_like_types::json::Map::new();
                if let Some(name) = anim.name {
                    props.insert("animation".to_string(), json!(name));
                }
                if let Some(playing) = anim.playing {
                    props.insert("animationPlaying".to_string(), json!(playing));
                }
                if let Some(loop_anim) = anim.loop_anim {
                    props.insert("animationLoop".to_string(), json!(loop_anim));
                }
                if let Some(speed) = anim.speed {
                    props.insert("animationSpeed".to_string(), json!(speed));
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
            .unwrap_or_else(|| "Source".to_string());

        let src_pin = node.get_pin_by_name("src").cloned();
        let transform_pin = node.get_pin_by_name("transform").cloned();
        let animation_pin = node.get_pin_by_name("animation").cloned();

        match property.as_str() {
            "Source" => {
                remove_pin(node, transform_pin);
                remove_pin(node, animation_pin);
                if src_pin.is_none() {
                    node.add_input_pin(
                        "src",
                        "Source URL",
                        "GLTF/GLB model URL",
                        VariableType::String,
                    );
                }
            }
            "Transform" => {
                remove_pin(node, src_pin);
                remove_pin(node, animation_pin);
                if transform_pin.is_none() {
                    node.add_input_pin(
                        "transform",
                        "Transform",
                        "Position, rotation, and scale",
                        VariableType::Struct,
                    )
                    .set_schema::<Model3dTransform>();
                }
            }
            "Animation" => {
                remove_pin(node, src_pin);
                remove_pin(node, transform_pin);
                if animation_pin.is_none() {
                    node.add_input_pin(
                        "animation",
                        "Animation",
                        "Animation configuration",
                        VariableType::Struct,
                    )
                    .set_schema::<Model3dAnimation>();
                }
            }
            _ => {}
        }
    }
}
