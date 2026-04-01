use crate::types::handles::AutomationSession;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StartSessionNode {}

impl StartSessionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StartSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "automation_start_session",
            "Start Automation Session",
            "Starts a unified automation session for desktop, browser, and RPA automation",
            "Automation",
        );
        node.add_icon("/flow/icons/automation.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "default_delay_ms",
            "Default Delay (ms)",
            "Default delay between actions in milliseconds",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_input_pin(
            "click_delay_ms",
            "Click Delay (ms)",
            "Delay between mouse move and click to ensure registration",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10)));

        node.add_input_pin(
            "debug_mode",
            "Debug Mode",
            "Enable debug mode for verbose logging",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "session",
            "Session",
            "Unified automation session for all operations",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let default_delay_ms: i64 = context.evaluate_pin("default_delay_ms").await?;
        let click_delay_ms: i64 = context.evaluate_pin("click_delay_ms").await?;
        let debug_mode: bool = context.evaluate_pin("debug_mode").await?;

        let session = AutomationSession::new(
            context,
            default_delay_ms as u64,
            click_delay_ms as u64,
            debug_mode,
        )
        .await?;

        context.set_pin_value("session", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Automation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct StopSessionNode {}

impl StopSessionNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StopSessionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "automation_stop_session",
            "Stop Automation Session",
            "Stops an automation session and releases all resources",
            "Automation",
        );
        node.add_icon("/flow/icons/automation.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(2)
                .set_security(3)
                .set_performance(9)
                .set_governance(5)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "session",
            "Session",
            "Automation session to stop",
            VariableType::Struct,
        )
        .set_schema::<AutomationSession>();

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: AutomationSession = context.evaluate_pin("session").await?;
        session.close(context).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Automation requires the 'execute' feature"
        ))
    }
}
