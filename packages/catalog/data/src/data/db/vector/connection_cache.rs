use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime},
};
use tokio::sync::OnceCell;

// Each connection owns a separately bounded Lance session. Limit the number
// retained as well as their age so a shared executor cannot accumulate tenants.
const MAX_CONNECTIONS: usize = 16;
const CONNECTION_TTL: Duration = Duration::from_secs(300);
const EXPIRY_MARGIN: Duration = Duration::from_secs(30);

struct Entry<T> {
    value: OnceCell<Arc<T>>,
    created_at: Instant,
    expires_at: SystemTime,
}

impl<T> Entry<T> {
    fn reusable(&self, now: Instant, wall_now: SystemTime) -> bool {
        now.duration_since(self.created_at) < CONNECTION_TTL
            && self.expires_at > wall_now + EXPIRY_MARGIN
    }
}

pub(super) struct ConnectionCache<T> {
    entries: Mutex<HashMap<[u8; 32], (Arc<Entry<T>>, Instant)>>,
}

impl<T> Default for ConnectionCache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<T> ConnectionCache<T> {
    pub(super) async fn get_or_try_init<E, F, Fut>(
        &self,
        key: [u8; 32],
        expires_at: SystemTime,
        create: F,
    ) -> Result<Arc<T>, E>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, E>>,
    {
        let now = Instant::now();
        let wall_now = SystemTime::now();
        if expires_at <= wall_now + EXPIRY_MARGIN {
            return create().await.map(Arc::new);
        }
        let entry = {
            let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
            entries.retain(|_, (entry, _)| entry.reusable(now, wall_now));
            if let Some((entry, last_used)) = entries.get_mut(&key) {
                *last_used = now;
                entry.clone()
            } else {
                if entries.len() >= MAX_CONNECTIONS
                    && let Some(oldest) = entries
                        .iter()
                        .min_by_key(|(_, (_, last_used))| *last_used)
                        .map(|(key, _)| *key)
                {
                    entries.remove(&oldest);
                }
                let entry = Arc::new(Entry {
                    value: OnceCell::new(),
                    created_at: now,
                    expires_at,
                });
                entries.insert(key, (entry.clone(), now));
                entry
            }
        };
        // One initializer per key; unrelated tenants never wait for its I/O.
        entry
            .value
            .get_or_try_init(|| async { create().await.map(Arc::new) })
            .await
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn expiry() -> SystemTime {
        SystemTime::now() + Duration::from_secs(3600)
    }

    #[tokio::test]
    async fn concurrent_runs_reuse_one_initialization() {
        let cache = Arc::new(ConnectionCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let cache = cache.clone();
            let calls = calls.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_try_init([1; 32], expiry(), || async {
                        calls.fetch_add(1, Ordering::SeqCst);
                        tokio::task::yield_now().await;
                        Ok::<_, ()>(42)
                    })
                    .await
                    .unwrap()
            }));
        }
        let first = tasks.remove(0).await.unwrap();
        for task in tasks {
            assert!(Arc::ptr_eq(&first, &task.await.unwrap()));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn another_authority_does_not_reuse_the_connection() {
        let cache = ConnectionCache::default();
        let first = cache
            .get_or_try_init([1; 32], expiry(), || async { Ok::<_, ()>("first") })
            .await
            .unwrap();
        let second = cache
            .get_or_try_init([2; 32], expiry(), || async { Ok::<_, ()>("second") })
            .await
            .unwrap();
        assert_eq!(*first, "first");
        assert_eq!(*second, "second");
        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn expiry_and_ttl_evict_retained_connections() {
        for credential_expired in [true, false] {
            let cache = ConnectionCache::default();
            cache
                .get_or_try_init([1; 32], expiry(), || async { Ok::<_, ()>(1) })
                .await
                .unwrap();
            {
                let mut entries = cache.entries.lock().unwrap();
                let entry = Arc::get_mut(&mut entries.get_mut(&[1; 32]).unwrap().0).unwrap();
                if credential_expired {
                    entry.expires_at = SystemTime::now();
                } else {
                    entry.created_at = Instant::now() - CONNECTION_TTL;
                }
            }
            let replacement = cache
                .get_or_try_init([1; 32], expiry(), || async { Ok::<_, ()>(2) })
                .await
                .unwrap();
            assert_eq!(*replacement, 2);
        }
    }

    #[tokio::test]
    async fn near_expiry_connections_are_not_retained() {
        let cache = ConnectionCache::default();
        for value in 0..2 {
            let connection = cache
                .get_or_try_init([1; 32], SystemTime::now() + EXPIRY_MARGIN, || async {
                    Ok::<_, ()>(value)
                })
                .await
                .unwrap();
            assert_eq!(*connection, value);
        }
        assert!(cache.entries.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn failed_initialization_can_be_retried() {
        let cache = ConnectionCache::default();
        assert!(
            cache
                .get_or_try_init([1; 32], expiry(), || async {
                    Err::<usize, _>("unavailable")
                })
                .await
                .is_err()
        );
        assert_eq!(
            *cache
                .get_or_try_init([1; 32], expiry(), || async { Ok::<_, &str>(1) })
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn capacity_evicts_the_least_recently_used_connection() {
        let cache = ConnectionCache::default();
        for key in 0..MAX_CONNECTIONS as u8 {
            cache
                .get_or_try_init([key; 32], expiry(), || async { Ok::<_, ()>(key) })
                .await
                .unwrap();
        }
        // Make the order deterministic even on a coarse monotonic clock.
        cache.entries.lock().unwrap().get_mut(&[0; 32]).unwrap().1 -= Duration::from_secs(1);
        cache
            .get_or_try_init([MAX_CONNECTIONS as u8; 32], expiry(), || async {
                Ok::<_, ()>(99)
            })
            .await
            .unwrap();
        let entries = cache.entries.lock().unwrap();
        assert_eq!(entries.len(), MAX_CONNECTIONS);
        assert!(!entries.contains_key(&[0; 32]));
    }
}
