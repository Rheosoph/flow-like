//! Clients post channel input to the API, which relays it to the configured waiter.
//! Older direct provider handles can use the same endpoint as their HTTP fallback.
//!
//! The API forwards with its own provider credentials, so it enforces what the provider rules
//! enforced on direct clients: the channel deadline and the provider's message size.

use std::sync::Arc;

use axum::http::StatusCode;
use flow_like_channels::ChannelForwarder;
use flow_like_types::channel::{
    CHANNEL_TRANSPORT_AWS_MQTT, CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB, CHANNEL_TRANSPORT_NATS,
    ChannelPush, ChannelPushKind, now_unix,
};

use super::redact_provider_error;
use crate::error::ApiError;

/// The Firebase rules cap the stored push string at 16384 characters (UTF-16 code units).
const GCP_MAX_PUSH_CHARS: usize = 16_384;
/// One AWS IoT message; replies with a request id are chunked up to 2 MiB. Keep in sync with
/// `channels/src/aws/reply_chunks.rs` and `ui/lib/channel/aws-reply-chunks.ts`.
const AWS_MAX_MESSAGE_BYTES: usize = 128 * 1024;
const AWS_MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;
/// NATS caps the `{id, push}` envelope at 1 MiB; the reserve covers the delivery id.
const NATS_MAX_PUSH_BYTES: usize = 1024 * 1024 - 256;
/// Azure Web PubSub's message limit.
const AZURE_MAX_PUSH_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForwardOutcome {
    Delivered,
    /// The transport reported that nobody is listening for this channel (the waiter finished or
    /// never connected); the client gets `accepted: false`.
    Undeliverable,
}

pub async fn forward_push(
    forwarder: Option<&Arc<dyn ChannelForwarder>>,
    push: &ChannelPush,
    expires_at: i64,
) -> Result<ForwardOutcome, ApiError> {
    forward_push_with_id(forwarder, push, None, expires_at).await
}

/// `expires_at` is the channel deadline from the responder token.
pub async fn forward_push_with_id(
    forwarder: Option<&Arc<dyn ChannelForwarder>>,
    push: &ChannelPush,
    delivery_id: Option<&str>,
    expires_at: i64,
) -> Result<ForwardOutcome, ApiError> {
    let forwarder = forwarder.ok_or_else(|| {
        ApiError::service_unavailable(
            "This channel expects a cloud transport reply but the API has no forwarder configured",
        )
    })?;
    if expires_at <= now_unix() {
        return Err(ApiError::gone("This channel has expired"));
    }
    check_push_size(forwarder.transport(), push)?;
    let result = match delivery_id {
        Some(id) => forwarder.forward_with_id(push, id).await,
        None => forwarder.forward(push).await,
    };
    let Err(error) = result else {
        return Ok(ForwardOutcome::Delivered);
    };
    let detail = redact_provider_error(&error);
    if is_not_connected(&detail) {
        tracing::debug!(error = %detail, channel_id = %push.channel_id, transport = forwarder.transport(), "channel push had no listener");
        return Ok(ForwardOutcome::Undeliverable);
    }
    tracing::warn!(error = %detail, channel_id = %push.channel_id, transport = forwarder.transport(), "channel push forward failed");
    Err(ApiError::coded(
        StatusCode::BAD_GATEWAY,
        "BAD_GATEWAY",
        "The channel transport did not accept the message; retry shortly",
    ))
}

/// Measures the push the way the provider did on the direct client path.
fn check_push_size(transport: &str, push: &ChannelPush) -> Result<(), ApiError> {
    let body = serde_json::to_string(push)
        .map_err(|e| ApiError::internal(format!("Serializing a channel push failed: {e}")))?;
    let chunked_reply = push.kind == ChannelPushKind::Reply
        && push.request_id.as_deref().is_some_and(|id| !id.is_empty());
    let (size, limit) = match transport {
        CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB => (body.encode_utf16().count(), GCP_MAX_PUSH_CHARS),
        CHANNEL_TRANSPORT_AWS_MQTT if chunked_reply => (body.len(), AWS_MAX_REPLY_BYTES),
        CHANNEL_TRANSPORT_AWS_MQTT => (body.len(), AWS_MAX_MESSAGE_BYTES),
        CHANNEL_TRANSPORT_NATS => (body.len(), NATS_MAX_PUSH_BYTES),
        _ => (body.len(), AZURE_MAX_PUSH_BYTES),
    };
    if size > limit {
        return Err(ApiError::coded(
            StatusCode::PAYLOAD_TOO_LARGE,
            "PAYLOAD_TOO_LARGE",
            format!("This message is too large for the channel transport ({size} > {limit})"),
        ));
    }
    Ok(())
}

