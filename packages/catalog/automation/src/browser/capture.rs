use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
#[cfg(any(feature = "execute", test))]
use flow_like_types::Value;
use flow_like_types::{async_trait, json::json};

/// Clip covering the whole scrollable document, from `Page.getLayoutMetrics`.
#[cfg(any(feature = "execute", test))]
pub(crate) fn full_page_clip(metrics: &Value) -> flow_like_types::Result<Value> {
    let size = metrics
        .get("cssContentSize")
        .filter(|size| size.is_object())
        .or_else(|| metrics.get("contentSize"))
        .ok_or_else(|| {
            flow_like_types::anyhow!("Page.getLayoutMetrics returned no content size: {metrics}")
        })?;
    let dimension = |name: &str| {
        size[name]
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 1.0)
            .map(f64::ceil)
            .ok_or_else(|| {
                flow_like_types::anyhow!(
                    "Page.getLayoutMetrics returned an unusable content {name}: {size}"
                )
            })
    };
    Ok(json!({
        "x": 0,
        "y": 0,
        "width": dimension("width")?,
        "height": dimension("height")?,
        "scale": 1,
    }))
}

/// `Page.captureScreenshot` clip JSON (`x`, `y`, `width`, `height`, `scale`) as a typed clip.
#[cfg(any(feature = "execute", test))]
pub(crate) fn screenshot_clip(
    clip: &Value,
) -> flow_like_types::Result<flow_like_browser::output::Clip> {
    let field = |name: &str| {
        clip[name]
            .as_f64()
            .filter(|value| value.is_finite())
            .ok_or_else(|| flow_like_types::anyhow!("Screenshot clip has no finite {name}: {clip}"))
    };
    Ok(flow_like_browser::output::Clip {
        x: field("x")?,
        y: field("y")?,
        width: field("width")?,
        height: field("height")?,
        scale: field("scale")?,
    })
}

#[cfg(feature = "execute")]
pub(crate) async fn capture_png_via_cdp(
    ctx: &super::driver::PageContext,
    clip: &Value,
    capture_beyond_viewport: bool,
) -> flow_like_types::Result<Vec<u8>> {
    let options = flow_like_browser::output::ScreenshotOptions {
        clip: Some(screenshot_clip(clip)?),
        capture_beyond_viewport,
    };
    ctx.page
        .screenshot(options)
        .await
        .map_err(|e| flow_like_types::anyhow!("Failed to capture screenshot of clip {clip}: {e}"))
}

