use flow_like::flow::execution::{
    context::ExecutionContext,
    internal_node::{InternalNode, InternalNodeError},
};
use flow_like_types::tokio_util::sync::CancellationToken;
use futures::FutureExt;
use std::{panic::AssertUnwindSafe, time::Duration};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum BranchResult {
    Completed,
    Failed(String),
    TimedOut,
}

/// Run a branch here, then clear its pin so the executor cannot run it a second time.
pub(crate) async fn run_branch(
    context: &mut ExecutionContext,
    pin_name: &str,
    timeout: Option<Duration>,
) -> flow_like_types::Result<BranchResult> {
    context.check_cancelled()?;
    let pin = context.get_pin_by_name(pin_name).await?;
    let nodes = pin.get_connected_nodes();
    let cancellation = context
        .get_cancellation_token()
        .map(|token| token.child_token())
        .unwrap_or_else(CancellationToken::new);
    let deadline = timeout
        .map(|duration| {
            tokio::time::Instant::now()
                .checked_add(duration)
                .ok_or_else(|| flow_like_types::anyhow!("Timeout is too large"))
        })
        .transpose()?;
    context.activate_exec_pin_ref(&pin).await?;
    let mut outcome = BranchResult::Completed;
    for node in nodes {
        let mut sub = context.create_sub_context(&node).await;
        sub.set_cancellation_token(cancellation.clone());
        outcome = {
            let mut recursion = None;
            let operation = AssertUnwindSafe(InternalNode::trigger(&mut sub, &mut recursion, true))
                .catch_unwind();
            tokio::pin!(operation);
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => BranchResult::Failed("Execution was cancelled".into()),
                _ = async {
                    if let Some(deadline) = deadline { tokio::time::sleep_until(deadline).await; }
                    else { std::future::pending::<()>().await; }
                } => { cancellation.cancel(); BranchResult::TimedOut },
                result = &mut operation => match result {
                    Ok(Ok(())) => BranchResult::Completed,
                    Ok(Err(InternalNodeError::DependencyFailed(message) | InternalNodeError::ExecutionFailed(message) | InternalNodeError::PinNotReady(message))) => BranchResult::Failed(message),
                    Err(_) => BranchResult::Failed("Action panicked".into()),
                }
            }
        };
        if outcome != BranchResult::Completed {
            sub.set_state(flow_like::flow::node::NodeState::Error).await;
        }
        sub.end_trace();
        context.push_sub_context(&mut sub);
        if outcome != BranchResult::Completed {
            break;
        }
    }
    context.deactivate_exec_pin_ref(&pin).await?;
    context.check_cancelled()?;
    Ok(outcome)
}

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
