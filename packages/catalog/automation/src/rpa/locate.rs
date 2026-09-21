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
        node.set_version(1);
        node.set_flowscript_name("rpa", "locateTemplate");
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
            "template",
            "Template",
            "Template image from any FlowPath store; preferred over a local path",
            VariableType::Struct,
        )
        .set_schema::<flow_like_catalog_core::FlowPath>();

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
        context.deactivate_exec_pin("exec_found").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let template_bytes = crate::types::screen_match::load_template(context).await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;

        let matches =
            crate::types::screen_match::match_desktop_async(template_bytes.clone(), confidence, -2)
                .await?;
        match matches.first() {
            Some(&(px, py, _)) => {
                let (x, y) = (px, py);
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
        node.set_version(1);
        node.set_flowscript_name("rpa", "locateColor");
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
        _session.ensure_active(context).await?;
        let red: i64 = context.evaluate_pin("red").await?;
        let green: i64 = context.evaluate_pin("green").await?;
        let blue: i64 = context.evaluate_pin("blue").await?;
        let tolerance: i64 = context.evaluate_pin("tolerance").await?;

        if [red, green, blue, tolerance]
            .iter()
            .any(|v| !(0..=255).contains(v))
        {
            return Err(flow_like_types::anyhow!(
                "Color channels and tolerance must be between 0 and 255"
            ));
        }
        let mut found_pos = None;
        'displays: for monitor in Monitor::all()? {
            let image = crate::types::screen_match::capture_monitor(&monitor)?;
            let (ox, oy, w, h) = crate::types::screen_match::monitor_input_bounds(&monitor)?;
            if w == 0 || h == 0 {
                continue;
            }
            for (x, y, pixel) in image.enumerate_pixels() {
                if (i64::from(pixel[0]) - red).abs() <= tolerance
                    && (i64::from(pixel[1]) - green).abs() <= tolerance
                    && (i64::from(pixel[2]) - blue).abs() <= tolerance
                {
                    let (dx, _) = crate::types::screen_match::map_capture_point(
                        x,
                        0,
                        (ox as f64, 0.0),
                        image.width() as f64 / w as f64,
                    )?;
                    let (_, dy) = crate::types::screen_match::map_capture_point(
                        0,
                        y,
                        (0.0, oy as f64),
                        image.height() as f64 / h as f64,
                    )?;
                    found_pos = Some((dx, dy));
                    break 'displays;
                }
            }
        }

        match found_pos {
            Some((px, py)) => {
                let (x, y) = (px, py);
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
