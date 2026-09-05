use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{
    async_trait,
    sync::Mutex,
    tokio::{self, time},
    tokio_util::sync::CancellationToken,
};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct TimeoutNode {}

impl TimeoutNode {
    pub fn new() -> Self {
        TimeoutNode {}
    }
}

#[async_trait]
impl NodeLogic for TimeoutNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_timeout",
            "Timeout",
            "Executes with a timeout, branching based on completion",
            "Control",
        );
        node.set_flowscript_name("control", "timeout");

        node.set_long_running(true);
        node.add_icon("/flow/icons/clock.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "timeout_ms",
            "Timeout (ms)",
            "Timeout duration in milliseconds",
            VariableType::Float,
        )
        .set_default_value(Some(flow_like_types::json::json!(5000.0)));

        node.add_output_pin(
            "exec_body",
            "Execute",
            "Execution path to run with timeout",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_completed",
            "Completed",
            "Execution completed within timeout",
            VariableType::Execution,
        );

        node.add_output_pin(
            "exec_timed_out",
            "Timed Out",
            "Execution exceeded timeout duration",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let timeout_duration: f64 = context.evaluate_pin("timeout_ms").await?;
        let timeout_duration = time::Duration::from_millis(timeout_duration as u64);

        let exec_body_pin = context.get_pin_by_name("exec_body").await?;
        let completed_pin = context.get_pin_by_name("exec_completed").await?;
        let timed_out_pin = context.get_pin_by_name("exec_timed_out").await?;

        context.deactivate_exec_pin_ref(&completed_pin).await?;
        context.deactivate_exec_pin_ref(&timed_out_pin).await?;
        context.activate_exec_pin_ref(&exec_body_pin).await?;

        let nodes = exec_body_pin.get_connected_nodes();

        let (timed_out, sub_contexts) = if nodes.is_empty() {
            (false, Vec::new())
        } else {
            let cancellation_token = CancellationToken::new();

            let mut initial_contexts = Vec::new();
            for node in &nodes {
                let mut sub = context.create_sub_context(node).await;
                sub.set_cancellation_token(cancellation_token.clone());
                initial_contexts.push(sub);
            }

            // Separate containers: pending (to execute) and completed (done)
            let pending = Arc::new(Mutex::new(initial_contexts));
            let completed: Arc<Mutex<Vec<ExecutionContext>>> = Arc::new(Mutex::new(Vec::new()));

            let pending_for_exec = pending.clone();
            let completed_for_exec = completed.clone();
            let token_for_exec = cancellation_token.clone();

            let execution = async move {
                loop {
                    // Take one context from pending
                    let maybe_sub = pending_for_exec.lock().await.pop();

                    let Some(mut sub) = maybe_sub else {
                        break;
                    };

                    if token_for_exec.is_cancelled() {
                        sub.log_message("Execution was cancelled due to timeout", LogLevel::Warn);
                        sub.end_trace();
                        completed_for_exec.lock().await.push(sub);
                        break;
                    }

                    let mut recursion_guard = None;
                    if let Err(err) =
                        InternalNode::trigger(&mut sub, &mut recursion_guard, true).await
                    {
                        sub.log_message(
                            &format!("Error running node in timeout: {err:?}"),
                            LogLevel::Error,
                        );
                    }

                    sub.end_trace();
                    completed_for_exec.lock().await.push(sub);
                }
            };

            let handle = tokio::spawn(execution);
            let abort_handle = handle.abort_handle();

            let timed_out = tokio::select! {
                biased;
                _ = time::sleep(timeout_duration) => {
                    cancellation_token.cancel();
                    // Brief wait for cooperative cancellation
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    abort_handle.abort();
                    true
                }
                _ = handle => {
                    false
                }
            };

            let result = {
                let mut guard = completed.lock().await;
                std::mem::take(&mut *guard)
            };

            (timed_out, result)
        };

        for mut sub in sub_contexts {
            context.push_sub_context(&mut sub);
        }

        context.deactivate_exec_pin_ref(&exec_body_pin).await?;

        if timed_out {
            context.log_message(
                &format!(
                    "Execution timed out after {} ms",
                    timeout_duration.as_millis()
                ),
                LogLevel::Warn,
            );
            context.activate_exec_pin_ref(&timed_out_pin).await?;
        } else {
            context.activate_exec_pin_ref(&completed_pin).await?;
        }

        Ok(())
    }
}
