use flow_like::flow::execution::context::ExecutionContext;
use flow_like_types::{Value, interaction::InteractionRequest};

pub struct InteractionWaitResult {
    pub responded: bool,
    pub value: Value,
}

#[cfg(all(feature = "remote", not(feature = "local")))]
fn get_app_id(context: &ExecutionContext) -> String {
    context
        .execution_cache
        .as_ref()
        .map(|cache| cache.app_id.clone())
        .unwrap_or_default()
}

/// Wait for an interaction response.
///
/// With `local` feature: Uses polling-based interaction handling (desktop app)
/// With `remote` feature: Uses SSE-based interaction via API endpoint
#[cfg(feature = "local")]
pub async fn wait_for_interaction_response(
    context: &mut ExecutionContext,
    request: InteractionRequest,
    ttl_seconds: u64,
) -> flow_like_types::Result<InteractionWaitResult> {
    use flow_like_types::interaction::{
        InteractionPollResult, poll_interaction_response, register_interaction,
    };

    let interaction_id = request.id.clone();

    register_interaction(request.clone()).await;
    context
        .stream_response("interaction_request", request)
        .await?;

    let poll_interval = std::time::Duration::from_millis(500);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(ttl_seconds);

    let mut responded = false;
    let mut response_value = Value::Null;

    while std::time::Instant::now() < deadline {
        context.check_cancelled()?;

        match poll_interaction_response(&interaction_id).await {
            InteractionPollResult::Responded { value } => {
                response_value = value;
                responded = true;
                break;
            }
            InteractionPollResult::Expired | InteractionPollResult::Cancelled => break,
            InteractionPollResult::Pending => {}
        }

        flow_like_types::tokio::time::sleep(poll_interval).await;
    }

    Ok(InteractionWaitResult {
        responded,
        value: response_value,
    })
}

/// Wait for an interaction response.
///
/// With `local` feature: Uses polling-based interaction handling (desktop app)
/// With `remote` feature: Uses SSE-based interaction via API endpoint
#[cfg(all(feature = "remote", not(feature = "local")))]
pub async fn wait_for_interaction_response(
    context: &mut ExecutionContext,
    request: InteractionRequest,
    ttl_seconds: u64,
) -> flow_like_types::Result<InteractionWaitResult> {
    use flow_like_types::interaction::{RemoteInteractionParams, create_remote_interaction_stream};

    let hub_url = context.profile.hub.clone();
    let token = context
        .token
        .clone()
        .ok_or_else(|| flow_like_types::anyhow!("No user token available for remote execution"))?;
    let app_id = get_app_id(context);

    if hub_url.is_empty() {
        return Err(flow_like_types::anyhow!(
            "No hub URL configured for remote execution"
        ));
    }

    let params = RemoteInteractionParams {
        hub_url: &hub_url,
        token: &token,
        app_id: &app_id,
        ttl_seconds,
        request,
    };

    let result = create_remote_interaction_stream(params, |request_with_jwt| {
        let context_weak = context.run.clone();
        let callback = context.callback().clone();
        Box::pin(async move {
            if let Some(_run) = context_weak.upgrade() {
                let event = flow_like_types::intercom::InterComEvent::with_type(
                    "interaction_request",
                    request_with_jwt,
                );
                if let Some(cb) = &callback {
                    let _ = cb(event).await;
                }
            }
        })
    })
    .await?;

    Ok(InteractionWaitResult {
        responded: result.responded,
        value: result.value,
    })
}

/// Fallback when neither local nor remote feature is enabled.
/// This should not happen in practice - one of the features should always be enabled.
#[cfg(not(any(feature = "local", feature = "remote")))]
pub async fn wait_for_interaction_response(
    _context: &mut ExecutionContext,
    _request: InteractionRequest,
    _ttl_seconds: u64,
) -> flow_like_types::Result<InteractionWaitResult> {
    Err(flow_like_types::anyhow!(
        "Either 'local' or 'remote' feature must be enabled for interaction support"
    ))
}