/// The forwarders phrase a missing listener as a 404 from the transport; every other failure
/// is an infrastructure error the client should retry against.
pub fn is_not_connected(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("404")
        || message.contains("no waiter connected")
        || message.contains("no connected waiter")
        || message.contains("not connected")
        || message.contains("not found")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;
    use flow_like_types::async_trait;
    use flow_like_types::channel::CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    struct Counting {
        transport: &'static str,
        calls: AtomicUsize,
    }

    impl Counting {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl ChannelForwarder for Counting {
        fn transport(&self) -> &'static str {
            self.transport
        }
        async fn forward(&self, _push: &ChannelPush) -> flow_like_types::Result<()> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
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

    fn live() -> i64 {
        now_unix() + 60
    }

    /// A push whose serialized body is exactly `size` ASCII bytes.
    fn sized(kind: ChannelPushKind, request_id: Option<&str>, size: usize) -> ChannelPush {
        let mut push = ChannelPush {
            request_id: request_id.map(str::to_owned),
            kind,
            value: serde_json::Value::String(String::new()),
            ..push()
        };
        let overhead = serde_json::to_string(&push).unwrap().len();
        push.value = serde_json::Value::String("x".repeat(size - overhead));
        push
    }

    fn with_text(push: &ChannelPush, text: String) -> ChannelPush {
        ChannelPush {
            value: serde_json::Value::String(text),
            ..push.clone()
        }
    }

    async fn body_text(error: ApiError) -> String {
        let response = error.into_response();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn missing_forwarder_is_503() {
        let error = forward_push(None, &push(), live()).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn outcomes_follow_the_transport_error() {
        let ok: Arc<dyn ChannelForwarder> = Arc::new(Stub(Ok(())));
        assert_eq!(
            forward_push(Some(&ok), &push(), live()).await.unwrap(),
            ForwardOutcome::Delivered
        );
        let gone: Arc<dyn ChannelForwarder> = Arc::new(Stub(Err(
            "channel 'run-1' has no waiter connected as 'run-run-1' (AWS IoT direct message returned 404)",
        )));
        assert_eq!(
            forward_push(Some(&gone), &push(), live()).await.unwrap(),
            ForwardOutcome::Undeliverable
        );
        let broken: Arc<dyn ChannelForwarder> = Arc::new(Stub(Err("tls handshake failed")));
        let error = forward_push(Some(&broken), &push(), live())
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn provider_errors_never_reach_the_client() {
        for detail in [
            "sts:AssumeRole on arn:aws:iam::123456789012:role/channels failed: AccessDenied",
            "channel run-1: Azure Web PubSub send to group run:run-1 returned 500 Internal Server Error: {\"code\":\"internal\",\"target\":\"https://demo.webpubsub.azure.com/api/hubs/channels\"}",
            "firebase POST channels/run-1/inbound failed: error sending request for url (https://demo.firebaseio.com/channels/run-1/inbound.json?access_token=ya29.a404secret&print=silent)",
        ] {
            let broken: Arc<dyn ChannelForwarder> = Arc::new(Stub(Err(detail)));
            let error = forward_push(Some(&broken), &push(), live())
                .await
                .unwrap_err();
            assert_eq!(error.status(), StatusCode::BAD_GATEWAY, "{detail}");
            let body = body_text(error).await;
            for secret in [
                "arn:aws",
                "123456789012",
                "webpubsub.azure.com",
                "firebaseio",
                "ya29",
                "access_token",
                "run-1",
                "stub",
            ] {
                assert!(!body.contains(secret), "{secret} leaked: {body}");
            }
        }
    }

    #[tokio::test]
    async fn oversized_pushes_never_reach_the_provider() {
        use ChannelPushKind::{Cancel, Inbound, Reply};
        let (gcp, aws, nats) = (
            CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB,
            CHANNEL_TRANSPORT_AWS_MQTT,
            CHANNEL_TRANSPORT_NATS,
        );
        let azure = CHANNEL_TRANSPORT_AZURE_WEB_PUBSUB;
        let cases = [
            (gcp, Inbound, None, GCP_MAX_PUSH_CHARS),
            (aws, Inbound, None, AWS_MAX_MESSAGE_BYTES),
            (aws, Reply, None, AWS_MAX_MESSAGE_BYTES),
            (aws, Reply, Some("r"), AWS_MAX_REPLY_BYTES),
            (nats, Inbound, None, NATS_MAX_PUSH_BYTES),
            (azure, Cancel, None, AZURE_MAX_PUSH_BYTES),
        ];
        for (transport, kind, request_id, limit) in cases {
            let counting = Arc::new(Counting {
                transport,
                calls: AtomicUsize::new(0),
            });
            let forwarder: Arc<dyn ChannelForwarder> = counting.clone();

            let fits = sized(kind, request_id, limit);
            assert_eq!(
                forward_push(Some(&forwarder), &fits, live()).await.unwrap(),
                ForwardOutcome::Delivered,
                "{transport} {kind:?}"
            );
            let oversized = sized(kind, request_id, limit + 1);
            let error = forward_push(Some(&forwarder), &oversized, live())
                .await
                .unwrap_err();
            assert_eq!(error.status(), StatusCode::PAYLOAD_TOO_LARGE, "{transport}");
            assert_eq!(counting.calls(), 1, "{transport} {kind:?}");
        }
    }

    #[tokio::test]
    async fn expired_channels_never_reach_the_provider() {
        let counting = Arc::new(Counting {
            transport: CHANNEL_TRANSPORT_NATS,
            calls: AtomicUsize::new(0),
        });
        let forwarder: Arc<dyn ChannelForwarder> = counting.clone();
        for expires_at in [now_unix() - 60, now_unix()] {
            let error = forward_push(Some(&forwarder), &push(), expires_at)
                .await
                .unwrap_err();
            assert_eq!(error.status(), StatusCode::GONE);
        }
        assert_eq!(counting.calls(), 0);
    }

    #[tokio::test]
    async fn firebase_measures_utf16_units_like_its_rules() {
        let forwarder: Arc<dyn ChannelForwarder> = Arc::new(Counting {
            transport: CHANNEL_TRANSPORT_GCP_FIREBASE_RTDB,
            calls: AtomicUsize::new(0),
        });
        let at_limit = sized(ChannelPushKind::Inbound, None, GCP_MAX_PUSH_CHARS);
        let serde_json::Value::String(ascii) = &at_limit.value else {
            unreachable!()
        };

        let accented = with_text(&at_limit, "é".repeat(ascii.len()));
        assert!(serde_json::to_string(&accented).unwrap().len() > GCP_MAX_PUSH_CHARS);
        assert_eq!(
            forward_push(Some(&forwarder), &accented, live())
                .await
                .unwrap(),
            ForwardOutcome::Delivered
        );

        let astral = with_text(&at_limit, format!("{}😀", &ascii[1..]));
        assert_eq!(
            forward_push(Some(&forwarder), &astral, live())
                .await
                .unwrap_err()
                .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[tokio::test]
    async fn api_forwarding_preserves_reply_identity_and_retry_delivery_ids() {
        #[derive(Default)]
        struct Recorded(std::sync::Mutex<Vec<(ChannelPush, String)>>);

        #[async_trait]
        impl ChannelForwarder for Recorded {
            fn transport(&self) -> &'static str {
                "nats"
            }
            async fn forward(&self, _: &ChannelPush) -> flow_like_types::Result<()> {
                Err(flow_like_types::anyhow!("delivery id was discarded"))
            }
            async fn forward_with_id(
                &self,
                push: &ChannelPush,
                id: &str,
            ) -> flow_like_types::Result<()> {
                self.0.lock().unwrap().push((push.clone(), id.to_owned()));
                Ok(())
            }
        }

        let recorded = Arc::new(Recorded::default());
        let forwarder: Arc<dyn ChannelForwarder> = recorded.clone();
        for kind in [
            ChannelPushKind::Reply,
            ChannelPushKind::Inbound,
            ChannelPushKind::Cancel,
        ] {
            let body = ChannelPush {
                kind,
                value: serde_json::json!({"nested": ["answer", 7]}),
                ..push()
            };
            for _ in 0..2 {
                assert_eq!(
                    forward_push_with_id(Some(&forwarder), &body, Some("retry-1"), live())
                        .await
                        .unwrap(),
                    ForwardOutcome::Delivered
                );
            }
            let calls = recorded.0.lock().unwrap();
            for (forwarded, id) in calls.iter().rev().take(2) {
                assert_eq!(forwarded, &body);
                assert_eq!(id, "retry-1");
            }
        }
        assert_eq!(recorded.0.lock().unwrap().len(), 6);
    }
}
