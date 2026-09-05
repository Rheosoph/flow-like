//! Live resources shared by nodes for the duration of one run.

use flow_like_types::{Result, anyhow, async_trait};
use parking_lot::Mutex;
use std::any::Any;
use std::collections::HashMap;
use std::sync::Arc;

/// A host resource that must stop when its run ends.
#[async_trait]
pub trait RunResource: Any + Send + Sync {
    /// Stop background work and release idle resources without waiting.
    /// This must be idempotent and also work when an execution future is dropped.
    fn abort(&self);

    /// Stop the resource and wait for its owned work to finish.
    async fn shutdown(&self) {
        self.abort();
    }
}

struct ResourceEntry {
    value: Arc<dyn Any + Send + Sync>,
    lifecycle: Arc<dyn RunResource>,
}

#[derive(Default)]
struct ResourceState {
    closed: bool,
    entries: HashMap<String, ResourceEntry>,
}

/// A registry created for one execution, never looked up by a reusable run ID.
/// Closing it permanently rejects new resources, including through retained contexts.
#[derive(Default)]
pub struct RunResources {
    state: Mutex<ResourceState>,
    shutdown_lock: flow_like_types::tokio::sync::Mutex<()>,
}

impl RunResources {
    /// Reuse a resource within this run. The initializer must not block or reenter
    /// this registry; put asynchronous initialization inside the returned resource.
    pub fn get_or_insert_with<T: RunResource>(
        &self,
        key: impl Into<String>,
        initialize: impl FnOnce() -> Arc<T>,
    ) -> Result<Arc<T>> {
        let mut state = self.state.lock();
        if state.closed {
            return Err(anyhow!("Run resources are closed"));
        }
        let key = key.into();
        if let Some(entry) = state.entries.get(&key) {
            return entry
                .value
                .clone()
                .downcast::<T>()
                .map_err(|_| anyhow!("Run resource has a different type: {key}"));
        }
        let value = initialize();
        state.entries.insert(
            key,
            ResourceEntry {
                value: value.clone(),
                lifecycle: value.clone(),
            },
        );
        Ok(value)
    }

    pub fn is_closed(&self) -> bool {
        self.state.lock().closed
    }

    /// Invalidate the registry immediately, including when no async runtime is available.
    /// Retain lifecycle handles so a later shutdown can still join cancelled work.
    pub fn abort(&self) {
        let resources = {
            let mut state = self.state.lock();
            state.closed = true;
            state
                .entries
                .values()
                .map(|entry| entry.lifecycle.clone())
                .collect::<Vec<_>>()
        };
        for resource in resources {
            resource.abort();
        }
    }

    /// Close every resource before returning. Dropping this future falls back to abort.
    pub async fn shutdown(&self) {
        let mut guard = AbortResourcesOnDrop::new(self);
        let _shutdown_lock = self.shutdown_lock.lock().await;
        let resources = {
            let mut state = self.state.lock();
            state.closed = true;
            state
                .entries
                .values()
                .map(|entry| entry.lifecycle.clone())
                .collect::<Vec<_>>()
        };
        futures::future::join_all(resources.iter().map(|resource| resource.shutdown())).await;
        self.state.lock().entries.clear();
        guard.disarm();
    }
}

impl Drop for RunResources {
    fn drop(&mut self) {
        self.abort();
    }
}

/// Contexts and diagnostic snapshots can retain the registry after its run is dropped.
/// Only InternalRun clones retain this owner, so those references cannot extend resource life.
pub(super) struct RunResourceOwner(pub(super) Arc<RunResources>);

impl Drop for RunResourceOwner {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Covers cancellation of a borrowed execution future while its run stays alive.
pub(super) struct AbortResourcesOnDrop<'a> {
    resources: &'a RunResources,
    armed: bool,
}

