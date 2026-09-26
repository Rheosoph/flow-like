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
            "Runs an action again after an error or a retry condition, with configurable backoff.",
            "Automation/RPA",
        );
        node.set_version(1);
        node.set_flowscript_name("rpa", "retryLoop");
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
            "Retry even when the action succeeds (connect a condition if needed)",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

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
        node.add_output_pin(
            "last_error",
            "Last Error",
            "Most recent action failure, empty after success",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_attempt").await?;
        context.deactivate_exec_pin("exec_success").await?;
        context.deactivate_exec_pin("exec_exhausted").await?;
        crate::browser::selector::optional_output(context, "last_error", json!("")).await?;
        context.set_pin_value("total_attempts", json!(0)).await?;

        let max_retries: i64 = context.evaluate_pin("max_retries").await?;
        let initial_delay_ms: i64 = context.evaluate_pin("initial_delay_ms").await?;
        let backoff_type: String = context.evaluate_pin("backoff_type").await?;

        if max_retries < 1 || initial_delay_ms < 0 {
            return Err(flow_like_types::anyhow!(
                "Retry count must be positive and delay nonnegative"
            ));
        }
        retry_delay(initial_delay_ms as u64, 1, &backoff_type)?;

        for attempt in 1..=max_retries {
            context.check_cancelled()?;
            context.set_pin_value("attempt", json!(attempt)).await?;

            let outcome = super::branch::run_branch(context, "exec_attempt", None).await?;
            crate::browser::selector::optional_output(
                context,
                "last_error",
                json!(match &outcome {
                    super::branch::BranchResult::Failed(error) => error.as_str(),
                    _ => "",
                }),
            )
            .await?;

            let should_retry: bool = context.evaluate_pin("should_retry").await?;

            if outcome == super::branch::BranchResult::Completed && !should_retry {
                context
                    .set_pin_value("total_attempts", json!(attempt))
                    .await?;
                context.activate_exec_pin("exec_success").await?;
                return Ok(());
            }

            if attempt < max_retries {
                let delay = retry_delay(initial_delay_ms as u64, attempt as u32, &backoff_type)?;
                super::branch::delay(context, std::time::Duration::from_millis(delay)).await?;
            }
        }

        context
            .set_pin_value("total_attempts", json!(max_retries))
            .await?;
        context.activate_exec_pin("exec_exhausted").await?;

        Ok(())
    }
}

fn retry_delay(initial: u64, attempt: u32, strategy: &str) -> flow_like_types::Result<u64> {
    let factor = match strategy {
        "Constant" => 1,
        "Linear" => attempt as u64,
        "Exponential" => 1u64
            .checked_shl(attempt.saturating_sub(1))
            .ok_or_else(|| flow_like_types::anyhow!("Retry delay overflow"))?,
        _ => return Err(flow_like_types::anyhow!("Unknown backoff strategy")),
    };
    initial
        .checked_mul(factor)
        .ok_or_else(|| flow_like_types::anyhow!("Retry delay overflow"))
}

#[cfg(test)]
mod tests {
    use super::retry_delay;
    #[test]
    fn backoff_rejects_overflow() {
        assert_eq!(retry_delay(100, 3, "Exponential").unwrap(), 400);
        assert!(retry_delay(u64::MAX, 2, "Linear").is_err());
        assert!(retry_delay(1, 65, "Exponential").is_err());
    }
}
