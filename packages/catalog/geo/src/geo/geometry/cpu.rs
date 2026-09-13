//! Bounded CPU execution shared by Geometry nodes and their nested Rayon work.

use flow_like_types::{Result, anyhow};
use std::panic::{AssertUnwindSafe, catch_unwind};

#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

#[cfg(not(target_arch = "wasm32"))]
const PARALLEL_WORK_THRESHOLD: usize = 16_384;

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::*;
    use std::{cell::Cell, sync::OnceLock};
    use tokio::sync::Semaphore;

    thread_local! {
        static IN_GEOMETRY_WORKER: Cell<bool> = const { Cell::new(false) };
    }

    pub(super) struct Workers {
        pub pool: rayon::ThreadPool,
        pub slots: Semaphore,
    }

    pub(super) fn in_worker() -> bool {
        IN_GEOMETRY_WORKER.with(Cell::get)
    }

    pub(super) fn workers() -> Result<&'static Workers> {
        static WORKERS: OnceLock<std::result::Result<Workers, String>> = OnceLock::new();
        WORKERS
            .get_or_init(|| {
                let count = std::thread::available_parallelism()
                    .map(usize::from)
                    .unwrap_or(1)
                    .clamp(1, 8);
                let pool = rayon::ThreadPoolBuilder::new()
                    .num_threads(count)
                    .thread_name(|index| format!("geometry-cpu-{index}"))
                    // Nested collection and overlay traversal needs more than
                    // the small platform default thread stack on some hosts.
                    .stack_size(8 * 1024 * 1024)
                    .start_handler(|_| IN_GEOMETRY_WORKER.with(|flag| flag.set(true)))
                    .build()
                    .map_err(|error| format!("Could not start Geometry CPU workers: {error}"))?;
                Ok(Workers {
                    pool,
                    slots: Semaphore::new(count),
                })
            })
            .as_ref()
            .map_err(|error| anyhow!("{error}"))
    }
}

fn guarded<T>(compute: impl FnOnce() -> Result<T>) -> Result<T> {
    catch_unwind(AssertUnwindSafe(compute)).unwrap_or_else(|panic| {
        let message = panic
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| panic.downcast_ref::<&str>().copied())
            .unwrap_or("unknown panic");
        Err(anyhow!("Geometry computation panicked: {message}"))
    })
}

