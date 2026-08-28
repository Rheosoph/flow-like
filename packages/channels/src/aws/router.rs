//! Routes decoded [`ChannelPush`] payloads from the MQTT event loop to whoever is waiting. The
//! same entry point serves tests, so routing is exercised without a broker.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};

use flow_like_types::Value;
use flow_like_types::channel::{ChannelPush, ChannelPushKind};
use flow_like_types::tokio::sync::oneshot;
use flow_like_types::tokio_util::sync::CancellationToken;

/// Unsolicited messages buffered between drains.
pub(crate) const MAX_INBOUND: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteResult {
    Reply,
    Inbound,
    Cancel,
    ForeignChannel,
    Malformed,
    UnknownRequest,
    Duplicate,
    InboundFull,
}

/// A registered request: the sender is consumed by the first reply, the receiver is parked
/// until `wait` picks it up, so a reply that lands before `wait` is buffered in the oneshot.
struct Pending {
    sender: Option<oneshot::Sender<Value>>,
    receiver: Option<oneshot::Receiver<Value>>,
}

pub(crate) struct PushRouter {
    channel_id: String,
    pending: Mutex<HashMap<String, Pending>>,
    inbound: Mutex<VecDeque<Value>>,
    cancelled: CancellationToken,
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl PushRouter {
    pub fn new(channel_id: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            pending: Mutex::new(HashMap::new()),
            inbound: Mutex::new(VecDeque::new()),
            cancelled: CancellationToken::new(),
        }
    }

    pub fn register(&self, request_id: &str) {
        let (sender, receiver) = oneshot::channel();
        lock(&self.pending).insert(
            request_id.to_string(),
            Pending {
                sender: Some(sender),
                receiver: Some(receiver),
            },
        );
    }

    pub fn take_receiver(&self, request_id: &str) -> Option<oneshot::Receiver<Value>> {
        lock(&self.pending).get_mut(request_id)?.receiver.take()
    }

    pub fn remove(&self, request_id: &str) {
        lock(&self.pending).remove(request_id);
    }

    /// Drops every registration; parked-out receivers resolve with an error (`Closed`).
    pub fn clear(&self) {
        lock(&self.pending).clear();
    }

    pub fn drain_inbound(&self) -> Vec<Value> {
        lock(&self.inbound).drain(..).collect()
    }

    pub fn cancelled(&self) -> &CancellationToken {
        &self.cancelled
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.is_cancelled()
    }

    pub fn route_payload(&self, payload: &[u8]) -> RouteResult {
        match serde_json::from_slice::<ChannelPush>(payload) {
            Ok(push) => self.route(push),
            Err(error) => {
                tracing::debug!(%error, channel_id = %self.channel_id, "dropping malformed channel push");
                RouteResult::Malformed
            }
        }
    }

    pub fn route(&self, push: ChannelPush) -> RouteResult {
        if push.channel_id != self.channel_id {
            tracing::debug!(
                channel_id = %self.channel_id,
                foreign = %push.channel_id,
                "dropping push addressed to another channel"
            );
            return RouteResult::ForeignChannel;
        }
        match push.kind {
            ChannelPushKind::Cancel => {
                self.cancelled.cancel();
                RouteResult::Cancel
            }
            ChannelPushKind::Inbound => {
                let mut inbound = lock(&self.inbound);
                if inbound.len() >= MAX_INBOUND {
                    tracing::warn!(channel_id = %self.channel_id, "inbound buffer full; dropping push");
                    return RouteResult::InboundFull;
                }
                inbound.push_back(push.value);
                RouteResult::Inbound
            }
            ChannelPushKind::Reply => {
                let Some(request_id) = push.request_id else {
                    tracing::debug!(channel_id = %self.channel_id, "dropping reply without request id");
                    return RouteResult::UnknownRequest;
                };
                let mut pending = lock(&self.pending);
                let Some(slot) = pending.get_mut(&request_id) else {
                    tracing::debug!(channel_id = %self.channel_id, %request_id, "dropping reply for unknown or finished request");
                    return RouteResult::UnknownRequest;
                };
                let Some(sender) = slot.sender.take() else {
                    tracing::debug!(channel_id = %self.channel_id, %request_id, "dropping duplicate reply");
                    return RouteResult::Duplicate;
                };
                match sender.send(push.value) {
                    Ok(()) => RouteResult::Reply,
                    Err(_) => RouteResult::UnknownRequest,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(request_id: &str, value: Value) -> Vec<u8> {
        serde_json::to_vec(&ChannelPush {
            channel_id: "run-1".into(),
            request_id: Some(request_id.into()),
            kind: ChannelPushKind::Reply,
            value,
        })
        .unwrap()
    }

    #[test]
    fn reply_before_wait_is_buffered_and_deduped() {
        let router = PushRouter::new("run-1");
        router.register("req");
        assert_eq!(
            router.route_payload(&reply("req", Value::from(1))),
            RouteResult::Reply
        );
        assert_eq!(
            router.route_payload(&reply("req", Value::from(2))),
            RouteResult::Duplicate
        );
        let mut receiver = router.take_receiver("req").unwrap();
        assert_eq!(receiver.try_recv().unwrap(), Value::from(1));
        assert!(router.take_receiver("req").is_none());
        router.remove("req");
        assert_eq!(
            router.route_payload(&reply("req", Value::from(3))),
            RouteResult::UnknownRequest
        );
    }

    #[test]
    fn foreign_channel_and_malformed_payloads_are_dropped() {
        let router = PushRouter::new("run-1");
        router.register("req");
        let foreign = br#"{"channel_id":"run-2","request_id":"req","kind":"reply","value":1}"#;
        assert_eq!(router.route_payload(foreign), RouteResult::ForeignChannel);
        assert_eq!(router.route_payload(b"not json"), RouteResult::Malformed);
        let no_request = br#"{"channel_id":"run-1","kind":"reply","value":1}"#;
        assert_eq!(
            router.route_payload(no_request),
            RouteResult::UnknownRequest
        );
        assert!(router.take_receiver("req").unwrap().try_recv().is_err());
    }

    #[test]
    fn inbound_is_capped_and_drained_in_order() {
        let router = PushRouter::new("run-1");
        for i in 0..MAX_INBOUND {
            let payload = format!(r#"{{"channel_id":"run-1","kind":"inbound","value":{i}}}"#);
            assert_eq!(
                router.route_payload(payload.as_bytes()),
                RouteResult::Inbound
            );
        }
        let overflow = br#"{"channel_id":"run-1","kind":"inbound","value":"late"}"#;
        assert_eq!(router.route_payload(overflow), RouteResult::InboundFull);
        let drained = router.drain_inbound();
        assert_eq!(drained.len(), MAX_INBOUND);
        assert_eq!(drained[0], Value::from(0));
        assert_eq!(drained[MAX_INBOUND - 1], Value::from(MAX_INBOUND - 1));
        assert!(router.drain_inbound().is_empty());
    }

    #[test]
    fn cancel_sets_flag_and_fires_token() {
        let router = PushRouter::new("run-1");
        assert!(!router.is_cancelled());
        let payload = br#"{"channel_id":"run-1","kind":"cancel"}"#;
        assert_eq!(router.route_payload(payload), RouteResult::Cancel);
        assert_eq!(router.route_payload(payload), RouteResult::Cancel);
        assert!(router.is_cancelled());
        assert!(router.cancelled().is_cancelled());
    }

    #[test]
    fn clear_drops_senders_so_waiters_see_closed() {
        let router = PushRouter::new("run-1");
        router.register("req");
        let mut receiver = router.take_receiver("req").unwrap();
        router.clear();
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
    }
}
