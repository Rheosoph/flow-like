use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::channel::ChannelOutcome;
use flow_like_types::{Value, interaction::InteractionRequest};
use std::time::Duration;

pub struct InteractionWaitResult {
    pub responded: bool,
    pub value: Value,
}

/// Register the interaction on the run's channel, stream it to the client with its reply handle
/// attached, and block until the client answers, the TTL elapses, or the run is cancelled.
pub async fn wait_for_interaction_response(
    context: &mut ExecutionContext,
    mut request: InteractionRequest,
    ttl_seconds: u64,
) -> flow_like_types::Result<InteractionWaitResult> {
    let channel = context.channel()?;
    let ticket = channel.open(Duration::from_secs(ttl_seconds)).await?;

    request.id = ticket.request_id.clone();
    request.expires_at = ticket.expires_at as u64;
    request.run_id = Some(context.run_id().to_string());
    request.app_id = context
        .execution_cache
        .as_ref()
        .map(|cache| cache.app_id.clone());
    request.channel = Some(ticket.handle.clone());

    if let Err(error) = context
        .stream_response("interaction_request", request)
        .await
    {
        channel.abandon(&ticket).await;
        return Err(error);
    }

    let outcome = channel.wait(&ticket, context.cancellation_token()).await?;

    Ok(match outcome {
        ChannelOutcome::Responded(value) => InteractionWaitResult {
            responded: true,
            value,
        },
        ChannelOutcome::Expired | ChannelOutcome::Cancelled | ChannelOutcome::Closed => {
            InteractionWaitResult {
                responded: false,
                value: Value::Null,
            }
        }
    })
}
