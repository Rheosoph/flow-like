use crate::types::artifacts::{ArtifactRef, ArtifactType};
use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, NodeImage};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::{async_trait, create_id, json::json};

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
            "Takes a screenshot of the screen, window, or region",
            "Automation/Computer/Capture",
        );
        node.set_version(1);
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
            "What to capture: full screen, specific display, or region",
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
            "Index of display for display or region capture",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "region_x",
            "Region X",
            "X coordinate of region (when capture_type=region)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "region_y",
            "Region Y",
            "Y coordinate of region",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "region_width",
            "Region Width",
            "Width of region",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "region_height",
            "Region Height",
            "Height of region",
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

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_storage::object_store::PutPayload;
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let capture_type: String = context.evaluate_pin("capture_type").await?;
        let display_index: i64 = context.evaluate_pin("display_index").await?;
        let region_x: i64 = context.evaluate_pin("region_x").await?;
        let region_y: i64 = context.evaluate_pin("region_y").await?;
        let region_width: i64 = context.evaluate_pin("region_width").await?;
        let region_height: i64 = context.evaluate_pin("region_height").await?;

        let image = {
            let monitors = Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;

            match capture_type.as_str() {
                "display" => {
                    let monitor = monitors.get(display_index as usize).ok_or_else(|| {
                        flow_like_types::anyhow!("Display index {} not found", display_index)
                    })?;
                    crate::types::screen_match::capture_monitor(&monitor)
                        .map_err(|e| flow_like_types::anyhow!("Failed to capture display: {}", e))?
                }
                "region" => {
                    let monitor = monitors.get(display_index as usize).ok_or_else(|| {
                        flow_like_types::anyhow!("Display index {} not found", display_index)
                    })?;
                    let full_image = crate::types::screen_match::capture_monitor(&monitor)
                        .map_err(|e| flow_like_types::anyhow!("Failed to capture screen: {}", e))?;

                    let img_w = full_image.width();
                    let img_h = full_image.height();
                    if region_x < 0
                        || region_y < 0
                        || region_width <= 0
                        || region_height <= 0
                        || region_x
                            .checked_add(region_width)
                            .is_none_or(|end| end > img_w as i64)
                        || region_y
                            .checked_add(region_height)
                            .is_none_or(|end| end > img_h as i64)
                    {
                        return Err(flow_like_types::anyhow!(
                            "Capture region must fit within the selected display's screenshot pixels"
                        ));
                    }
                    let (x, y, w, h) = (
                        region_x as u32,
                        region_y as u32,
                        region_width as u32,
                        region_height as u32,
                    );

                    let cropped = image::imageops::crop_imm(&full_image, x, y, w, h);
                    cropped.to_image()
                }
                _ => {
                    let monitor = monitors
                        .iter()
                        .find(|m| m.is_primary().unwrap_or(false))
                        .or_else(|| monitors.first())
                        .ok_or_else(|| flow_like_types::anyhow!("No monitors found"))?;
                    crate::types::screen_match::capture_monitor(&monitor)
                        .map_err(|e| flow_like_types::anyhow!("Failed to capture screen: {}", e))?
                }
            }
        };

        let mut buffer = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut buffer);
        image
            .write_with_encoder(encoder)
            .map_err(|e| flow_like_types::anyhow!("Failed to encode screenshot: {}", e))?;

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
