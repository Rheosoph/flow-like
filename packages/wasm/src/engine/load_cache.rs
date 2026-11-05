use crate::error::{WasmError, WasmResult};
use dashmap::DashMap;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::watch;

// Only compilation and export validation errors cross this boundary. Keep their
// variants so callers can still distinguish an invalid package from a compiler error.
#[derive(Clone)]
enum LoadError {
    Compilation(String),
    MissingExport(String),
    InvalidExportSignature {
        export_name: String,
        expected: String,
        actual: String,
    },
    Internal(String),
}

impl From<WasmError> for LoadError {
    fn from(error: WasmError) -> Self {
        match error {
            WasmError::Compilation { message, .. } => Self::Compilation(message),
            WasmError::MissingExport { export_name } => Self::MissingExport(export_name),
            WasmError::InvalidExportSignature {
                export_name,
                expected,
                actual,
            } => Self::InvalidExportSignature {
                export_name,
                expected,
                actual,
            },
            WasmError::Internal(message) => Self::Internal(message),
            error => Self::Internal(error.to_string()),
        }
    }
}

impl From<LoadError> for WasmError {
    fn from(error: LoadError) -> Self {
        match error {
            LoadError::Compilation(message) => Self::compilation(message),
            LoadError::MissingExport(export_name) => Self::MissingExport { export_name },
            LoadError::InvalidExportSignature {
                export_name,
                expected,
                actual,
            } => Self::InvalidExportSignature {
                export_name,
                expected,
                actual,
            },
            LoadError::Internal(message) => Self::Internal(message),
        }
    }
}

type PendingLoad<T> = watch::Receiver<Option<Result<Arc<T>, LoadError>>>;

pub(super) struct LoadCache<T> {
    pub(super) ready: DashMap<String, Arc<T>>,
    pending: Mutex<HashMap<String, PendingLoad<T>>>,
}

impl<T: Send + Sync + 'static> LoadCache<T> {
    pub(super) fn new() -> Self {
        Self {
            ready: DashMap::new(),
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub(super) async fn load<L>(
        self: &Arc<Self>,
        hash: String,
        make_loader: impl FnOnce() -> L,
    ) -> WasmResult<Arc<T>>
    where
        L: FnOnce() -> WasmResult<T> + Send + 'static,
    {
        let mut receiver = {
            let mut pending = self.pending.lock();
            if let Some(cached) = self.ready.get(&hash) {
                return Ok(cached.clone());
            }
            if let Some(receiver) = pending.get(&hash) {
                drop(make_loader);
                receiver.clone()
            } else {
                // Copy the module bytes only for the caller that starts work.
                let loader = make_loader();
                let (sender, receiver) = watch::channel(None);
                pending.insert(hash.clone(), receiver.clone());
                let cache = self.clone();
                let hash = hash.clone();
                // The task owns publication. Cancelling a caller must not abandon
                // a running compiler or allow another caller to start it again.
                tokio::spawn(async move {
                    let result = match tokio::task::spawn_blocking(loader).await {
                        Ok(result) => result.map(Arc::new).map_err(LoadError::from),
                        Err(error) => Err(LoadError::Internal(format!(
                            "WASM loading task failed: {error}"
                        ))),
                    };
                    {
                        let mut pending = cache.pending.lock();
                        if let Ok(value) = &result {
                            cache.ready.insert(hash.clone(), value.clone());
                        }
                        pending.remove(&hash);
                    }
                    sender.send_replace(Some(result));
                });
                receiver
            }
        };

        loop {
            if let Some(result) = receiver.borrow().as_ref() {
                return result.clone().map_err(WasmError::from);
            }
            if receiver.changed().await.is_err() {
                // A runtime shutdown can cancel the publication task. Remove
                // its closed channel so a later runtime can retry this hash.
                let mut pending = self.pending.lock();
                if pending
                    .get(&hash)
                    .is_some_and(|current| current.same_channel(&receiver))
                {
                    pending.remove(&hash);
                }
                return Err(WasmError::Internal("WASM loading task stopped".into()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_loads_share_one_result_and_can_be_evicted() {
        let cache = Arc::new(LoadCache::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let load = || {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(42)
            }
        };
        let (first, second) = tokio::join!(
            cache.load("same".into(), &load),
            cache.load("same".into(), &load),
        );
        let first = first.unwrap();
        assert!(Arc::ptr_eq(&first, &second.unwrap()));
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        cache.ready.remove("same");
        let reloaded = cache.load("same".into(), load).await.unwrap();
        assert!(!Arc::ptr_eq(&first, &reloaded));
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn concurrent_failures_keep_their_type_and_allow_retry() {
        let cache = Arc::new(LoadCache::<usize>::new());
        let calls = Arc::new(AtomicUsize::new(0));
        let load = || {
            let calls = calls.clone();
            move || {
                calls.fetch_add(1, Ordering::SeqCst);
                Err(WasmError::MissingExport {
                    export_name: "run".into(),
                })
            }
        };
        let (first, second) = tokio::join!(
            cache.load("invalid".into(), &load),
            cache.load("invalid".into(), &load),
        );
        for result in [first, second] {
            assert!(matches!(
                result,
                Err(WasmError::MissingExport { export_name }) if export_name == "run"
            ));
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(cache.ready.is_empty());
        assert_eq!(*cache.load("invalid".into(), || || Ok(7)).await.unwrap(), 7);
    }

    #[tokio::test]
    async fn cancelling_a_waiter_keeps_the_load_and_other_hashes_can_progress() {
        let cache = Arc::new(LoadCache::new());
        let (started, started_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let first = tokio::spawn({
            let cache = cache.clone();
            async move {
                cache
                    .load("slow".into(), || {
                        move || {
                            started.send(()).unwrap();
                            release_rx.recv().unwrap();
                            Ok(42)
                        }
                    })
                    .await
            }
        });
        started_rx.await.unwrap();
        first.abort();
        assert!(first.await.unwrap_err().is_cancelled());

        // This test uses a single runtime thread. Both another key and a waiter
        // must make progress while the first blocking loader is still running.
        assert_eq!(*cache.load("other".into(), || || Ok(9)).await.unwrap(), 9);
        let mut second = Box::pin(cache.load("slow".into(), || -> fn() -> WasmResult<i32> {
            panic!("cancelled caller copied bytes for a duplicate load")
        }));
        assert!(futures::poll!(&mut second).is_pending());
        release.send(()).unwrap();
        assert_eq!(*second.await.unwrap(), 42);
        assert_eq!(*cache.ready.get("slow").unwrap().value().as_ref(), 42);
    }

    #[tokio::test]
    async fn a_panicking_loader_does_not_leave_the_hash_pending() {
        let cache = Arc::new(LoadCache::new());
        assert!(matches!(
            cache
                .load("panic".into(), || || panic!("compile failed"))
                .await,
            Err(WasmError::Internal(_))
        ));
        assert_eq!(*cache.load("panic".into(), || || Ok(5)).await.unwrap(), 5);
    }
}
