//! The default transport: a request row the client flips through the API push endpoint and the
//! waiter short-polls with exponential backoff.

use std::time::Duration;

use super::{
    Channel, ChannelHandle, ChannelOutcome, ChannelTicket, DEFAULT_POLL_INTERVAL,
    MAX_POLL_INTERVAL, clamp_ttl, new_request_id, now_unix,
};
use crate::Value;
use crate::async_trait;
use crate::tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
pub enum ChannelPoll {
    Pending,
    Responded(Value),
    /// The row is gone: the channel was closed or swept.
    Missing,
}

/// Where request rows live. The API implements this on Postgres; executors implement it over
/// the API's HTTP surface ([`super::HubChannelStore`]).
#[async_trait]
pub trait ChannelStore: Send + Sync {
    async fn register(
        &self,
        channel_id: &str,
        request_id: &str,
        expires_at: i64,
    ) -> crate::Result<()>;
    async fn poll(&self, channel_id: &str, request_id: &str) -> crate::Result<ChannelPoll>;
    async fn remove(&self, channel_id: &str, request_id: &str) -> crate::Result<()>;
    async fn drain_inbound(&self, channel_id: &str) -> crate::Result<Vec<Value>>;
    async fn is_cancelled(&self, channel_id: &str) -> crate::Result<bool>;
    async fn close(&self, channel_id: &str) -> crate::Result<()>;
}

/// Every fourth poll also asks the store whether the whole run was cancelled, so a stop lands
/// during a long wait without doubling the read rate.
const CANCEL_CHECK_EVERY: u32 = 4;

pub struct PollingChannel<S: ChannelStore> {
    store: S,
    channel_id: String,
    handle: ChannelHandle,
    initial_interval: Duration,
    max_interval: Duration,
}

impl<S: ChannelStore> PollingChannel<S> {
    pub fn new(store: S, handle: ChannelHandle) -> Self {
        Self {
            store,
            channel_id: handle.channel_id.clone(),
            handle,
            initial_interval: DEFAULT_POLL_INTERVAL,
            max_interval: MAX_POLL_INTERVAL,
        }
    }

    pub fn with_intervals(mut self, initial: Duration, max: Duration) -> Self {
        self.initial_interval = initial;
        self.max_interval = max.max(initial);
        self
    }

    pub fn store(&self) -> &S {
        &self.store
    }
}

#[async_trait]
impl<S: ChannelStore> Channel for PollingChannel<S> {
    fn channel_id(&self) -> &str {
        &self.channel_id
    }

    fn handle(&self) -> ChannelHandle {
        self.handle.clone()
    }

    async fn open(&self, ttl: Duration) -> crate::Result<ChannelTicket> {
        let request_id = new_request_id();
        let expires_at = now_unix() + clamp_ttl(ttl).as_secs() as i64;
        self.store
            .register(&self.channel_id, &request_id, expires_at)
            .await?;
        Ok(ChannelTicket {
            handle: self.handle.for_request(&request_id, expires_at),
            request_id,
            expires_at,
        })
    }

    async fn wait(
        &self,
        ticket: &ChannelTicket,
        cancel: Option<CancellationToken>,
    ) -> crate::Result<ChannelOutcome> {
        let mut interval = self.initial_interval;
        let mut polls: u32 = 0;
        loop {
            let remaining = ticket.expires_at - now_unix();
            if remaining <= 0 {
                self.abandon(ticket).await;
                return Ok(ChannelOutcome::Expired);
            }
            let sleep_for = interval.min(Duration::from_secs(remaining as u64));
            match &cancel {
                Some(token) => {
                    crate::tokio::select! {
                        _ = token.cancelled() => {
                            self.abandon(ticket).await;
                            return Ok(ChannelOutcome::Cancelled);
                        }
                        _ = crate::tokio::time::sleep(sleep_for) => {}
                    }
                }
                None => crate::tokio::time::sleep(sleep_for).await,
            }
            interval = (interval * 2).min(self.max_interval);
            polls = polls.wrapping_add(1);

            match self.store.poll(&self.channel_id, &ticket.request_id).await {
                Ok(ChannelPoll::Responded(value)) => {
                    self.abandon(ticket).await;
                    return Ok(ChannelOutcome::Responded(value));
                }
                Ok(ChannelPoll::Missing) => return Ok(ChannelOutcome::Closed),
                Ok(ChannelPoll::Pending) => {}
                Err(error) => {
                    tracing::warn!(
                        %error,
                        channel_id = %self.channel_id,
                        request_id = %ticket.request_id,
                        "channel poll failed; retrying"
                    );
                }
            }

            if polls.is_multiple_of(CANCEL_CHECK_EVERY) && self.is_cancelled().await {
                self.abandon(ticket).await;
                return Ok(ChannelOutcome::Cancelled);
            }
        }
    }

