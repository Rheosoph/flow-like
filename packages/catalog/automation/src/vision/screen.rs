use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, NodeImage};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ScreenshotToFileNode {}

impl ScreenshotToFileNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ScreenshotToFileNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "vision_screenshot_to_file",
            "Screenshot To File",
            "Captures a screenshot and saves it to a file",
            "Automation/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("automation.vision", "screenshotToFile");
        node.add_icon("/flow/icons/vision.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(8)
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
            "Automation session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to save the screenshot",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "monitor",
            "Monitor",
            "Monitor index from List Displays (-1 = primary)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "success",
            "Success",
            "Whether the screenshot was saved",
            VariableType::Boolean,
        );

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
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let file_path: Option<FlowPath> = context.evaluate_pin("file_path").await.ok();
        let monitor_index: i64 = context.evaluate_pin("monitor").await?;

        let screenshot = {
            let monitors = Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;
            let monitor = select_monitor(&monitors, monitor_index)?;
            crate::types::screen_match::capture_monitor(&monitor)
                .map_err(|e| flow_like_types::anyhow!("Failed to capture screen: {}", e))?
        };

        let success = if let Some(path) = file_path {
            let mut bytes = Vec::new();
            screenshot.write_with_encoder(image::codecs::png::PngEncoder::new(&mut bytes))?;
            path.put(context, bytes, false).await?;
            true
        } else {
            true
        };

        context.set_pin_value("success", json!(success)).await?;

        // Create NodeImage from the screenshot
        let dyn_image = flow_like_types::image::DynamicImage::ImageRgba8(screenshot);
        let node_image = NodeImage::new(context, dyn_image).await;
        context.set_pin_value("image", json!(node_image)).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Vision automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ScreenshotRegionNode {}

impl ScreenshotRegionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ScreenshotRegionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "vision_screenshot_region",
            "Screenshot Region",
            "Captures a region of the screen and saves it",
            "Automation/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("automation.vision", "screenshotRegion");
        node.add_icon("/flow/icons/vision.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(8)
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
            "Automation session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("x", "X", "Left position", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Top position", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("width", "Width", "Region width", VariableType::Integer)
            .set_default_value(Some(json!(100)));

        node.add_input_pin("height", "Height", "Region height", VariableType::Integer)
            .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to save the screenshot",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "monitor",
            "Monitor",
            "Monitor index from List Displays (-1 = primary); coordinates are screenshot pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "success",
            "Success",
            "Whether the screenshot was saved",
            VariableType::Boolean,
        );

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
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let monitor_index: i64 = context.evaluate_pin("monitor").await.unwrap_or(-1);
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let width: i64 = context.evaluate_pin("width").await?;
        let height: i64 = context.evaluate_pin("height").await?;
        let file_path: Option<FlowPath> = context.evaluate_pin("file_path").await.ok();

        let cropped_image = {
            let monitors = Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;
            let monitor = select_monitor(&monitors, monitor_index)?;
            let full_image = crate::types::screen_match::capture_monitor(&monitor)
                .map_err(|e| flow_like_types::anyhow!("Failed to capture screen: {}", e))?;

            if x < 0
                || y < 0
                || width <= 0
                || height <= 0
                || x.checked_add(width)
                    .is_none_or(|end| end > full_image.width() as i64)
                || y.checked_add(height)
                    .is_none_or(|end| end > full_image.height() as i64)
            {
                return Err(flow_like_types::anyhow!(
                    "Region must fit inside the selected display's screenshot pixels"
                ));
            }

            let cropped = image::imageops::crop_imm(
                &full_image,
                x as u32,
                y as u32,
                width as u32,
                height as u32,
            );
            cropped.to_image()
        };

        let success = if let Some(path) = file_path {
            let mut bytes = Vec::new();
            cropped_image.write_with_encoder(image::codecs::png::PngEncoder::new(&mut bytes))?;
            path.put(context, bytes, false).await?;
            true
        } else {
            true
        };
        context.set_pin_value("success", json!(success)).await?;

        // Create NodeImage from the cropped region
        let dyn_image = flow_like_types::image::DynamicImage::ImageRgba8(cropped_image);
        let node_image = NodeImage::new(context, dyn_image).await;
        context.set_pin_value("image", json!(node_image)).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Vision automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetPixelColorNode {}

impl GetPixelColorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetPixelColorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "vision_get_pixel_color",
            "Get Pixel Color",
            "Gets the color of a pixel at a screen position",
            "Automation/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("automation.vision", "getPixelColor");
        node.add_icon("/flow/icons/vision.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(9)
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
            "Automation session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("x", "X", "X position", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Y position", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "monitor",
            "Monitor",
            "Monitor index from List Displays (-1 = primary); coordinates are screenshot pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));
        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin("red", "Red", "Red component (0-255)", VariableType::Integer);
        node.add_output_pin(
            "green",
            "Green",
            "Green component (0-255)",
            VariableType::Integer,
        );
        node.add_output_pin(
            "blue",
            "Blue",
            "Blue component (0-255)",
            VariableType::Integer,
        );
        node.add_output_pin(
            "hex",
            "Hex",
            "Hex color code (#RRGGBB)",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let monitor_index: i64 = context.evaluate_pin("monitor").await.unwrap_or(-1);
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;

        let (r, g, b) = {
            let monitors = Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;
            let monitor = select_monitor(&monitors, monitor_index)?;
            let image = crate::types::screen_match::capture_monitor(&monitor)
                .map_err(|e| flow_like_types::anyhow!("Failed to capture screen: {}", e))?;

            if x < 0 || y < 0 || x >= i64::from(image.width()) || y >= i64::from(image.height()) {
                return Err(flow_like_types::anyhow!("Pixel position out of bounds"));
            }

            let pixel = image.get_pixel(x as u32, y as u32);
            (pixel[0], pixel[1], pixel[2])
        };

        let hex = format!("#{:02X}{:02X}{:02X}", r, g, b);

        context.set_pin_value("red", json!(r as i64)).await?;
        context.set_pin_value("green", json!(g as i64)).await?;
        context.set_pin_value("blue", json!(b as i64)).await?;
        context.set_pin_value("hex", json!(hex)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Vision automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetScreenSizeNode {}

impl GetScreenSizeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetScreenSizeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "vision_get_screen_size",
            "Get Screen Size",
            "Gets the dimensions of a monitor",
            "Automation/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("automation.vision", "getScreenSize");
        node.add_icon("/flow/icons/vision.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(4)
                .set_security(6)
                .set_performance(10)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "monitor",
            "Monitor",
            "Monitor index from List Displays (-1 = primary)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin("width", "Width", "Screen width", VariableType::Integer);
        node.add_output_pin("height", "Height", "Screen height", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let monitor_index: i64 = context.evaluate_pin("monitor").await?;

        let monitors = xcap::Monitor::all()?;
        let monitor = select_monitor(&monitors, monitor_index)?;
        let (width, height) = (monitor.width()?, monitor.height()?);

        context.set_pin_value("width", json!(width as i64)).await?;
        context
            .set_pin_value("height", json!(height as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Vision automation requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn select_monitor(
    monitors: &[xcap::Monitor],
    index: i64,
) -> flow_like_types::Result<&xcap::Monitor> {
    if index == -1 {
        monitors
            .iter()
            .find(|m| m.is_primary().unwrap_or(false))
            .or_else(|| monitors.first())
    } else {
        usize::try_from(index).ok().and_then(|i| monitors.get(i))
    }
    .ok_or_else(|| flow_like_types::anyhow!("Monitor index {} is unavailable", index))
}
