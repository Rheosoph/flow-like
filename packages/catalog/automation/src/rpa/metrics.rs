use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct StartTimerNode {}

impl StartTimerNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StartTimerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_start_timer",
            "Start Timer",
            "Returns the current timestamp for measuring action duration",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(10)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "start_time",
            "Start Time",
            "Timestamp when timer started (ms since epoch)",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};

        context.deactivate_exec_pin("exec_out").await?;

        let start_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        context
            .set_pin_value("start_time", json!(start_time))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CalculateElapsedNode {}

impl CalculateElapsedNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CalculateElapsedNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_calculate_elapsed",
            "Calculate Elapsed",
            "Calculates elapsed time from a start timestamp",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(10)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "start_time",
            "Start Time",
            "Start timestamp (ms since epoch) from Start Timer node",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_output_pin("exec_out", "▶", "Continue", VariableType::Execution);

        node.add_output_pin(
            "elapsed_ms",
            "Elapsed (ms)",
            "Time elapsed in milliseconds",
            VariableType::Integer,
        );

        node.add_output_pin(
            "elapsed_sec",
            "Elapsed (sec)",
            "Time elapsed in seconds",
            VariableType::Float,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::{SystemTime, UNIX_EPOCH};

        context.deactivate_exec_pin("exec_out").await?;

        let start_time: i64 = context.evaluate_pin("start_time").await?;

        let end_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let elapsed_ms = end_time - start_time;
        let elapsed_sec = elapsed_ms as f64 / 1000.0;

        context
            .set_pin_value("elapsed_ms", json!(elapsed_ms))
            .await?;
        context
            .set_pin_value("elapsed_sec", json!(elapsed_sec))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