/// Only synchronous computation crosses this boundary. Pin evaluation and
/// mutation remain on the caller's executor; no Tokio runtime is created or entered.
pub(super) async fn run<T, F>(compute: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T> + Send + 'static,
{
    #[cfg(target_arch = "wasm32")]
    {
        guarded(compute)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // A synchronous nested caller already owns admission to this pool.
        if native::in_worker() {
            return guarded(compute);
        }
        let workers = native::workers()?;
        let permit = workers
            .slots
            .acquire()
            .await
            .map_err(|_| anyhow!("Geometry CPU workers are unavailable"))?;
        let (sender, receiver) = futures::channel::oneshot::channel();
        workers.pool.spawn(move || {
            // Keep admission until computation finishes, even if the caller
            // cancels its future. Unstarted cancelled work can be skipped.
            let _permit = permit;
            if sender.is_canceled() {
                return;
            }
            let result = guarded(compute);
            let _ = sender.send(result);
        });
        receiver
            .await
            .map_err(|_| anyhow!("Geometry CPU worker ended without a result"))?
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parallel(work: usize) -> bool {
    work >= PARALLEL_WORK_THRESHOLD && native::in_worker()
}

pub(super) fn map_indices<T, F>(len: usize, work: usize, compute: F) -> Vec<T>
where
    T: Send,
    F: Fn(usize) -> T + Send + Sync,
{
    #[cfg(not(target_arch = "wasm32"))]
    if len > 1 && parallel(work) {
        return (0..len).into_par_iter().map(compute).collect();
    }
    let _ = work;
    (0..len).map(compute).collect()
}

pub(super) fn map_ordered<T, U, F>(items: &[T], work: usize, compute: F) -> Vec<U>
where
    T: Sync,
    U: Send,
    F: Fn(usize, &T) -> U + Send + Sync,
{
    map_indices(items.len(), work, |index| compute(index, &items[index]))
}

pub(super) fn map_owned<T, U, F>(items: Vec<T>, work: usize, compute: F) -> Vec<U>
where
    T: Send,
    U: Send,
    F: Fn(usize, T) -> U + Send + Sync,
{
    #[cfg(not(target_arch = "wasm32"))]
    if items.len() > 1 && parallel(work) {
        return items
            .into_par_iter()
            .enumerate()
            .map(|(index, item)| compute(index, item))
            .collect();
    }
    let _ = work;
    items
        .into_iter()
        .enumerate()
        .map(|(index, item)| compute(index, item))
        .collect()
}

pub(super) fn any_indexed<F>(len: usize, work: usize, predicate: F) -> bool
where
    F: Fn(usize) -> bool + Send + Sync,
{
    #[cfg(not(target_arch = "wasm32"))]
    if len > 1 && parallel(work) {
        return (0..len).into_par_iter().any(predicate);
    }
    let _ = work;
    (0..len).any(predicate)
}

pub(super) fn find_map_first_ordered<T, U, F>(items: &[T], work: usize, compute: F) -> Option<U>
where
    T: Sync,
    U: Send,
    F: Fn(usize, &T) -> Option<U> + Send + Sync,
{
    #[cfg(not(target_arch = "wasm32"))]
    if items.len() > 1 && parallel(work) {
        return items
            .par_iter()
            .enumerate()
            .map(|(index, item)| compute(index, item))
            .find_first(Option::is_some)
            .flatten();
    }
    let _ = work;
    items
        .iter()
        .enumerate()
        .find_map(|(index, item)| compute(index, item))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use std::{collections::HashSet, sync::Arc, time::Duration};

    #[test]
    fn cpu_execution_does_not_require_a_tokio_runtime() {
        let result = futures::executor::block_on(run(|| {
            assert!(tokio::runtime::Handle::try_current().is_err());
            assert!(native::in_worker());
            Ok(42)
        }));
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn current_thread_executor_stays_responsive_during_cpu_work() {
        let (started, ready) = futures::channel::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let task = tokio::spawn(run(move || {
            let _ = started.send(());
            gate.recv_timeout(Duration::from_secs(5))?;
            Ok(42)
        }));
        ready.await.unwrap();
        tokio::task::yield_now().await;
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap(), 42);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_jobs_and_nested_parallel_work_share_one_bounded_pool() {
        let workers = native::workers().unwrap();
        let ids = Arc::new(std::sync::Mutex::new(HashSet::new()));
        let jobs = (0..32).map(|index| {
            let ids = ids.clone();
            run(move || {
                assert!(tokio::runtime::Handle::try_current().is_err());
                assert!(native::in_worker());
                let values = map_indices(128, PARALLEL_WORK_THRESHOLD, |inner| {
                    assert!(native::in_worker());
                    ids.lock().unwrap().insert(std::thread::current().id());
                    index + inner
                });
                Ok(values)
            })
        });
        for (index, result) in futures::future::join_all(jobs)
            .await
            .into_iter()
            .enumerate()
        {
            assert_eq!(
                result.unwrap(),
                (0..128).map(|i| index + i).collect::<Vec<_>>()
            );
        }
        assert!(ids.lock().unwrap().len() <= workers.pool.current_num_threads());
        assert!(workers.pool.current_num_threads() <= 8);
        let nested_workers = run(|| {
            Ok(rayon::broadcast(|_| {
                assert!(native::in_worker());
                assert!(tokio::runtime::Handle::try_current().is_err());
                std::thread::current().id()
            }))
        })
        .await
        .unwrap();
        assert_eq!(
            nested_workers.into_iter().collect::<HashSet<_>>().len(),
            workers.pool.current_num_threads()
        );
    }

    #[tokio::test]
    async fn errors_and_parallel_panics_do_not_destroy_the_workers() {
        let error = run(|| -> Result<()> { Err(anyhow!("invalid geometry")) })
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "invalid geometry");
        let error = run(|| {
            Ok(map_indices(128, PARALLEL_WORK_THRESHOLD, |index| {
                assert_ne!(index, 17, "geometry test panic");
                index
            }))
        })
        .await
        .unwrap_err();
        assert!(error.to_string().contains("Geometry computation panicked"));
        assert_eq!(run(|| Ok(42)).await.unwrap(), 42);
    }

    #[tokio::test]
    async fn cancellation_retains_admission_until_the_cpu_job_finishes() {
        let (started, ready) = futures::channel::oneshot::channel();
        let (finished, completion) = futures::channel::oneshot::channel();
        let (release, gate) = std::sync::mpsc::channel();
        let task = tokio::spawn(run(move || {
            let _ = started.send(());
            gate.recv_timeout(Duration::from_secs(5))?;
            let _ = finished.send(());
            Ok(())
        }));
        ready.await.unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        // Acquiring every slot must wait for the cancelled CPU job to finish.
        let workers = native::workers().unwrap();
        {
            let all_slots = workers
                .slots
                .acquire_many(workers.pool.current_num_threads() as u32);
            futures::pin_mut!(all_slots);
            assert!(futures::poll!(all_slots).is_pending());
        }
        release.send(()).unwrap();
        completion.await.unwrap();
        assert_eq!(run(|| Ok(42)).await.unwrap(), 42);
    }

    #[test]
    fn nested_cpu_calls_reuse_admission_without_deadlock() {
        let result =
            futures::executor::block_on(run(|| futures::executor::block_on(run(|| Ok(42)))));
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn cancellation_while_waiting_for_admission_never_submits_the_job() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let workers = native::workers().unwrap();
        let permits = workers
            .slots
            .acquire_many(workers.pool.current_num_threads() as u32)
            .await
            .unwrap();
        let ran = Arc::new(AtomicBool::new(false));
        {
            let ran = ran.clone();
            let waiting = run(move || {
                ran.store(true, Ordering::Relaxed);
                Ok(())
            });
            futures::pin_mut!(waiting);
            assert!(futures::poll!(waiting).is_pending());
        }
        drop(permits);
        assert_eq!(run(|| Ok(42)).await.unwrap(), 42);
        assert!(!ran.load(Ordering::Relaxed));
    }
}
