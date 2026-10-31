use super::{Acknowledgement, Envelope};
use flow_like_types::Value;
use flow_like_types::channel::ChannelPushKind;
use flow_like_types::tokio_util::sync::CancellationToken;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, MutexGuard};
use tokio::sync::oneshot;

const MAX_INBOUND: usize = 8;
const MAX_PENDING: usize = 32;
const MAX_RECEIPTS: usize = 256;

struct Pending {
    sender: Option<oneshot::Sender<Value>>,
    receiver: Option<oneshot::Receiver<Value>>,
    expires_at: i64,
}

#[derive(Default)]
struct State {
    pending: HashMap<String, Pending>,
    inbound: VecDeque<Value>,
    receipts: VecDeque<(String, [u8; 32], Acknowledgement)>,
    closed: bool,
}

pub(crate) struct Router {
    channel_id: String,
    state: Mutex<State>,
    pub cancelled: CancellationToken,
    pub closed: CancellationToken,
}

fn lock<T>(value: &Mutex<T>) -> MutexGuard<'_, T> {
    value.lock().unwrap_or_else(|error| error.into_inner())
}

impl Router {
    pub fn new(channel_id: &str) -> Self {
        Self {
            channel_id: channel_id.into(),
            state: Mutex::default(),
            cancelled: CancellationToken::new(),
            closed: CancellationToken::new(),
        }
    }

    pub fn register(&self, id: &str, expires_at: i64) -> flow_like_types::Result<()> {
        let mut state = lock(&self.state);
        if state.closed || self.cancelled.is_cancelled() {
            flow_like_types::bail!("NATS channel is closed or cancelled");
        }
        if state.pending.len() >= MAX_PENDING {
            flow_like_types::bail!("NATS channel has too many pending requests");
        }
        let (sender, receiver) = oneshot::channel();
        state.pending.insert(
            id.into(),
            Pending {
                sender: Some(sender),
                receiver: Some(receiver),
                expires_at,
            },
        );
        Ok(())
    }

    pub fn take_receiver(&self, id: &str) -> Option<oneshot::Receiver<Value>> {
        lock(&self.state).pending.get_mut(id)?.receiver.take()
    }

    pub fn remove(&self, id: &str) {
        lock(&self.state).pending.remove(id);
    }
    pub fn drain(&self) -> Vec<Value> {
        lock(&self.state).inbound.drain(..).collect()
    }

    pub fn close(&self) {
        let mut state = lock(&self.state);
        state.closed = true;
        state.pending.clear();
        state.inbound.clear();
        self.closed.cancel();
    }

    pub fn route(&self, envelope: Envelope) -> Acknowledgement {
        // HTTP forwarding may serialize object keys in a different order than the browser.
        let mut canonical = serde_json::to_value(&envelope.push).unwrap_or_default();
        canonical.sort_all_objects();
        let fingerprint: [u8; 32] =
            Sha256::digest(serde_json::to_vec(&canonical).unwrap_or_default()).into();
        let mut state = lock(&self.state);
        if let Some((_, previous, ack)) =
            state.receipts.iter().find(|(id, _, _)| id == &envelope.id)
        {
            return if previous == &fingerprint {
                ack.clone()
            } else {
                Acknowledgement::rejected("Delivery id was already used for another message")
            };
        }
        let push = envelope.push;
        let ack = if push.channel_id != self.channel_id {
            Acknowledgement::rejected("Push belongs to another channel")
        } else if state.closed {
            Acknowledgement::rejected("Channel is closed")
        } else if self.cancelled.is_cancelled() && !matches!(push.kind, ChannelPushKind::Cancel) {
            Acknowledgement::rejected("Channel is cancelled")
        } else {
            match push.kind {
                ChannelPushKind::Cancel => {
                    self.cancelled.cancel();
                    Acknowledgement::accepted()
                }
                ChannelPushKind::Inbound => {
                    if state.inbound.len() >= MAX_INBOUND {
                        Acknowledgement::rejected("Channel inbound buffer is full")
                    } else {
                        state.inbound.push_back(push.value);
                        Acknowledgement::accepted()
                    }
                }
                ChannelPushKind::Reply => {
                    match push
                        .request_id
                        .as_ref()
                        .and_then(|id| state.pending.get_mut(id))
                    {
                        None => Acknowledgement::rejected("No pending request with this id"),
                        Some(pending)
                            if pending.expires_at <= flow_like_types::channel::now_unix() =>
                        {
                            Acknowledgement::rejected("Request has expired")
                        }
                        Some(pending) => match pending.sender.take() {
                            Some(sender) => {
                                if sender.send(push.value).is_ok() {
                                    Acknowledgement::accepted()
                                } else {
                                    Acknowledgement::rejected("Request waiter is closed")
                                }
                            }
                            None => Acknowledgement::rejected("Request was already answered"),
                        },
                    }
                }
            }
        };
        // Retain successful receipts only. A temporarily full inbox can accept a later retry.
        if ack.accepted {
            state
                .receipts
                .push_back((envelope.id, fingerprint, ack.clone()));
            if state.receipts.len() > MAX_RECEIPTS {
                state.receipts.pop_front();
            }
        }
        ack
    }
}
