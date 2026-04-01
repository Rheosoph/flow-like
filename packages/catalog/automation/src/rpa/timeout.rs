use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct WithTimeoutNode {}

impl WithTimeoutNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WithTimeoutNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_with_timeout",
            "With Timeout",
            "Executes an action with a timeout constraint",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Maximum time to wait for action",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30000)));

        node.add_input_pin(
            "completed",
            "Completed",
            "Whether the action completed (wire from action result)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_action",
            "Action",
            "Execute the action",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_success",
            "Success",
            "Action completed in time",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_timeout",
            "Timeout",
            "Action timed out",
            VariableType::Execution,
        );

        node.add_output_pin(
            "elapsed_ms",
            "Elapsed (ms)",
            "Time elapsed",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use std::time::Instant;

        context.deactivate_exec_pin("exec_action").await?;
        context.deactivate_exec_pin("exec_success").await?;
        context.deactivate_exec_pin("exec_timeout").await?;

        let timeout_ms: i64 = context.evaluate_pin("timeout_ms").await?;
        let start = Instant::now();

        context.activate_exec_pin("exec_action").await?;

        let elapsed = start.elapsed().as_millis() as i64;
        context.set_pin_value("elapsed_ms", json!(elapsed)).await?;

        let completed: bool = context.evaluate_pin("completed").await?;

        if elapsed > timeout_ms {
            context.activate_exec_pin("exec_timeout").await?;
        } else if completed {
            context.activate_exec_pin("exec_success").await?;
        } else {
            context.activate_exec_pin("exec_timeout").await?;
        }

        Ok(())
    }
}
