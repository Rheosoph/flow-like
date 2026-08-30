//! Cloud transports for Channels (see `flow_like_types::channel`).
//!
//! Two halves, both feature-gated per cloud so an executor links only its own transport:
//! - **waiter side** — a [`flow_like_types::channel::Channel`] the executor holds for the run's
//!   lifetime, built from the [`ChannelExecutorGrant`] shipped in the execution payload;
//! - **API side** — a [`ChannelForwarder`] that pushes a [`ChannelPush`] received on the HTTP
//!   fallback endpoint onto the transport, plus the token/credential helpers the API needs to
//!   mint client and executor grants.

use std::sync::Arc;
use std::time::Duration;

use flow_like_types::Value;
use flow_like_types::async_trait;
use flow_like_types::channel::{
    Channel, ChannelExecutorGrant, ChannelGrant, ChannelHandle, ChannelOutcome, ChannelPush,
    ChannelTicket,
};
use flow_like_types::tokio::sync::OnceCell;
use flow_like_types::tokio_util::sync::CancellationToken;

#[cfg(feature = "aws")]
pub mod aws;
#[cfg(feature = "azure")]
pub mod azure;
#[cfg(feature = "gcp")]
pub mod gcp;

/// API-side: deliver a push the client could not send itself onto the transport the waiter
/// listens on.
#[async_trait]
pub trait ChannelForwarder: Send + Sync {
    fn transport(&self) -> &'static str;
    async fn forward(&self, push: &ChannelPush) -> flow_like_types::Result<()>;
}

fn transport_name(grant: &ChannelExecutorGrant) -> &'static str {
    match grant {
        ChannelExecutorGrant::Http {} => "http",
        ChannelExecutorGrant::AwsMqtt { .. } => "aws_mqtt",
        ChannelExecutorGrant::AzureWebPubSub { .. } => "azure_web_pubsub",
        ChannelExecutorGrant::GcpFirebaseRtdb { .. } => "gcp_firebase_rtdb",
    }
}

/// Whether this binary can serve the grant's transport. Checked before any network work so a
/// misbuilt executor fails at run init instead of at the first request.
pub fn transport_compiled(grant: &ChannelExecutorGrant) -> bool {
    match grant {
        ChannelExecutorGrant::Http {} => true,
        ChannelExecutorGrant::AwsMqtt { .. } => cfg!(feature = "aws"),
        ChannelExecutorGrant::AzureWebPubSub { .. } => cfg!(feature = "azure"),
        ChannelExecutorGrant::GcpFirebaseRtdb { .. } => cfg!(feature = "gcp"),
    }
}

/// Open the transport connection for a cloud grant now (subscribe / join / stream). Use this
/// where unsolicited pushes must be received before the first request goes out — the global
/// chat, whose stop/steer can arrive at any time. Board runs use [`build_executor_channel`].
pub async fn connect_executor_channel(
    grant: &ChannelGrant,
) -> flow_like_types::Result<Arc<dyn Channel>> {
    match &grant.executor {
        ChannelExecutorGrant::Http {} => Err(flow_like_types::anyhow!(
            "the http channel transport has no connection to open; build a PollingChannel instead"
        )),
        #[cfg(feature = "aws")]
        ChannelExecutorGrant::AwsMqtt { .. } => aws::AwsIotChannel::connect(grant).await,
        #[cfg(feature = "azure")]
        ChannelExecutorGrant::AzureWebPubSub { .. } => {
            azure::AzureWebPubSubChannel::connect(grant).await
        }
        #[cfg(feature = "gcp")]
        ChannelExecutorGrant::GcpFirebaseRtdb { .. } => {
            gcp::FirebaseRtdbChannel::connect(grant).await
        }
        #[allow(unreachable_patterns)]
        other => Err(flow_like_types::anyhow!(
            "channel transport '{}' is not compiled into this executor",
            transport_name(other)
        )),
    }
}

/// Build the waiter-side channel for a non-HTTP grant without touching the network: the
/// connection is opened by the first [`Channel::open`], so a run that never asks its client for
/// anything never connects. Returns `Ok(None)` for HTTP grants (the caller builds a
/// `PollingChannel` over the hub) and an error when the grant's transport is not compiled in.
pub async fn build_executor_channel(
    grant: &ChannelGrant,
) -> flow_like_types::Result<Option<Arc<dyn Channel>>> {
    if matches!(grant.executor, ChannelExecutorGrant::Http {}) {
        return Ok(None);
    }
    if !transport_compiled(&grant.executor) {
        return Err(flow_like_types::anyhow!(
            "channel transport '{}' is not compiled into this executor",
            transport_name(&grant.executor)
        ));
    }
    Ok(Some(Arc::new(LazyChannel::new(grant.clone()))))
}

