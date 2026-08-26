use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::ValueType,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub is_primary: bool,
    pub scale_factor: f32,
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerListDisplaysNode {}

impl ComputerListDisplaysNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerListDisplaysNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_list_displays",
            "List Displays",
            "Enumerates all connected monitors/displays",
            "Automation/Computer/Display",
        );
        node.set_flowscript_name("computer", "listDisplays");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "displays",
            "Displays",
            "List of connected displays",
            VariableType::Struct,
        )
        .set_schema::<DisplayInfo>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "count",
            "Count",
            "Number of connected displays",
            VariableType::Integer,
        );

        node.add_output_pin(
            "primary_index",
            "Primary Index",
            "Index of the primary display",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;

        let (displays, primary_index) = {
            let monitors = Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;

            let mut displays = Vec::new();
            let mut primary_index: i64 = 0;

            for (i, monitor) in monitors.iter().enumerate() {
                let is_primary = monitor.is_primary().unwrap_or(false);
                if is_primary {
                    primary_index = i as i64;
                }

                displays.push(DisplayInfo {
                    id: monitor.id().unwrap_or(0),
                    name: monitor.name().unwrap_or_default(),
                    x: monitor.x().unwrap_or(0),
                    y: monitor.y().unwrap_or(0),
                    width: monitor.width().unwrap_or(0),
                    height: monitor.height().unwrap_or(0),
                    is_primary,
                    scale_factor: monitor.scale_factor().unwrap_or(1.0),
                });
            }
            (displays, primary_index)
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("displays", json!(displays)).await?;
        context
            .set_pin_value("count", json!(displays.len() as i64))
            .await?;
        context
            .set_pin_value("primary_index", json!(primary_index))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerGetDisplayNode {}

impl ComputerGetDisplayNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerGetDisplayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_get_display",
            "Get Display",
            "Gets information about a specific display by index",
            "Automation/Computer/Display",
        );
        node.set_flowscript_name("computer", "getDisplay");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_input_pin(
            "index",
            "Index",
            "Display index (0-based)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "display",
            "Display",
            "Display information",
            VariableType::Struct,
        )
        .set_schema::<DisplayInfo>();

        node.add_output_pin(
            "width",
            "Width",
            "Display width in pixels",
            VariableType::Integer,
        );
        node.add_output_pin(
            "height",
            "Height",
            "Display height in pixels",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        let index: i64 = context.evaluate_pin("index").await?;

        let display = {
            let monitors = Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;

            let monitor = monitors
                .get(index as usize)
                .ok_or_else(|| flow_like_types::anyhow!("Display index {} not found", index))?;

            DisplayInfo {
                id: monitor.id().unwrap_or(0),
                name: monitor.name().unwrap_or_default(),
                x: monitor.x().unwrap_or(0),
                y: monitor.y().unwrap_or(0),
                width: monitor.width().unwrap_or(0),
                height: monitor.height().unwrap_or(0),
                is_primary: monitor.is_primary().unwrap_or(false),
                scale_factor: monitor.scale_factor().unwrap_or(1.0),
            }
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("display", json!(display)).await?;
        context
            .set_pin_value("width", json!(display.width as i64))
            .await?;
        context
            .set_pin_value("height", json!(display.height as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ComputerGetPrimaryDisplayNode {}

impl ComputerGetPrimaryDisplayNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ComputerGetPrimaryDisplayNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "computer_get_primary_display",
            "Get Primary Display",
            "Gets information about the primary display",
            "Automation/Computer/Display",
        );
        node.set_flowscript_name("computer", "getPrimaryDisplay");
        node.add_icon("/flow/icons/computer.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Computer session handle",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session_out",
            "Session",
            "Computer session handle (pass-through)",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin(
            "display",
            "Display",
            "Primary display information",
            VariableType::Struct,
        )
        .set_schema::<DisplayInfo>();

        node.add_output_pin(
            "width",
            "Width",
            "Display width in pixels",
            VariableType::Integer,
        );
        node.add_output_pin(
            "height",
            "Height",
            "Display height in pixels",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use xcap::Monitor;

        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;

        let display = {
            let monitors = Monitor::all()
                .map_err(|e| flow_like_types::anyhow!("Failed to enumerate monitors: {}", e))?;

            let monitor = monitors
                .iter()
                .find(|m| m.is_primary().unwrap_or(false))
                .or_else(|| monitors.first())
                .ok_or_else(|| flow_like_types::anyhow!("No displays found"))?;

            DisplayInfo {
                id: monitor.id().unwrap_or(0),
                name: monitor.name().unwrap_or_default(),
                x: monitor.x().unwrap_or(0),
                y: monitor.y().unwrap_or(0),
                width: monitor.width().unwrap_or(0),
                height: monitor.height().unwrap_or(0),
                is_primary: monitor.is_primary().unwrap_or(false),
                scale_factor: monitor.scale_factor().unwrap_or(1.0),
            }
        };

        context.set_pin_value("session_out", json!(session)).await?;
        context.set_pin_value("display", json!(display)).await?;
        context
            .set_pin_value("width", json!(display.width as i64))
            .await?;
        context
            .set_pin_value("height", json!(display.height as i64))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Computer automation requires the 'execute' feature"
        ))
    }
}
