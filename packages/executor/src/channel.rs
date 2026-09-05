//! Builds the run's reply channel from the grant the API shipped in the execution payload.

use std::sync::Arc;

use flow_like_types::channel::{
    Channel, ChannelClientDescriptor, ChannelExecutorGrant, ChannelGrant, ChannelHandle,
    HubChannelStore, PollingChannel, now_unix,
};

/// Grant-less runs (an API that predates channels) still get a hub-polling channel so the run
/// can register requests; clients cannot answer them without a token, so waits simply expire.
const LEGACY_HANDLE_TTL_SECS: i64 = 24 * 60 * 60;

pub async fn build_run_channel(
    grant: Option<&ChannelGrant>,
    run_id: &str,
    hub_url: &str,
    token: Option<&str>,
) -> flow_like_types::Result<Arc<dyn Channel>> {
    let Some(grant) = grant else {
        tracing::warn!(
            run_id,
            "execution payload carries no channel grant; client replies will not reach this run"
        );
        return hub_channel(hub_url, token, legacy_handle(run_id, hub_url));
    };

    if grant.channel_id != run_id {
        return Err(flow_like_types::anyhow!(
            "channel grant is for '{}' but the run is '{}'",
            grant.channel_id,
            run_id
        ));
    }

    match &grant.executor {
        ChannelExecutorGrant::Http {} => hub_channel(hub_url, token, grant.client.clone()),
        _ => flow_like_channels::build_executor_channel(grant)
            .await?
            .ok_or_else(|| flow_like_types::anyhow!("channel grant produced no transport")),
    }
}

fn hub_channel(
    hub_url: &str,
    token: Option<&str>,
    handle: ChannelHandle,
) -> flow_like_types::Result<Arc<dyn Channel>> {
    let token = token.ok_or_else(|| {
        flow_like_types::anyhow!("no token available to register channel requests on the hub")
    })?;
    Ok(Arc::new(PollingChannel::new(
        HubChannelStore::new(hub_url, token),
        handle,
    )))
}

fn legacy_handle(run_id: &str, hub_url: &str) -> ChannelHandle {
    ChannelHandle {
        channel_id: run_id.to_string(),
        request_id: None,
        expires_at: now_unix() + LEGACY_HANDLE_TTL_SECS,
        transport: ChannelClientDescriptor::Http {
            push_url: format!(
                "{}/api/v1/channels/{}/push",
                hub_url.trim_end_matches('/'),
                run_id
            ),
            token: String::new(),
        },
        fallback: None,
    }
}
