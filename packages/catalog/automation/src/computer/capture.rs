use crate::types::artifacts::ArtifactRef;
#[cfg(feature = "execute")]
use crate::types::artifacts::ArtifactType;
use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
#[cfg(feature = "execute")]
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_storage::object_store::ObjectStoreExt;
#[cfg(feature = "execute")]
use flow_like_types::create_id;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ComputerScreenshotNode {}

impl ComputerScreenshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerScreenshotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_screenshot",
            "Screenshot",
            "Takes a screenshot of the primary display, a chosen display, or a region of one display. Frame maps image pixels back to the desktop coordinates the mouse nodes use",
            "Automation/Computer/Capture",
        );
        node.set_version(2);
        node.set_flowscript_name("computer", "screenshot");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "capture_type",
            "Capture Type",
            "full: the primary display only. display: the display at Display Index. region: a rectangle of one display",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec![
                    "full".to_string(),
                    "display".to_string(),
                    "region".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("full")));

        node.add_input_pin(
            "display_index",
            "Display Index",
            "Display for display capture and pixel-space region capture: index from List Displays, -1 = primary",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "region_space",
            "Region Space",
            "pixels: region is in screenshot pixels of the display at Display Index (Retina displays have twice as many pixels as desktop units). desktop: region is in desktop input coordinates, like the mouse nodes, and may be on any display",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec!["pixels".to_string(), "desktop".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("pixels")));

        node.add_input_pin(
            "region_x",
            "Region X",
            "Left edge of the region (capture_type=region), in Region Space units",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "region_y",
            "Region Y",
            "Top edge of the region, in Region Space units",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "region_width",
            "Region Width",
            "Width of the region, in Region Space units",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "region_height",
            "Region Height",
            "Height of the region, in Region Space units",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "screenshot",
            "Screenshot",
            "Reference to the captured screenshot",
            VariableType::Struct,
        )
        .set_schema::<ArtifactRef>();

        node.add_output_pin(
            "image",
            "Image",
            "Screenshot as NodeImage",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        node.add_output_pin(
            "frame",
            "Frame",
            "Desktop rectangle and pixel size of the screenshot; converts image pixels to mouse coordinates",
            VariableType::Struct,
        )
        .set_schema::<ScreenFrame>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::object_store::PutPayload;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let capture_type: String = context.evaluate_pin("capture_type").await?;
        let display_index: i64 = context.evaluate_pin("display_index").await?;
        let region_space: String = context
            .evaluate_pin("region_space")
            .await
            .unwrap_or_else(|_| "pixels".to_string());
        let region = (
            context.evaluate_pin::<i64>("region_x").await?,
            context.evaluate_pin::<i64>("region_y").await?,
            context.evaluate_pin::<i64>("region_width").await?,
            context.evaluate_pin::<i64>("region_height").await?,
        );

        let (image, frame, buffer) = tokio::task::spawn_blocking(move || {
            let (image, frame) =
                capture_screenshot(&capture_type, display_index, &region_space, region)?;
            let mut buffer = Vec::new();
            image
                .write_with_encoder(image::codecs::png::PngEncoder::new(&mut buffer))
                .map_err(|e| flow_like_types::anyhow!("Failed to encode screenshot: {}", e))?;
            flow_like_types::Ok((image, frame, buffer))
        })
        .await??;

        let artifact_id = create_id();
        let path = format!("artifacts/screenshots/{}.png", artifact_id);

        let exec_cache = context
            .execution_cache
            .clone()
            .ok_or_else(|| flow_like_types::anyhow!("Execution cache not available"))?;
        let store = exec_cache
            .stores
            .temporary_store
            .as_ref()
            .ok_or_else(|| flow_like_types::anyhow!("Temporary store not configured"))?;
        let store_generic = store.as_generic();
        let payload = PutPayload::from_bytes(buffer.into());
        store_generic
            .put(&flow_like_storage::Path::from(path.clone()), payload)
            .await?;

        let store_ref = format!("automation_screenshot_store_{}", artifact_id);
        context
            .set_cache(&store_ref, std::sync::Arc::new(store.clone()))
            .await;
        let flow_path = FlowPath::new(path, store_ref, None);
        let artifact = ArtifactRef::new(
            artifact_id,
            ArtifactType::Screenshot,
            flow_path,
            "image/png",
        );

        // Create NodeImage from the captured image
        let dyn_image = flow_like_types::image::DynamicImage::ImageRgba8(image);
        let node_image = NodeImage::new(context, dyn_image).await;

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("screenshot", json!(artifact)).await?;
        context.set_pin_value("image", json!(node_image)).await?;
        context.set_pin_value("frame", json!(frame)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn capture_screenshot(
    capture_type: &str,
    display_index: i64,
    region_space: &str,
    (x, y, width, height): (i64, i64, i64, i64),
) -> flow_like_types::Result<(image::RgbaImage, ScreenFrame)> {
    use crate::types::screen_frame::{capture_display, capture_input_region};
    match capture_type {
        "full" => capture_display(-1),
        "display" => capture_display(display_index),
        "region" => {
            if width <= 0 || height <= 0 {
                return Err(flow_like_types::anyhow!(
                    "Capture region size {}x{} must be positive",
                    width,
                    height
                ));
            }
            match region_space {
                "desktop" => capture_input_region(
                    None,
                    i32::try_from(x)?,
                    i32::try_from(y)?,
                    u32::try_from(width)?,
                    u32::try_from(height)?,
                ),
                "pixels" => {
                    let (full, frame) = capture_display(display_index)?;
                    let fits = x >= 0
                        && y >= 0
                        && x.checked_add(width)
                            .is_some_and(|end| end <= full.width() as i64)
                        && y.checked_add(height)
                            .is_some_and(|end| end <= full.height() as i64);
                    if !fits {
                        return Err(flow_like_types::anyhow!(
                            "Capture region {}x{} at ({}, {}) must fit the {}x{} screenshot pixels of display {}; set Region Space to desktop for mouse coordinates",
                            width,
                            height,
                            x,
                            y,
                            full.width(),
                            full.height(),
                            display_index
                        ));
                    }
                    let (x, y, width, height) = (x as u32, y as u32, width as u32, height as u32);
                    let cropped = image::imageops::crop_imm(&full, x, y, width, height).to_image();
                    Ok((cropped, frame.crop(x, y, width, height)?))
                }
                other => Err(flow_like_types::anyhow!(
                    "Unknown region space '{}'; use pixels or desktop",
                    other
                )),
            }
        }
        other => Err(flow_like_types::anyhow!(
            "Unknown capture type '{}'; use full, display or region",
            other
        )),
    }
}
