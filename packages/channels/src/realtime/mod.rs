//! Read-only cloud wakeups and the replies a host is still writing. Stored account data and
//! replay cursors always come from the API.
#![cfg_attr(
    not(any(
        feature = "aws",
        feature = "aws-receiver",
        feature = "azure",
        feature = "gcp",
        feature = "nats"
    )),
    allow(dead_code, unused_variables)
)]

use flow_like_types::{
    Result, anyhow,
    realtime::{MAX_REALTIME_WAKEUP_BYTES, RealtimeReply, RealtimeTransport, RealtimeWakeup},
};
use std::sync::Arc;
use tokio::sync::{Notify, watch};

/// Takes each reply a host streams to this account while it writes it.
pub type ReplySink = Arc<dyn Fn(RealtimeReply) + Send + Sync>;

#[cfg(any(feature = "aws", feature = "aws-receiver"))]
mod aws;
#[cfg(feature = "azure")]
mod azure;
#[cfg(feature = "gcp")]
mod gcp;
#[cfg(feature = "nats")]
mod nats;

const MAX_WAKE_BYTES: usize = MAX_REALTIME_WAKEUP_BYTES;

fn valid_wakeup(payload: &[u8]) -> bool {
    if payload.len() > MAX_WAKE_BYTES {
        return false;
    }
    serde_json::from_slice::<RealtimeWakeup>(payload).is_ok_and(|wake| wake.is_valid())
}

fn valid_reply(payload: &[u8]) -> Option<RealtimeReply> {
    if payload.len() > MAX_WAKE_BYTES {
        return None;
    }
    serde_json::from_slice::<RealtimeReply>(payload)
        .ok()
        .filter(RealtimeReply::is_valid)
}

/// Where a listener hands what its provider delivers.
struct Inbox {
    wake: Arc<Notify>,
    replies: Option<ReplySink>,
}

impl Inbox {
    /// Wakeups coalesce into one pending replay; replies reach their sink one by one. Anything
    /// else is dropped.
    fn receive(&self, payload: &[u8]) {
        if valid_wakeup(payload) {
            self.wake.notify_one();
        } else if let Some(replies) = &self.replies
            && let Some(reply) = valid_reply(payload)
        {
            replies(reply);
        }
    }

    fn ready(&self, healthy: &watch::Sender<bool>) {
        healthy.send_replace(true);
        // Reconcile after every connection, including reconnects that missed a wakeup.
        self.wake.notify_one();
    }
}

/// Runs one read-only subscription until it disconnects. Dropping this future closes it.
/// The owner renews grants and falls back to HTTP while this future is unavailable. Without
/// `replies` the listener only wakes the replay.
pub async fn listen(
    transport: &RealtimeTransport,
    wake: Arc<Notify>,
    healthy: watch::Sender<bool>,
    replies: Option<ReplySink>,
) -> Result<()> {
    let inbox = Inbox { wake, replies };
    match transport {
        #[cfg(any(feature = "aws", feature = "aws-receiver"))]
        RealtimeTransport::AwsMqtt {
            endpoint,
            region,
            client_id,
            topic,
            credentials,
        } => {
            aws::listen(
                endpoint,
                region,
                client_id,
                topic,
                credentials,
                &inbox,
                &healthy,
            )
            .await
        }
        #[cfg(feature = "azure")]
        RealtimeTransport::AzureWebPubSub { url, group } => {
            azure::listen(url, group, &inbox, &healthy).await
        }
        #[cfg(feature = "gcp")]
        RealtimeTransport::GcpFirebaseRtdb {
            database_url,
            api_key,
            custom_token,
            path,
            ..
        } => gcp::listen(database_url, api_key, custom_token, path, &inbox, &healthy).await,
        #[cfg(feature = "nats")]
        RealtimeTransport::Nats {
            url,
            subject,
            token,
        } => nats::listen(url, subject, token, &inbox, &healthy).await,
        #[allow(unreachable_patterns)]
        _ => Err(anyhow!("Realtime provider has no compiled native listener")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wakeups_cannot_carry_execution_data_or_unbounded_content() {
        assert!(valid_wakeup(
            br#"{"protocol_version":1,"kind":"invalidate","nonce":"n1"}"#
        ));
        for value in [
            serde_json::json!({"protocol_version":2,"kind":"invalidate","nonce":"n"}),
            serde_json::json!({"protocol_version":1,"kind":"event","nonce":"n"}),
            serde_json::json!({"protocol_version":1,"kind":"invalidate","nonce":""}),
            serde_json::json!({"protocol_version":1,"kind":"invalidate","nonce":"n","run_id":"foreign"}),
            serde_json::json!({"protocol_version":1,"kind":"invalidate","nonce":"x".repeat(MAX_WAKE_BYTES)}),
        ] {
            assert!(!valid_wakeup(&serde_json::to_vec(&value).unwrap()));
        }
    }

    fn reply(text: &str) -> serde_json::Value {
        serde_json::json!({
            "protocol_version": 1, "kind": "reply", "conversation_id": "c-1",
            "message_id": "re-1", "reply_to": "m-1", "sender_id": "bot-1",
            "offset": 0, "text": text, "state": "writing"
        })
    }

    #[test]
    fn a_reply_reaches_its_sink_and_never_wakes_the_replay() {
        let received = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = received.clone();
        let inbox = Inbox {
            wake: Arc::new(Notify::new()),
            replies: Some(Arc::new(move |reply: RealtimeReply| {
                sink.lock().unwrap().push(reply.text)
            })),
        };
        inbox.receive(&serde_json::to_vec(&reply("Hello")).unwrap());
        let mut unknown_state = reply("Hello");
        unknown_state["state"] = "paused".into();
        let mut no_writer = reply("Hello");
        no_writer["sender_id"] = "".into();
        for dropped in [
            unknown_state,
            no_writer,
            reply(&"x".repeat(MAX_WAKE_BYTES)),
            serde_json::json!({"protocol_version":1,"kind":"event","nonce":"n"}),
        ] {
            inbox.receive(&serde_json::to_vec(&dropped).unwrap());
        }
        assert_eq!(*received.lock().unwrap(), ["Hello"]);
        // A stored permit would make this complete at once.
        let woken = std::pin::pin!(inbox.wake.notified());
        assert!(
            futures_util::FutureExt::now_or_never(woken).is_none(),
            "a reply must not start a replay"
        );
    }

    #[test]
    fn a_listener_without_a_sink_still_wakes_and_drops_replies() {
        let inbox = Inbox {
            wake: Arc::new(Notify::new()),
            replies: None,
        };
        inbox.receive(&serde_json::to_vec(&reply("Hello")).unwrap());
        inbox.receive(br#"{"protocol_version":1,"kind":"invalidate","nonce":"n1"}"#);
        let woken = std::pin::pin!(inbox.wake.notified());
        assert!(futures_util::FutureExt::now_or_never(woken).is_some());
    }
}
