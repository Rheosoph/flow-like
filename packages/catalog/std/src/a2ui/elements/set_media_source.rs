use super::element_utils::extract_element_id;
use super::update_schemas::{AvatarSource, ImageSource, VideoSource};
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};
use std::sync::Arc;

/// Unified media source setter node.
///
/// Set the source URL for various media elements with a single node.
/// The input pins change dynamically based on the selected media type.
///
/// **Media Types:**
/// - Image: src + alt text
/// - Avatar: src + fallback text
/// - Video: src + poster image
/// - Lottie: animation JSON URL
/// - Iframe: page URL
#[crate::register_node]
#[derive(Default)]
pub struct SetMediaSource;

impl SetMediaSource {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for SetMediaSource {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_set_media_source",
            "Set Media Source",
            "Set source URL for image, video, avatar, lottie, or iframe elements",
            "UI/Elements/Media",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the media element",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "media_type",
            "Media Type",
            "Type of media element",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Image".to_string(),
                    "Avatar".to_string(),
                    "Video".to_string(),
                    "Lottie".to_string(),
                    "Iframe".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Image")));

        // Default: Image pins
        node.add_input_pin(
            "image",
            "Image",
            "Image source and alt",
            VariableType::Struct,
        )
        .set_schema::<ImageSource>();

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let media_type: String = context.evaluate_pin("media_type").await?;

        let update = match media_type.as_str() {
            "Image" => {
                let image: ImageSource = context.evaluate_pin("image").await?;
                let mut u = json!({
                    "type": "setImageSrc",
                    "src": image.src
                });
                if let Some(alt) = image.alt {
                    u["alt"] = json!(alt);
                }
                u
            }
            "Avatar" => {
                let avatar: AvatarSource = context.evaluate_pin("avatar").await?;
                let mut u = json!({
                    "type": "setAvatarSrc",
                    "src": avatar.src
                });
                if let Some(fallback) = avatar.fallback {
                    u["fallback"] = json!(fallback);
                }
                u
            }
            "Video" => {
                let video: VideoSource = context.evaluate_pin("video").await?;
                let mut u = json!({
                    "type": "setVideoSrc",
                    "src": video.src
                });
                if let Some(poster) = video.poster {
                    u["poster"] = json!(poster);
                }
                u
            }
            "Lottie" => {
                let src: String = context.evaluate_pin("src").await?;
                json!({
                    "type": "setLottieSrc",
                    "src": src
                })
            }
            "Iframe" => {
                let src: String = context.evaluate_pin("src").await?;
                json!({
                    "type": "setIframeSrc",
                    "src": src
                })
            }
            _ => {
                return Err(flow_like_types::anyhow!(
                    "Unknown media type: {}",
                    media_type
                ));
            }
        };

        context.upsert_element(&element_id, update).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let media_type = node
            .get_pin_by_name("media_type")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Image".to_string());

        // Remove dynamic pins
        let pins_to_check = ["image", "avatar", "video", "src"];
        for pin_name in pins_to_check {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match media_type.as_str() {
            "Image" => {
                node.add_input_pin(
                    "image",
                    "Image",
                    "Image source and alt",
                    VariableType::Struct,
                )
                .set_schema::<ImageSource>();
            }
            "Avatar" => {
                node.add_input_pin(
                    "avatar",
                    "Avatar",
                    "Avatar source and fallback",
                    VariableType::Struct,
                )
                .set_schema::<AvatarSource>();
            }
            "Video" => {
                node.add_input_pin(
                    "video",
                    "Video",
                    "Video source and poster",
                    VariableType::Struct,
                )
                .set_schema::<VideoSource>();
            }
            "Lottie" => {
                node.add_input_pin("src", "URL", "Lottie animation URL", VariableType::String);
            }
            "Iframe" => {
                node.add_input_pin("src", "URL", "Iframe page URL", VariableType::String);
            }
            _ => {}
        }
    }
}
