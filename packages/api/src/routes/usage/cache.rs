use std::{future::Future, sync::Arc, time::Duration};

use axum::http::{HeaderMap, header};
use flow_like_types::tokio::sync::Mutex;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{
    cache::{CacheBackendHandle, best_effort},
    error::ApiError,
};

const NAMESPACE: &str = "execution-usage-v1";
const TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CacheMode {
    Cached,
    Refresh,
    NoStore,
}

impl CacheMode {
    pub(super) fn from_headers(headers: &HeaderMap) -> Self {
        let mut mode = Self::Cached;
        for value in headers.get_all(header::CACHE_CONTROL) {
            let Ok(value) = value.to_str() else { continue };
            for directive in value.split(',') {
                let (name, value) = directive
                    .trim()
                    .split_once('=')
                    .unwrap_or((directive.trim(), ""));
                if name.trim().eq_ignore_ascii_case("no-store") {
                    return Self::NoStore;
                }
                if name.trim().eq_ignore_ascii_case("no-cache")
                    || (name.trim().eq_ignore_ascii_case("max-age")
                        && value.trim().trim_matches('"') == "0")
                {
                    mode = Self::Refresh;
                }
            }
        }
        mode
    }
}

#[derive(Deserialize, Serialize)]
struct Snapshot<T> {
    expires_at_ms: i64,
    value: T,
}

/// Shared snapshots survive Lambda turnover. Local storage and lock stripes
/// bound memory and coalesce concurrent requests within one API instance.
pub(crate) struct UsageCache {
    local: moka::sync::Cache<String, Arc<str>>,
    locks: [Mutex<()>; 256],
}

pub(super) fn cache_key(scope: impl Serialize) -> String {
    let encoded = serde_json::to_vec(&scope).expect("execution cache scope is JSON serializable");
    blake3::hash(&encoded).to_hex().to_string()
}

impl Default for UsageCache {
    fn default() -> Self {
        Self {
            local: moka::sync::Cache::builder()
                .max_capacity(8 * 1024 * 1024)
                .weigher(|key: &String, value: &Arc<str>| {
                    (key.len() + value.len()).min(u32::MAX as usize) as u32
                })
                .time_to_live(TTL)
                .build(),
            locks: std::array::from_fn(|_| Mutex::new(())),
        }
    }
}

impl UsageCache {
    fn local_get<T: DeserializeOwned>(&self, key: &str) -> Option<T> {
        let encoded = self.local.get(key)?;
        let snapshot: Snapshot<T> = serde_json::from_str(&encoded).ok()?;
        if snapshot.expires_at_ms <= chrono::Utc::now().timestamp_millis() {
            self.local.invalidate(key);
            return None;
        }
        Some(snapshot.value)
    }

    fn local_set<T: Serialize>(&self, key: &str, snapshot: &Snapshot<T>) {
        if let Ok(encoded) = serde_json::to_string(snapshot)
            && encoded.len() <= best_effort::MAX_ENTRY_BYTES
        {
            self.local.insert(key.to_owned(), Arc::from(encoded));
        }
    }

