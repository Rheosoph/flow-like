use crate::{IngestApi, IngestRequest, Result};
use async_trait::async_trait;
use std::{
    collections::HashMap,
    sync::{Mutex, PoisonError},
    time::{Duration, Instant},
};

/// Postfix looks up each recipient at SMTP time and again over LMTP. Short
/// lived results absorb those repeats and dictionary probes; ingestion still
/// resolves every recipient against the API.
pub struct RecipientCache<A> {
    inner: A,
    accepted_ttl: Duration,
    rejected_ttl: Duration,
    capacity: usize,
    entries: Mutex<HashMap<String, (bool, Instant)>>,
}

impl<A: IngestApi> RecipientCache<A> {
    pub fn new(inner: A, accepted_ttl: Duration, rejected_ttl: Duration, capacity: usize) -> Self {
        Self {
            inner,
            accepted_ttl,
            rejected_ttl,
            capacity,
            entries: Mutex::default(),
        }
    }

    fn cached(&self, key: &str) -> Option<bool> {
        let entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        entries
            .get(key)
            .filter(|(_, expires)| *expires > Instant::now())
            .map(|(accepted, _)| *accepted)
    }

    fn remember(&self, key: String, accepted: bool) {
        let now = Instant::now();
        let ttl = if accepted {
            self.accepted_ttl
        } else {
            self.rejected_ttl
        };
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        if entries.len() >= self.capacity && !entries.contains_key(&key) {
            entries.retain(|_, (_, expires)| *expires > now);
            if entries.len() >= self.capacity {
                entries.clear();
            }
        }
        entries.insert(key, (accepted, now + ttl));
    }
}

#[async_trait]
impl<A: IngestApi> IngestApi for RecipientCache<A> {
    async fn accepts(&self, recipient: &str) -> Result<bool> {
        let key = recipient.to_ascii_lowercase();
        if let Some(accepted) = self.cached(&key) {
            return Ok(accepted);
        }
        let accepted = self.inner.accepts(recipient).await?;
        self.remember(key, accepted);
        Ok(accepted)
    }

    async fn ingest(&self, request: &IngestRequest) -> Result<()> {
        self.inner.ingest(request).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invalid;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct CountingApi {
        lookups: AtomicUsize,
    }

    #[async_trait]
    impl IngestApi for CountingApi {
        async fn accepts(&self, recipient: &str) -> Result<bool> {
            self.lookups.fetch_add(1, Ordering::SeqCst);
            if recipient.starts_with("error@") {
                return Err(invalid("offline"));
            }
            Ok(recipient.starts_with("known@"))
        }

        async fn ingest(&self, _: &IngestRequest) -> Result<()> {
            Ok(())
        }
    }

    fn cache(ttl: Duration, capacity: usize) -> RecipientCache<CountingApi> {
        RecipientCache::new(CountingApi::default(), ttl, ttl, capacity)
    }

    fn lookups(cache: &RecipientCache<CountingApi>) -> usize {
        cache.inner.lookups.load(Ordering::SeqCst)
    }

    #[tokio::test]
    async fn caches_accepted_and_rejected_recipients_case_insensitively() {
        let cache = cache(Duration::from_secs(60), 16);
        assert!(cache.accepts("known@example.test").await.unwrap());
        assert!(cache.accepts("KNOWN@Example.Test").await.unwrap());
        assert!(!cache.accepts("missing@example.test").await.unwrap());
        assert!(!cache.accepts("missing@example.test").await.unwrap());
        assert_eq!(lookups(&cache), 2);
    }

    #[tokio::test]
    async fn lookup_failures_are_not_cached() {
        let cache = cache(Duration::from_secs(60), 16);
        assert!(cache.accepts("error@example.test").await.is_err());
        assert!(cache.accepts("error@example.test").await.is_err());
        assert_eq!(lookups(&cache), 2);
    }

    #[tokio::test]
    async fn expired_results_are_looked_up_again() {
        let cache = cache(Duration::ZERO, 16);
        assert!(cache.accepts("known@example.test").await.unwrap());
        assert!(cache.accepts("known@example.test").await.unwrap());
        assert_eq!(lookups(&cache), 2);
    }

    #[tokio::test]
    async fn entry_count_stays_within_capacity() {
        let cache = cache(Duration::from_secs(60), 2);
        for index in 0..5 {
            cache
                .accepts(&format!("probe{index}@example.test"))
                .await
                .unwrap();
            assert!(cache.entries.lock().unwrap().len() <= 2);
        }
        assert!(!cache.accepts("probe4@example.test").await.unwrap());
        assert_eq!(lookups(&cache), 5);
    }
}
