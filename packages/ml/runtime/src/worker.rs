//! Synchronous native training worker boundary. Hosts run it on a worker thread/process.
use crate::*;
use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

/// The lock survives lease expiry and is released by the OS when its process exits.
pub(crate) struct LocalRunLock {
    _file: File,
}
impl LocalRunLock {
    pub(crate) fn acquire(repository: &TrainingRepository, key: &str) -> Result<Self> {
        repository.writable()?;
        let database = repository.path().canonicalize()?;
        let directory = database
            .parent()
            .ok_or_else(|| invalid("training repository has no parent directory"))?
            .join(".training-locks");
        std::fs::create_dir_all(&directory)?;
        let mut identity = database.as_os_str().as_encoded_bytes().to_vec();
        identity.push(0);
        identity.extend_from_slice(key.as_bytes());
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.join(format!("{}.lock", digest(&identity))))?;
        file.try_lock()
            .map_err(|error| Error::Conflict(format!("training operation is locked: {error}")))?;
        Ok(Self { _file: file })
    }
}

/// Reconcile an expired lease only after confirming no worker still holds the job lock.
/// An interrupted cancellation stays cancelled; an interrupted run remains resumable.
pub fn reconcile_expired_job(repository: &TrainingRepository, job_id: &str) -> Result<TrainingJob> {
    let _lock = LocalRunLock::acquire(repository, &format!("job:{job_id}"))?;
    reconcile_locked_job(repository, job_id)
}

fn reconcile_locked_job(repository: &TrainingRepository, job_id: &str) -> Result<TrainingJob> {
    repository.reconcile_stopped_job(job_id, now_ms())
}

#[derive(Clone, Debug)]
pub struct TrainingWork {
    pub job: TrainingJob,
    pub snapshot: DatasetSnapshot,
    pub resume_checkpoint: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct EngineOutput {
    pub model_bytes: Vec<u8>,
    pub manifest: Value,
}

/// Implementations perform native training and call check_cancel between batches. Hosts
/// needing a hard deadline should isolate the worker in a supervised child process.
pub trait TrainingEngine: Send + Sync {
    fn estimated_memory_bytes(&self, work: &TrainingWork) -> Result<u64>;
    fn resource_key(&self, work: &TrainingWork) -> Result<String> {
        Ok(format!(
            "{}:{}",
            work.job.request.compute["backend"]
                .as_str()
                .unwrap_or("cpu"),
            work.job.request.compute["device_index"]
                .as_u64()
                .unwrap_or(0)
        ))
    }
    fn train(&self, work: &TrainingWork, control: &WorkerControl) -> Result<EngineOutput>;
}

pub struct WorkerControl {
    repository: TrainingRepository,
    lease: JobLease,
    limits: WorkerLimits,
    started: Instant,
    lease_lost: Arc<AtomicBool>,
}
impl WorkerControl {
    pub fn limits(&self) -> &WorkerLimits {
        &self.limits
    }

