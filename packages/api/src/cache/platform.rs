//! Cache space owned by the API process itself.
//!
//! Coordination state that every replica must see — idempotency reservations,
//! single-flight guards — lives under a reserved partition that no app can address:
//! the app-facing cache routes refuse the id, flows only ever reach their own app's
//! partition, and no `App` row exists that could grant a permission on it.

use std::sync::Arc;
use std::time::Duration;

use flow_like_types::tokio::sync::OnceCell;
use sea_orm::DatabaseConnection;
use serde::{Serialize, de::DeserializeOwned};

use super::{CacheKey, CacheStore, CacheStoreConfig, CacheStoreError, SetCacheEntry};
use crate::db::DbDialect;
use crate::error::ApiError;

/// Partition holding platform-owned entries. Not a valid app id anywhere else.
pub const PLATFORM_APP_ID: &str = "__platform__";

pub fn is_platform_app_id(app_id: &str) -> bool {
    app_id == PLATFORM_APP_ID
}

/// Lazily initialized cache backend shared by the app-facing cache and the platform cache.
///
/// The backend is built on first use — serverless replicas that never touch the cache
/// never pay for a Redis handshake or an SDK load — and concurrent first callers share
/// one initialization. A failed attempt is not memoized, so the next caller retries
/// instead of pinning the process to a dead backend.
pub struct CacheBackendHandle {
    db: Arc<DatabaseConnection>,
    dialect: Option<DbDialect>,
    #[cfg(feature = "aws")]
    aws_config: Option<Arc<aws_config::SdkConfig>>,
    store: OnceCell<Arc<dyn CacheStore>>,
}

impl std::fmt::Debug for CacheBackendHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheBackendHandle")
            .field(
                "backend",
                &self.store.get().map(|store| store.backend_name()),
            )
            .finish()
    }
}

impl CacheBackendHandle {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self {
            db,
            dialect: None,
            #[cfg(feature = "aws")]
            aws_config: None,
            store: OnceCell::new(),
        }
    }

    /// The engine behind the connection, so the Postgres backend need not
    /// probe it again on first use.
    pub fn with_dialect(mut self, dialect: DbDialect) -> Self {
        self.dialect = Some(dialect);
        self
    }

    #[cfg(feature = "aws")]
    pub fn with_aws_config(mut self, config: Arc<aws_config::SdkConfig>) -> Self {
        self.aws_config = Some(config);
        self
    }

    /// The cache backend, initializing it on first use.
    pub async fn store(&self) -> Result<Arc<dyn CacheStore>, CacheStoreError> {
        self.store
            .get_or_try_init(|| async {
                let config = CacheStoreConfig {
                    dialect: self.dialect,
                    ..CacheStoreConfig::default()
                }
                .with_db(self.db.clone());
                #[cfg(feature = "aws")]
                let config = match &self.aws_config {
                    Some(aws_config) => config.with_aws_config(aws_config.clone()),
                    None => config,
                };
                let store = super::create_cache_store(config).await?;
                tracing::info!(backend = store.backend_name(), "Initialized cache backend");
                Ok(store)
            })
            .await
            .cloned()
    }

    /// The backend if some earlier call already initialized it; never initializes.
    pub fn initialized(&self) -> Option<Arc<dyn CacheStore>> {
        self.store.get().cloned()
    }

    pub async fn platform(&self) -> Result<PlatformCache, CacheStoreError> {
        Ok(PlatformCache::new(self.store().await?))
    }
}

/// The cache backend, or the 503 every cache-dependent endpoint answers without one.
pub async fn require_cache_store(
    handle: &CacheBackendHandle,
) -> Result<Arc<dyn CacheStore>, ApiError> {
    handle.store().await.map_err(|error| {
        tracing::error!(error = %error, "Cache backend unavailable");
        ApiError::service_unavailable(
            "Cache backend is not configured or failed to initialize on this deployment",
        )
    })
}

/// Outcome of [`PlatformCache::try_insert`].
#[derive(Debug)]
pub enum Reservation<T> {
    /// This call wrote the entry; the caller owns the key until it expires or is replaced.
    Acquired,
    /// A live entry already existed.
    Held(T),
}

/// Typed access to the platform partition. Values are JSON; every write carries a TTL
/// because platform state is coordination state, never a store of record.
#[derive(Clone, Debug)]
pub struct PlatformCache {
    store: Arc<dyn CacheStore>,
}

impl PlatformCache {
    pub fn new(store: Arc<dyn CacheStore>) -> Self {
        Self { store }
    }