    pub(super) async fn get_or_compute<T: Serialize + DeserializeOwned>(
        &self,
        shared: &CacheBackendHandle,
        key: String,
        mode: CacheMode,
        compute: impl Future<Output = Result<T, ApiError>>,
    ) -> Result<T, ApiError> {
        if mode == CacheMode::NoStore {
            return compute.await;
        }
        if mode == CacheMode::Cached
            && let Some(value) = self.local_get(&key)
        {
            return Ok(value);
        }
        let stripe = blake3::hash(key.as_bytes()).as_bytes()[0] as usize;
        let _guard = self.locks[stripe].lock().await;
        if mode == CacheMode::Cached
            && let Some(value) = self.local_get(&key)
        {
            return Ok(value);
        }
        if mode == CacheMode::Cached
            && let Some(snapshot) = best_effort::get::<Snapshot<T>>(shared, NAMESPACE, &key).await
            && snapshot.expires_at_ms > chrono::Utc::now().timestamp_millis()
        {
            // Promotion retains the original deadline, even if the backend
            // physically keeps expired entries until its next cleanup pass.
            self.local_set(&key, &snapshot);
            return Ok(snapshot.value);
        }

        let expires_at_ms = chrono::Utc::now().timestamp_millis() + TTL.as_millis() as i64;
        let value = compute.await?;
        let snapshot = Snapshot {
            expires_at_ms,
            value,
        };
        self.local_set(&key, &snapshot);
        best_effort::set(shared, NAMESPACE, &key, &snapshot, TTL).await;
        Ok(snapshot.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn request_directives_choose_refresh_or_no_storage() {
        let mut headers = HeaderMap::new();
        assert_eq!(CacheMode::from_headers(&headers), CacheMode::Cached);
        for directive in [
            "no-cache",
            "max-age=0",
            "private, MAX-AGE = \"0\"",
            "NO-CACHE",
        ] {
            headers.insert(header::CACHE_CONTROL, directive.parse().unwrap());
            assert_eq!(CacheMode::from_headers(&headers), CacheMode::Refresh);
        }
        headers.append(header::CACHE_CONTROL, "no-store".parse().unwrap());
        assert_eq!(CacheMode::from_headers(&headers), CacheMode::NoStore);
        headers.insert(header::CACHE_CONTROL, "max-age=300".parse().unwrap());
        assert_eq!(CacheMode::from_headers(&headers), CacheMode::Cached);
    }

    #[tokio::test]
    async fn explicit_refresh_ignores_both_cache_levels_and_replaces_the_snapshot() {
        let cache = UsageCache::default();
        let shared = CacheBackendHandle::memory_for_test();
        assert_eq!(
            cache
                .get_or_compute(&shared, "refresh".into(), CacheMode::Cached, async {
                    Ok(1_u64)
                })
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            cache
                .get_or_compute(&shared, "refresh".into(), CacheMode::Refresh, async {
                    Ok(2_u64)
                })
                .await
                .unwrap(),
            2
        );
        assert_eq!(
            cache
                .get_or_compute(&shared, "refresh".into(), CacheMode::Cached, async {
                    Ok(99_u64)
                })
                .await
                .unwrap(),
            2
        );
        let other = UsageCache::default();
        assert_eq!(
            other
                .get_or_compute(&shared, "refresh".into(), CacheMode::Refresh, async {
                    Ok(3_u64)
                })
                .await
                .unwrap(),
            3
        );
        let cold = UsageCache::default();
        assert_eq!(
            cold.get_or_compute(&shared, "refresh".into(), CacheMode::Cached, async {
                Ok(99_u64)
            })
            .await
            .unwrap(),
            3
        );
        let failed = cold
            .get_or_compute::<u64>(&shared, "refresh".into(), CacheMode::Refresh, async {
                Err(ApiError::FORBIDDEN)
            })
            .await;
        assert!(
            failed.is_err(),
            "explicit refresh must not fall back to stale data"
        );
    }

    #[tokio::test]
    async fn no_store_does_not_read_or_replace_cached_snapshots() {
        let cache = UsageCache::default();
        let shared = CacheBackendHandle::memory_for_test();
        cache
            .get_or_compute(&shared, "private".into(), CacheMode::Cached, async {
                Ok(1_u64)
            })
            .await
            .unwrap();
        assert_eq!(
            cache
                .get_or_compute(&shared, "private".into(), CacheMode::NoStore, async {
                    Ok(2_u64)
                })
                .await
                .unwrap(),
            2
        );
        assert_eq!(cache.local_get::<u64>("private"), Some(1));
        let cold = UsageCache::default();
        assert_eq!(
            cold.get_or_compute(&shared, "private".into(), CacheMode::Cached, async {
                Ok(99_u64)
            })
            .await
            .unwrap(),
            1
        );
    }

    #[test]
    fn scope_keys_distinguish_users_apps_windows_pages_and_dates() {
        let key = cache_key(("activity", "alice", None::<&str>, 7, "2026-10-21"));
        for other in [
            cache_key(("activity", "bob", None::<&str>, 7, "2026-10-21")),
            cache_key(("activity", "alice", Some("app"), 7, "2026-10-21")),
            cache_key(("activity", "alice", None::<&str>, 30, "2026-10-21")),
            cache_key(("activity", "alice", None::<&str>, 7, "2026-10-22")),
            cache_key(("history", "alice", None::<&str>, 0, 50)),
        ] {
            assert_ne!(key, other);
        }
        assert_ne!(cache_key(("alice:a", "b")), cache_key(("alice", "a:b")));
    }

    #[tokio::test]
    async fn concurrent_misses_share_one_load_and_other_instances_reuse_the_snapshot() {
        let cache = UsageCache::default();
        let shared = CacheBackendHandle::memory_for_test();
        let calls = AtomicUsize::new(0);
        let requests = (0..20).map(|_| {
            cache.get_or_compute(
                &shared,
                "same".into(),
                super::super::cache::CacheMode::Cached,
                async {
                    calls.fetch_add(1, Ordering::SeqCst);
                    flow_like_types::tokio::task::yield_now().await;
                    Ok(42_u64)
                },
            )
        });
        for result in futures::future::join_all(requests).await {
            assert_eq!(result.unwrap(), 42);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let other = UsageCache::default();
        assert_eq!(
            other
                .get_or_compute(
                    &shared,
                    "same".into(),
                    super::super::cache::CacheMode::Cached,
                    async {
                        panic!("shared snapshot should avoid the query");
                        #[allow(unreachable_code)]
                        Ok(0_u64)
                    }
                )
                .await
                .unwrap(),
            42
        );
    }

    #[tokio::test]
    async fn failures_are_not_cached_and_expired_shared_values_are_ignored() {
        let cache = UsageCache::default();
        let shared = CacheBackendHandle::memory_for_test();
        let failed = cache
            .get_or_compute::<u64>(
                &shared,
                "retry".into(),
                super::super::cache::CacheMode::Cached,
                async { Err(ApiError::FORBIDDEN) },
            )
            .await;
        assert!(failed.is_err());
        assert_eq!(
            cache
                .get_or_compute(
                    &shared,
                    "retry".into(),
                    super::super::cache::CacheMode::Cached,
                    async { Ok(7_u64) }
                )
                .await
                .unwrap(),
            7
        );
        best_effort::set(
            &shared,
            NAMESPACE,
            "expired",
            &Snapshot {
                expires_at_ms: 0,
                value: 1_u64,
            },
            TTL,
        )
        .await;
        assert_eq!(
            cache
                .get_or_compute(
                    &shared,
                    "expired".into(),
                    super::super::cache::CacheMode::Cached,
                    async { Ok(2_u64) }
                )
                .await
                .unwrap(),
            2
        );
        cache.local_set(
            "expired-local",
            &Snapshot {
                expires_at_ms: 0,
                value: 3_u64,
            },
        );
        assert!(cache.local_get::<u64>("expired-local").is_none());
    }

    #[tokio::test]
    async fn promotion_preserves_deadline_and_oversized_responses_are_not_cached() {
        let cache = UsageCache::default();
        let shared = CacheBackendHandle::memory_for_test();
        let deadline = chrono::Utc::now().timestamp_millis() + 60_000;
        best_effort::set(
            &shared,
            NAMESPACE,
            "promote",
            &Snapshot {
                expires_at_ms: deadline,
                value: 1_u64,
            },
            TTL,
        )
        .await;
        assert_eq!(
            cache
                .get_or_compute(
                    &shared,
                    "promote".into(),
                    super::super::cache::CacheMode::Cached,
                    async { Ok(2_u64) }
                )
                .await
                .unwrap(),
            1
        );
        let stored: Snapshot<u64> =
            serde_json::from_str(&cache.local.get("promote").unwrap()).unwrap();
        assert_eq!(stored.expires_at_ms, deadline);
        let large = "x".repeat(best_effort::MAX_ENTRY_BYTES);
        assert_eq!(
            cache
                .get_or_compute(
                    &shared,
                    "large".into(),
                    super::super::cache::CacheMode::Cached,
                    async { Ok(large.clone()) }
                )
                .await
                .unwrap(),
            large
        );
        assert!(cache.local.get("large").is_none());
        assert!(
            best_effort::get::<Snapshot<String>>(&shared, NAMESPACE, "large")
                .await
                .is_none()
        );
    }
}