    /// A stopped engine may persist the completed batch after cancellation was requested.
    /// The lease still has to be current; this never changes cancellation into success.
    pub fn checkpoint_after_stop(
        &self,
        bytes: &[u8],
        step: u64,
        metadata: Value,
    ) -> Result<CheckpointRef> {
        if bytes.len() as u64 > self.limits.maximum_checkpoint_bytes {
            return Err(invalid("checkpoint exceeds worker size budget"));
        }
        self.repository
            .publish_checkpoint(&self.lease, bytes, step, metadata, false, now_ms())
    }
    pub fn check_cancel(&self) -> Result<()> {
        if self.lease_lost.load(Ordering::Acquire) {
            return Err(Error::LeaseLost);
        }
        let job = self.repository.get_job(&self.lease.job_id)?;
        if job.generation != self.lease.generation
            || job.lease_owner.as_deref() != Some(&self.lease.owner)
            || job
                .lease_expires_at_ms
                .is_none_or(|deadline| deadline <= now_ms())
        {
            return Err(Error::LeaseLost);
        }
        if job.status == JobStatus::CancelRequested {
            return Err(Error::Cancelled);
        }
        if job.status != JobStatus::Running {
            return Err(Error::LeaseLost);
        }
        if job
            .request
            .recipe
            .get("experiment_deadline_ms")
            .and_then(Value::as_i64)
            .is_some_and(|deadline| now_ms() > deadline)
        {
            return Err(Error::Engine(
                "experiment wall time budget exhausted".into(),
            ));
        }
        if self.started.elapsed().as_millis() > self.limits.maximum_duration_ms as u128 {
            return Err(Error::Engine("training duration budget exhausted".into()));
        }
        Ok(())
    }
    pub fn progress(&self, progress: Value) -> Result<()> {
        self.check_cancel()?;
        self.repository.heartbeat(
            &self.lease,
            self.limits.lease_duration_ms,
            progress,
            now_ms(),
        )
    }
    pub fn checkpoint(&self, bytes: &[u8], step: u64, metadata: Value) -> Result<CheckpointRef> {
        self.check_cancel()?;
        if bytes.len() as u64 > self.limits.maximum_checkpoint_bytes {
            return Err(invalid("checkpoint exceeds worker size budget"));
        }
        self.repository
            .publish_checkpoint(&self.lease, bytes, step, metadata, false, now_ms())
    }
}

/// One invocation trains one durable job. The hosting scheduler controls concurrency and
/// device admission; returning from the triggering workflow does not cancel this worker.
pub struct TrainingWorker {
    repository: TrainingRepository,
    limits: WorkerLimits,
    engines: HashMap<String, Arc<dyn TrainingEngine>>,
}
impl TrainingWorker {
    pub fn limits(&self) -> &WorkerLimits {
        &self.limits
    }

    pub fn new(repository: TrainingRepository, limits: WorkerLimits) -> Result<Self> {
        if limits.memory_budget_bytes == 0
            || limits.maximum_artifact_bytes == 0
            || limits.maximum_checkpoint_bytes == 0
            || limits.maximum_duration_ms == 0
            || limits.lease_duration_ms < 300
        {
            return Err(invalid(
                "worker budgets must be positive; lease must be at least 300ms",
            ));
        }
        Ok(Self {
            repository,
            limits,
            engines: HashMap::new(),
        })
    }
    pub fn register(
        &mut self,
        name: impl Into<String>,
        engine: Arc<dyn TrainingEngine>,
    ) -> Result<()> {
        let name = name.into();
        if name.trim().is_empty() || self.engines.contains_key(&name) {
            return Err(invalid("engine name is empty or already registered"));
        }
        self.engines.insert(name, engine);
        Ok(())
    }
    pub fn run(&self, job_id: &str) -> Result<ModelArtifact> {
        let _lock = LocalRunLock::acquire(&self.repository, &format!("job:{job_id}"))?;
        let mut job = reconcile_locked_job(&self.repository, job_id)?;
        if matches!(job.status, JobStatus::Paused | JobStatus::Interrupted) {
            job = self.repository.resume_stopped_job(job_id, now_ms())?;
        }
        if matches!(
            job.status,
            JobStatus::Cancelled | JobStatus::CancelRequested
        ) {
            return Err(Error::Cancelled);
        }
        if job.status != JobStatus::Queued {
            return Err(Error::Conflict(
                "training job is not queued or stopped".into(),
            ));
        }
        let engine = self.engines.get(&job.request.engine).ok_or_else(|| {
            Error::Engine(format!("engine {} is not registered", job.request.engine))
        })?;
        let requested = job
            .request
            .compute
            .get("memory_limit_bytes")
            .and_then(Value::as_u64)
            .unwrap_or(self.limits.memory_budget_bytes);
        let budget = requested.min(self.limits.memory_budget_bytes);
        let checkpoint_bytes = job
            .checkpoint
            .as_ref()
            .map_or(0, |checkpoint| checkpoint.blob.bytes);
        let snapshot_budget = budget
            .checked_sub(checkpoint_bytes)
            .ok_or_else(|| invalid("resume checkpoint exceeds the worker memory budget"))?;
        let snapshot = self
            .repository
            .get_snapshot_limited(&job.snapshot_id, snapshot_budget)?;
        let resume_checkpoint = job
            .checkpoint
            .as_ref()
            .map(|checkpoint| {
                if checkpoint.request_digest != digest(&serde_json::to_vec(&job.request)?) {
                    return Err(invalid("checkpoint recipe/compute contract changed"));
                }
                self.repository.read_blob_limited(
                    &checkpoint.blob,
                    self.limits.maximum_checkpoint_bytes.min(budget),
                )
            })
            .transpose()?;
        let work = TrainingWork {
            snapshot,
            job,
            resume_checkpoint,
        };
        let memory = engine.estimated_memory_bytes(&work)?;
        if memory > budget {
            return Err(Error::Engine(format!(
                "training needs an estimated {memory} bytes; budget is {budget}"
            )));
        }
        let lease = self.repository.claim_job_with_resources(
            job_id,
            &format!("{}:{}", std::process::id(), id()),
            self.limits.lease_duration_ms,
            now_ms(),
            Some(ResourceRequest {
                key: engine.resource_key(&work)?,
                estimated_bytes: memory,
                budget_bytes: self.limits.memory_budget_bytes,
            }),
        )?;
        let lost = Arc::new(AtomicBool::new(false));
        let control = WorkerControl {
            repository: self.repository.clone(),
            lease: lease.clone(),
            limits: self.limits.clone(),
            started: Instant::now(),
            lease_lost: lost.clone(),
        };
        let heartbeat = Heartbeat::start(
            self.repository.clone(),
            lease.clone(),
            self.limits.lease_duration_ms,
            lost,
        );
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut output = engine.train(&work, &control)?;
            control.check_cancel()?;
            for key in ["preprocessing_manifest", "inspection_spec"] {
                if let Some(value) = work.job.request.recipe.get(key) {
                    let manifest = output
                        .manifest
                        .as_object_mut()
                        .ok_or_else(|| invalid("model manifest must be an object"))?;
                    manifest.insert(key.into(), value.clone());
                }
            }
            if output.model_bytes.len() as u64 > self.limits.maximum_artifact_bytes {
                return Err(invalid("model exceeds worker artifact budget"));
            }
            self.repository
                .finish_training(&lease, &output.model_bytes, output.manifest, now_ms())
        }))
        .unwrap_or_else(|_| Err(Error::Engine("training engine panicked".into())));
        // Stop the heartbeat before settling so it cannot renew a terminal job.
        drop(heartbeat);
        if let Err(error) = &result {
            if matches!(error, Error::Cancelled) {
                let _ = self.repository.acknowledge_cancel(&lease, now_ms());
            } else {
                let _ = self
                    .repository
                    .fail_job(&lease, error.to_string(), now_ms());
            }
        }
        result
    }
}

