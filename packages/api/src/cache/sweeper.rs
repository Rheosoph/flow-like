//! Periodic removal of expired cache entries.
//!
//! Only the Postgres backend needs this — Redis and DynamoDB evict on their own and
//! report zero reclaimed entries. Long-running deployments run the ticker; serverless
//! deployments drive the same work through `POST /maintenance/run` with the
//! `cache_cleanup` job, or `POST /admin/cache/sweep` for an ad-hoc run.

use std::sync::Arc;
use std::time::Duration;

use flow_like_types::tokio::{self, task::JoinHandle};

use super::types::{CacheStore, CacheStoreError};

const DEFAULT_INTERVAL_SECS: u64 = 900;

#[derive(Clone, Debug)]
pub struct CacheSweeperConfig {
    pub interval: Duration,
}

impl CacheSweeperConfig {
    /// Build config from environment variables.
    /// - `CACHE_SWEEPER_INTERVAL_SECS`: how often to sweep (default 900)
    pub fn from_env() -> Self {
        let interval = std::env::var("CACHE_SWEEPER_INTERVAL_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(DEFAULT_INTERVAL_SECS);
        Self {
            interval: Duration::from_secs(interval),
        }
    }
}

/// Spawn the cache sweeper as a background task.
///
/// Returns `None` when `CACHE_SWEEPER_DISABLED=1` is set, or when the selected backend
/// evicts on its own — there is no point waking up every 15 minutes to do nothing.
pub fn spawn_cache_sweeper(
    store: Arc<dyn CacheStore>,
    config: CacheSweeperConfig,
) -> Option<JoinHandle<()>> {
    if std::env::var("CACHE_SWEEPER_DISABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        tracing::info!("Cache sweeper disabled via CACHE_SWEEPER_DISABLED");
        return None;
    }

    if store.backend_name() != "postgres" {
        tracing::info!(
            backend = store.backend_name(),
            "Cache backend expires entries natively; not spawning the cache sweeper"
        );
        return None;
    }

    tracing::info!(
        interval_secs = config.interval.as_secs(),
        backend = store.backend_name(),
        "Spawning cache sweeper"
    );

    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(config.interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // First tick fires immediately; let services come up before we hit the DB.
        ticker.tick().await;

        loop {
            ticker.tick().await;
            match sweep_once(store.as_ref()).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(deleted = n, "Cache sweeper removed expired entries"),
                Err(e) => tracing::error!(error = %e, "Cache sweeper iteration failed"),
            }
        }
    });

    Some(handle)
}

/// Run one sweep. Returns the number of entries removed.
pub async fn sweep_once(store: &dyn CacheStore) -> Result<i64, CacheStoreError> {
    store.delete_expired().await
}
