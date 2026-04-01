use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct WaitForTemplateNode {}

impl WaitForTemplateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WaitForTemplateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_wait_for_template",
            "Wait For Template",
            "Waits for a template to appear on screen",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

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
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "template_path",
            "Template Path",
            "Path to the template image",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
            "Maximum wait time in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_input_pin(
            "poll_interval_ms",
            "Poll Interval (ms)",
            "Check interval in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(500)));

        node.add_output_pin(
            "exec_found",
            "Found",
            "Template appeared",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Timeout reached",
            VariableType::Execution,
        );

        node.add_output_pin("x", "X", "X coordinate", VariableType::Integer);
        node.add_output_pin("y", "Y", "Y coordinate", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use rustautogui::MatchMode;
        use std::time::{Duration, Instant};

        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let template_path: String = context.evaluate_pin("template_path").await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        let poll_interval_ms: i64 = context.evaluate_pin("poll_interval_ms").await?;

        let autogui = session.get_autogui(context).await?;
        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);
        let poll_interval = Duration::from_millis(poll_interval_ms.max(0) as u64);

        loop {
            {
                let mut gui = autogui.lock().await;

                gui.prepare_template_from_file(&template_path, None, MatchMode::Segmented)
                    .map_err(|e| flow_like_types::anyhow!("Failed to prepare template: {}", e))?;

                if let Ok(Some(matches)) = gui.find_image_on_screen(confidence as f32)
                    && let Some((x, y, _conf)) = matches.first()
                {
                    context.set_pin_value("x", json!(*x as i64)).await?;
                    context.set_pin_value("y", json!(*y as i64)).await?;
                    context.activate_exec_pin("exec_found").await?;
                    return Ok(());
                }
            }

            if start.elapsed() >= timeout {
                context.set_pin_value("x", json!(0)).await?;
                context.set_pin_value("y", json!(0)).await?;
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }

            tokio::time::sleep(poll_interval).await;
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RPA automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct WaitForColorNode {}

impl WaitForColorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WaitForColorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_wait_for_color",
            "Wait For Color",
            "Waits for a specific color to appear at a position",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(6)
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
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("x", "X", "X position to check", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Y position to check", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("red", "Red", "Expected red (0-255)", VariableType::Integer)
            .set_default_value(Some(json!(255)));

        node.add_input_pin(
            "green",
            "Green",
            "Expected green (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(255)));

        node.add_input_pin(
            "blue",
            "Blue",
            "Expected blue (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(255)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Color tolerance (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum wait time",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_output_pin(
            "exec_found",
            "Found",
            "Color matched",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Timeout reached",
            VariableType::Execution,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{Duration, Instant};

        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let target_r: i64 = context.evaluate_pin("red").await?;
        let target_g: i64 = context.evaluate_pin("green").await?;
        let target_b: i64 = context.evaluate_pin("blue").await?;
        let tolerance: i64 = context.evaluate_pin("tolerance").await?;
        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;

        use xcap::Monitor;

        let start = Instant::now();
        let timeout = Duration::from_millis(timeout_ms.max(0) as u64);

        loop {
            {
                let monitors = Monitor::all()
                    .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;
                let monitor = monitors
                    .first()
                    .ok_or_else(|| flow_like_types::anyhow!("No monitors found"))?;
                let image = monitor
                    .capture_image()
                    .map_err(|e| flow_like_types::anyhow!("Failed to capture screen: {}", e))?;

                if x >= 0 && y >= 0 && (x as u32) < image.width() && (y as u32) < image.height() {
                    let pixel = image.get_pixel(x as u32, y as u32);
                    let r = pixel[0] as i64;
                    let g = pixel[1] as i64;
                    let b = pixel[2] as i64;

                    let matches = (r - target_r).abs() <= tolerance
                        && (g - target_g).abs() <= tolerance
                        && (b - target_b).abs() <= tolerance;

                    if matches {
                        context.activate_exec_pin("exec_found").await?;
                        return Ok(());
                    }
                }
            }

            if start.elapsed() >= timeout {
                context.activate_exec_pin("exec_timeout").await?;
                return Ok(());
            }

            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RPA automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DelayNode {}

impl DelayNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DelayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_delay",
            "Delay",
            "Pauses execution for a specified duration",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(10)
                .set_governance(6)
                .set_reliability(10)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "duration_ms",
            "Duration (ms)",
            "Delay duration in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1000)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let duration_ms: i64 = context.evaluate_pin("duration_ms").await?;

        flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(
            duration_ms.max(0) as u64
        ))
        .await;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
