use std::path::{Path, PathBuf};
use tokio::sync::{Mutex, MutexGuard};

/// Serializes initialization and remembers which cache finished loading its catalog.
#[derive(Default)]
pub(crate) struct RegistryInitialization(Mutex<Option<PathBuf>>);

pub(crate) struct InitializationGuard<'a>(MutexGuard<'a, Option<PathBuf>>);

impl RegistryInitialization {
    pub(crate) async fn lock(&self) -> InitializationGuard<'_> {
        InitializationGuard(self.0.lock().await)
    }
}

impl InitializationGuard<'_> {
    pub(crate) fn is_ready(&self, cache_dir: &Path) -> bool {
        self.0.as_deref() == Some(cache_dir)
    }

    pub(crate) fn needs_rebuild(&mut self, cache_dir: &Path) -> bool {
        if self.is_ready(cache_dir) {
            return false;
        }
        // Forget the old directory before replacing its client. Cancellation or an error
        // must make the next caller retry, including a caller switching back to that directory.
        self.invalidate();
        true
    }

    pub(crate) fn invalidate(&mut self) {
        *self.0 = None;
    }

    pub(crate) fn complete(mut self, cache_dir: PathBuf) {
        *self.0 = Some(cache_dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use tokio::sync::Notify;

    #[tokio::test]
    async fn concurrent_callers_share_one_successful_initialization() {
        let gate = Arc::new(RegistryInitialization::default());
        let builds = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..16 {
            let gate = gate.clone();
            let builds = builds.clone();
            tasks.push(tokio::spawn(async move {
                let mut initialization = gate.lock().await;
                let cache = PathBuf::from("cache");
                if initialization.needs_rebuild(&cache) {
                    builds.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    initialization.complete(cache);
                }
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        assert_eq!(builds.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failed_initialization_can_retry() {
        let gate = RegistryInitialization::default();
        let cache = Path::new("cache");
        let attempt: Result<(), ()> = async {
            let mut initialization = gate.lock().await;
            assert!(initialization.needs_rebuild(cache));
            Err(())
        }
        .await;
        assert!(attempt.is_err());
        let mut retry = gate.lock().await;
        assert!(retry.needs_rebuild(cache));
        retry.complete(cache.to_owned());
        assert!(!gate.lock().await.needs_rebuild(cache));
    }

    #[tokio::test]
    async fn changing_cache_rebuilds_and_cancellation_does_not_restore_stale_readiness() {
        let gate = Arc::new(RegistryInitialization::default());
        let old = PathBuf::from("old");
        gate.lock().await.complete(old.clone());
        let entered = Arc::new(Notify::new());
        let changing = {
            let gate = gate.clone();
            let entered = entered.clone();
            tokio::spawn(async move {
                let mut initialization = gate.lock().await;
                assert!(initialization.needs_rebuild(Path::new("new")));
                entered.notify_one();
                std::future::pending::<()>().await;
            })
        };
        entered.notified().await;
        changing.abort();
        assert!(changing.await.unwrap_err().is_cancelled());

        let mut retry_old = gate.lock().await;
        assert!(retry_old.needs_rebuild(&old));
        retry_old.complete(old);
        let mut change = gate.lock().await;
        let new = PathBuf::from("new");
        assert!(change.needs_rebuild(&new));
        change.complete(new.clone());
        assert!(!gate.lock().await.needs_rebuild(&new));
    }
}
