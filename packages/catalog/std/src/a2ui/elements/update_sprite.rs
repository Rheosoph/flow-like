use super::element_utils::extract_element_id;
use super::update_schemas::{SpriteTransform, Vec2};
use flow_like::a2ui::components::SpriteProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unified Sprite update node.
///
/// Update any property of a sprite element with a single node.
/// The input pins change dynamically based on the selected property type.
///
/// **Properties:**
/// - Source: URL string (image file)
/// - Position: x, y coordinates
/// - Transform: scale, rotation, opacity
#[crate::register_node]
#[derive(Default)]
pub struct UpdateSprite;

impl UpdateSprite {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateSprite {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_sprite",
            "Update Sprite",
            "Update any property of a sprite",
            "UI/Elements/Game",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Sprite",
            "Reference to the sprite element",
            VariableType::Struct,
        )
        .set_schema::<SpriteProps>()
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
                    "Position".to_string(),
                    "Transform".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Source")));

        // Default to Source input
        node.add_input_pin("src", "Source URL", "Image URL", VariableType::String);

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
            "Position" => {
                let pos: Vec2 = context.evaluate_pin("position").await?;
                json!({ "type": "setProps", "props": { "x": pos.x, "y": pos.y } })
            }
            "Transform" => {
                let transform: SpriteTransform = context.evaluate_pin("transform").await?;
                let mut props = flow_like_types::json::Map::new();
                if let Some(s) = transform.scale {
                    props.insert("scale".to_string(), json!(s));
                }
                if let Some(r) = transform.rotation {
                    props.insert("rotation".to_string(), json!(r));
                }
                if let Some(o) = transform.opacity {
                    props.insert("opacity".to_string(), json!(o));
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
        let position_pin = node.get_pin_by_name("position").cloned();
        let transform_pin = node.get_pin_by_name("transform").cloned();

        match property.as_str() {
            "Source" => {
                remove_pin(node, position_pin);
                remove_pin(node, transform_pin);
                if src_pin.is_none() {
                    node.add_input_pin("src", "Source URL", "Image URL", VariableType::String);
                }
            }
            "Position" => {
                remove_pin(node, src_pin);
                remove_pin(node, transform_pin);
                if position_pin.is_none() {
                    node.add_input_pin(
                        "position",
                        "Position",
                        "X and Y coordinates",
                        VariableType::Struct,
                    )
                    .set_schema::<Vec2>();
                }
            }
            "Transform" => {
                remove_pin(node, src_pin);
                remove_pin(node, position_pin);
                if transform_pin.is_none() {
                    node.add_input_pin(
                        "transform",
                        "Transform",
                        "Scale, rotation, and opacity",
                        VariableType::Struct,
                    )
                    .set_schema::<SpriteTransform>();
                }
            }
            _ => {}
        }
    }
}
