use crate::error::{Result, SecretError};
use crate::{SecretProviderKind, SecretRef, SecretValue};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct CacheKey {
    pub provider: Option<SecretProviderKind>,
    pub key: String,
    pub version: Option<String>,
}

impl From<&SecretRef> for CacheKey {
    fn from(value: &SecretRef) -> Self {
        Self {
            provider: value.provider,
            key: value.key.clone(),
            version: value.version.clone(),
        }
    }
}

#[derive(Clone)]
enum CachePayload {
    Hit(SecretValue),
    Negative(SecretError),
}

#[derive(Clone)]
struct CacheEntry {
    payload: CachePayload,
    inserted_at: Instant,
    expires_at: Instant,
}

pub(crate) struct SecretCache {
    ttl: Duration,
    negative_ttl: Duration,
    max_entries: usize,
    entries: RwLock<HashMap<CacheKey, CacheEntry>>,
}

impl SecretCache {
    pub fn new(ttl: Duration, negative_ttl: Duration, max_entries: usize) -> Self {
        Self {
            ttl,
            negative_ttl,
            max_entries,
            entries: RwLock::new(HashMap::new()),
        }
    }

    pub async fn get(&self, key: &CacheKey) -> Option<Result<SecretValue>> {
        let now = Instant::now();
        let mut entries = self.entries.write().await;

        match entries.get(key) {
            Some(entry) if entry.expires_at > now => Some(match &entry.payload {
                CachePayload::Hit(value) => Ok(value.clone()),
                CachePayload::Negative(error) => Err(error.clone()),
            }),
            Some(_) => {
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    pub async fn insert_success(&self, key: CacheKey, value: SecretValue) {
        self.insert_with_payload(key, CachePayload::Hit(value), self.ttl)
            .await;
    }

    pub async fn insert_failure(&self, key: CacheKey, error: SecretError) {
        self.insert_with_payload(key, CachePayload::Negative(error), self.negative_ttl)
            .await;
    }

    pub async fn invalidate(&self, key: &CacheKey) {
        self.entries.write().await.remove(key);
    }

    async fn insert_with_payload(&self, key: CacheKey, payload: CachePayload, ttl: Duration) {
        let now = Instant::now();
        let mut entries = self.entries.write().await;

        if entries.len() >= self.max_entries {
            evict_oldest_entry(&mut entries);
        }

        entries.insert(
            key,
            CacheEntry {
                payload,
                inserted_at: now,
                expires_at: now + ttl,
            },
        );
    }
}

fn evict_oldest_entry(entries: &mut HashMap<CacheKey, CacheEntry>) {
    if let Some(oldest_key) = entries
        .iter()
        .min_by_key(|(_, value)| value.inserted_at)
        .map(|(key, _)| key.clone())
    {
        entries.remove(&oldest_key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SecretValue;

    fn must_some<T>(value: Option<T>, context: &str) -> T {
        match value {
            Some(value) => value,
            None => panic!("{context}"),
        }
    }

    #[tokio::test]
    async fn expires_entries() {
        let cache = SecretCache::new(Duration::from_millis(10), Duration::from_millis(10), 8);
        let key = CacheKey {
            provider: Some(SecretProviderKind::Env),
            key: "A".to_string(),
            version: None,
        };

        cache
            .insert_success(key.clone(), SecretValue::from_string("value".to_string()))
            .await;

        assert!(cache.get(&key).await.is_some());
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(cache.get(&key).await.is_none());
    }

    #[tokio::test]
    async fn stores_negative_cache_entries() {
        let cache = SecretCache::new(Duration::from_secs(1), Duration::from_secs(1), 8);
        let key = CacheKey {
            provider: Some(SecretProviderKind::Env),
            key: "missing".to_string(),
            version: None,
        };

        cache
            .insert_failure(
                key.clone(),
                SecretError::SecretNotFound(SecretProviderKind::Env),
            )
            .await;

        let hit = must_some(cache.get(&key).await, "must have entry");
        match hit {
            Ok(_) => panic!("expected cached error"),
            Err(error) => assert_eq!(error, SecretError::SecretNotFound(SecretProviderKind::Env)),
        }
    }
}
