//! Same-process transport (desktop, tests): the host resolves pushes straight into the waiter's
//! oneshot. A process-wide registry keyed by channel id lets the host command find the channel
//! a push belongs to.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};

use super::{
    Channel, ChannelClientDescriptor, ChannelHandle, ChannelOutcome, ChannelPush, ChannelPushKind,
    ChannelTicket, clamp_ttl, new_request_id, now_unix,
};
use crate::Value;
use crate::async_trait;
use crate::sync::Mutex;
use crate::tokio::sync::oneshot;
use crate::tokio_util::sync::CancellationToken;

/// Late pushes for a request that already timed out get a distinct answer for this long.
const EXPIRED_REQUEST_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_EXPIRED_REQUESTS: usize = 256;
/// Unsolicited messages buffered per channel between drains.
const MAX_INBOUND: usize = 8;

static REGISTRY: LazyLock<Mutex<HashMap<String, Weak<InProcessChannel>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InProcessPushResult {
    Delivered,
    UnknownChannel,
    UnknownRequest,
    /// The waiter already gave up on this request.
    Expired,
    /// A reply for this request was already delivered.
    Duplicate,
    /// The inbound buffer is full; the push was dropped.
    Full,
}

pub struct InProcessChannel {
    channel_id: String,
    expires_at: i64,
    pending: Mutex<HashMap<String, oneshot::Sender<Value>>>,
    expired: Mutex<VecDeque<(String, Instant)>>,
    inbound: Mutex<VecDeque<Value>>,
    cancelled: AtomicBool,
}

impl InProcessChannel {
    /// Create and register a channel. Dropping every `Arc` (or calling [`Channel::close`])
    /// unregisters it.
    pub async fn register(channel_id: impl Into<String>, lifetime: Duration) -> Arc<Self> {
        let channel_id = channel_id.into();
        let channel = Arc::new(Self {
            expires_at: now_unix() + clamp_ttl(lifetime).as_secs() as i64,
            channel_id: channel_id.clone(),
            pending: Mutex::new(HashMap::new()),
            expired: Mutex::new(VecDeque::new()),
            inbound: Mutex::new(VecDeque::new()),
            cancelled: AtomicBool::new(false),
        });
        REGISTRY
            .lock()
            .await
            .insert(channel_id, Arc::downgrade(&channel));
        channel
    }

    pub async fn lookup(channel_id: &str) -> Option<Arc<Self>> {
        REGISTRY.lock().await.get(channel_id)?.upgrade()
    }

    /// Host entry point: deliver a client push to whichever registered channel it addresses.
    pub async fn deliver(push: ChannelPush) -> InProcessPushResult {
        match Self::lookup(&push.channel_id).await {
            Some(channel) => channel.push(push).await,
            None => InProcessPushResult::UnknownChannel,
        }
    }

    pub async fn push(&self, push: ChannelPush) -> InProcessPushResult {
        match push.kind {
            ChannelPushKind::Cancel => {
                self.cancelled.store(true, Ordering::SeqCst);
                InProcessPushResult::Delivered
            }
            ChannelPushKind::Inbound => {
                let mut inbound = self.inbound.lock().await;
                if inbound.len() >= MAX_INBOUND {
                    return InProcessPushResult::Full;
                }
                inbound.push_back(push.value);
                InProcessPushResult::Delivered
            }
            ChannelPushKind::Reply => {
                let Some(request_id) = push.request_id else {
                    return InProcessPushResult::UnknownRequest;
                };
                let sender = self.pending.lock().await.remove(&request_id);
                match sender {
                    Some(sender) => match sender.send(push.value) {
                        Ok(()) => InProcessPushResult::Delivered,
                        Err(_) => InProcessPushResult::Expired,
                    },
                    None if self.was_expired(&request_id).await => InProcessPushResult::Expired,
                    None => InProcessPushResult::UnknownRequest,
                }
            }
        }
    }

    async fn was_expired(&self, request_id: &str) -> bool {
        let mut expired = self.expired.lock().await;
        let cutoff = Instant::now() - EXPIRED_REQUEST_TTL;
        expired.retain(|(_, at)| *at > cutoff);
        expired.iter().any(|(id, _)| id == request_id)
    }

    async fn mark_expired(&self, request_id: &str) {
        let mut expired = self.expired.lock().await;
        while expired.len() >= MAX_EXPIRED_REQUESTS {
            expired.pop_front();
        }
        expired.push_back((request_id.to_string(), Instant::now()));
    }
}

#[async_trait]
impl Channel for InProcessChannel {
    fn channel_id(&self) -> &str {
        &self.channel_id
    }

    fn handle(&self) -> ChannelHandle {
        ChannelHandle {
            channel_id: self.channel_id.clone(),
            request_id: None,
            expires_at: self.expires_at,
            transport: ChannelClientDescriptor::InProcess {},
            fallback: None,
        }
    }

    async fn open(&self, ttl: Duration) -> crate::Result<ChannelTicket> {
        let request_id = new_request_id();
        let expires_at = now_unix() + clamp_ttl(ttl).as_secs() as i64;
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().await.insert(request_id.clone(), sender);
        // The receiver is re-created in `wait` from the registry entry; keep the sender alive by
        // parking the receiver alongside it.
        RECEIVERS.lock().await.insert(request_id.clone(), receiver);
        Ok(ChannelTicket {
            handle: self.handle().for_request(&request_id, expires_at),
            request_id,
            expires_at,
        })
    }

