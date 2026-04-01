use crate::types::handles::AutomationSession;
use crate::types::templates::TemplateMatchResult;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct WaitTemplateNode {}

impl WaitTemplateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WaitTemplateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "vision_wait_template",
            "Wait For Template",
            "Waits for a template image to appear on screen",
            "Automation/Vision",
        );
        node.add_icon("/flow/icons/vision.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(5)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(8)
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
            "template",
            "Template",
            "Template image file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "confidence",
            "Confidence",
            "Minimum match confidence (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_input_pin(
            "poll_interval_ms",
            "Poll Interval (ms)",
            "How often to check for template",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(500)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Triggered if template not found within timeout",
            VariableType::Execution,
        );

        node.add_output_pin(
            "found",
            "Found",
            "Whether the template was found",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "result",
            "Result",
            "Match result with location",
            VariableType::Struct,
        )
        .set_schema::<TemplateMatchResult>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{Duration, Instant};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let template: FlowPath = context.evaluate_pin("template").await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        let poll_interval_ms: i64 = context.evaluate_pin("poll_interval_ms").await?;

        let template_bytes = template.get(context, false).await?;
        let gray_template = crate::types::screen_match::to_grayscale(&template_bytes)
            .ok_or_else(|| flow_like_types::anyhow!("Failed to decode template image"))?;

        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);
        let poll_interval = Duration::from_millis(poll_interval_ms.max(0) as u64);

        loop {
            // Capture fresh screen each iteration and match
            if let Some(gray_screen) = crate::types::screen_match::capture_screen_grayscale() {
                let matches = crate::types::screen_match::find_template_in_image(
                    &gray_screen,
                    &gray_template,
                    confidence as f32,
                );
                if let Some(&(px, py, _conf)) = matches.first() {
                    let (lx, ly) = crate::types::screen_match::physical_to_logical(px, py);
                    let result = TemplateMatchResult {
                        found: true,
                        x: lx,
                        y: ly,
                        confidence,
                        template_path: template.path.clone(),
                    };

                    context.set_pin_value("found", json!(true)).await?;
                    context.set_pin_value("result", json!(result)).await?;
                    context.activate_exec_pin("exec_out").await?;
                    return Ok(());
                }
            }

            if start.elapsed() >= timeout {
                let result = TemplateMatchResult {
                    found: false,
                    x: 0,
                    y: 0,
                    confidence: 0.0,
                    template_path: template.path.clone(),
                };

                context.set_pin_value("found", json!(false)).await?;
                context.set_pin_value("result", json!(result)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }

            tokio::time::sleep(poll_interval).await;
        }
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
pub struct WaitTemplateDisappearNode {}

impl WaitTemplateDisappearNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WaitTemplateDisappearNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "vision_wait_template_disappear",
            "Wait Template Disappear",
            "Waits for a template image to disappear from screen",
            "Automation/Vision",
        );
        node.add_icon("/flow/icons/vision.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(5)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(8)
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
            "template",
            "Template",
            "Template image file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "confidence",
            "Confidence",
            "Minimum match confidence (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Triggered if template still visible after timeout",
            VariableType::Execution,
        );

        node.add_output_pin(
            "disappeared",
            "Disappeared",
            "Whether the template disappeared",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{Duration, Instant};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let template: FlowPath = context.evaluate_pin("template").await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;

        let template_bytes = template.get(context, false).await?;
        let gray_template = crate::types::screen_match::to_grayscale(&template_bytes)
            .ok_or_else(|| flow_like_types::anyhow!("Failed to decode template image"))?;

        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);

        loop {
            let is_visible = crate::types::screen_match::capture_screen_grayscale()
                .map(|gray_screen| {
                    let matches = crate::types::screen_match::find_template_in_image(
                        &gray_screen,
                        &gray_template,
                        confidence as f32,
                    );
                    !matches.is_empty()
                })
                .unwrap_or(false);

            if !is_visible {
                context.set_pin_value("disappeared", json!(true)).await?;
                context.activate_exec_pin("exec_out").await?;
                return Ok(());
            }

            if start.elapsed() >= timeout {
                context.set_pin_value("disappeared", json!(false)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Vision automation requires the 'execute' feature"
        ))
    }
}