struct Heartbeat {
    stop: Arc<(Mutex<bool>, Condvar)>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Heartbeat {
    fn start(
        repository: TrainingRepository,
        lease: JobLease,
        duration: i64,
        lost: Arc<AtomicBool>,
    ) -> Self {
        let stop = Arc::new((Mutex::new(false), Condvar::new()));
        let signal = stop.clone();
        let thread = std::thread::spawn(move || {
            let interval = Duration::from_millis((duration / 3) as u64);
            loop {
                let guard = signal.0.lock().unwrap_or_else(|e| e.into_inner());
                let (guard, _) = signal
                    .1
                    .wait_timeout_while(guard, interval, |stop| !*stop)
                    .unwrap_or_else(|e| e.into_inner());
                if *guard {
                    break;
                }
                drop(guard);
                let result = repository
                    .get_job(&lease.job_id)
                    .and_then(|job| repository.heartbeat(&lease, duration, job.progress, now_ms()));
                if result.is_err() {
                    lost.store(true, Ordering::Release);
                    break;
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Heartbeat {
    fn drop(&mut self) {
        *self.stop.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
        self.stop.1.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod lock_tests {
    use super::*;
    use serde_json::json;

    struct FixtureEngine;
    impl TrainingEngine for FixtureEngine {
        fn estimated_memory_bytes(&self, _: &TrainingWork) -> Result<u64> {
            Ok(1024)
        }
        fn train(&self, _: &TrainingWork, control: &WorkerControl) -> Result<EngineOutput> {
            control.check_cancel()?;
            Ok(EngineOutput {
                model_bytes: b"fixture".to_vec(),
                manifest: json!({}),
            })
        }
    }

    fn job(repository: &TrainingRepository) -> TrainingJob {
        let stream = StreamKey {
            project_id: "test".into(),
            stream_id: "locks".into(),
            inspection_version: "v1".into(),
        };
        for i in 0..12 {
            repository
                .record_sample(
                    &stream,
                    &TrainingSample {
                        id: format!("sample-{i}"),
                        annotation_revision: 1,
                        group_id: format!("group-{i}"),
                        captured_at_ms: 1,
                        label_available_at_ms: 1,
                        content_digest: format!("{i:064x}"),
                        source: LabelSource::Reviewed,
                        accepted: true,
                        payload: json!({"label":"normal"}),
                    },
                )
                .unwrap();
        }
        repository
            .trigger_after_n(
                &stream,
                12,
                TrainingRequest {
                    engine: "fixture".into(),
                    recipe: json!({}),
                    compute: json!({}),
                },
                SplitPolicy::Group {
                    train_fraction: 0.5,
                    validation_fraction: 0.25,
                    seed: 42,
                },
                now_ms(),
            )
            .unwrap()
            .unwrap()
    }

    #[test]
    fn active_os_lock_prevents_expired_worker_recovery_and_releasing_resources() {
        let directory = tempfile::tempdir().unwrap();
        let repository =
            TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
        let job = job(&repository);
        let lease = repository
            .claim_job_with_resources(
                &job.id,
                "old-worker",
                1000,
                now_ms() - 2000,
                Some(ResourceRequest {
                    key: "cpu:0".into(),
                    estimated_bytes: 1024,
                    budget_bytes: 1024,
                }),
            )
            .unwrap();
        let lock = LocalRunLock::acquire(&repository, &format!("job:{}", job.id)).unwrap();
        let mut worker = TrainingWorker::new(repository.clone(), WorkerLimits::default()).unwrap();
        worker.register("fixture", Arc::new(FixtureEngine)).unwrap();
        assert!(matches!(worker.run(&job.id), Err(Error::Conflict(_))));
        assert!(matches!(
            reconcile_expired_job(&repository, &job.id),
            Err(Error::Conflict(_))
        ));
        let unchanged = repository.get_job(&job.id).unwrap();
        assert_eq!(unchanged.status, JobStatus::Running);
        assert_eq!(unchanged.generation, lease.generation);
        let reserved: u64 = repository
            .connection()
            .unwrap()
            .query_row(
                "SELECT bytes FROM resource_reservations WHERE job_id=?",
                [&job.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reserved, 1024);
        drop(lock);
        let reopened = TrainingRepository::open(repository.path()).unwrap();
        let mut recovered = TrainingWorker::new(reopened.clone(), WorkerLimits::default()).unwrap();
        recovered
            .register("fixture", Arc::new(FixtureEngine))
            .unwrap();
        let artifact = recovered.run(&job.id).unwrap();
        let completed = reopened.get_job(&job.id).unwrap();
        assert_eq!(completed.status, JobStatus::Succeeded);
        assert_eq!(completed.artifact_id.as_deref(), Some(artifact.id.as_str()));
        assert!(completed.generation > lease.generation);
        assert!(
            repository
                .heartbeat(&lease, 1000, json!({}), now_ms())
                .is_err()
        );
    }

    #[test]
    fn expired_cancelled_worker_reconciliation_never_restarts_training() {
        let directory = tempfile::tempdir().unwrap();
        let repository =
            TrainingRepository::open(directory.path().join("training.sqlite")).unwrap();
        let job = job(&repository);
        repository
            .claim_job(&job.id, "old-worker", 1000, now_ms() - 2000)
            .unwrap();
        repository.request_cancel(&job.id, now_ms()).unwrap();
        let reconciled = reconcile_expired_job(&repository, &job.id).unwrap();
        assert_eq!(reconciled.status, JobStatus::Cancelled);
        let mut worker = TrainingWorker::new(repository.clone(), WorkerLimits::default()).unwrap();
        worker.register("fixture", Arc::new(FixtureEngine)).unwrap();
        assert!(matches!(worker.run(&job.id), Err(Error::Cancelled)));
        assert_eq!(
            repository.get_job(&job.id).unwrap().generation,
            reconciled.generation
        );
    }
}