/// Writes the `screenshot` (base64 PNG) and `image` (NodeImage) outputs.
#[cfg(feature = "execute")]
pub(crate) async fn set_screenshot_outputs(
    context: &mut ExecutionContext,
    png: Vec<u8>,
) -> flow_like_types::Result<()> {
    use flow_like_types::base64::Engine;
    let (encoded, image) = tokio::task::spawn_blocking(move || {
        let image = flow_like_types::image::load_from_memory(&png)
            .map_err(|e| flow_like_types::anyhow!("Failed to decode screenshot PNG: {e}"))?;
        let encoded = flow_like_types::base64::engine::general_purpose::STANDARD.encode(&png);
        Ok::<_, flow_like_types::Error>((encoded, image))
    })
    .await??;
    let node_image = NodeImage::new(context, image).await;
    context.set_pin_value("screenshot", json!(encoded)).await?;
    context.set_pin_value("image", json!(node_image)).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_page_clip_uses_css_content_size() {
        let clip = full_page_clip(&json!({
            "cssContentSize": {"x": 0, "y": 0, "width": 1280.4, "height": 5000},
            "contentSize": {"x": 0, "y": 0, "width": 2561, "height": 10000},
        }))
        .unwrap();
        assert_eq!(
            clip,
            json!({"x": 0, "y": 0, "width": 1281.0, "height": 5000.0, "scale": 1})
        );
        let legacy =
            full_page_clip(&json!({"contentSize": {"width": 800, "height": 600}})).unwrap();
        assert_eq!(legacy["width"], json!(800.0));
        assert!(full_page_clip(&json!({"cssContentSize": {"width": 0, "height": 10}})).is_err());
        assert!(full_page_clip(&json!({})).is_err());
    }

    #[test]
    fn screenshot_clip_reads_every_field() {
        let page = full_page_clip(&json!({"cssContentSize": {"width": 800, "height": 600}}))
            .and_then(|clip| screenshot_clip(&clip))
            .unwrap();
        assert_eq!(
            (page.x, page.y, page.width, page.height, page.scale),
            (0.0, 0.0, 800.0, 600.0, 1.0)
        );
        let zoom = screenshot_clip(
            &json!({"x": 15.0, "y": 1220.5, "width": 100.0, "height": 50.0, "scale": 3.0}),
        )
        .unwrap();
        assert_eq!(
            (zoom.x, zoom.y, zoom.width, zoom.height, zoom.scale),
            (15.0, 1220.5, 100.0, 50.0, 3.0)
        );
        let missing = screenshot_clip(&json!({"x": 0, "y": 0, "width": 10, "height": 10}))
            .unwrap_err()
            .to_string();
        assert!(missing.contains("scale"), "{missing}");
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserScreenshotNode {}

impl BrowserScreenshotNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserScreenshotNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_screenshot",
            "Take Screenshot",
            "Takes a screenshot of the current page",
            "Automation/Browser/Capture",
        );
        node.set_flowscript_name("browser", "screenshot");
        node.add_icon("/flow/icons/browser.svg");

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
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "full_page",
            "Full Page",
            "Capture the entire scrollable page instead of the viewport (Chrome and Edge only)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "screenshot",
            "Screenshot",
            "Screenshot as base64 PNG data",
            VariableType::String,
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
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let full_page: bool = context.evaluate_pin("full_page").await?;
        if full_page {
            super::emulation::require_chromium(&session, "Full-page screenshots")?;
        }

        let page = session.browser_page(context).await?;

        let screenshot_bytes = if full_page {
            let metrics = super::cdp::send(&page, "Page.getLayoutMetrics", json!({})).await?;
            capture_png_via_cdp(&page, &full_page_clip(&metrics)?, true).await?
        } else {
            page.page
                .screenshot(flow_like_browser::output::ScreenshotOptions::default())
                .await
                .map_err(|e| flow_like_types::anyhow!("Failed to take screenshot: {}", e))?
        };
        drop(page);

        set_screenshot_outputs(context, screenshot_bytes).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BrowserScreenshotElementNode {}

impl BrowserScreenshotElementNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BrowserScreenshotElementNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "browser_screenshot_element",
            "Screenshot Element",
            "Takes a screenshot of a specific element",
            "Automation/Browser/Capture",
        );
        node.set_flowscript_name("browser", "screenshotElement");
        node.add_icon("/flow/icons/browser.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(4)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "selector",
            "Selector",
            "CSS selector of element to screenshot",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Automation session (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "screenshot",
            "Screenshot",
            "Screenshot as base64 PNG data",
            VariableType::String,
        );

        node.add_output_pin(
            "image",
            "Image",
            "Screenshot as NodeImage",
            VariableType::Struct,
        )
        .set_schema::<NodeImage>();

        super::selector::add_locator_pin(&mut node);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;
        let locator = super::selector::evaluate_locator(context, &selector).await?;

        let page = session.browser_page(context).await?;

        let element = super::selector::find_element(&page, &locator)
            .await
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to find element '{}': {}", selector, e)
            })?;

        let screenshot_bytes = element
            .screenshot_png()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to take element screenshot: {}", e))?;
        drop(page);

        set_screenshot_outputs(context, screenshot_bytes).await?;
        context.set_pin_value("session_out", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Browser automation requires the 'execute' feature"
        ))
    }
}
