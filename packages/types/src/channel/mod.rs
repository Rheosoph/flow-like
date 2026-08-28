//! Channels: the one primitive a waiting run uses to get a reply from its client.
//!
//! The waiter streams a request to the client over the run's existing event stream (with a
//! [`ChannelHandle`] embedded) and blocks in [`Channel::wait`]. How the reply travels back is the
//! transport's business: the API push endpoint polled through a [`polling::ChannelStore`], an
//! in-process oneshot on the desktop, or a cloud pub/sub connection held by the executor.

pub mod hub;
pub mod in_process;
pub mod polling;

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub use flow_like_types_contracts::channel::*;
pub use hub::HubChannelStore;
pub use in_process::{InProcessChannel, InProcessPushResult};
pub use polling::{ChannelPoll, ChannelStore, PollingChannel};

use crate::Value;
use crate::async_trait;
use crate::tokio_util::sync::CancellationToken;

/// Initial delay between reads of a pending request; doubles up to [`MAX_POLL_INTERVAL`] so a
/// long human-in-the-loop wait does not hammer the store while fast replies still land quickly.
pub const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(500);
pub const MAX_POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Bounds a caller-supplied TTL. Cloud runs are capped by their execution environment anyway.
pub const MIN_TTL: Duration = Duration::from_secs(1);
pub const MAX_TTL: Duration = Duration::from_secs(9 * 60 * 60);

pub fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

pub fn new_request_id() -> String {
    crate::create_id()
}

pub fn clamp_ttl(ttl: Duration) -> Duration {
    ttl.clamp(MIN_TTL, MAX_TTL)
}

/// One registered request the waiter can block on.
#[derive(Debug, Clone)]
pub struct ChannelTicket {
    pub request_id: String,
    pub expires_at: i64,
    /// Ready to embed in the request streamed to the client.
    pub handle: ChannelHandle,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelOutcome {
    Responded(Value),
    /// The ticket's TTL elapsed without a reply.
    Expired,
    /// The run (or the wait) was cancelled.
    Cancelled,
    /// The channel went away underneath the waiter (run finished elsewhere, rows swept).
    Closed,
}

#[async_trait]
pub trait Channel: Send + Sync {
    fn channel_id(&self) -> &str;

    /// Channel-level handle (`request_id: None`) for unsolicited pushes such as cancel/steer.
    fn handle(&self) -> ChannelHandle;

    /// Register a request. Must complete before the request is streamed to the client so a
    /// reply can never race its own registration.
    async fn open(&self, ttl: Duration) -> crate::Result<ChannelTicket>;

    /// Block until the client replies, the ticket expires, or `cancel` fires. Cleans the
    /// registration up on every exit path.
    async fn wait(
        &self,
        ticket: &ChannelTicket,
        cancel: Option<CancellationToken>,
    ) -> crate::Result<ChannelOutcome>;

    /// Drop a registration the waiter gave up on before calling [`Channel::wait`].
    async fn abandon(&self, ticket: &ChannelTicket);

    /// Take every unsolicited message pushed since the last drain, oldest first.
    async fn drain_inbound(&self) -> Vec<Value>;

    async fn is_cancelled(&self) -> bool;

    /// Release everything the channel holds (rows, connections, registry entries).
    async fn close(&self);
}
