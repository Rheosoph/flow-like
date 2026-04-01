use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct RetryLoopNode {}

impl RetryLoopNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RetryLoopNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "rpa_retry_loop",
            "Retry Loop",
            "Retries an action multiple times with configurable backoff. WARNING: This node activates exec_attempt in a loop but the current executor does not re-enter downstream nodes -- the retry semantics require executor-level loop support to work correctly.",
            "Automation/RPA",
        );
        node.add_icon("/flow/icons/rpa.svg");

        node.set_scores(
            flow_like::flow::node::NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(7)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );
        node.set_only_offline(true);

        node.add_input_pin("exec_in", "▶", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "max_retries",
            "Max Retries",
            "Maximum number of retry attempts",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(3)));

        node.add_input_pin(
            "initial_delay_ms",
            "Initial Delay (ms)",
            "Initial delay before first retry",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1000)));

        node.add_input_pin(
            "backoff_type",
            "Backoff Type",
            "Type of backoff strategy",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Constant".to_string(),
                    "Linear".to_string(),
                    "Exponential".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Exponential")));

        node.add_input_pin(
            "should_retry",
            "Should Retry",
            "Whether to retry (connect to condition check)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_attempt",
            "Attempt",
            "Execute the action",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_success",
            "Success",
            "Action succeeded",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_exhausted",
            "Exhausted",
            "All retries failed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "attempt",
            "Attempt",
            "Current attempt number",
            VariableType::Integer,
        );
        node.add_output_pin(
            "total_attempts",
            "Total Attempts",
            "Total attempts made",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_attempt").await?;
        context.deactivate_exec_pin("exec_success").await?;
        context.deactivate_exec_pin("exec_exhausted").await?;

        let max_retries: i64 = context.evaluate_pin("max_retries").await?;
        let initial_delay_ms: i64 = context.evaluate_pin("initial_delay_ms").await?;
        let backoff_type: String = context.evaluate_pin("backoff_type").await?;

        for attempt in 1..=max_retries {
            context.set_pin_value("attempt", json!(attempt)).await?;

            context.activate_exec_pin("exec_attempt").await?;

            let should_retry: bool = context.evaluate_pin("should_retry").await?;

            if !should_retry {
                context
                    .set_pin_value("total_attempts", json!(attempt))
                    .await?;
                context.activate_exec_pin("exec_success").await?;
                return Ok(());
            }

            if attempt < max_retries {
                let delay = match backoff_type.as_str() {
                    "Constant" => initial_delay_ms as u64,
                    "Linear" => (initial_delay_ms * attempt) as u64,
                    "Exponential" => (initial_delay_ms as u64) * 2u64.pow((attempt - 1) as u32),
                    _ => initial_delay_ms as u64,
                };

                flow_like_types::tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
            }
        }

        context
            .set_pin_value("total_attempts", json!(max_retries))
            .await?;
        context.activate_exec_pin("exec_exhausted").await?;

        Ok(())
    }
}
