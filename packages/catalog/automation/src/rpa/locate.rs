use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct LocateByTemplateNode {}

impl LocateByTemplateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LocateByTemplateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_locate_template",
            "Locate By Template",
            "Finds an element on screen using template matching",
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
            "RPA session handle",
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

        node.add_output_pin(
            "exec_found",
            "Found",
            "Template was found",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Template was not found",
            VariableType::Execution,
        );

        node.add_output_pin("x", "X", "X coordinate", VariableType::Integer);
        node.add_output_pin("y", "Y", "Y coordinate", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use rustautogui::MatchMode;

        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let template_path: String = context.evaluate_pin("template_path").await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;

        let autogui = session.get_autogui(context).await?;
        let mut gui = autogui.lock().await;

        gui.prepare_template_from_file(&template_path, None, MatchMode::Segmented)
            .map_err(|e| flow_like_types::anyhow!("Failed to prepare template: {}", e))?;

        match gui.find_image_on_screen(confidence as f32) {
            Ok(Some(matches)) if !matches.is_empty() => {
                let (x, y, _conf) = matches[0];
                context.set_pin_value("x", json!(x as i64)).await?;
                context.set_pin_value("y", json!(y as i64)).await?;
                context.activate_exec_pin("exec_found").await?;
            }
            _ => {
                context.set_pin_value("x", json!(0)).await?;
                context.set_pin_value("y", json!(0)).await?;
                context.activate_exec_pin("exec_not_found").await?;
            }
        }

        Ok(())
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
pub struct LocateByColorNode {}

impl LocateByColorNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LocateByColorNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_locate_color",
            "Locate By Color",
            "Finds a pixel on screen matching a specific color",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(6)
                .set_cost(9)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "RPA session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("red", "Red", "Red component (0-255)", VariableType::Integer)
            .set_default_value(Some(json!(255)));

        node.add_input_pin(
            "green",
            "Green",
            "Green component (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "blue",
            "Blue",
            "Blue component (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "tolerance",
            "Tolerance",
            "Color matching tolerance (0-255)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_output_pin(
            "exec_found",
            "Found",
            "Color was found",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Color was not found",
            VariableType::Execution,
        );

        node.add_output_pin("x", "X", "X coordinate", VariableType::Integer);
        node.add_output_pin("y", "Y", "Y coordinate", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let _session: AutomationSession = context.evaluate_pin("session").await?;
        let red: i64 = context.evaluate_pin("red").await?;
        let green: i64 = context.evaluate_pin("green").await?;
        let blue: i64 = context.evaluate_pin("blue").await?;
        let tolerance: i64 = context.evaluate_pin("tolerance").await?;

        let monitors = Monitor::all()
            .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;
        let monitor = monitors
            .first()
            .ok_or_else(|| flow_like_types::anyhow!("No monitors found"))?;
        let image = monitor
            .capture_image()
            .map_err(|e| flow_like_types::anyhow!("Failed to capture screen: {}", e))?;

        let mut found_pos: Option<(u32, u32)> = None;
        'outer: for y in 0..image.height() {
            for x in 0..image.width() {
                let pixel = image.get_pixel(x, y);
                let r = pixel[0] as i64;
                let g = pixel[1] as i64;
                let b = pixel[2] as i64;
                if (r - red).abs() <= tolerance
                    && (g - green).abs() <= tolerance
                    && (b - blue).abs() <= tolerance
                {
                    found_pos = Some((x, y));
                    break 'outer;
                }
            }
        }

        match found_pos {
            Some((x, y)) => {
                context.set_pin_value("x", json!(x as i64)).await?;
                context.set_pin_value("y", json!(y as i64)).await?;
                context.activate_exec_pin("exec_found").await?;
            }
            None => {
                context.set_pin_value("x", json!(0)).await?;
                context.set_pin_value("y", json!(0)).await?;
                context.activate_exec_pin("exec_not_found").await?;
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RPA automation requires the 'execute' feature"
        ))
    }
}