    async fn abandon(&self, ticket: &ChannelTicket) {
        if let Err(error) = self
            .store
            .remove(&self.channel_id, &ticket.request_id)
            .await
        {
            tracing::debug!(
                %error,
                channel_id = %self.channel_id,
                request_id = %ticket.request_id,
                "channel request cleanup failed"
            );
        }
    }

    async fn drain_inbound(&self) -> Vec<Value> {
        match self.store.drain_inbound(&self.channel_id).await {
            Ok(messages) => messages,
            Err(error) => {
                tracing::warn!(%error, channel_id = %self.channel_id, "channel inbound drain failed");
                Vec::new()
            }
        }
    }

    async fn is_cancelled(&self) -> bool {
        self.store
            .is_cancelled(&self.channel_id)
            .await
            .unwrap_or(false)
    }

    async fn close(&self) {
        if let Err(error) = self.store.close(&self.channel_id).await {
            tracing::warn!(%error, channel_id = %self.channel_id, "channel close failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::Mutex;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Default, Clone)]
    struct MemoryStore {
        rows: Arc<Mutex<HashMap<String, Option<Value>>>>,
        cancelled: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl ChannelStore for MemoryStore {
        async fn register(&self, _c: &str, r: &str, _e: i64) -> crate::Result<()> {
            self.rows.lock().await.insert(r.to_string(), None);
            Ok(())
        }
        async fn poll(&self, _c: &str, r: &str) -> crate::Result<ChannelPoll> {
            Ok(match self.rows.lock().await.get(r) {
                None => ChannelPoll::Missing,
                Some(None) => ChannelPoll::Pending,
                Some(Some(v)) => ChannelPoll::Responded(v.clone()),
            })
        }
        async fn remove(&self, _c: &str, r: &str) -> crate::Result<()> {
            self.rows.lock().await.remove(r);
            Ok(())
        }
        async fn drain_inbound(&self, _c: &str) -> crate::Result<Vec<Value>> {
            Ok(Vec::new())
        }
        async fn is_cancelled(&self, _c: &str) -> crate::Result<bool> {
            Ok(self.cancelled.load(std::sync::atomic::Ordering::Relaxed))
        }
        async fn close(&self, _c: &str) -> crate::Result<()> {
            self.rows.lock().await.clear();
            Ok(())
        }
    }

    fn handle() -> ChannelHandle {
        ChannelHandle {
            channel_id: "run".into(),
            request_id: None,
            expires_at: now_unix() + 60,
            transport: super::super::ChannelClientDescriptor::Http {
                push_url: "http://x".into(),
                token: "t".into(),
            },
            fallback: None,
        }
    }

    #[crate::tokio::test]
    async fn responds_after_row_flips() {
        let store = MemoryStore::default();
        let channel = PollingChannel::new(store.clone(), handle())
            .with_intervals(Duration::from_millis(5), Duration::from_millis(10));
        let ticket = channel.open(Duration::from_secs(5)).await.unwrap();
        assert_eq!(
            ticket.handle.request_id.as_deref(),
            Some(ticket.request_id.as_str())
        );
        let rows = store.rows.clone();
        let id = ticket.request_id.clone();
        crate::tokio::spawn(async move {
            crate::tokio::time::sleep(Duration::from_millis(20)).await;
            rows.lock().await.insert(id, Some(Value::from("yes")));
        });
        let outcome = channel.wait(&ticket, None).await.unwrap();
        assert_eq!(outcome, ChannelOutcome::Responded(Value::from("yes")));
        assert!(store.rows.lock().await.is_empty());
    }

    #[crate::tokio::test]
    async fn expires_and_cancels() {
        let store = MemoryStore::default();
        let channel = PollingChannel::new(store.clone(), handle())
            .with_intervals(Duration::from_millis(5), Duration::from_millis(10));
        let ticket = channel.open(Duration::from_secs(1)).await.unwrap();
        assert_eq!(
            channel.wait(&ticket, None).await.unwrap(),
            ChannelOutcome::Expired
        );

        let ticket = channel.open(Duration::from_secs(30)).await.unwrap();
        let token = CancellationToken::new();
        let t2 = token.clone();
        crate::tokio::spawn(async move {
            crate::tokio::time::sleep(Duration::from_millis(15)).await;
            t2.cancel();
        });
        assert_eq!(
            channel.wait(&ticket, Some(token)).await.unwrap(),
            ChannelOutcome::Cancelled
        );
    }
}
