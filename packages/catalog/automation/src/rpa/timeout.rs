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
        node.set_version(1);
        node.set_flowscript_name("rpa", "withTimeout");
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
            "Legacy input; completion is determined by the action branch",
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

        if timeout_ms < 0 {
            return Err(flow_like_types::anyhow!("Timeout must be nonnegative"));
        }
        let outcome = super::branch::run_branch(
            context,
            "exec_action",
            Some(std::time::Duration::from_millis(timeout_ms as u64)),
        )
        .await?;
        context
            .set_pin_value("elapsed_ms", json!(start.elapsed().as_millis() as i64))
            .await?;
        match outcome {
            super::branch::BranchResult::Completed => {
                context.activate_exec_pin("exec_success").await?
            }
            super::branch::BranchResult::TimedOut => {
                context.activate_exec_pin("exec_timeout").await?
            }
            super::branch::BranchResult::Failed(error) => {
                return Err(flow_like_types::anyhow!(error));
            }
        }

        Ok(())
    }
}
