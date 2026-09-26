use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_catalog_core::FlowPath;
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ClickTemplateNode {}

impl ClickTemplateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClickTemplateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "vision_click_template",
            "Click Template",
            "Finds a template image on screen and clicks on it",
            "Automation/Vision",
        );
        node.set_version(1);
        node.set_flowscript_name("automation.vision", "clickTemplate");
        node.add_icon("/flow/icons/vision.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(6)
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
            "Automation session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "template",
            "Template",
            "Path to the template image file (FlowPath with caching support)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_input_pin(
            "monitor",
            "Monitor",
            "Display index, -1 for primary, or -2 for all displays",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-2)));

        node.add_input_pin(
            "confidence",
            "Confidence",
            "Minimum match confidence (0.0-1.0)",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.8)));

        node.add_input_pin(
            "click_type",
            "Click Type",
            "Type of click to perform",
            VariableType::String,
        )
        .set_options(
            flow_like::flow::pin::PinOptions::new()
                .set_valid_values(vec![
                    "Left".to_string(),
                    "Right".to_string(),
                    "Double".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Left")));

        node.add_input_pin(
            "offset_x",
            "Offset X",
            "X offset from center of matched template",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "offset_y",
            "Offset Y",
            "Y offset from center of matched template",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "fallback_x",
            "Fallback X",
            "X coordinate to click if template not found (use -1 to disable fallback)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_input_pin(
            "fallback_y",
            "Fallback Y",
            "Y coordinate to click if template not found (use -1 to disable fallback)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(-1)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);
        node.add_output_pin(
            "exec_not_found",
            "Not Found",
            "Triggered if template not found",
            VariableType::Execution,
        );

        node.add_output_pin(
            "found",
            "Found",
            "Whether the template was found and clicked",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "x",
            "X",
            "X coordinate where clicked",
            VariableType::Integer,
        );
        node.add_output_pin(
            "y",
            "Y",
            "Y coordinate where clicked",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Button, Coordinate, Direction, Mouse};

        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("exec_not_found").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let template: FlowPath = context.evaluate_pin("template").await?;
        let confidence: f64 = context.evaluate_pin("confidence").await?;
        let monitor: i64 = context.evaluate_pin("monitor").await.unwrap_or(-2);
        let click_type: String = context.evaluate_pin("click_type").await?;
        let offset_x: i64 = context.evaluate_pin("offset_x").await?;
        let offset_y: i64 = context.evaluate_pin("offset_y").await?;

        let fallback_x: i64 = context.evaluate_pin("fallback_x").await?;
        let fallback_y: i64 = context.evaluate_pin("fallback_y").await?;

        // Download template image using FlowPath's caching mechanism
        let template_bytes = template.get(context, false).await?;

        let matches = crate::types::screen_match::match_desktop_async(
            template_bytes.clone(),
            confidence,
            monitor,
        )
        .await?;
        let target = if let Some(&(x, y, _)) = matches.first() {
            Some((
                i32::try_from(
                    i64::from(x)
                        .checked_add(offset_x)
                        .ok_or_else(|| flow_like_types::anyhow!("Click X overflow"))?,
                )?,
                i32::try_from(
                    i64::from(y)
                        .checked_add(offset_y)
                        .ok_or_else(|| flow_like_types::anyhow!("Click Y overflow"))?,
                )?,
                true,
            ))
        } else if fallback_x != -1 && fallback_y != -1 {
            Some((
                i32::try_from(fallback_x)?,
                i32::try_from(fallback_y)?,
                false,
            ))
        } else {
            None
        };
        if let Some((x, y, found)) = target {
            let (button, double) = match click_type.to_lowercase().as_str() {
                "left" => (Button::Left, false),
                "right" => (Button::Right, false),
                "double" => (Button::Left, true),
                _ => return Err(flow_like_types::anyhow!("Unknown click type")),
            };
            let mut input = session.create_enigo(context).await?;
            let cancellation = context.get_cancellation_token();
            tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
                input.move_mouse(x, y, Coordinate::Abs)?;
                input.button(button, Direction::Click)?;
                if double {
                    crate::computer::mouse::interruptible_sleep(80, cancellation.as_ref())?;
                    input.button(button, Direction::Click)?;
                }
                Ok(())
            })
            .await??;
            session.apply_delay(context).await?;
            context.set_pin_value("found", json!(found)).await?;
            context.set_pin_value("x", json!(x)).await?;
            context.set_pin_value("y", json!(y)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        context.set_pin_value("found", json!(false)).await?;
        context.set_pin_value("x", json!(0)).await?;
        context.set_pin_value("y", json!(0)).await?;
        context.activate_exec_pin("exec_not_found").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Vision automation requires the 'execute' feature"
        ))
    }
}
