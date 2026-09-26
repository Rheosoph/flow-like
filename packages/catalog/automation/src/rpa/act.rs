use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct ClickAtPositionNode {}

impl ClickAtPositionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClickAtPositionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_click_at_position",
            "Click At Position",
            "Performs a click at a specific screen position",
            "Automation/RPA",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "clickAtPosition");
        node.add_icon("/flow/icons/rpa.svg");

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
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("x", "X", "X coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("y", "Y", "Y coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "click_type",
            "Click Type",
            "Type of click to perform",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Left".to_string(),
                    "Right".to_string(),
                    "Double".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Left")));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use enigo::{Button, Coordinate, Direction, Mouse};

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let click_type: String = context.evaluate_pin("click_type").await?;

        let mut input = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
            input.move_mouse(i32::try_from(x)?, i32::try_from(y)?, Coordinate::Abs)?;
            match click_type.to_lowercase().as_str() {
                "right" => input.button(Button::Right, Direction::Click)?,
                "double" => {
                    input.button(Button::Left, Direction::Click)?;
                    crate::computer::mouse::interruptible_sleep(80, cancellation.as_ref())?;
                    crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
                    input.button(Button::Left, Direction::Click)?;
                }
                "left" => input.button(Button::Left, Direction::Click)?,
                _ => return Err(flow_like_types::anyhow!("Unknown click type")),
            }

            Ok(())
        })
        .await??;
        session.apply_delay(context).await?;
        context.activate_exec_pin("exec_out").await?;

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
pub struct TypeTextNode {}

impl TypeTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TypeTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_type_text",
            "Type Text",
            "Types text using keyboard simulation",
            "Automation/RPA",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "typeText");
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
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
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin("text", "Text", "Text to type", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "interval_ms",
            "Interval (ms)",
            "Delay between keystrokes",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let text: String = context.evaluate_pin("text").await?;
        let interval_ms: i64 = context.evaluate_pin("interval_ms").await?;

        use enigo::Keyboard;
        if !(0..=60_000).contains(&interval_ms) {
            return Err(flow_like_types::anyhow!(
                "Keystroke interval must be between 0 and 60000 ms"
            ));
        }
        let mut input = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
            if interval_ms == 0 {
                input.text(&text)?;
            } else {
                for character in text.chars() {
                    crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
                    input.text(&character.to_string())?;
                    crate::computer::mouse::interruptible_sleep(
                        interval_ms as u64,
                        cancellation.as_ref(),
                    )?;
                }
            }

            Ok(())
        })
        .await??;
        session.apply_delay(context).await?;
        context.activate_exec_pin("exec_out").await?;

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
pub struct DragAndDropNode {}

impl DragAndDropNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DragAndDropNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_drag_drop",
            "Drag And Drop",
            "Performs a drag and drop operation",
            "Automation/RPA",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "dragDrop");
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(4)
                .set_performance(8)
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
            "from_x",
            "From X",
            "Start X coordinate",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "from_y",
            "From Y",
            "Start Y coordinate",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin("to_x", "To X", "End X coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin("to_y", "To Y", "End Y coordinate", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "duration_sec",
            "Duration (sec)",
            "Duration of drag in seconds",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let from_x: i64 = context.evaluate_pin("from_x").await?;
        let from_y: i64 = context.evaluate_pin("from_y").await?;
        let to_x: i64 = context.evaluate_pin("to_x").await?;
        let to_y: i64 = context.evaluate_pin("to_y").await?;
        let duration: f64 = context.evaluate_pin("duration_sec").await?;

        use enigo::{Button, Coordinate, Direction, Mouse};
        if !duration.is_finite() || !(0.0..=60.0).contains(&duration) {
            return Err(flow_like_types::anyhow!(
                "Drag duration must be between 0 and 60 seconds"
            ));
        }
        let (from_x, from_y, to_x, to_y) = (
            i32::try_from(from_x)?,
            i32::try_from(from_y)?,
            i32::try_from(to_x)?,
            i32::try_from(to_y)?,
        );
        let mut input = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
            input.move_mouse(from_x, from_y, Coordinate::Abs)?;
            input.button(Button::Left, Direction::Press)?;
            let steps = (duration * 60.0).ceil().max(1.0) as u32;
            for step in 1..=steps {
                crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
                let t = step as f64 / steps as f64;
                input.move_mouse(
                    (from_x as f64 + (to_x as f64 - from_x as f64) * t).round() as i32,
                    (from_y as f64 + (to_y as f64 - from_y as f64) * t).round() as i32,
                    Coordinate::Abs,
                )?;
                crate::computer::mouse::interruptible_sleep(
                    (duration * 1000.0 / steps as f64).round() as u64,
                    cancellation.as_ref(),
                )?;
            }
            input.button(Button::Left, Direction::Release)?;

            Ok(())
        })
        .await??;
        session.apply_delay(context).await?;
        context.activate_exec_pin("exec_out").await?;

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
pub struct ScrollNode {}

impl ScrollNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ScrollNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_scroll",
            "Scroll",
            "Performs a scroll action at the current mouse position",
            "Automation/RPA",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "scroll");
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(3)
                .set_security(5)
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
            "Automation session",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "clicks",
            "Clicks",
            "Number of scroll clicks (positive = up, negative = down)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.ensure_active(context).await?;
        let clicks: i64 = context.evaluate_pin("clicks").await?;

        use enigo::{Axis, Mouse};
        if clicks.unsigned_abs() > 1000 {
            return Err(flow_like_types::anyhow!(
                "Scroll clicks must be between -1000 and 1000"
            ));
        }
        let mut input = session.create_enigo(context).await?;
        let cancellation = context.get_cancellation_token();
        tokio::task::spawn_blocking(move || -> flow_like_types::Result<()> {
            crate::computer::mouse::check_cancellation(cancellation.as_ref())?;
            input.scroll(-(clicks as i32), Axis::Vertical)?;

            Ok(())
        })
        .await??;
        session.apply_delay(context).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "RPA automation requires the 'execute' feature"
        ))
    }
}