    pub fn backend_name(&self) -> &'static str {
        self.store.backend_name()
    }

    fn key(namespace: &str, key: &str) -> CacheKey {
        CacheKey::app(PLATFORM_APP_ID, namespace, key)
    }

    fn entry<T: Serialize>(
        namespace: &str,
        key: &str,
        value: &T,
        ttl: Duration,
    ) -> Result<SetCacheEntry, CacheStoreError> {
        let value = serde_json::to_value(value)
            .map_err(|error| CacheStoreError::Serialization(error.to_string()))?;
        let expires_at = chrono::Utc::now().timestamp_millis() + ttl.as_millis() as i64;
        Ok(SetCacheEntry {
            key: Self::key(namespace, key),
            value,
            expires_at: Some(expires_at),
        })
    }

    fn decode<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, CacheStoreError> {
        serde_json::from_value(value)
            .map_err(|error| CacheStoreError::Serialization(error.to_string()))
    }

    pub async fn get<T: DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
    ) -> Result<Option<T>, CacheStoreError> {
        match self.store.get(&Self::key(namespace, key)).await? {
            Some(entry) => Self::decode(entry.value).map(Some),
            None => Ok(None),
        }
    }

    pub async fn set<T: Serialize>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
        ttl: Duration,
    ) -> Result<(), CacheStoreError> {
        self.store
            .set(Self::entry(namespace, key, value, ttl)?)
            .await?;
        Ok(())
    }

    /// Atomically claim `key` if no live entry holds it; otherwise return the holder's value.
    pub async fn try_insert<T: Serialize + DeserializeOwned>(
        &self,
        namespace: &str,
        key: &str,
        value: &T,
        ttl: Duration,
    ) -> Result<Reservation<T>, CacheStoreError> {
        let entry = Self::entry(namespace, key, value, ttl)?;
        let cache_key = entry.key.clone();
        if self.store.try_insert(entry).await?.is_some() {
            return Ok(Reservation::Acquired);
        }
        match self.store.get(&cache_key).await? {
            Some(existing) => Self::decode(existing.value).map(Reservation::Held),
            None => Err(CacheStoreError::Contention(format!(
                "key '{key}' in namespace '{namespace}' was claimed and released during try_insert"
            ))),
        }
    }

    pub async fn delete(&self, namespace: &str, key: &str) -> Result<bool, CacheStoreError> {
        self.store.delete(&Self::key(namespace, key)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CacheEntry;
    use async_trait::async_trait;
    use flow_like_types::cache::CacheScope;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Debug, Default)]
    struct MemoryStore {
        entries: Mutex<HashMap<String, CacheEntry>>,
        now_ms: Mutex<i64>,
    }

    impl MemoryStore {
        fn slot(key: &CacheKey) -> String {
            format!("{}|{}", key.app_id, key.sort_key())
        }

        fn advance(&self, ms: i64) {
            *self.now_ms.lock().unwrap() += ms;
        }

        fn now(&self) -> i64 {
            *self.now_ms.lock().unwrap()
        }

        fn live(&self, key: &CacheKey) -> Option<CacheEntry> {
            let entries = self.entries.lock().unwrap();
            entries
                .get(&Self::slot(key))
                .filter(|entry| !entry.is_expired_at(self.now()))
                .cloned()
        }

        fn stored(entry: SetCacheEntry, now: i64) -> CacheEntry {
            CacheEntry {
                key: entry.key.key,
                value: entry.value,
                expires_at: entry.expires_at,
                updated_at: now,
            }
        }
    }

    #[async_trait]
    impl CacheStore for MemoryStore {
        fn backend_name(&self) -> &'static str {
            "memory"
        }

        async fn get(&self, key: &CacheKey) -> Result<Option<CacheEntry>, CacheStoreError> {
            Ok(self.live(key))
        }

        async fn exists(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
            Ok(self.live(key).is_some())
        }

        async fn set(&self, entry: SetCacheEntry) -> Result<CacheEntry, CacheStoreError> {
            let slot = Self::slot(&entry.key);
            let stored = Self::stored(entry, self.now());
            self.entries.lock().unwrap().insert(slot, stored.clone());
            Ok(stored)
        }

        async fn try_insert(
            &self,
            entry: SetCacheEntry,
        ) -> Result<Option<CacheEntry>, CacheStoreError> {
            if self.live(&entry.key).is_some() {
                return Ok(None);
            }
            self.set(entry).await.map(Some)
        }

        async fn delete(&self, key: &CacheKey) -> Result<bool, CacheStoreError> {
            Ok(self
                .entries
                .lock()
                .unwrap()
                .remove(&Self::slot(key))
                .is_some())
        }

        async fn delete_namespace(
            &self,
            _app_id: &str,
            _scope: CacheScope,
            _user_id: &str,
            _namespace: &str,
        ) -> Result<i64, CacheStoreError> {
            unimplemented!("not exercised")
        }

        async fn delete_app(&self, app_id: &str) -> Result<i64, CacheStoreError> {
            let mut entries = self.entries.lock().unwrap();
            let before = entries.len();
            entries.retain(|slot, _| !slot.starts_with(&format!("{app_id}|")));
            Ok((before - entries.len()) as i64)
        }

        async fn delete_expired(&self) -> Result<i64, CacheStoreError> {
            let now = self.now();
            let mut entries = self.entries.lock().unwrap();
            let before = entries.len();
            entries.retain(|_, entry| !entry.is_expired_at(now));
            Ok((before - entries.len()) as i64)
        }
    }

    #[derive(Debug, Clone, PartialEq, Serialize, serde::Deserialize)]
    #[serde(tag = "state", rename_all = "snake_case")]
    enum Record {
        InFlight { since_ms: i64 },
        Done { run_id: String },
    }

    fn cache() -> (Arc<MemoryStore>, PlatformCache) {
        let store = Arc::new(MemoryStore::default());
        // Wall-clock expiries from `PlatformCache::entry` are far in the future relative
        // to the double's synthetic clock, so tests expire entries explicitly.
        (store.clone(), PlatformCache::new(store))
    }

    #[flow_like_types::tokio::test]
    async fn first_claim_is_acquired_and_second_sees_the_holder() {
        let (_, cache) = cache();
        let ttl = Duration::from_secs(60);

        let first = cache
            .try_insert("ns", "k", &Record::InFlight { since_ms: 1 }, ttl)
            .await
            .unwrap();
        assert!(matches!(first, Reservation::Acquired));

        let second = cache
            .try_insert("ns", "k", &Record::InFlight { since_ms: 2 }, ttl)
            .await
            .unwrap();
        assert!(matches!(
            second,
            Reservation::Held(Record::InFlight { since_ms: 1 })
        ));
    }

    #[flow_like_types::tokio::test]
    async fn set_replaces_the_reservation_and_get_reads_it_back() {
        let (_, cache) = cache();
        let ttl = Duration::from_secs(60);
        cache
            .try_insert("ns", "k", &Record::InFlight { since_ms: 1 }, ttl)
            .await
            .unwrap();
        cache
            .set(
                "ns",
                "k",
                &Record::Done {
                    run_id: "run".into(),
                },
                ttl,
            )
            .await
            .unwrap();

        let read: Option<Record> = cache.get("ns", "k").await.unwrap();
        assert_eq!(
            read,
            Some(Record::Done {
                run_id: "run".into()
            })
        );
        let held = cache
            .try_insert("ns", "k", &Record::InFlight { since_ms: 3 }, ttl)
            .await
            .unwrap();
        assert!(matches!(held, Reservation::Held(Record::Done { .. })));
    }

    #[flow_like_types::tokio::test]
    async fn expired_holder_does_not_block_a_new_claim() {
        let (store, cache) = cache();
        cache
            .try_insert(
                "ns",
                "k",
                &Record::InFlight { since_ms: 1 },
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        // Push the double's clock past any wall-clock expiry the entry could carry.
        store.advance(chrono::Utc::now().timestamp_millis() + 120_000);

        let claim = cache
            .try_insert(
                "ns",
                "k",
                &Record::InFlight { since_ms: 2 },
                Duration::from_secs(60),
            )
            .await
            .unwrap();
        assert!(matches!(claim, Reservation::Acquired));
    }

    #[flow_like_types::tokio::test]
    async fn platform_entries_live_under_the_reserved_partition_only() {
        let (store, cache) = cache();
        cache
            .set("ns", "k", &1u8, Duration::from_secs(60))
            .await
            .unwrap();

        assert_eq!(store.delete_app("some-app").await.unwrap(), 0);
        assert!(
            store
                .live(&CacheKey::app(PLATFORM_APP_ID, "ns", "k"))
                .is_some()
        );
        assert!(store.live(&CacheKey::app("some-app", "ns", "k")).is_none());
        assert!(is_platform_app_id(PLATFORM_APP_ID));
        assert!(!is_platform_app_id("platform"));
    }
}
