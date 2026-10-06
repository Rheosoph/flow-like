use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use flow_like_types::{Result, tokio::sync::OnceCell};
use parking_lot::Mutex;

pub(crate) const MODEL_IDLE_TTL: Duration = Duration::from_secs(300);

struct Entry<T: ?Sized> {
    value: OnceCell<Arc<T>>,
    last_used: Mutex<Instant>,
}

impl<T: ?Sized> Entry<T> {
    fn touch(&self, now: Instant) {
        *self.last_used.lock() = now;
    }

    fn in_use(self: &Arc<Self>) -> bool {
        Arc::strong_count(self) > 1 || self.value.get().is_some_and(|v| Arc::strong_count(v) > 1)
    }
}

/// Model cache shared by every run. Builds of one key coalesce, builds of different keys run in
/// parallel, and no lock is held while a build awaits.
pub(crate) struct FactoryCache<T: ?Sized> {
    entries: Mutex<HashMap<String, Arc<Entry<T>>>>,
}

impl<T: ?Sized> Default for FactoryCache<T> {
    fn default() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }
}

impl<T: ?Sized + Send + Sync> FactoryCache<T> {
    pub async fn get_or_build<F, Fut>(&self, key: &str, build: F) -> Result<Arc<T>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Arc<T>>>,
    {
        let entry = self.entry(key);
        match entry.value.get_or_try_init(build).await {
            Ok(value) => {
                entry.touch(Instant::now());
                Ok(value.clone())
            }
            Err(error) => {
                self.forget_failed(key, &entry);
                Err(error)
            }
        }
    }

    /// Hosts that never run GC must not keep an entry per failed key; a caller still waiting on
    /// the entry retries the build itself.
    fn forget_failed(&self, key: &str, entry: &Arc<Entry<T>>) {
        let mut entries = self.entries.lock();
        let unused = entries.get(key).is_some_and(|current| {
            Arc::ptr_eq(current, entry)
                && Arc::strong_count(entry) == 2
                && !entry.value.initialized()
        });
        if unused {
            entries.remove(key);
        }
    }

    fn entry(&self, key: &str) -> Arc<Entry<T>> {
        let now = Instant::now();
        let mut entries = self.entries.lock();
        let entry = entries.entry(key.to_string()).or_insert_with(|| {
            Arc::new(Entry {
                value: OnceCell::new(),
                last_used: Mutex::new(now),
            })
        });
        entry.touch(now);
        entry.clone()
    }

    pub fn remove(&self, key: &str) {
        let removed = self.entries.lock().remove(key);
        drop(removed);
    }

    #[cfg(test)]
    pub fn contains(&self, key: &str) -> bool {
        self.entries
            .lock()
            .get(key)
            .is_some_and(|entry| entry.value.initialized())
    }

    /// Evicts values idle for longer than `ttl`. A value referenced outside the cache, or a build
    /// in flight, is in use: it stays and its idle time starts again. Evicted values drop after
    /// the map is unlocked, because dropping a local model stops its server.
    pub fn gc(&self, now: Instant, ttl: Duration) {
        let mut evicted = Vec::new();
        self.entries.lock().retain(|_, entry| {
            if entry.in_use() {
                entry.touch(now);
                return true;
            }
            let idle = now.saturating_duration_since(*entry.last_used.lock());
            if entry.value.initialized() && idle <= ttl {
                return true;
            }
            evicted.push(entry.clone());
            false
        });
        drop(evicted);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio::{
        self,
        sync::{Barrier, Notify},
        time::timeout,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn different_keys_build_in_parallel() {
        let cache = FactoryCache::<String>::default();
        let barrier = Barrier::new(2);
        let build = |name: &'static str| {
            let barrier = &barrier;
            async move {
                barrier.wait().await;
                Ok(Arc::new(name.to_string()))
            }
        };

        let both = async {
            tokio::join!(
                cache.get_or_build("first", || build("first")),
                cache.get_or_build("second", || build("second")),
            )
        };
        let (first, second) = timeout(Duration::from_secs(5), both)
            .await
            .expect("a build of one key must not wait for a build of another");

        assert_eq!(*first.unwrap(), "first");
        assert_eq!(*second.unwrap(), "second");
    }

    #[tokio::test]
    async fn concurrent_builds_of_one_key_coalesce() {
        let cache = FactoryCache::<String>::default();
        let builds = AtomicUsize::new(0);
        let release = Notify::new();
        let build = || async {
            builds.fetch_add(1, Ordering::SeqCst);
            release.notified().await;
            Ok(Arc::new("model".to_string()))
        };

        let first = cache.get_or_build("bit", build);
        let second = cache.get_or_build("bit", build);
        let unblock = async {
            while builds.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
            release.notify_waiters();
        };
        let (first, second, ()) = timeout(Duration::from_secs(5), async {
            tokio::join!(first, second, unblock)
        })
        .await
        .expect("coalesced builds finish");

        assert!(Arc::ptr_eq(&first.unwrap(), &second.unwrap()));
        assert_eq!(builds.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn a_failed_build_is_retried_by_the_next_caller() {
        let cache = FactoryCache::<String>::default();
        let failed = cache
            .get_or_build("bit", || async {
                Err(flow_like_types::anyhow!("no server"))
            })
            .await;
        assert!(failed.is_err());
        assert!(
            cache.entries.lock().is_empty(),
            "a failed key leaves no entry"
        );

        let built = cache
            .get_or_build("bit", || async { Ok(Arc::new("model".to_string())) })
            .await
            .unwrap();
        assert_eq!(*built, "model");
        assert!(cache.contains("bit"));
    }

    #[tokio::test]
    async fn gc_keeps_a_referenced_value_and_evicts_it_a_ttl_after_release() {
        let cache = FactoryCache::<String>::default();
        let held = cache
            .get_or_build("bit", || async { Ok(Arc::new("model".to_string())) })
            .await
            .unwrap();
        let start = Instant::now();

        cache.gc(start + MODEL_IDLE_TTL * 3, MODEL_IDLE_TTL);
        assert!(cache.contains("bit"), "a held model must survive GC");

        drop(held);
        let released = start + MODEL_IDLE_TTL * 3 + Duration::from_secs(1);
        cache.gc(released, MODEL_IDLE_TTL);
        assert!(cache.contains("bit"), "release restarts the idle time");

        cache.gc(
            released + MODEL_IDLE_TTL + Duration::from_secs(1),
            MODEL_IDLE_TTL,
        );
        assert!(!cache.contains("bit"));
    }

    #[tokio::test]
    async fn gc_evicts_an_unreferenced_value_only_after_the_ttl() {
        let cache = FactoryCache::<String>::default();
        drop(
            cache
                .get_or_build("bit", || async { Ok(Arc::new("model".to_string())) })
                .await
                .unwrap(),
        );
        let start = Instant::now();

        cache.gc(
            start + MODEL_IDLE_TTL - Duration::from_secs(1),
            MODEL_IDLE_TTL,
        );
        assert!(cache.contains("bit"));

        cache.gc(
            start + MODEL_IDLE_TTL + Duration::from_secs(1),
            MODEL_IDLE_TTL,
        );
        assert!(!cache.contains("bit"));
    }
}
