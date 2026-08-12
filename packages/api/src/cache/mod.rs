//! Key/value cache abstraction for flows.
//!
//! Flows use this to keep small values between runs — feature flags, throttling
//! counters, memoized lookups, resolved configuration — without paying for an object
//! store round trip. Entries are scoped to an app, optionally to a single user, and may
//! carry a TTL.
//!
//! ## Backend Selection
//!
//! | Backend | Latency | TTL | Best For |
//! |---------|---------|-----|----------|
//! | PostgreSQL | Medium | Swept | Default; no extra infrastructure |
//! | Redis | Low | Native | High-throughput, already deployed in cluster |
//! | DynamoDB | Low | Native | Serverless, AWS-native |
//!
//! ## Recommended Configuration
//!
//! | Deployment | Backend | Reason |
//! |------------|---------|--------|
//! | AWS Lambda/ECS | `dynamodb` | Native TTL, serverless, no connection pool to babysit |
//! | Kubernetes | `redis` | Native TTL, lowest latency, already in the cluster |
//! | Docker Compose | `redis` | Simple setup, native TTL |
//! | Local / small | `postgres` | Nothing else to run |
//!
//! ## Configuration
//!
//! ```bash
//! # Select backend
//! CACHE_BACKEND=redis            # postgres (default), redis, dynamodb
//!
//! # PostgreSQL (default; expiry handled by the cache sweeper)
//! DATABASE_URL=postgres://...
//!
//! # Redis
//! CACHE_REDIS_URL=redis://...    # falls back to REDIS_URL
//!
//! # DynamoDB (table AppCache, TTL attribute `expires_at`)
//! DYNAMODB_TABLE_PREFIX=flowlike-  # optional, shared with the execution state store
//!
//! # Limits, enforced identically for every backend
//! CACHE_MAX_KEY_BYTES=512
//! CACHE_MAX_VALUE_BYTES=1048576  # larger data belongs in app storage, not the cache;
//!                                # DynamoDB stores values above ~300 KB as chunked items
//! CACHE_MAX_TTL_SECONDS=2592000
//! CACHE_DEFAULT_TTL_SECONDS=0    # 0 keeps entries until they are deleted
//! ```
//!
//! Offline apps never reach this module — their cache nodes write to the local
//! filesystem instead. See `packages/catalog/data/src/data/cache/`.

mod postgres;
pub mod sweeper;
mod types;

#[cfg(feature = "redis")]
mod redis;

#[cfg(feature = "dynamodb")]
mod dynamodb;

pub use postgres::PostgresCacheStore;
pub use types::*;

#[cfg(feature = "redis")]
pub use redis::RedisCacheStore;

#[cfg(feature = "dynamodb")]
pub use dynamodb::DynamoDbCacheStore;

use std::sync::Arc;

#[cfg(feature = "aws")]
use aws_config::SdkConfig;

/// Backend type for cache storage.
#[derive(Clone, Debug, Default)]
pub enum CacheBackend {
    #[default]
    Postgres,
    #[cfg(feature = "redis")]
    Redis,
    #[cfg(feature = "dynamodb")]
    DynamoDB,
}

impl CacheBackend {
    pub fn from_env() -> Self {
        let requested = std::env::var("CACHE_BACKEND").unwrap_or_default();
        match requested.trim().to_lowercase().as_str() {
            #[cfg(feature = "redis")]
            "redis" => Self::Redis,
            #[cfg(feature = "dynamodb")]
            "dynamodb" | "dynamo" => Self::DynamoDB,
            "" | "postgres" | "postgresql" => Self::Postgres,
            other => {
                // A typo, or a backend whose Cargo feature is off for this deployment
                // target. Falling back silently would look like the cache is simply slow.
                tracing::warn!(
                    requested = other,
                    "CACHE_BACKEND is not available in this build; falling back to postgres"
                );
                Self::Postgres
            }
        }
    }
}

/// Dependencies the cache backends may need, supplied from `AppState`.
#[derive(Default)]
pub struct CacheStoreConfig {
    pub db: Option<Arc<sea_orm::DatabaseConnection>>,
    #[cfg(feature = "aws")]
    pub aws_config: Option<Arc<SdkConfig>>,
}

impl CacheStoreConfig {
    pub fn with_db(mut self, db: Arc<sea_orm::DatabaseConnection>) -> Self {
        self.db = Some(db);
        self
    }

    #[cfg(feature = "aws")]
    pub fn with_aws_config(mut self, config: Arc<SdkConfig>) -> Self {
        self.aws_config = Some(config);
        self
    }
}

/// Build the cache store selected by the environment.
///
/// Called once during `AppState` construction — the returned store is held for the
/// lifetime of the process so Redis connections are not rebuilt per request.
pub async fn create_cache_store(
    config: CacheStoreConfig,
) -> Result<Arc<dyn CacheStore>, CacheStoreError> {
    let backend = CacheBackend::from_env();

    match backend {
        CacheBackend::Postgres => {
            let db = config.db.ok_or_else(|| {
                CacheStoreError::Configuration(
                    "Database connection required for the Postgres cache backend".into(),
                )
            })?;
            Ok(Arc::new(PostgresCacheStore::new(db)))
        }

        #[cfg(feature = "redis")]
        CacheBackend::Redis => Ok(Arc::new(RedisCacheStore::from_env().await?)),

        #[cfg(feature = "dynamodb")]
        CacheBackend::DynamoDB => match config.aws_config {
            Some(aws_cfg) => Ok(Arc::new(DynamoDbCacheStore::new(&aws_cfg))),
            None => Ok(Arc::new(DynamoDbCacheStore::from_env().await?)),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_backends_fall_back_to_postgres() {
        // The enum variants for disabled features do not exist, so an unrecognized value
        // must still produce a usable backend rather than panicking.
        unsafe { std::env::set_var("CACHE_BACKEND", "cassandra") };
        assert!(matches!(CacheBackend::from_env(), CacheBackend::Postgres));
        unsafe { std::env::remove_var("CACHE_BACKEND") };
        assert!(matches!(CacheBackend::from_env(), CacheBackend::Postgres));
    }
}