/// A cloud channel that connects on first use. Until then it answers as an idle channel: no
/// inbound messages, not cancelled, nothing to close. A failed connect is retried by the next
/// `open`.
pub struct LazyChannel {
    grant: ChannelGrant,
    inner: OnceCell<Arc<dyn Channel>>,
}

impl LazyChannel {
    pub fn new(grant: ChannelGrant) -> Self {
        Self {
            grant,
            inner: OnceCell::new(),
        }
    }

    pub fn is_connected(&self) -> bool {
        self.inner.initialized()
    }

    async fn connected(&self) -> flow_like_types::Result<&Arc<dyn Channel>> {
        self.inner
            .get_or_try_init(|| connect_executor_channel(&self.grant))
            .await
    }
}

#[async_trait]
impl Channel for LazyChannel {
    fn channel_id(&self) -> &str {
        &self.grant.channel_id
    }

    fn handle(&self) -> ChannelHandle {
        self.grant.client.clone()
    }

    async fn open(&self, ttl: Duration) -> flow_like_types::Result<ChannelTicket> {
        self.connected().await?.open(ttl).await
    }

    async fn wait(
        &self,
        ticket: &ChannelTicket,
        cancel: Option<CancellationToken>,
    ) -> flow_like_types::Result<ChannelOutcome> {
        match self.inner.get() {
            Some(channel) => channel.wait(ticket, cancel).await,
            None => Err(flow_like_types::anyhow!(
                "channel '{}' request '{}' was never opened on this channel",
                self.grant.channel_id,
                ticket.request_id
            )),
        }
    }

    async fn abandon(&self, ticket: &ChannelTicket) {
        if let Some(channel) = self.inner.get() {
            channel.abandon(ticket).await;
        }
    }

    async fn drain_inbound(&self) -> Vec<Value> {
        match self.inner.get() {
            Some(channel) => channel.drain_inbound().await,
            None => Vec::new(),
        }
    }

    async fn is_cancelled(&self) -> bool {
        match self.inner.get() {
            Some(channel) => channel.is_cancelled().await,
            None => false,
        }
    }

    async fn close(&self) {
        if let Some(channel) = self.inner.get() {
            channel.close().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::{ChannelClientDescriptor, now_unix};

    fn grant(executor: ChannelExecutorGrant) -> ChannelGrant {
        ChannelGrant {
            channel_id: "run".into(),
            expires_at: now_unix() + 60,
            executor,
            client: ChannelHandle {
                channel_id: "run".into(),
                request_id: None,
                expires_at: now_unix() + 60,
                transport: ChannelClientDescriptor::Http {
                    push_url: "https://api/api/v1/channels/run/push".into(),
                    token: "t".into(),
                },
                fallback: None,
            },
        }
    }

    #[flow_like_types::tokio::test]
    async fn http_grant_builds_nothing() {
        let built = build_executor_channel(&grant(ChannelExecutorGrant::Http {}))
            .await
            .unwrap();
        assert!(built.is_none());
    }

    #[flow_like_types::tokio::test]
    async fn lazy_channel_is_idle_until_opened() {
        let lazy = LazyChannel::new(grant(ChannelExecutorGrant::AzureWebPubSub {
            url: "wss://example.invalid/client/hubs/h?access_token=x".into(),
            group: "run:run".into(),
        }));
        assert!(!lazy.is_connected());
        assert_eq!(lazy.channel_id(), "run");
        assert!(lazy.drain_inbound().await.is_empty());
        assert!(!lazy.is_cancelled().await);
        lazy.close().await;
        assert!(!lazy.is_connected());
        assert!(matches!(
            lazy.handle().transport,
            ChannelClientDescriptor::Http { .. }
        ));
    }

    #[flow_like_types::tokio::test]
    async fn uncompiled_transport_fails_at_build() {
        let executor = ChannelExecutorGrant::AzureWebPubSub {
            url: "wss://example.invalid/client/hubs/h".into(),
            group: "run:run".into(),
        };
        let result = build_executor_channel(&grant(executor.clone())).await;
        if transport_compiled(&executor) {
            assert!(result.unwrap().is_some());
        } else {
            let error = result.err().expect("missing transport must fail at build");
            assert!(error.to_string().contains("not compiled"), "{error}");
        }
    }
}