impl<'a> AbortResourcesOnDrop<'a> {
    pub(super) fn new(resources: &'a RunResources) -> Self {
        Self {
            resources,
            armed: true,
        }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for AbortResourcesOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.resources.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::tokio;
    use flow_like_types::tokio_util::sync::CancellationToken;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[derive(Default)]
    struct ProbeResource {
        aborted: AtomicBool,
        shutdowns: AtomicUsize,
    }

    #[async_trait]
    impl RunResource for ProbeResource {
        fn abort(&self) {
            self.aborted.store(true, Ordering::SeqCst);
        }

        async fn shutdown(&self) {
            self.shutdowns.fetch_add(1, Ordering::SeqCst);
            self.abort();
        }
    }

    #[tokio::test]
    async fn resources_share_within_one_run_and_close_with_retained_handles() {
        let run = Arc::new(RunResources::default());
        let resource = run
            .get_or_insert_with("package", || Arc::new(ProbeResource::default()))
            .unwrap();
        let reused = run
            .get_or_insert_with::<ProbeResource>("package", || panic!("already initialized"))
            .unwrap();
        assert!(Arc::ptr_eq(&resource, &reused));

        let other_run = RunResources::default();
        let independent = other_run
            .get_or_insert_with("package", || Arc::new(ProbeResource::default()))
            .unwrap();
        assert!(!Arc::ptr_eq(&resource, &independent));

        run.shutdown().await;
        run.shutdown().await;
        assert!(resource.aborted.load(Ordering::SeqCst));
        assert_eq!(resource.shutdowns.load(Ordering::SeqCst), 1);
        assert!(!independent.aborted.load(Ordering::SeqCst));
        assert!(
            run.get_or_insert_with::<ProbeResource>("other", || panic!("closed"))
                .is_err()
        );
    }

    #[test]
    fn dropping_registry_aborts_resources_even_with_retained_handles() {
        let run = RunResources::default();
        let resource = run
            .get_or_insert_with("package", || Arc::new(ProbeResource::default()))
            .unwrap();
        drop(run);
        assert!(resource.aborted.load(Ordering::SeqCst));
    }

    struct PendingShutdown {
        aborted: AtomicBool,
        started: tokio::sync::Notify,
    }

    #[async_trait]
    impl RunResource for PendingShutdown {
        fn abort(&self) {
            self.aborted.store(true, Ordering::SeqCst);
        }

        async fn shutdown(&self) {
            self.started.notify_one();
            std::future::pending::<()>().await;
        }
    }

    #[tokio::test]
    async fn interrupted_shutdown_aborts_in_flight_resources() {
        let run = Arc::new(RunResources::default());
        let resource = run
            .get_or_insert_with("package", || {
                Arc::new(PendingShutdown {
                    aborted: AtomicBool::new(false),
                    started: tokio::sync::Notify::new(),
                })
            })
            .unwrap();
        let shutdown = tokio::spawn({
            let run = run.clone();
            async move { run.shutdown().await }
        });
        resource.started.notified().await;
        shutdown.abort();
        let _ = shutdown.await;
        assert!(resource.aborted.load(Ordering::SeqCst));
        assert!(run.is_closed());
    }

    struct BackgroundResource {
        cancellation: CancellationToken,
        shutdown_started: tokio::sync::Notify,
        task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    }

    #[async_trait]
    impl RunResource for BackgroundResource {
        fn abort(&self) {
            self.cancellation.cancel();
        }

        async fn shutdown(&self) {
            self.abort();
            self.shutdown_started.notify_one();
            let mut task = self.task.lock().await;
            if let Some(handle) = task.as_mut() {
                handle.await.unwrap();
                task.take();
            }
        }
    }

    #[tokio::test]
    async fn shutdown_after_abort_waits_for_background_work() {
        let run = Arc::new(RunResources::default());
        let cancellation = CancellationToken::new();
        let finish = Arc::new(tokio::sync::Notify::new());
        let finished = Arc::new(AtomicBool::new(false));
        let resource = run
            .get_or_insert_with("package", || {
                Arc::new(BackgroundResource {
                    cancellation: cancellation.clone(),
                    shutdown_started: tokio::sync::Notify::new(),
                    task: tokio::sync::Mutex::new(Some(tokio::spawn({
                        let finish = finish.clone();
                        let finished = finished.clone();
                        let cancellation = cancellation.clone();
                        async move {
                            cancellation.cancelled().await;
                            finish.notified().await;
                            finished.store(true, Ordering::SeqCst);
                        }
                    }))),
                })
            })
            .unwrap();

        run.abort();
        assert!(cancellation.is_cancelled());
        assert!(run.is_closed());
        let mut shutdown = Box::pin(run.shutdown());
        tokio::select! {
            _ = &mut shutdown => panic!("shutdown returned before background work ended"),
            _ = resource.shutdown_started.notified() => {}
        }
        assert!(!finished.load(Ordering::SeqCst));
        finish.notify_one();
        shutdown.await;
        assert!(finished.load(Ordering::SeqCst));
        assert!(resource.task.lock().await.is_none());
        assert!(run.state.lock().entries.is_empty());
    }
}
