use flow_like::flow::execution::context::ExecutionContext;
use std::time::Duration;

pub(crate) use flow_like::flow::execution::branch::{BranchResult, run_branch};

pub(crate) async fn delay(
    context: &ExecutionContext,
    duration: Duration,
) -> flow_like_types::Result<()> {
    let deadline = tokio::time::Instant::now()
        .checked_add(duration)
        .ok_or_else(|| flow_like_types::anyhow!("Delay is too large"))?;
    if let Some(token) = context.get_cancellation_token() {
        tokio::select! {
            biased;
            _ = token.cancelled() => return Err(flow_like_types::anyhow!("Execution was cancelled")),
            _ = tokio::time::sleep_until(deadline) => {}
        }
    } else {
        tokio::time::sleep_until(deadline).await;
    }
    Ok(())
}
