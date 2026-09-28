use crate::types::handles::AutomationSession;
use crate::types::screen_frame::ScreenFrame;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{FlowPath, NodeImage};
use flow_like_types::{async_trait, json::json};

fn add_frame_output(node: &mut Node, description: &str) {
    node.add_output_pin("frame", "Frame", description, VariableType::Struct)
        .set_schema::<ScreenFrame>();
}

#[cfg(feature = "execute")]
fn encode_png(image: &image::RgbaImage) -> flow_like_types::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    image
        .write_with_encoder(image::codecs::png::PngEncoder::new(&mut bytes))
        .map_err(|e| {
            flow_like_types::anyhow!(
                "Failed to encode {}x{} screenshot: {}",
                image.width(),
                image.height(),
                e
            )
        })?;
    Ok(bytes)
}

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
            "Captures a display and optionally saves it as PNG",
            "Automation/Vision",
        );
        node.set_version(2);
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

        add_frame_output(
            &mut node,
            "Desktop rectangle and pixel size of the display; converts image pixels to mouse coordinates",
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let file_path: Option<FlowPath> = context.evaluate_pin("file_path").await.ok();
        let monitor_index: i64 = context.evaluate_pin("monitor").await?;

        let encode = file_path.is_some();
        let (screenshot, frame, bytes) = tokio::task::spawn_blocking(move || {
            let (image, frame) = crate::types::screen_frame::capture_display(monitor_index)?;
            let bytes = if encode {
                Some(encode_png(&image)?)
            } else {
                None
            };
            flow_like_types::Ok((image, frame, bytes))
        })
        .await??;

        if let (Some(path), Some(bytes)) = (file_path, bytes) {
            path.put(context, bytes, false).await?;
        }

        context.set_pin_value("success", json!(true)).await?;
        let dyn_image = flow_like_types::image::DynamicImage::ImageRgba8(screenshot);
        let node_image = NodeImage::new(context, dyn_image).await;
        context.set_pin_value("image", json!(node_image)).await?;
        context.set_pin_value("frame", json!(frame)).await?;

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
            "Captures a region of a display, given in that display's screenshot pixels, and optionally saves it. Frame converts pixels of the result to mouse coordinates",
            "Automation/Vision",
        );
        node.set_version(2);
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

        node.add_input_pin(
            "x",
            "X",
            "Left edge in screenshot pixels of the display",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "y",
            "Y",
            "Top edge in screenshot pixels of the display",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "width",
            "Width",
            "Region width in screenshot pixels",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "height",
            "Height",
            "Region height in screenshot pixels",
            VariableType::Integer,
        )
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

        add_frame_output(
            &mut node,
            "Desktop rectangle and pixel size of the captured region",
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let monitor_index: i64 = context.evaluate_pin("monitor").await.unwrap_or(-1);
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let width: i64 = context.evaluate_pin("width").await?;
        let height: i64 = context.evaluate_pin("height").await?;
        let file_path: Option<FlowPath> = context.evaluate_pin("file_path").await.ok();

        let encode = file_path.is_some();
        let (cropped_image, frame, bytes) = tokio::task::spawn_blocking(move || {
            let (full_image, frame) = crate::types::screen_frame::capture_display(monitor_index)?;
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
                    "Region {}x{} at ({}, {}) must fit inside the {}x{} screenshot pixels of monitor {}",
                    width,
                    height,
                    x,
                    y,
                    full_image.width(),
                    full_image.height(),
                    monitor_index
                ));
            }
            let (x, y, width, height) = (x as u32, y as u32, width as u32, height as u32);
            let cropped = image::imageops::crop_imm(&full_image, x, y, width, height).to_image();
            let frame = frame.crop(x, y, width, height)?;
            let bytes = if encode {
                Some(encode_png(&cropped)?)
            } else {
                None
            };
            flow_like_types::Ok((cropped, frame, bytes))
        })
        .await??;

        if let (Some(path), Some(bytes)) = (file_path, bytes) {
            path.put(context, bytes, false).await?;
        }
        context.set_pin_value("success", json!(true)).await?;

        let dyn_image = flow_like_types::image::DynamicImage::ImageRgba8(cropped_image);
        let node_image = NodeImage::new(context, dyn_image).await;
        context.set_pin_value("image", json!(node_image)).await?;
        context.set_pin_value("frame", json!(frame)).await?;

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
            "Gets the color at a screen position. By default X/Y are desktop coordinates, the same ones the mouse nodes and Assert Color use; set Coordinate Space to pixels for the version 1 behaviour (screenshot pixels of Monitor)",
            "Automation/Vision",
        );
        node.set_version(2);
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
            "coordinate_space",
            "Coordinate Space",
            "desktop: X/Y are desktop input coordinates on any display (Monitor is ignored). pixels: X/Y are screenshot pixels of Monitor",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["desktop".to_string(), "pixels".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("desktop")));

        node.add_input_pin(
            "monitor",
            "Monitor",
            "Monitor index from List Displays (-1 = primary); only used when Coordinate Space is pixels",
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
        context.deactivate_exec_pin("exec_out").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        _session.ensure_active(context).await?;
        let monitor_index: i64 = context.evaluate_pin("monitor").await.unwrap_or(-1);
        let space: String = context
            .evaluate_pin("coordinate_space")
            .await
            .unwrap_or_else(|_| "pixels".to_string());
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;

        let [r, g, b] = tokio::task::spawn_blocking(move || match space.as_str() {
            "desktop" => crate::types::screen_match::capture_pixel(x, y),
            "pixels" => {
                let (image, _) = crate::types::screen_frame::capture_display(monitor_index)?;
                if x < 0 || y < 0 || x >= i64::from(image.width()) || y >= i64::from(image.height())
                {
                    return Err(flow_like_types::anyhow!(
                        "Pixel ({}, {}) is outside the {}x{} screenshot of monitor {}",
                        x,
                        y,
                        image.width(),
                        image.height(),
                        monitor_index
                    ));
                }
                let pixel = image.get_pixel(x as u32, y as u32);
                Ok([pixel[0], pixel[1], pixel[2]])
            }
            other => Err(flow_like_types::anyhow!(
                "Unknown coordinate space '{}'; use desktop or pixels",
                other
            )),
        })
        .await??;

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
            "Gets the size of a monitor in desktop coordinates (as Get Display reports it) and in screenshot pixels",
            "Automation/Vision",
        );
        node.set_version(2);
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

        node.add_output_pin(
            "width",
            "Width",
            "Width in desktop input coordinates",
            VariableType::Integer,
        );
        node.add_output_pin(
            "height",
            "Height",
            "Height in desktop input coordinates",
            VariableType::Integer,
        );
        node.add_output_pin(
            "pixel_width",
            "Pixel Width",
            "Width of a screenshot of this monitor in pixels",
            VariableType::Integer,
        );
        node.add_output_pin(
            "pixel_height",
            "Pixel Height",
            "Height of a screenshot of this monitor in pixels",
            VariableType::Integer,
        );
        add_frame_output(
            &mut node,
            "Desktop rectangle and screenshot pixel size of the monitor",
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let monitor_index: i64 = context.evaluate_pin("monitor").await?;

        let frame = tokio::task::spawn_blocking(move || {
            let monitors = xcap::Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate displays: {}", e))?;
            let (position, monitor) =
                crate::types::screen_frame::select_monitor(&monitors, monitor_index)?;
            crate::types::screen_frame::monitor_frame(monitor, Some(position as u32))
        })
        .await??;

        context
            .set_pin_value("width", json!(frame.width as i64))
            .await?;
        context
            .set_pin_value("height", json!(frame.height as i64))
            .await?;
        context
            .set_pin_value("pixel_width", json!(frame.pixel_width as i64))
            .await?;
        context
            .set_pin_value("pixel_height", json!(frame.pixel_height as i64))
            .await?;
        context.set_pin_value("frame", json!(frame)).await?;
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
