use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::NodeImage;
use flow_like_types::{async_trait, json::json};

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
            "Capture entire scrollable page",
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
        use flow_like_types::base64::Engine;
        use flow_like_types::image;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let _full_page: bool = context.evaluate_pin("full_page").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let screenshot_bytes = driver
            .screenshot_as_png()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to take screenshot: {}", e))?;

        let base64_screenshot =
            flow_like_types::base64::engine::general_purpose::STANDARD.encode(&screenshot_bytes);

        // Create NodeImage from the screenshot bytes
        let dyn_image = image::load_from_memory(&screenshot_bytes)
            .map_err(|e| flow_like_types::anyhow!("Failed to decode screenshot: {}", e))?;
        let node_image = NodeImage::new(context, dyn_image).await;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("screenshot", json!(base64_screenshot))
            .await?;
        context.set_pin_value("image", json!(node_image)).await?;
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

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like_types::base64::Engine;
        use flow_like_types::image;
        use thirtyfour::prelude::*;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let selector: String = context.evaluate_pin("selector").await?;

        let driver = session.get_browser_driver_and_switch(context).await?;

        let element = driver.find(By::Css(&selector)).await.map_err(|e| {
            flow_like_types::anyhow!("Failed to find element '{}': {}", selector, e)
        })?;

        let screenshot_bytes = element
            .screenshot_as_png()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to take element screenshot: {}", e))?;

        let base64_screenshot =
            flow_like_types::base64::engine::general_purpose::STANDARD.encode(&screenshot_bytes);

        // Create NodeImage from the screenshot bytes
        let dyn_image = image::load_from_memory(&screenshot_bytes)
            .map_err(|e| flow_like_types::anyhow!("Failed to decode screenshot: {}", e))?;
        let node_image = NodeImage::new(context, dyn_image).await;

        context.set_pin_value("session_out", json!(session)).await?;
        context
            .set_pin_value("screenshot", json!(base64_screenshot))
            .await?;
        context.set_pin_value("image", json!(node_image)).await?;
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
