//! Waiter-side state shared by the socket task and the `Channel` impl, and the single path
//! every received frame takes: the socket loop and the tests both feed [`handle_text`].

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard, PoisonError};

use flow_like_types::Value;
use flow_like_types::channel::{ChannelPush, ChannelPushKind};
use flow_like_types::tokio_util::sync::CancellationToken;
use tokio::sync::oneshot;
use tracing::{debug, warn};

use super::protocol::{ServerFrame, push_from_message};

pub(crate) const MAX_INBOUND: usize = 8;

enum Pending {
    Open(oneshot::Sender<Value>),
    Replied,
}

#[derive(Default)]
struct State {
    pending: HashMap<String, Pending>,
    receivers: HashMap<String, oneshot::Receiver<Value>>,
    inbound: VecDeque<Value>,
}

pub(crate) struct Shared {
    pub channel_id: String,
    pub cancelled: CancellationToken,
    state: Mutex<State>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RouteResult {
    Delivered,
    ForeignChannel,
    MissingRequestId,
    UnknownRequest,
    Duplicate,
    InboundFull,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum FrameAction {
    Continue,
    Ack {
        ack_id: u64,
        success: bool,
        error: Option<String>,
    },
    Disconnected,
}

impl Shared {
    pub fn new(channel_id: impl Into<String>) -> Self {
        Self {
            channel_id: channel_id.into(),
            cancelled: CancellationToken::new(),
            state: Mutex::new(State::default()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Reserve a request slot; a reply landing before [`Shared::take_receiver`] is buffered.
    pub fn register(&self, request_id: &str) {
        let (sender, receiver) = oneshot::channel();
        let mut state = self.lock();
        state
            .pending
            .insert(request_id.to_string(), Pending::Open(sender));
        state.receivers.insert(request_id.to_string(), receiver);
    }

    pub fn take_receiver(&self, request_id: &str) -> Option<oneshot::Receiver<Value>> {
        self.lock().receivers.remove(request_id)
    }

    pub fn forget(&self, request_id: &str) {
        let mut state = self.lock();
        state.pending.remove(request_id);
        state.receivers.remove(request_id);
    }

    pub fn drain_inbound(&self) -> Vec<Value> {
        self.lock().inbound.drain(..).collect()
    }

    /// Drops every pending sender so parked waiters resolve to `Closed`.
    pub fn clear(&self) {
        let mut state = self.lock();
        state.pending.clear();
        state.receivers.clear();
        state.inbound.clear();
    }

    pub fn route(&self, push: ChannelPush) -> RouteResult {
        if push.channel_id != self.channel_id {
            return RouteResult::ForeignChannel;
        }
        match push.kind {
            ChannelPushKind::Cancel => {
                self.cancelled.cancel();
                RouteResult::Delivered
            }
            ChannelPushKind::Inbound => {
                let mut state = self.lock();
                if state.inbound.len() >= MAX_INBOUND {
                    return RouteResult::InboundFull;
                }
                state.inbound.push_back(push.value);
                RouteResult::Delivered
            }
            ChannelPushKind::Reply => {
                let Some(request_id) = push.request_id else {
                    return RouteResult::MissingRequestId;
                };
                let mut state = self.lock();
                let Some(slot) = state.pending.get_mut(&request_id) else {
                    return RouteResult::UnknownRequest;
                };
                match std::mem::replace(slot, Pending::Replied) {
                    Pending::Open(sender) => {
                        let _ = sender.send(push.value);
                        RouteResult::Delivered
                    }
                    Pending::Replied => RouteResult::Duplicate,
                }
            }
        }
    }
}

pub(crate) fn handle_frame(shared: &Shared, frame: ServerFrame) -> FrameAction {
    match frame {
        ServerFrame::Ack {
            ack_id,
            success,
            error,
        } => FrameAction::Ack {
            ack_id,
            success,
            error: error.map(|e| format!("{}: {}", e.name, e.message)),
        },
        ServerFrame::Message {
            from,
            data_type,
            data,
            from_user_id,
            ..
        } => {
            match push_from_message(&data_type, data) {
                Ok(push) => {
                    let request_id = push.request_id.clone().unwrap_or_default();
                    let kind = push.kind;
                    let result = shared.route(push);
                    let from_user = from_user_id.unwrap_or_default();
                    match result {
                        RouteResult::InboundFull | RouteResult::MissingRequestId => warn!(
                            channel_id = %shared.channel_id,
                            request_id = %request_id,
                            ?kind,
                            ?result,
                            from_user = %from_user,
                            "dropped Azure Web PubSub push"
                        ),
                        _ => debug!(
                            channel_id = %shared.channel_id,
                            request_id = %request_id,
                            ?kind,
                            ?result,
                            from_user = %from_user,
                            "routed Azure Web PubSub push"
                        ),
                    }
                }
                Err(error) => warn!(
                    channel_id = %shared.channel_id,
                    from = %from,
                    data_type = %data_type,
                    error = %error,
                    "ignoring undecodable Azure Web PubSub message"
                ),
            }
            FrameAction::Continue
        }
        ServerFrame::System {
            event,
            connection_id,
            user_id,
            message,
        } => match event.as_str() {
            "connected" => {
                debug!(
                    channel_id = %shared.channel_id,
                    connection_id = connection_id.as_deref().unwrap_or_default(),
                    user_id = user_id.as_deref().unwrap_or_default(),
                    "Azure Web PubSub connection established"
                );
                FrameAction::Continue
            }
            "disconnected" => {
                warn!(
                    channel_id = %shared.channel_id,
                    reason = message.as_deref().unwrap_or_default(),
                    "Azure Web PubSub disconnected the connection"
                );
                FrameAction::Disconnected
            }
            other => {
                debug!(channel_id = %shared.channel_id, event = other, "unhandled Azure Web PubSub system event");
                FrameAction::Continue
            }
        },
        ServerFrame::Pong => FrameAction::Continue,
        ServerFrame::Unknown => {
            debug!(channel_id = %shared.channel_id, "ignoring unknown Azure Web PubSub frame type");
            FrameAction::Continue
        }
    }
}

pub(crate) fn handle_text(shared: &Shared, text: &str) -> FrameAction {
    match ServerFrame::parse(text) {
        Ok(frame) => handle_frame(shared, frame),
        Err(error) => {
            warn!(channel_id = %shared.channel_id, error = %error, "ignoring unparsable Azure Web PubSub frame");
            FrameAction::Continue
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn message(data: Value) -> String {
        json!({
            "type": "message",
            "from": "group",
            "group": "run:chan",
            "dataType": "json",
            "data": data,
            "fromUserId": "user"
        })
        .to_string()
    }

    fn reply(channel_id: &str, request_id: &str, value: Value) -> String {
        message(json!({
            "channel_id": channel_id,
            "request_id": request_id,
            "kind": "reply",
            "value": value
        }))
    }

    #[test]
    fn reply_before_take_is_buffered_and_duplicates_drop() {
        let shared = Shared::new("chan");
        shared.register("r1");
        assert_eq!(
            handle_text(&shared, &reply("chan", "r1", json!(1))),
            FrameAction::Continue
        );
        let push = ChannelPush {
            channel_id: "chan".into(),
            request_id: Some("r1".into()),
            kind: ChannelPushKind::Reply,
            value: json!(2),
        };
        assert_eq!(shared.route(push), RouteResult::Duplicate);
        let mut receiver = shared.take_receiver("r1").unwrap();
        assert_eq!(receiver.try_recv().unwrap(), json!(1));
    }

    #[test]
    fn foreign_channel_and_unknown_request_are_ignored() {
        let shared = Shared::new("chan");
        let foreign = ChannelPush {
            channel_id: "other".into(),
            request_id: Some("r".into()),
            kind: ChannelPushKind::Reply,
            value: Value::Null,
        };
        assert_eq!(shared.route(foreign), RouteResult::ForeignChannel);
        let unknown = ChannelPush {
            channel_id: "chan".into(),
            request_id: Some("nope".into()),
            kind: ChannelPushKind::Reply,
            value: Value::Null,
        };
        assert_eq!(shared.route(unknown), RouteResult::UnknownRequest);
        let missing = ChannelPush {
            channel_id: "chan".into(),
            request_id: None,
            kind: ChannelPushKind::Reply,
            value: Value::Null,
        };
        assert_eq!(shared.route(missing), RouteResult::MissingRequestId);
        assert!(!shared.cancelled.is_cancelled());
    }

    #[test]
    fn inbound_is_capped_and_drains_in_order() {
        let shared = Shared::new("chan");
        for i in 0..(MAX_INBOUND + 2) {
            let result = shared.route(ChannelPush {
                channel_id: "chan".into(),
                request_id: None,
                kind: ChannelPushKind::Inbound,
                value: json!(i),
            });
            let expected = if i < MAX_INBOUND {
                RouteResult::Delivered
            } else {
                RouteResult::InboundFull
            };
            assert_eq!(result, expected, "push {i}");
        }
        let drained = shared.drain_inbound();
        assert_eq!(drained.len(), MAX_INBOUND);
        assert_eq!(drained[0], json!(0));
        assert_eq!(drained[MAX_INBOUND - 1], json!(MAX_INBOUND - 1));
        assert!(shared.drain_inbound().is_empty());
    }

    #[test]
    fn cancel_sets_flag() {
        let shared = Shared::new("chan");
        handle_text(
            &shared,
            &message(json!({"channel_id": "chan", "kind": "cancel"})),
        );
        assert!(shared.cancelled.is_cancelled());
    }

    #[test]
    fn text_data_type_is_parsed() {
        let shared = Shared::new("chan");
        shared.register("r2");
        let frame = json!({
            "type": "message",
            "from": "group",
            "group": "run:chan",
            "dataType": "text",
            "data": r#"{"channel_id":"chan","request_id":"r2","value":"ok"}"#
        })
        .to_string();
        handle_text(&shared, &frame);
        let mut receiver = shared.take_receiver("r2").unwrap();
        assert_eq!(receiver.try_recv().unwrap(), json!("ok"));
    }

    #[test]
    fn control_frames_map_to_actions() {
        let shared = Shared::new("chan");
        assert_eq!(
            handle_text(&shared, r#"{"type":"ack","ackId":3,"success":true}"#),
            FrameAction::Ack {
                ack_id: 3,
                success: true,
                error: None
            }
        );
        assert_eq!(
            handle_text(
                &shared,
                r#"{"type":"ack","ackId":4,"success":false,"error":{"name":"Forbidden","message":"no"}}"#
            ),
            FrameAction::Ack {
                ack_id: 4,
                success: false,
                error: Some("Forbidden: no".into())
            }
        );
        assert_eq!(
            handle_text(
                &shared,
                r#"{"type":"system","event":"disconnected","message":"bye"}"#
            ),
            FrameAction::Disconnected
        );
        assert_eq!(
            handle_text(&shared, r#"{"type":"system","event":"connected"}"#),
            FrameAction::Continue
        );
        assert_eq!(
            handle_text(&shared, r#"{"type":"pong"}"#),
            FrameAction::Continue
        );
        assert_eq!(
            handle_text(&shared, r#"{"type":"future"}"#),
            FrameAction::Continue
        );
        assert_eq!(handle_text(&shared, "not json"), FrameAction::Continue);
    }

    #[test]
    fn clear_drops_pending_senders() {
        let shared = Shared::new("chan");
        shared.register("r3");
        let mut receiver = shared.take_receiver("r3").unwrap();
        shared.clear();
        assert!(matches!(
            receiver.try_recv(),
            Err(oneshot::error::TryRecvError::Closed)
        ));
    }
}