    async fn wait(
        &self,
        ticket: &ChannelTicket,
        cancel: Option<CancellationToken>,
    ) -> crate::Result<ChannelOutcome> {
        let Some(receiver) = RECEIVERS.lock().await.remove(&ticket.request_id) else {
            return Ok(ChannelOutcome::Closed);
        };
        let remaining = (ticket.expires_at - now_unix()).max(0) as u64;
        let timeout = crate::tokio::time::sleep(Duration::from_secs(remaining));
        crate::tokio::pin!(timeout);
        let cancel = cancel.unwrap_or_default();
        let mut receiver = receiver;
        loop {
            crate::tokio::select! {
                result = &mut receiver => {
                    return Ok(match result {
                        Ok(value) => ChannelOutcome::Responded(value),
                        Err(_) => ChannelOutcome::Closed,
                    });
                }
                _ = &mut timeout => {
                    self.abandon(ticket).await;
                    self.mark_expired(&ticket.request_id).await;
                    return Ok(ChannelOutcome::Expired);
                }
                _ = cancel.cancelled() => {
                    self.abandon(ticket).await;
                    return Ok(ChannelOutcome::Cancelled);
                }
                _ = crate::tokio::time::sleep(Duration::from_millis(250)) => {
                    if self.cancelled.load(Ordering::SeqCst) {
                        self.abandon(ticket).await;
                        return Ok(ChannelOutcome::Cancelled);
                    }
                }
            }
        }
    }

    async fn abandon(&self, ticket: &ChannelTicket) {
        self.pending.lock().await.remove(&ticket.request_id);
        RECEIVERS.lock().await.remove(&ticket.request_id);
    }

    async fn drain_inbound(&self) -> Vec<Value> {
        self.inbound.lock().await.drain(..).collect()
    }

    async fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }

    async fn close(&self) {
        let ids: Vec<String> = self
            .pending
            .lock()
            .await
            .drain()
            .map(|(id, _)| id)
            .collect();
        let mut receivers = RECEIVERS.lock().await;
        for id in ids {
            receivers.remove(&id);
        }
        drop(receivers);
        let mut registry = REGISTRY.lock().await;
        let is_self = registry
            .get(&self.channel_id)
            .is_some_and(|entry| std::ptr::eq(entry.as_ptr(), self));
        if is_self {
            registry.remove(&self.channel_id);
        }
    }
}

static RECEIVERS: LazyLock<Mutex<HashMap<String, oneshot::Receiver<Value>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[cfg(test)]
mod tests {
    use super::*;

    #[crate::tokio::test]
    async fn reply_roundtrip_and_late_push() {
        let channel = InProcessChannel::register("run-a", Duration::from_secs(60)).await;
        let ticket = channel.open(Duration::from_secs(5)).await.unwrap();
        let push = ChannelPush {
            channel_id: "run-a".into(),
            request_id: Some(ticket.request_id.clone()),
            kind: ChannelPushKind::Reply,
            value: Value::from(42),
        };
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            crate::tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        crate::tokio::time::sleep(Duration::from_millis(10)).await;
        assert_eq!(
            InProcessChannel::deliver(push.clone()).await,
            InProcessPushResult::Delivered
        );
        assert_eq!(
            waiter.await.unwrap(),
            ChannelOutcome::Responded(Value::from(42))
        );
        assert_eq!(
            InProcessChannel::deliver(push).await,
            InProcessPushResult::UnknownRequest
        );
        channel.close().await;
        assert!(InProcessChannel::lookup("run-a").await.is_none());
    }

    #[crate::tokio::test]
    async fn close_keeps_a_newer_registration_with_the_same_id() {
        let old = InProcessChannel::register("run-dup", Duration::from_secs(60)).await;
        let new = InProcessChannel::register("run-dup", Duration::from_secs(60)).await;
        old.close().await;
        let found = InProcessChannel::lookup("run-dup")
            .await
            .expect("newer channel stays");
        assert!(Arc::ptr_eq(&found, &new));
        new.close().await;
        assert!(InProcessChannel::lookup("run-dup").await.is_none());
    }

    #[crate::tokio::test]
    async fn expired_request_is_reported_as_expired() {
        let channel = InProcessChannel::register("run-b", Duration::from_secs(60)).await;
        let ticket = channel.open(Duration::from_secs(1)).await.unwrap();
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Expired
        );
        let result = channel
            .push(ChannelPush {
                channel_id: "run-b".into(),
                request_id: Some(ticket.request_id.clone()),
                kind: ChannelPushKind::Reply,
                value: Value::Null,
            })
            .await;
        assert_eq!(result, InProcessPushResult::Expired);
    }

    #[crate::tokio::test]
    async fn cancel_and_inbound() {
        let channel = InProcessChannel::register("run-c", Duration::from_secs(60)).await;
        channel
            .push(ChannelPush {
                channel_id: "run-c".into(),
                request_id: None,
                kind: ChannelPushKind::Inbound,
                value: Value::from("steer"),
            })
            .await;
        assert_eq!(channel.drain_inbound().await, vec![Value::from("steer")]);
        assert!(channel.drain_inbound().await.is_empty());

        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            crate::tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        crate::tokio::time::sleep(Duration::from_millis(10)).await;
        channel
            .push(ChannelPush {
                channel_id: "run-c".into(),
                request_id: None,
                kind: ChannelPushKind::Cancel,
                value: Value::Null,
            })
            .await;
        assert_eq!(waiter.await.unwrap(), ChannelOutcome::Cancelled);
        assert!(channel.is_cancelled().await);
    }
}
