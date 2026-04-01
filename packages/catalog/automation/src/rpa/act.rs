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
        use rustautogui::MouseClick;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let x: i64 = context.evaluate_pin("x").await?;
        let y: i64 = context.evaluate_pin("y").await?;
        let click_type: String = context.evaluate_pin("click_type").await?;

        let autogui = session.get_autogui(context).await?;
        let gui = autogui.lock().await;

        gui.move_mouse_to_pos(x as u32, y as u32, 0.0)
            .map_err(|e| flow_like_types::anyhow!("Failed to move mouse: {}", e))?;

        match click_type.as_str() {
            "Right" => gui
                .click(MouseClick::RIGHT)
                .map_err(|e| flow_like_types::anyhow!("Failed to right click: {}", e))?,
            "Double" => gui
                .double_click()
                .map_err(|e| flow_like_types::anyhow!("Failed to double click: {}", e))?,
            _ => gui
                .click(MouseClick::LEFT)
                .map_err(|e| flow_like_types::anyhow!("Failed to click: {}", e))?,
        }

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
        let text: String = context.evaluate_pin("text").await?;
        let _interval_ms: i64 = context.evaluate_pin("interval_ms").await?;

        let autogui = session.get_autogui(context).await?;
        let gui = autogui.lock().await;

        gui.keyboard_input(&text)
            .map_err(|e| flow_like_types::anyhow!("Failed to type text: {}", e))?;

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
        let from_x: i64 = context.evaluate_pin("from_x").await?;
        let from_y: i64 = context.evaluate_pin("from_y").await?;
        let to_x: i64 = context.evaluate_pin("to_x").await?;
        let to_y: i64 = context.evaluate_pin("to_y").await?;
        let duration: f64 = context.evaluate_pin("duration_sec").await?;

        let autogui = session.get_autogui(context).await?;
        let gui = autogui.lock().await;

        gui.move_mouse_to_pos(from_x as u32, from_y as u32, 0.0)
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to move mouse to start position: {}", e)
            })?;

        gui.drag_mouse_to(Some(to_x as u32), Some(to_y as u32), duration as f32)
            .map_err(|e| flow_like_types::anyhow!("Failed to drag: {}", e))?;

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
        let clicks: i64 = context.evaluate_pin("clicks").await?;

        let autogui = session.get_autogui(context).await?;
        let gui = autogui.lock().await;

        let intensity = clicks.unsigned_abs() as u32;
        if clicks > 0 {
            gui.scroll_up(intensity)
                .map_err(|e| flow_like_types::anyhow!("Failed to scroll up: {}", e))?;
        } else if clicks < 0 {
            gui.scroll_down(intensity)
                .map_err(|e| flow_like_types::anyhow!("Failed to scroll down: {}", e))?;
        }

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
