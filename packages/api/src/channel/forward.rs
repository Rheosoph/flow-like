//! Push fallback for cloud transports: a client that could not reach the transport posts to
//! the HTTP endpoint and the API relays the push to the waiter's connection.

use std::sync::Arc;

use flow_like_channels::ChannelForwarder;
use flow_like_types::channel::ChannelPush;

use crate::error::ApiError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardOutcome {
    Delivered,
    /// The transport reported that nobody is listening for this channel (the waiter finished or
    /// never connected); the client gets `accepted: false` and this message.
    Undeliverable(String),
}

pub async fn forward_push(
    forwarder: Option<&Arc<dyn ChannelForwarder>>,
    push: &ChannelPush,
) -> Result<ForwardOutcome, ApiError> {
    let forwarder = forwarder.ok_or_else(|| {
        ApiError::service_unavailable(
            "This channel expects a cloud transport reply but the API has no forwarder configured",
        )
    })?;
    match forwarder.forward(push).await {
        Ok(()) => Ok(ForwardOutcome::Delivered),
        Err(error) if is_not_connected(&error.to_string()) => {
            tracing::debug!(%error, channel_id = %push.channel_id, transport = forwarder.transport(), "channel push had no listener");
            Ok(ForwardOutcome::Undeliverable(error.to_string()))
        }
        Err(error) => {
            tracing::warn!(%error, channel_id = %push.channel_id, transport = forwarder.transport(), "channel push forward failed");
            Err(ApiError::bad_gateway(format!(
                "Forwarding the channel push onto {} failed: {error}",
                forwarder.transport()
            )))
        }
    }
}

/// The forwarders phrase a missing listener as a 404 from the transport; every other failure
/// is an infrastructure error the client should retry against.
pub fn is_not_connected(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("404")
        || message.contains("no waiter connected")
        || message.contains("not connected")
        || message.contains("not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::async_trait;
    use flow_like_types::channel::ChannelPushKind;

    struct Stub(Result<(), &'static str>);

    #[async_trait]
    impl ChannelForwarder for Stub {
        fn transport(&self) -> &'static str {
            "stub"
        }
        async fn forward(&self, _push: &ChannelPush) -> flow_like_types::Result<()> {
            self.0.map_err(|e| flow_like_types::anyhow!("{e}"))
        }
    }

    fn push() -> ChannelPush {
        ChannelPush {
            channel_id: "run-1".into(),
            request_id: Some("r".into()),
            kind: ChannelPushKind::Reply,
            value: serde_json::Value::Null,
        }
    }

    #[tokio::test]
    async fn missing_forwarder_is_503() {
        let error = forward_push(None, &push()).await.unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn outcomes_follow_the_transport_error() {
        let ok: Arc<dyn ChannelForwarder> = Arc::new(Stub(Ok(())));
        assert_eq!(
            forward_push(Some(&ok), &push()).await.unwrap(),
            ForwardOutcome::Delivered
        );
        let gone: Arc<dyn ChannelForwarder> = Arc::new(Stub(Err(
            "channel 'run-1' has no waiter connected as 'run-run-1' (AWS IoT direct message returned 404)",
        )));
        assert!(matches!(
            forward_push(Some(&gone), &push()).await.unwrap(),
            ForwardOutcome::Undeliverable(_)
        ));
        let broken: Arc<dyn ChannelForwarder> = Arc::new(Stub(Err("tls handshake failed")));
        let error = forward_push(Some(&broken), &push()).await.unwrap_err();
        assert_eq!(error.status(), axum::http::StatusCode::BAD_GATEWAY);
    }
}
