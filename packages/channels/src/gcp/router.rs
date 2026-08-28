//! Turns raw stream events into channel state: pending-reply slots, the inbound queue and the
//! cancelled flag. Pure with respect to the network so the stream loop and the tests share it.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Mutex;

use flow_like_types::Value;
use flow_like_types::channel::{ChannelPush, ChannelPushKind};
use serde::Deserialize;
use tokio::sync::{oneshot, watch};

/// Unsolicited messages buffered between drains.
pub(crate) const MAX_INBOUND: usize = 8;
/// Node keys remembered for replay dedupe; the initial `put` at `/` re-sends every child on
/// each reconnect.
const MAX_SEEN_KEYS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamKind {
    Inbox,
    Inbound,
}

impl StreamKind {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Inbox => "inbox",
            Self::Inbound => "inbound",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Flags {
    pub cancelled: bool,
    pub closed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamAction {
    Continue,
    /// `auth_revoked`: renew the ID token before reconnecting.
    Reauthenticate,
    /// `cancel`: the rules no longer permit the read; reconnect with backoff.
    Reconnect,
}

pub(crate) enum Subscription {
    Ready(Value),
    Pending(oneshot::Receiver<Value>),
    Unknown,
}

enum PendingRequest {
    Open,
    Waiting(oneshot::Sender<Value>),
    /// `Some` = reply buffered for a waiter that has not subscribed yet; `None` = delivered.
    Answered(Option<Value>),
}

#[derive(Default)]
struct SeenKeys {
    set: HashSet<String>,
    order: VecDeque<String>,
}

impl SeenKeys {
    fn insert(&mut self, key: &str) -> bool {
        if !self.set.insert(key.to_string()) {
            return false;
        }
        self.order.push_back(key.to_string());
        while self.order.len() > MAX_SEEN_KEYS {
            if let Some(oldest) = self.order.pop_front() {
                self.set.remove(&oldest);
            }
        }
        true
    }
}

#[derive(Default)]
struct State {
    pending: HashMap<String, PendingRequest>,
    inbound: VecDeque<Value>,
    seen_inbox: SeenKeys,
    seen_inbound: SeenKeys,
}

#[derive(Deserialize)]
struct Frame {
    #[serde(default)]
    path: String,
    #[serde(default)]
    data: Value,
}

pub(crate) struct Router {
    channel_id: String,
    state: Mutex<State>,
    flags: watch::Sender<Flags>,
}

impl Router {
    pub(crate) fn new(channel_id: &str) -> Self {
        Self {
            channel_id: channel_id.to_string(),
            state: Mutex::new(State::default()),
            flags: watch::channel(Flags::default()).0,
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn register(&self, request_id: &str) {
        self.state()
            .pending
            .insert(request_id.to_string(), PendingRequest::Open);
    }

    pub(crate) fn subscribe(&self, request_id: &str) -> Subscription {
        let mut state = self.state();
        let Some(slot) = state.pending.get_mut(request_id) else {
            return Subscription::Unknown;
        };
        match std::mem::replace(slot, PendingRequest::Open) {
            PendingRequest::Answered(Some(value)) => {
                state.pending.remove(request_id);
                Subscription::Ready(value)
            }
            PendingRequest::Answered(None) => {
                state.pending.remove(request_id);
                Subscription::Unknown
            }
            PendingRequest::Open | PendingRequest::Waiting(_) => {
                let (sender, receiver) = oneshot::channel();
                *slot = PendingRequest::Waiting(sender);
                Subscription::Pending(receiver)
            }
        }
    }

    pub(crate) fn remove(&self, request_id: &str) {
        self.state().pending.remove(request_id);
    }

    pub(crate) fn drain_inbound(&self) -> Vec<Value> {
        self.state().inbound.drain(..).collect()
    }

    pub(crate) fn flags(&self) -> watch::Receiver<Flags> {
        self.flags.subscribe()
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.flags.borrow().cancelled
    }

    pub(crate) fn close(&self) {
        self.state().pending.clear();
        self.flags.send_modify(|flags| flags.closed = true);
    }

    /// One SSE frame from either stream.
    pub(crate) fn handle_event(&self, stream: StreamKind, event: &str, data: &str) -> StreamAction {
        match event {
            "put" | "patch" => {
                let frame: Frame = match serde_json::from_str(data) {
                    Ok(frame) => frame,
                    Err(err) => {
                        tracing::warn!(
                            channel = %self.channel_id,
                            stream = stream.name(),
                            error = %err,
                            "firebase stream frame is not valid JSON"
                        );
                        return StreamAction::Continue;
                    }
                };
                for (key, payload) in children(&frame.path, frame.data) {
                    self.ingest(stream, &key, &payload);
                }
                StreamAction::Continue
            }
            "keep-alive" => StreamAction::Continue,
            "auth_revoked" => StreamAction::Reauthenticate,
            "cancel" => StreamAction::Reconnect,
            other => {
                tracing::debug!(
                    channel = %self.channel_id,
                    stream = stream.name(),
                    event = other,
                    "ignoring unknown firebase stream event"
                );
                StreamAction::Continue
            }
        }
    }

    fn ingest(&self, stream: StreamKind, key: &str, payload: &str) {
        {
            let mut state = self.state();
            let seen = match stream {
                StreamKind::Inbox => &mut state.seen_inbox,
                StreamKind::Inbound => &mut state.seen_inbound,
            };
            if !seen.insert(key) {
                return;
            }
        }
        let push: ChannelPush = match serde_json::from_str(payload) {
            Ok(push) => push,
            Err(err) => {
                tracing::warn!(
                    channel = %self.channel_id,
                    stream = stream.name(),
                    key,
                    error = %err,
                    "firebase node payload is not a channel push"
                );
                return;
            }
        };
        if push.channel_id != self.channel_id {
            tracing::warn!(
                channel = %self.channel_id,
                other = %push.channel_id,
                key,
                "ignoring push addressed to another channel"
            );
            return;
        }
        self.deliver(push, key);
    }

    pub(crate) fn deliver(&self, push: ChannelPush, node_key: &str) {
        match push.kind {
            ChannelPushKind::Cancel => {
                tracing::debug!(channel = %self.channel_id, "channel cancelled by client");
                self.flags.send_modify(|flags| flags.cancelled = true);
            }
            ChannelPushKind::Inbound => {
                let mut state = self.state();
                if state.inbound.len() >= MAX_INBOUND {
                    tracing::warn!(
                        channel = %self.channel_id,
                        "inbound buffer full ({MAX_INBOUND}); dropping message"
                    );
                    return;
                }
                state.inbound.push_back(push.value);
            }
            ChannelPushKind::Reply => {
                let request_id = push
                    .request_id
                    .filter(|id| !id.is_empty())
                    .unwrap_or_else(|| node_key.to_string());
                let mut state = self.state();
                let Some(slot) = state.pending.get_mut(&request_id) else {
                    tracing::debug!(
                        channel = %self.channel_id,
                        request = %request_id,
                        "reply for an unknown or abandoned request"
                    );
                    return;
                };
                match std::mem::replace(slot, PendingRequest::Open) {
                    PendingRequest::Open => *slot = PendingRequest::Answered(Some(push.value)),
                    PendingRequest::Waiting(sender) => {
                        *slot = PendingRequest::Answered(sender.send(push.value).err());
                    }
                    answered @ PendingRequest::Answered(_) => {
                        *slot = answered;
                        tracing::debug!(
                            channel = %self.channel_id,
                            request = %request_id,
                            "duplicate reply ignored; first reply wins"
                        );
                    }
                }
            }
        }
    }
}

/// `(node key, payload string)` pairs a `put`/`patch` frame carries. A frame at `/` holds a map
/// of children (`patch` may key them as `key/payload`); a frame at `/{key}` holds one child, at
/// `/{key}/payload` its payload string. Deletions (`null`) and deeper paths carry nothing.
fn children(path: &str, data: Value) -> Vec<(String, String)> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.split_first() {
        None => match data {
            Value::Object(map) => map
                .into_iter()
                .filter_map(|(key, value)| {
                    let (key, rest) = key.split_once('/').unwrap_or((key.as_str(), ""));
                    payload_of(rest, value).map(|payload| (key.to_string(), payload))
                })
                .collect(),
            _ => Vec::new(),
        },
        Some((key, rest)) => payload_of(&rest.join("/"), data)
            .map(|payload| vec![(key.to_string(), payload)])
            .unwrap_or_default(),
    }
}

fn payload_of(rest: &str, value: Value) -> Option<String> {
    match (rest, value) {
        ("", Value::Object(mut child)) => match child.remove("payload") {
            Some(Value::String(payload)) => Some(payload),
            _ => None,
        },
        ("payload", Value::String(payload)) => Some(payload),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CHANNEL: &str = "run-1";

    fn push_json(kind: &str, request_id: Option<&str>, value: Value) -> String {
        let mut push = json!({ "channel_id": CHANNEL, "kind": kind, "value": value });
        if let Some(id) = request_id {
            push["request_id"] = Value::from(id);
        }
        push.to_string()
    }

    fn child(payload: String) -> Value {
        json!({ "payload": payload })
    }

    fn put(router: &Router, stream: StreamKind, path: &str, data: Value) -> StreamAction {
        router.handle_event(
            stream,
            "put",
            &json!({ "path": path, "data": data }).to_string(),
        )
    }

    fn ready(subscription: Subscription) -> Option<Value> {
        match subscription {
            Subscription::Ready(value) => Some(value),
            _ => None,
        }
    }

    #[test]
    fn initial_snapshot_routes_every_child_by_kind() {
        let router = Router::new(CHANNEL);
        router.register("r1");
        router.register("r2");
        let snapshot = json!({
            "r1": child(push_json("reply", Some("r1"), json!(1))),
            "r2": child(push_json("reply", None, json!(2))),
            "n1": child(push_json("inbound", None, json!("steer"))),
            "junk": { "payload": 5 },
            "other": child(json!({ "channel_id": "someone-else", "kind": "cancel" }).to_string()),
        });
        assert_eq!(
            put(&router, StreamKind::Inbox, "/", snapshot),
            StreamAction::Continue
        );
        assert_eq!(ready(router.subscribe("r1")), Some(json!(1)));
        assert_eq!(ready(router.subscribe("r2")), Some(json!(2)));
        assert_eq!(router.drain_inbound(), vec![json!("steer")]);
        assert!(!router.is_cancelled());
        assert!(matches!(router.subscribe("r1"), Subscription::Unknown));
    }

    #[test]
    fn single_child_put_and_patch_shapes() {
        let router = Router::new(CHANNEL);
        router.register("r1");
        router.register("r2");
        router.register("r3");
        put(
            &router,
            StreamKind::Inbox,
            "/r1",
            child(push_json("reply", None, json!("a"))),
        );
        put(
            &router,
            StreamKind::Inbox,
            "/r2/payload",
            Value::from(push_json("reply", Some("r2"), json!("b"))),
        );
        router.handle_event(
            StreamKind::Inbox,
            "patch",
            &json!({ "path": "/", "data": { "r3/payload": push_json("reply", None, json!("c")) } })
                .to_string(),
        );
        assert_eq!(ready(router.subscribe("r1")), Some(json!("a")));
        assert_eq!(ready(router.subscribe("r2")), Some(json!("b")));
        assert_eq!(ready(router.subscribe("r3")), Some(json!("c")));
        assert_eq!(
            put(&router, StreamKind::Inbox, "/r1", Value::Null),
            StreamAction::Continue
        );
        assert_eq!(
            put(&router, StreamKind::Inbox, "/", Value::Null),
            StreamAction::Continue
        );
        assert_eq!(
            put(&router, StreamKind::Inbox, "/r1/payload/deeper", json!("x")),
            StreamAction::Continue
        );
    }

    #[test]
    fn replayed_snapshot_is_deduped_by_node_key() {
        let router = Router::new(CHANNEL);
        router.register("r1");
        let snapshot = json!({
            "r1": child(push_json("reply", Some("r1"), json!("first"))),
            "n1": child(push_json("inbound", None, json!("steer"))),
        });
        put(&router, StreamKind::Inbound, "/", snapshot.clone());
        put(&router, StreamKind::Inbound, "/", snapshot.clone());
        put(
            &router,
            StreamKind::Inbound,
            "/n1",
            child(push_json("inbound", None, json!("steer"))),
        );
        assert_eq!(router.drain_inbound(), vec![json!("steer")]);
        assert_eq!(ready(router.subscribe("r1")), Some(json!("first")));

        router.register("r1");
        put(&router, StreamKind::Inbox, "/", snapshot);
        assert_eq!(ready(router.subscribe("r1")), Some(json!("first")));
    }

    #[test]
    fn first_reply_wins_and_buffers_before_subscribe() {
        let router = Router::new(CHANNEL);
        router.register("r1");
        put(
            &router,
            StreamKind::Inbox,
            "/a",
            child(push_json("reply", Some("r1"), json!("first"))),
        );
        put(
            &router,
            StreamKind::Inbox,
            "/b",
            child(push_json("reply", Some("r1"), json!("second"))),
        );
        assert_eq!(ready(router.subscribe("r1")), Some(json!("first")));
        put(
            &router,
            StreamKind::Inbox,
            "/c",
            child(push_json("reply", Some("r1"), json!("late"))),
        );
        assert!(matches!(router.subscribe("r1"), Subscription::Unknown));
    }

    #[tokio::test]
    async fn waiting_subscriber_is_woken() {
        let router = Router::new(CHANNEL);
        router.register("r1");
        let Subscription::Pending(receiver) = router.subscribe("r1") else {
            panic!("expected a pending subscription");
        };
        put(
            &router,
            StreamKind::Inbox,
            "/r1",
            child(push_json("reply", None, json!(7))),
        );
        assert_eq!(receiver.await.unwrap(), json!(7));
        assert!(matches!(router.subscribe("r1"), Subscription::Unknown));
        router.remove("r1");
    }

    #[test]
    fn inbound_is_capped_and_cancel_sets_the_flag() {
        let router = Router::new(CHANNEL);
        let mut flags = router.flags();
        for i in 0..(MAX_INBOUND + 3) {
            put(
                &router,
                StreamKind::Inbound,
                &format!("/n{i}"),
                child(push_json("inbound", None, json!(i))),
            );
        }
        let drained = router.drain_inbound();
        assert_eq!(drained.len(), MAX_INBOUND);
        assert_eq!(drained[0], json!(0));
        assert!(router.drain_inbound().is_empty());

        assert!(!flags.has_changed().unwrap());
        put(
            &router,
            StreamKind::Inbound,
            "/cancel-1",
            child(push_json("cancel", None, Value::Null)),
        );
        assert!(router.is_cancelled());
        assert!(flags.has_changed().unwrap());
        assert!(flags.borrow_and_update().cancelled);
    }

    #[test]
    fn control_events_and_garbage() {
        let router = Router::new(CHANNEL);
        assert_eq!(
            router.handle_event(StreamKind::Inbox, "keep-alive", "null"),
            StreamAction::Continue
        );
        assert_eq!(
            router.handle_event(
                StreamKind::Inbox,
                "auth_revoked",
                "\"credential has expired\""
            ),
            StreamAction::Reauthenticate
        );
        assert_eq!(
            router.handle_event(StreamKind::Inbox, "cancel", "null"),
            StreamAction::Reconnect
        );
        assert_eq!(
            router.handle_event(StreamKind::Inbox, "mystery", ""),
            StreamAction::Continue
        );
        assert_eq!(
            router.handle_event(StreamKind::Inbox, "put", "{not json"),
            StreamAction::Continue
        );
        router.register("r1");
        put(
            &router,
            StreamKind::Inbox,
            "/r1",
            child("{not a push".into()),
        );
        assert!(matches!(router.subscribe("r1"), Subscription::Pending(_)));
    }

    #[test]
    fn close_clears_pending_and_flags_closed() {
        let router = Router::new(CHANNEL);
        router.register("r1");
        let mut flags = router.flags();
        router.close();
        assert!(matches!(router.subscribe("r1"), Subscription::Unknown));
        assert!(flags.borrow_and_update().closed);
    }
}
