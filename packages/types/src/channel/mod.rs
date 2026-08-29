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

/// Deadline for one request opened on `handle`: the caller's TTL, clamped to the channel bounds
/// and never past the credential the client would answer with. Waiting on a handle whose token
/// has already died is not patience, it is a stalled run — the waiter gives up instead.
pub fn ticket_deadline(handle: &ChannelHandle, ttl: Duration) -> i64 {
    let requested = now_unix() + clamp_ttl(ttl).as_secs() as i64;
    if handle.expires_at > 0 {
        requested.min(handle.expires_at)
    } else {
        requested
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(expires_at: i64) -> ChannelHandle {
        ChannelHandle {
            channel_id: "run".into(),
            request_id: None,
            expires_at,
            transport: ChannelClientDescriptor::Http {
                push_url: "https://api/api/v1/channels/run/push".into(),
                token: "t".into(),
            },
            fallback: None,
        }
    }

    #[test]
    fn ticket_never_outlives_the_credential_that_would_answer_it() {
        let now = now_unix();

        // A wait that fits inside the channel keeps its own deadline.
        let deadline = ticket_deadline(&handle(now + 3600), Duration::from_secs(120));
        assert!((deadline - now - 120).abs() <= 1);

        // A node asking for longer than the channel lives is cut back to it:
        // past that point no client can authenticate a reply, so the run would
        // be waiting on nothing.
        let deadline = ticket_deadline(&handle(now + 300), Duration::from_secs(8 * 60 * 60));
        assert_eq!(deadline, now + 300);

        // Nine hours stays the ceiling for a channel that outlives it.
        let deadline = ticket_deadline(&handle(now + 100 * 60 * 60), Duration::from_secs(u64::MAX));
        assert!((deadline - now - MAX_TTL.as_secs() as i64).abs() <= 1);

        // An already-dead handle expires the wait immediately rather than
        // parking the run until its own TTL runs out.
        assert!(ticket_deadline(&handle(now - 10), Duration::from_secs(600)) <= now);
    }
}
