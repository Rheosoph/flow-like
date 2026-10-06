use super::{
    db::JobRecord,
    fetch::{FetchError, Fetcher, file_digest},
    store::{DiskBudgetExceeded, GcReport, ModelStore, Reservation},
};
use crate::enrollment::unix_time;
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::{
    MODEL_ASSET_MAX_SOURCES, ModelAssetDescriptor, ModelAssetDigest, ModelAssetFailure,
    ModelAssetState, ModelAssetStatus, ModelJob,
};
use std::{
    collections::HashMap,
    fs::File,
    future::Future,
    io::{Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex, Weak},
    time::{Duration, Instant},
};
use tokio::sync::{Semaphore, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub const PUSH_CHUNK_MAX_BYTES: usize = 1024 * 1024;
/// Jobs a run, a verification or a push works on; settled jobs beyond it make way.
const MAX_JOBS: usize = 256;
const MAX_OPERATION_IDS: usize = 16;
const OPERATION_ID_MAX_LEN: usize = 128;
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const SETTLED_JOB_RETENTION_SECONDS: i64 = 7 * 24 * 60 * 60;
/// A push session without bytes for this long is abandoned: the job counts as settled, and a
/// request for the asset downloads it again when it has sources.
const PUSH_IDLE: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug)]
pub struct AcquisitionConfig {
    pub concurrent_downloads: usize,
    pub attempts_per_source: u32,
    /// Delay before the second attempt on a source; it doubles per attempt, up to a minute.
    pub retry_backoff: Duration,
    /// How often the store and the settled jobs are collected, starting one period in.
    pub gc_every: Duration,
    /// A released reference triggers a collection this much later, so a burst collects once.
    pub gc_after_release: Duration,
}

impl Default for AcquisitionConfig {
    fn default() -> Self {
        Self {
            concurrent_downloads: 2,
            attempts_per_source: 3,
            retry_backoff: Duration::from_secs(2),
            gc_every: Duration::from_secs(60 * 60),
            gc_after_release: Duration::from_secs(60),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobSnapshot {
    pub job_id: String,
    pub asset: ModelAssetDescriptor,
    /// `Fetching` carries the bytes staged so far.
    pub state: ModelAssetState,
    /// Host of the source being fetched.
    pub source_host: Option<String>,
    pub attempts: u32,
    pub bytes_per_second: Option<u64>,
    pub operation_ids: Vec<String>,
    pub updated_at: i64,
}

impl JobSnapshot {
    /// The job as `ModelsRequest::Jobs` lists it.
    pub fn wire(&self) -> ModelJob {
        ModelJob {
            job_id: self.job_id.clone(),
            digest: self.asset.digest.clone(),
            size: self.asset.size,
            file_name: self.asset.file_name.clone(),
            source_host: self.source_host.clone(),
            bytes_per_second: self.bytes_per_second,
            sources: ModelJob::listed_sources(&self.asset.sources),
            updated_at: self.updated_at,
            state: self.state.clone(),
        }
    }

    /// The asset's state with the job a controller pushes to.
    pub fn status(&self) -> ModelAssetStatus {
        ModelAssetStatus {
            digest: self.asset.digest.clone(),
            job_id: Some(self.job_id.clone()),
            state: self.state.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Failure {
    reason: ModelAssetFailure,
    http_status: Option<u16>,
}

impl Failure {
    fn new(reason: ModelAssetFailure) -> Self {
        Self {
            reason,
            http_status: None,
        }
    }

    fn of(error: &FetchError) -> Self {
        let (reason, http_status) = error.failure();
        Self {
            reason,
            http_status,
        }
    }

    fn of_store(error: &anyhow::Error) -> Self {
        Self::new(if error.downcast_ref::<DiskBudgetExceeded>().is_some() {
            ModelAssetFailure::DiskBudget
        } else {
            ModelAssetFailure::Io
        })
    }

    /// After every source failed, content problems outrank HTTP answers, and network
    /// trouble is reported only when nothing else happened.
    fn severity(self) -> u8 {
        match self.reason {
            ModelAssetFailure::DigestMismatch => 5,
            ModelAssetFailure::SizeMismatch => 4,
            ModelAssetFailure::HttpStatus => 3,
            ModelAssetFailure::Io => 2,
            ModelAssetFailure::EgressBlocked => 1,
            _ => 0,
        }
    }

    fn worse(self, earlier: Option<Self>) -> Self {
        match earlier {
            Some(earlier) if earlier.severity() >= self.severity() => earlier,
            _ => self,
        }
    }

    fn state(self) -> ModelAssetState {
        ModelAssetState::Failed {
            reason: self.reason,
            http_status: self.http_status,
        }
    }
}

#[derive(Default)]
struct Progress {
    bytes: u64,
    since: Option<(Instant, u64)>,
}

impl Progress {
    fn rate(&self) -> Option<u64> {
        let (since, from) = self.since?;
        let elapsed = since.elapsed().as_secs_f64();
        (elapsed >= 1.0).then(|| (self.bytes.saturating_sub(from) as f64 / elapsed) as u64)
    }
}

struct Control {
    asset: ModelAssetDescriptor,
    operation_ids: Vec<String>,
    attempts: u32,
    updated_at: i64,
    /// Bumped whenever a fetch run, push session or cancellation takes the job over;
    /// state written by an older owner is dropped.
    generation: u64,
    run: Option<CancellationToken>,
    reservation: Option<Reservation>,
    progress: Progress,
    /// When a push session last opened or took a chunk; no push survives a restart.
    pushed_at: Option<Instant>,
}

impl Control {
    fn pushing(&self) -> bool {
        self.pushed_at.is_some_and(|at| at.elapsed() < PUSH_IDLE)
    }
}

struct Job {
    id: String,
    created_at: i64,
    state: watch::Sender<ModelAssetState>,
    control: Mutex<Control>,
    /// Exclusive access to the staging file.
    file: tokio::sync::Mutex<()>,
}

impl Job {
    fn restore(record: JobRecord) -> Arc<Self> {
        Arc::new(Self {
            id: record.job_id,
            created_at: record.created_at,
            state: watch::Sender::new(record.state),
            control: Mutex::new(Control {
                asset: record.asset,
                operation_ids: record.operation_ids,
                attempts: record.attempts,
                updated_at: record.updated_at,
                generation: 0,
                run: None,
                reservation: None,
                progress: Progress::default(),
                pushed_at: None,
            }),
            file: tokio::sync::Mutex::new(()),
        })
    }

    fn current(&self) -> ModelAssetState {
        self.state.borrow().clone()
    }

    fn asset(&self) -> ModelAssetDescriptor {
        lock!(self.control).asset.clone()
    }

    fn updated_at(&self) -> i64 {
        lock!(self.control).updated_at
    }

    /// Worked on by a fetch run, a verification or a push that sent bytes lately. Only
    /// these count against `MAX_JOBS`; a failed job or an abandoned push has settled.
    fn live(&self) -> bool {
        match self.current() {
            ModelAssetState::Failed { .. } | ModelAssetState::Present => false,
            ModelAssetState::AwaitingPush { .. } => lock!(self.control).pushing(),
            _ => true,
        }
    }

    fn pushed(&self) {
        lock!(self.control).pushed_at = Some(Instant::now());
    }

    fn journal(&self, store: &ModelStore, control: &Control, state: ModelAssetState) {
        let record = JobRecord {
            job_id: self.id.clone(),
            asset: control.asset.clone(),
            state,
            attempts: control.attempts,
            operation_ids: control.operation_ids.clone(),
            created_at: self.created_at,
            updated_at: control.updated_at,
        };
        if let Err(error) = store.with_db(|db| db.put_job(&record)) {
            tracing::warn!(
                "Journal model asset job {}: {error:#}",
                control.asset.digest.store_key()
            );
        }
    }

    /// Applies and journals `state` unless a newer owner took the job over.
    fn set_state(
        &self,
        store: &ModelStore,
        generation: Option<u64>,
        state: ModelAssetState,
    ) -> bool {
        let mut control = lock!(self.control);
        if generation.is_some_and(|generation| generation != control.generation) {
            return false;
        }
        control.updated_at = unix_time().unwrap_or(control.updated_at);
        self.state.send_replace(state.clone());
        self.journal(store, &control, state);
        true
    }

    /// Progress that a restart recomputes from the staging file is not journaled.
    fn show(&self, generation: u64, state: ModelAssetState) {
        if lock!(self.control).generation == generation {
            self.state.send_replace(state);
        }
    }

    fn persist(&self, store: &ModelStore) {
        let control = lock!(self.control);
        let state = self.current();
        if state != ModelAssetState::Present {
            self.journal(store, &control, state);
        }
    }

    /// Stops the current run and returns the generation of the next owner.
    fn take_over(&self) -> u64 {
        let mut control = lock!(self.control);
        if let Some(run) = control.run.take() {
            run.cancel();
        }
        control.generation += 1;
        control.generation
    }

    fn owns(&self, generation: u64) -> bool {
        lock!(self.control).generation == generation
    }

    fn reserve(&self, store: &ModelStore) -> Result<()> {
        let mut control = lock!(self.control);
        if control.reservation.is_none() {
            let reservation = store.reserve(&control.asset.digest, control.asset.size)?;
            control.reservation = Some(reservation);
        }
        Ok(())
    }

    fn release(&self, generation: u64) {
        let mut control = lock!(self.control);
        if control.generation == generation {
            control.reservation = None;
            control.run = None;
        }
    }

    fn begin_attempt(
        &self,
        store: &ModelStore,
        generation: u64,
        source_index: usize,
        held: u64,
    ) -> bool {
        {
            let mut control = lock!(self.control);
            if control.generation != generation {
                return false;
            }
            control.attempts += 1;
            control.progress = Progress {
                bytes: held,
                since: Some((Instant::now(), held)),
            };
        }
        let source_index = u8::try_from(source_index).unwrap_or(u8::MAX);
        self.set_state(
            store,
            Some(generation),
            ModelAssetState::Fetching {
                source_index,
                bytes: held,
            },
        )
    }

    fn progress(&self, bytes: u64) {
        lock!(self.control).progress.bytes = bytes;
    }

    fn snapshot(&self) -> JobSnapshot {
        let state = self.current();
        let control = lock!(self.control);
        let mut snapshot = JobSnapshot {
            job_id: self.id.clone(),
            asset: control.asset.clone(),
            state: state.clone(),
            source_host: None,
            attempts: control.attempts,
            bytes_per_second: None,
            operation_ids: control.operation_ids.clone(),
            updated_at: control.updated_at,
        };
        if let ModelAssetState::Fetching { source_index, .. } = state {
            snapshot.state = ModelAssetState::Fetching {
                source_index,
                bytes: control.progress.bytes,
            };
            snapshot.source_host = source_host(&control.asset, source_index);
            snapshot.bytes_per_second = control.progress.rate();
        }
        snapshot
    }
}

/// Only a host the protocol accepts, since a client parses a jobs page as a whole.
fn source_host(asset: &ModelAssetDescriptor, index: u8) -> Option<String> {
    ModelJob::listed_source_host(asset.sources.get(usize::from(index))?)
}

fn validate_operation_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty() && id.len() <= OPERATION_ID_MAX_LEN && !id.chars().any(char::is_control),
        "Invalid operation id for a model asset job"
    );
    Ok(())
}

fn merge_sources(sources: &mut Vec<String>, more: &[String]) {
    for source in more {
        if sources.len() >= MODEL_ASSET_MAX_SOURCES {
            return;
        }
        if !sources.contains(source) {
            sources.push(source.clone());
        }
    }
}

/// A job that starts over tries the latest request's sources first: the earlier ones got
/// nowhere, and they must not crowd it out of the source slots.
fn lead_with(sources: &mut Vec<String>, first: &[String]) {
    let earlier = std::mem::take(sources);
    merge_sources(sources, first);
    merge_sources(sources, &earlier);
}

fn remember(ids: &mut Vec<String>, id: Option<&str>) {
    let Some(id) = id else {
        return;
    };
    if ids.iter().any(|known| known == id) {
        return;
    }
    ids.push(id.to_owned());
    if ids.len() > MAX_OPERATION_IDS {
        ids.remove(0);
    }
}

fn store_error(error: &anyhow::Error) -> FetchError {
    FetchError::Local {
        message: format!("{error:#}"),
        storage_full: false,
    }
}

/// Appends at `offset` when it continues the staged bytes. A retried chunk must repeat
/// bytes already received exactly. Returns the staged length.
fn write_chunk(mut file: File, offset: u64, bytes: &[u8]) -> Result<u64> {
    let length = file.metadata()?.len();
    let end = offset + bytes.len() as u64;
    ensure!(
        offset <= length,
        "the chunk at offset {offset} does not continue the {length} bytes received"
    );
    file.seek(SeekFrom::Start(offset))?;
    if offset < length {
        ensure!(
            end <= length,
            "the retried chunk at offset {offset} overlaps bytes not received yet"
        );
        let mut existing = vec![0; bytes.len()];
        file.read_exact(&mut existing)?;
        ensure!(
            existing == bytes,
            "the retried chunk at offset {offset} differs from the bytes received"
        );
        return Ok(length);
    }
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(end)
}

struct Inner {
    store: Arc<ModelStore>,
    fetcher: Fetcher,
    config: AcquisitionConfig,
    downloads: Arc<Semaphore>,
    jobs: Mutex<HashMap<ModelAssetDigest, Arc<Job>>>,
    shutdown: CancellationToken,
    tasks: TaskTracker,
}

impl Inner {
    fn job(&self, digest: &ModelAssetDigest) -> Option<Arc<Job>> {
        lock!(self.jobs).get(digest).cloned()
    }

    /// No push stream survives a restart, so a job awaiting one downloads again when it can.
    fn recover(self: &Arc<Self>, record: JobRecord) -> Result<()> {
        let digest = record.asset.digest.clone();
        if self.store.contains(&digest, record.asset.size)? {
            return self.store.with_db(|db| db.delete_job(&digest));
        }
        let job = Job::restore(record);
        lock!(self.jobs).insert(digest.clone(), Arc::clone(&job));
        match job.current() {
            ModelAssetState::Failed { .. } => {}
            ModelAssetState::AwaitingPush { .. } if job.asset().sources.is_empty() => {
                let bytes = self.store.partial_len(&digest)?;
                job.set_state(&self.store, None, ModelAssetState::AwaitingPush { bytes });
            }
            _ => self.spawn(&job),
        }
        Ok(())
    }

    /// Runs `work` as a task of its own: a caller that goes away, such as a reset tunnel
    /// stream, never stops a take-over, a write or a verification halfway.
    async fn detached<T: Send + 'static>(
        &self,
        work: impl Future<Output = Result<T>> + Send + 'static,
    ) -> Result<T> {
        self.tasks
            .spawn(work)
            .await
            .context("A model asset task stopped before it finished")?
    }

    /// Keeps the map at `MAX_JOBS`: when it is full, the job that settled longest ago goes
    /// with its staged bytes. Jobs that are worked on, or held by a caller, stay.
    fn make_room(&self, jobs: &mut HashMap<ModelAssetDigest, Arc<Job>>, key: &str) -> Result<()> {
        if jobs.len() < MAX_JOBS {
            return Ok(());
        }
        let oldest = jobs
            .iter()
            .filter(|(_, job)| Arc::strong_count(job) == 1 && !job.live())
            .min_by_key(|(_, job)| (job.updated_at(), job.created_at))
            .map(|(digest, _)| digest.clone())
            .with_context(|| {
                format!("Ensure model asset {key}: {MAX_JOBS} jobs are already in progress")
            })?;
        self.drop_job(jobs, &oldest)
    }

    /// Removes a settled job with its journal row and staged bytes.
    fn drop_job(
        &self,
        jobs: &mut HashMap<ModelAssetDigest, Arc<Job>>,
        digest: &ModelAssetDigest,
    ) -> Result<()> {
        self.store.remove_partial(digest)?;
        self.store.with_db(|db| db.delete_job(digest))?;
        jobs.remove(digest);
        Ok(())
    }

    /// Removes `job` from the map and the journal unless another job took its place.
    fn forget(&self, job: &Arc<Job>) -> Result<()> {
        let digest = job.asset().digest;
        let mut jobs = lock!(self.jobs);
        if jobs
            .get(&digest)
            .is_some_and(|current| Arc::ptr_eq(current, job))
        {
            self.store.with_db(|db| db.delete_job(&digest))?;
            jobs.remove(&digest);
        }
        Ok(())
    }

    /// Starts a fetch run that owns the job from now on.
    fn spawn(self: &Arc<Self>, job: &Arc<Job>) {
        let cancel = self.shutdown.child_token();
        let generation = {
            let mut control = lock!(job.control);
            if let Some(previous) = control.run.replace(cancel.clone()) {
                previous.cancel();
            }
            control.generation += 1;
            control.attempts = 0;
            control.generation
        };
        job.set_state(&self.store, Some(generation), ModelAssetState::Queued);
        let (inner, job) = (Arc::clone(self), Arc::clone(job));
        self.tasks
            .spawn(async move { inner.run(job, generation, cancel).await });
    }

    async fn run(self: Arc<Self>, job: Arc<Job>, generation: u64, cancel: CancellationToken) {
        let outcome = self.acquire(&job, generation, &cancel).await;
        if self.shutdown.is_cancelled() {
            return;
        }
        match outcome {
            Ok(()) => self.complete(&job, generation),
            Err(failure) => self.fail(&job, generation, failure),
        }
    }

    async fn acquire(
        &self,
        job: &Job,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        let cancelled = Failure::new(ModelAssetFailure::Cancelled);
        if job.asset().sources.is_empty() {
            return Err(Failure::new(ModelAssetFailure::NoSources));
        }
        let _permit = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(cancelled),
            permit = Arc::clone(&self.downloads).acquire_owned() => permit.map_err(|_| cancelled)?,
        };
        let _file = tokio::select! {
            biased;
            () = cancel.cancelled() => return Err(cancelled),
            file = job.file.lock() => file,
        };
        if self.prepare(job)? {
            return Ok(());
        }
        self.try_sources(job, generation, cancel).await
    }

    /// True when the store already holds the asset; otherwise its bytes are reserved.
    fn prepare(&self, job: &Job) -> Result<bool, Failure> {
        let asset = job.asset();
        let stored = self.store.contains(&asset.digest, asset.size);
        if stored.map_err(|error| Failure::of_store(&error))? {
            return Ok(true);
        }
        job.reserve(&self.store).map_err(|error| {
            tracing::warn!("{error:#}");
            Failure::of_store(&error)
        })?;
        Ok(false)
    }

    async fn try_sources(
        &self,
        job: &Job,
        generation: u64,
        cancel: &CancellationToken,
    ) -> Result<(), Failure> {
        let mut worst = None;
        let mut index = 0;
        while let Some(source) = job.asset().sources.get(index).cloned() {
            match self
                .fetch_source(job, generation, cancel, index, &source)
                .await
            {
                Ok(()) => return self.publish(job, generation),
                Err(error) if error.ends_job() => return Err(Failure::of(&error)),
                Err(error) => worst = Some(Failure::of(&error).worse(worst)),
            }
            index += 1;
        }
        Err(worst.unwrap_or(Failure::new(ModelAssetFailure::NoSources)))
    }

    async fn fetch_source(
        &self,
        job: &Job,
        generation: u64,
        cancel: &CancellationToken,
        index: usize,
        source: &str,
    ) -> Result<(), FetchError> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let asset = job.asset();
            let held = self
                .store
                .partial_len(&asset.digest)
                .map_err(|error| store_error(&error))?;
            if !job.begin_attempt(&self.store, generation, index, held) {
                return Err(FetchError::Cancelled);
            }
            let file = self
                .store
                .open_partial(&asset.digest)
                .map_err(|error| store_error(&error))?;
            let progress = |bytes| job.progress(bytes);
            let fetched = self
                .fetcher
                .fetch(source, &asset, file, cancel, &progress)
                .await;
            let Err(error) = fetched else {
                return Ok(());
            };
            tracing::warn!("Attempt {attempt} failed: {error}");
            if !error.is_retryable() || attempt >= self.config.attempts_per_source {
                return Err(error);
            }
            let delay = self
                .config
                .retry_backoff
                .saturating_mul(1 << (attempt - 1).min(16))
                .min(MAX_BACKOFF);
            tokio::select! {
                biased;
                () = cancel.cancelled() => return Err(FetchError::Cancelled),
                () = tokio::time::sleep(delay) => {}
            }
        }
    }

    fn publish(&self, job: &Job, generation: u64) -> Result<(), Failure> {
        job.set_state(&self.store, Some(generation), ModelAssetState::Verifying);
        self.store.publish(&job.asset()).map_err(|error| {
            tracing::warn!("{error:#}");
            Failure::new(ModelAssetFailure::Io)
        })
    }

    fn complete(&self, job: &Arc<Job>, generation: u64) {
        let digest = {
            let mut control = lock!(job.control);
            if control.generation != generation {
                return;
            }
            control.reservation = None;
            control.run = None;
            let closed = self
                .store
                .with_db(|db| db.delete_job(&control.asset.digest));
            if let Err(error) = closed {
                tracing::warn!(
                    "Close model asset job {}: {error:#}",
                    control.asset.digest.store_key()
                );
            }
            job.state.send_replace(ModelAssetState::Present);
            control.asset.digest.clone()
        };
        let mut jobs = lock!(self.jobs);
        if jobs
            .get(&digest)
            .is_some_and(|current| Arc::ptr_eq(current, job))
        {
            jobs.remove(&digest);
        }
    }

    fn fail(&self, job: &Job, generation: u64, failure: Failure) {
        job.release(generation);
        job.set_state(&self.store, Some(generation), failure.state());
    }

    /// Takes the job over and opens its push session.
    async fn open_push(&self, job: &Arc<Job>) -> Result<ModelAssetState> {
        let generation = job.take_over();
        let _file = job.file.lock().await;
        self.open_session(job, generation).await
    }

    /// A staged file that already holds every byte is verified at once, since no chunk
    /// would ever arrive to trigger it.
    async fn open_session(&self, job: &Arc<Job>, generation: u64) -> Result<ModelAssetState> {
        let asset = job.asset();
        let key = asset.digest.store_key();
        if self.store.contains(&asset.digest, asset.size)? {
            self.complete(job, generation);
            return Ok(ModelAssetState::Present);
        }
        let mut held = self.store.partial_len(&asset.digest)?;
        if held > asset.size {
            self.store.remove_partial(&asset.digest)?;
            held = 0;
        }
        if let Err(error) = job.reserve(&self.store) {
            self.fail(job, generation, Failure::of_store(&error));
            return Err(error.context(format!("Push model asset {key}")));
        }
        drop(self.store.open_partial(&asset.digest)?);
        ensure!(
            job.owns(generation),
            "Push model asset {key}: another request took the job over"
        );
        job.pushed();
        if held == asset.size {
            return self.verify_push(job, generation).await;
        }
        ensure!(
            job.set_state(
                &self.store,
                Some(generation),
                ModelAssetState::AwaitingPush { bytes: held }
            ),
            "Push model asset {key}: another request took the job over"
        );
        Ok(job.current())
    }

    /// Appends one chunk under the job's file lock; the chunk that completes the file
    /// starts its verification.
    async fn push(&self, job: &Arc<Job>, offset: u64, chunk: Vec<u8>) -> Result<ModelAssetState> {
        let digest = job.asset().digest;
        let key = digest.store_key();
        let _file = job.file.lock().await;
        let Some((generation, size)) = push_session(job, &key)? else {
            return Ok(ModelAssetState::Present);
        };
        ensure!(
            offset
                .checked_add(chunk.len() as u64)
                .is_some_and(|end| end <= size),
            "Push model asset {key}: {} bytes at offset {offset} exceed the declared {size}",
            chunk.len()
        );
        let length = self.write_staged(&digest, offset, chunk).await?;
        job.pushed();
        if length < size {
            job.show(generation, ModelAssetState::AwaitingPush { bytes: length });
            return Ok(job.current());
        }
        match self.verify_push(job, generation).await? {
            ModelAssetState::AwaitingPush { .. } => bail!(
                "Push model asset {key}: the bytes do not match the pinned {} digest; restart from offset zero",
                digest.algorithm.as_str()
            ),
            state => Ok(state),
        }
    }

    /// Stops the job and discards its staged bytes. A failed job, which nothing works on,
    /// is forgotten as well: its map entry and journal row go.
    async fn cancel(&self, job: &Arc<Job>) -> Result<ModelAssetState> {
        let settled = matches!(job.current(), ModelAssetState::Failed { .. });
        let generation = job.take_over();
        let cancelled = Failure::new(ModelAssetFailure::Cancelled).state();
        job.set_state(&self.store, Some(generation), cancelled.clone());
        let _file = job.file.lock().await;
        if !job.owns(generation) {
            return Ok(job.current());
        }
        self.store.remove_partial(&job.asset().digest)?;
        job.release(generation);
        if settled {
            self.forget(job)?;
        }
        Ok(cancelled)
    }

    async fn staged_digest(&self, asset: &ModelAssetDescriptor) -> Result<(File, Option<String>)> {
        let mut file = self.store.open_partial(&asset.digest)?;
        let (algorithm, size) = (asset.digest.algorithm, asset.size);
        let (file, actual) = tokio::task::spawn_blocking(move || {
            let actual = file_digest(&mut file, algorithm, size);
            (file, actual)
        })
        .await?;
        let key = asset.digest.store_key();
        Ok((
            file,
            actual.with_context(|| format!("Verify pushed model asset {key}"))?,
        ))
    }

    /// Hashes the whole staged file under the job's file lock only, so other jobs never
    /// wait. Answers `Present`, or `AwaitingPush{0}` after a mismatch emptied the file; a
    /// read error fails the job, so it never stays `Verifying`.
    async fn verify_push(&self, job: &Arc<Job>, generation: u64) -> Result<ModelAssetState> {
        let asset = job.asset();
        job.set_state(&self.store, Some(generation), ModelAssetState::Verifying);
        let matches = match self.staged_matches(&asset).await {
            Ok(matches) => matches,
            Err(error) => {
                self.fail(job, generation, Failure::new(ModelAssetFailure::Io));
                return Err(error);
            }
        };
        if matches {
            return self.publish_pushed(job, generation, &asset);
        }
        let restart = ModelAssetState::AwaitingPush { bytes: 0 };
        job.set_state(&self.store, Some(generation), restart.clone());
        Ok(restart)
    }

    /// Whether the staged file holds the pinned bytes; one that does not is emptied.
    async fn staged_matches(&self, asset: &ModelAssetDescriptor) -> Result<bool> {
        let (file, actual) = self.staged_digest(asset).await?;
        if actual.as_deref() == Some(asset.digest.hex.as_str()) {
            return Ok(true);
        }
        file.set_len(0)?;
        file.sync_all()?;
        Ok(false)
    }

    /// Verified bytes go into the store only while this session still owns the job.
    fn publish_pushed(
        &self,
        job: &Arc<Job>,
        generation: u64,
        asset: &ModelAssetDescriptor,
    ) -> Result<ModelAssetState> {
        ensure!(
            job.owns(generation),
            "Push model asset {}: another request took the job over during verification",
            asset.digest.store_key()
        );
        if let Err(error) = self.store.publish(asset) {
            self.fail(job, generation, Failure::new(ModelAssetFailure::Io));
            return Err(error);
        }
        self.complete(job, generation);
        Ok(ModelAssetState::Present)
    }

    async fn write_staged(
        &self,
        digest: &ModelAssetDigest,
        offset: u64,
        chunk: Vec<u8>,
    ) -> Result<u64> {
        let file = self.store.open_partial(digest)?;
        tokio::task::spawn_blocking(move || write_chunk(file, offset, &chunk))
            .await?
            .with_context(|| format!("Push model asset {}", digest.store_key()))
    }
}

/// The generation and size of the open push session, or `None` once the asset is present.
fn push_session(job: &Job, key: &str) -> Result<Option<(u64, u64)>> {
    let control = lock!(job.control);
    match job.current() {
        ModelAssetState::Present => Ok(None),
        ModelAssetState::AwaitingPush { .. } => Ok(Some((control.generation, control.asset.size))),
        state => bail!("Push model asset {key}: no push session is open ({state:?})"),
    }
}

fn validate_request(asset: &ModelAssetDescriptor, operation_id: Option<&str>) -> Result<()> {
    asset
        .validate()
        .with_context(|| format!("Ensure model asset {}", asset.digest.store_key()))?;
    operation_id.map_or(Ok(()), validate_operation_id)
}

fn new_job(asset: &ModelAssetDescriptor, operation_id: Option<&str>) -> Result<Arc<Job>> {
    let now = unix_time()?;
    Ok(Job::restore(JobRecord {
        job_id: uuid::Uuid::new_v4().to_string(),
        asset: asset.clone(),
        state: ModelAssetState::Queued,
        attempts: 0,
        operation_ids: operation_id.into_iter().map(str::to_owned).collect(),
        created_at: now,
        updated_at: now,
    }))
}

/// Collects every `gc_every`, and `gc_after_release` after a reference goes, so a model's
/// removal frees its files at once under disk pressure. Ends with the shutdown, or with the
/// last owner of the acquisition.
fn collect_periodically(inner: &Arc<Inner>) {
    let (every, after_release) = (inner.config.gc_every, inner.config.gc_after_release);
    let (shutdown, releases) = (inner.shutdown.clone(), inner.store.releases());
    let owner = Arc::downgrade(inner);
    inner.tasks.spawn(async move {
        loop {
            let delay = tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(every) => Duration::ZERO,
                () = releases.notified() => after_release,
            };
            tokio::select! {
                () = shutdown.cancelled() => return,
                () = tokio::time::sleep(delay) => {}
            }
            if !collect_once(&owner).await {
                return;
            }
        }
    });
}

/// One collection on a blocking thread; false once the acquisition is gone.
async fn collect_once(owner: &Weak<Inner>) -> bool {
    let Some(inner) = owner.upgrade() else {
        return false;
    };
    let manager = AcquisitionManager { inner };
    match tokio::task::spawn_blocking(move || manager.collect_garbage()).await {
        Ok(Ok(report)) if !report.removed.is_empty() || report.partials_removed > 0 => {
            tracing::info!(
                "Model store collection removed {} blobs ({} bytes) and {} staging files",
                report.removed.len(),
                report.freed_bytes,
                report.partials_removed
            );
        }
        Ok(Ok(_)) => {}
        Ok(Err(error)) => tracing::warn!("Collect the model store: {error:#}"),
        Err(error) => tracing::warn!("Collect the model store: {error}"),
    }
    true
}

/// Acquires model assets into the store: one journaled job per digest, fetched from its
/// sources in order, or pushed by a controller when no source works.
#[derive(Clone)]
pub struct AcquisitionManager {
    inner: Arc<Inner>,
}

impl AcquisitionManager {
    /// Resumes the journaled jobs and starts collecting garbage. Runs inside a Tokio runtime.
    pub fn start(
        store: Arc<ModelStore>,
        fetcher: Fetcher,
        config: AcquisitionConfig,
    ) -> Result<Self> {
        ensure!(
            config.concurrent_downloads > 0 && config.attempts_per_source > 0,
            "Model acquisition needs a download slot and an attempt per source"
        );
        let inner = Arc::new(Inner {
            downloads: Arc::new(Semaphore::new(config.concurrent_downloads)),
            store,
            fetcher,
            config,
            jobs: Mutex::default(),
            shutdown: CancellationToken::new(),
            tasks: TaskTracker::new(),
        });
        let records = inner.store.with_db(|db| db.jobs())?;
        for record in records {
            inner.recover(record)?;
        }
        collect_periodically(&inner);
        Ok(Self { inner })
    }

    pub fn store(&self) -> &Arc<ModelStore> {
        &self.inner.store
    }

    /// Starts a job for the asset or joins the open one, which then also tries the new
    /// sources. A failed job, or one whose push was abandoned, starts over download-first
    /// with the new sources leading. An asset already stored answers without a job.
    pub fn ensure(
        &self,
        asset: &ModelAssetDescriptor,
        operation_id: Option<&str>,
    ) -> Result<ModelAssetStatus> {
        let key = asset.digest.store_key();
        validate_request(asset, operation_id)?;
        ensure!(
            !self.inner.shutdown.is_cancelled(),
            "Ensure model asset {key}: acquisition is shutting down"
        );
        let mut jobs = lock!(self.inner.jobs);
        if let Some(job) = jobs.get(&asset.digest).cloned() {
            return self.join(&job, asset, operation_id);
        }
        if self.present(asset)? {
            return Ok(ModelAssetStatus {
                digest: asset.digest.clone(),
                job_id: None,
                state: ModelAssetState::Present,
            });
        }
        self.inner.make_room(&mut jobs, &key)?;
        let job = new_job(asset, operation_id)?;
        jobs.insert(asset.digest.clone(), Arc::clone(&job));
        self.inner.spawn(&job);
        Ok(job.snapshot().status())
    }

    /// Whether the store holds the asset already; a stored size that differs is refused.
    fn present(&self, asset: &ModelAssetDescriptor) -> Result<bool> {
        let store = &self.inner.store;
        if let Some(stored) = store.with_db(|db| db.asset(&asset.digest))? {
            ensure!(
                stored.size == asset.size,
                "Ensure model asset {}: it is stored with {} bytes, not {}",
                asset.digest.store_key(),
                stored.size,
                asset.size
            );
        }
        if !store.contains(&asset.digest, asset.size)? {
            return Ok(false);
        }
        store.touch(&asset.digest)?;
        Ok(true)
    }

    /// A job nothing works on takes the request's size, so a wrong first request cannot
    /// block the asset; while a run or push works on the job, a different size is refused.
    fn join(
        &self,
        job: &Arc<Job>,
        asset: &ModelAssetDescriptor,
        operation_id: Option<&str>,
    ) -> Result<ModelAssetStatus> {
        let (restart, resized) = {
            let mut control = lock!(job.control);
            let state = job.current();
            let failed = matches!(state, ModelAssetState::Failed { .. });
            let abandoned =
                matches!(state, ModelAssetState::AwaitingPush { .. }) && !control.pushing();
            let unstarted = state == ModelAssetState::AwaitingPush { bytes: 0 };
            let resized = control.asset.size != asset.size;
            ensure!(
                !resized || failed || abandoned || unstarted,
                "Ensure model asset {}: it is already requested with {} bytes, not {}",
                asset.digest.store_key(),
                control.asset.size,
                asset.size
            );
            if resized {
                control.asset = asset.clone();
                control.reservation = None;
            } else if failed || abandoned {
                lead_with(&mut control.asset.sources, &asset.sources);
            } else {
                merge_sources(&mut control.asset.sources, &asset.sources);
            }
            remember(&mut control.operation_ids, operation_id);
            let downloads = abandoned && !control.asset.sources.is_empty();
            (resized || failed || downloads, resized)
        };
        if resized {
            self.inner.store.remove_partial(&asset.digest)?;
        }
        if restart {
            self.inner.spawn(job);
        } else {
            job.persist(&self.inner.store);
        }
        Ok(job.snapshot().status())
    }

    pub fn state(&self, digest: &ModelAssetDigest) -> Result<Option<ModelAssetState>> {
        if let Some(job) = self.inner.job(digest) {
            return Ok(Some(job.snapshot().state));
        }
        Ok(self
            .inner
            .store
            .path_of(digest)?
            .map(|_| ModelAssetState::Present))
    }

    /// Open jobs, oldest first.
    pub fn jobs(&self) -> Vec<JobSnapshot> {
        let mut jobs: Vec<_> = lock!(self.inner.jobs)
            .values()
            .map(|job| (job.created_at, job.snapshot()))
            .collect();
        jobs.sort_by(|left, right| (left.0, &left.1.job_id).cmp(&(right.0, &right.1.job_id)));
        jobs.into_iter().map(|(_, snapshot)| snapshot).collect()
    }

    pub fn job_digest(&self, job_id: &str) -> Option<ModelAssetDigest> {
        lock!(self.inner.jobs)
            .iter()
            .find(|(_, job)| job.id == job_id)
            .map(|(digest, _)| digest.clone())
    }

    /// Waits until the job stops progressing by itself: present, failed or awaiting a push.
    pub async fn settled(&self, digest: &ModelAssetDigest) -> Result<ModelAssetState> {
        let Some(job) = self.inner.job(digest) else {
            return self
                .state(digest)?
                .with_context(|| format!("Model asset {} has no job", digest.store_key()));
        };
        let mut states = job.state.subscribe();
        loop {
            let state = states.borrow_and_update().clone();
            if !matches!(
                state,
                ModelAssetState::Queued
                    | ModelAssetState::Fetching { .. }
                    | ModelAssetState::Verifying
            ) {
                return Ok(state);
            }
            if states.changed().await.is_err() {
                return Ok(job.current());
            }
        }
    }

    /// Stops the job and discards its staged bytes; a failed job is removed altogether.
    /// The work finishes even when the caller goes away.
    pub async fn cancel(&self, digest: &ModelAssetDigest) -> Result<ModelAssetState> {
        let key = digest.store_key();
        let job = self
            .inner
            .job(digest)
            .with_context(|| format!("Cancel model asset {key}: no job requests it"))?;
        if job.current() == ModelAssetState::Present {
            return Ok(ModelAssetState::Present);
        }
        let inner = Arc::clone(&self.inner);
        self.inner
            .detached(async move { inner.cancel(&job).await })
            .await
    }

    /// Opens a push session and returns `AwaitingPush` with the offset to continue from,
    /// or `Present` once a staged copy that already holds every byte verifies. A running
    /// fetch is stopped only when `take_over_fetch` asks for it. The take-over finishes
    /// even when the caller goes away.
    pub async fn begin_push(
        &self,
        digest: &ModelAssetDigest,
        take_over_fetch: bool,
    ) -> Result<ModelAssetState> {
        let key = digest.store_key();
        let job = self
            .inner
            .job(digest)
            .with_context(|| format!("Push model asset {key}: no job requests it"))?;
        match job.current() {
            ModelAssetState::Present => return Ok(ModelAssetState::Present),
            ModelAssetState::Verifying => {
                bail!("Push model asset {key}: the asset is being verified")
            }
            ModelAssetState::Fetching { .. } if !take_over_fetch => {
                bail!(
                    "Push model asset {key}: the device is fetching it; take the fetch over explicitly"
                )
            }
            _ => {}
        }
        let inner = Arc::clone(&self.inner);
        self.inner
            .detached(async move { inner.open_push(&job).await })
            .await
    }

    /// Follows the artifact chunk rules: contiguous offsets, a retried chunk repeats the
    /// received bytes exactly, every write is synced, and the last chunk triggers
    /// verification. A digest mismatch empties the staged file. The write and the
    /// verification finish even when the caller goes away.
    pub async fn push_chunk(
        &self,
        digest: &ModelAssetDigest,
        offset: u64,
        bytes: &[u8],
    ) -> Result<ModelAssetState> {
        let key = digest.store_key();
        ensure!(
            !bytes.is_empty() && bytes.len() <= PUSH_CHUNK_MAX_BYTES,
            "Push model asset {key}: a chunk holds 1 to {PUSH_CHUNK_MAX_BYTES} bytes, not {}",
            bytes.len()
        );
        let Some(job) = self.inner.job(digest) else {
            return match self.state(digest)? {
                Some(ModelAssetState::Present) => Ok(ModelAssetState::Present),
                _ => bail!("Push model asset {key}: no job requests it"),
            };
        };
        let (inner, chunk) = (Arc::clone(&self.inner), bytes.to_vec());
        self.inner
            .detached(async move { inner.push(&job, offset, chunk).await })
            .await
    }

    /// Drops settled jobs (failed, or awaiting a push nobody sends) untouched for a week
    /// with their staged bytes, then collects the store.
    pub fn collect_garbage(&self) -> Result<GcReport> {
        let cutoff = unix_time()? - SETTLED_JOB_RETENTION_SECONDS;
        let mut jobs = lock!(self.inner.jobs);
        let stale: Vec<ModelAssetDigest> = jobs
            .iter()
            .filter(|(_, job)| {
                Arc::strong_count(job) == 1 && !job.live() && job.updated_at() <= cutoff
            })
            .map(|(digest, _)| digest.clone())
            .collect();
        for digest in stale {
            self.inner.drop_job(&mut jobs, &digest)?;
        }
        drop(jobs);
        self.inner.store.collect_garbage()
    }

    /// Stops every run without failing it, so the next start resumes it from the journal.
    pub async fn shutdown(&self) {
        self.inner.shutdown.cancel();
        self.inner.tasks.close();
        self.inner.tasks.wait().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        db::{AssetOwner, OwnerKind},
        fetch::AddressPolicy,
        store::ModelStoreConfig,
        test_server::{Origin, asset, pattern, ranged, serving, trickling, truncated},
    };
    use axum::{
        Router,
        body::Body,
        http::{HeaderMap, StatusCode},
        response::{IntoResponse, Response},
        routing::get,
    };
    use std::{
        path::Path,
        sync::atomic::{AtomicUsize, Ordering},
    };

    const SIZE: usize = 300_000;
    const CUT: usize = 100_000;

    fn config() -> AcquisitionConfig {
        AcquisitionConfig {
            retry_backoff: Duration::from_millis(1),
            ..AcquisitionConfig::default()
        }
    }

    /// Setup failures panic, so the tests count only the steps they exercise.
    fn manager(state: &Path, origin: &Origin, max_bytes: Option<u64>) -> AcquisitionManager {
        let config_store = ModelStoreConfig {
            max_bytes,
            ..ModelStoreConfig::default()
        };
        let store = ModelStore::open(state, config_store).expect("a model store");
        AcquisitionManager::start(Arc::new(store), origin.fetcher(), config())
            .expect("an acquisition manager")
    }

    fn failed(reason: ModelAssetFailure) -> ModelAssetState {
        Failure::new(reason).state()
    }

    /// Answers the first request with `first` and later ones with whole or ranged bytes.
    fn first_then_ranged(
        bytes: Arc<Vec<u8>>,
        first: fn(&[u8]) -> Response,
    ) -> axum::routing::MethodRouter {
        let calls = Arc::new(AtomicUsize::new(0));
        get(move |headers: HeaderMap| {
            let (bytes, calls) = (Arc::clone(&bytes), Arc::clone(&calls));
            async move {
                if calls.fetch_add(1, Ordering::SeqCst) == 0 {
                    first(&bytes)
                } else {
                    ranged(&bytes, &headers)
                }
            }
        })
    }

    async fn staged(
        manager: &AcquisitionManager,
        digest: &ModelAssetDigest,
        bytes: u64,
    ) -> Result<()> {
        for _ in 0..1000 {
            if manager.store().partial_len(digest)? >= bytes {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        bail!("The staged bytes never reached {bytes}")
    }

    fn blob(manager: &AcquisitionManager, digest: &ModelAssetDigest) -> Vec<u8> {
        let path = manager.store().path_of(digest).expect("a store lookup");
        std::fs::read(path.expect("a present blob")).expect("the blob bytes")
    }

    #[tokio::test]
    async fn sources_are_tried_in_order_until_one_serves_the_pinned_bytes() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let origin = Origin::start(
            Router::new()
                .route("/missing", get(|| async { StatusCode::NOT_FOUND }))
                .route("/busy", get(|| async { StatusCode::SERVICE_UNAVAILABLE }))
                .route("/blob", serving(bytes.clone())),
        )
        .await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let sources = ["/missing", "/busy", "/blob"].map(|path| origin.url(path));
        let wanted = asset(&bytes, sources.to_vec());
        let queued = manager.ensure(&wanted, Some("deploy-1"))?;
        assert_eq!(queued.state, ModelAssetState::Queued);
        assert_eq!(
            manager.settled(&wanted.digest).await?,
            ModelAssetState::Present
        );
        assert_eq!(blob(&manager, &wanted.digest), *bytes);
        assert_eq!(origin.hits.to("/missing").len(), 1);
        assert_eq!(origin.hits.to("/busy").len(), 3);
        let fetched = origin.hits.to("/blob");
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].accept_encoding.as_deref(), Some("identity"));
        assert!(manager.jobs().is_empty());
        let closed = manager.store().with_db(|db| db.job(&wanted.digest))?;
        assert!(closed.is_none());
        let stored = manager.ensure(&wanted, None)?;
        assert_eq!(
            (stored.job_id, stored.state),
            (None, ModelAssetState::Present)
        );
        assert_eq!(origin.hits.to("/blob").len(), 1);
        Ok(())
    }

    #[tokio::test]
    async fn a_cut_transfer_resumes_from_the_staged_bytes() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let cut = |bytes: &[u8]| truncated(bytes, CUT, false);
        let origin =
            Origin::start(Router::new().route("/blob", first_then_ranged(bytes.clone(), cut)))
                .await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let wanted = asset(&bytes, vec![origin.url("/blob")]);
        manager.ensure(&wanted, None)?;
        assert_eq!(
            manager.settled(&wanted.digest).await?,
            ModelAssetState::Present
        );
        assert_eq!(blob(&manager, &wanted.digest), *bytes);
        let hits = origin.hits.to("/blob");
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].range, None);
        let resumed = format!("bytes={CUT}-");
        assert_eq!(hits[1].range.as_deref(), Some(resumed.as_str()));
        Ok(())
    }

    #[tokio::test]
    async fn tampered_bytes_fail_as_a_digest_mismatch() -> Result<()> {
        let bytes = pattern(SIZE);
        let mut tampered = bytes.clone();
        tampered[SIZE / 2] ^= 0xff;
        let routes = Router::new().route("/tampered", serving(Arc::new(tampered)));
        let origin = Origin::start(routes).await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let wrong = asset(&bytes, vec![origin.url("/tampered")]);
        manager.ensure(&wrong, None)?;
        let reason = manager.settled(&wrong.digest).await?;
        assert_eq!(reason, failed(ModelAssetFailure::DigestMismatch));
        assert_eq!(manager.store().partial_len(&wrong.digest)?, 0);
        assert_eq!(origin.hits.to("/tampered").len(), 1);
        Ok(())
    }

    /// Serves more bytes than declared, with and without announcing the length.
    fn oversized_routes(longer: Arc<Vec<u8>>) -> Router {
        let streamed = Arc::clone(&longer);
        let unannounced = get(move || {
            let chunk = bytes::Bytes::copy_from_slice(&streamed);
            let stream = futures_util::stream::iter([Ok::<_, std::io::Error>(chunk)]);
            async move { Body::from_stream(stream) }
        });
        Router::new()
            .route("/longer", serving(longer))
            .route("/unannounced", unannounced)
    }

    #[tokio::test]
    async fn oversized_responses_fail_as_a_size_mismatch() -> Result<()> {
        let origin = Origin::start(oversized_routes(Arc::new(pattern(SIZE + 10)))).await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let declared = pattern(SIZE);
        for path in ["/longer", "/unannounced"] {
            let mut sized = asset(&declared, vec![origin.url(path)]);
            sized.digest.hex = blake3::hash(path.as_bytes()).to_hex().to_string();
            manager.ensure(&sized, None)?;
            let reason = manager.settled(&sized.digest).await?;
            assert_eq!(reason, failed(ModelAssetFailure::SizeMismatch), "{path}");
            assert_eq!(manager.store().partial_len(&sized.digest)?, 0, "{path}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn concurrent_requesters_share_one_job() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let gate = Arc::new(Semaphore::new(0));
        let origin = Origin::start(Router::new().route(
            "/blob",
            get({
                let (bytes, gate) = (bytes.clone(), gate.clone());
                move |headers: HeaderMap| {
                    let (bytes, gate) = (bytes.clone(), gate.clone());
                    async move {
                        let _open = gate.acquire().await;
                        ranged(&bytes, &headers)
                    }
                }
            }),
        ))
        .await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let wanted = asset(&bytes, vec![origin.url("/blob")]);
        let mirrored = asset(&bytes, vec![origin.url("/mirror")]);
        let first = manager.ensure(&wanted, Some("op-a"))?;
        let joined = manager.ensure(&mirrored, Some("op-b"))?;
        assert_eq!(first.job_id, joined.job_id);
        let mut resized = wanted.clone();
        resized.size += 1;
        assert!(manager.ensure(&resized, None).is_err());

        let jobs = manager.jobs();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].operation_ids, ["op-a", "op-b"]);
        assert_eq!(
            jobs[0].asset.sources,
            [origin.url("/blob"), origin.url("/mirror")]
        );
        assert_eq!(
            manager.job_digest(&jobs[0].job_id),
            Some(wanted.digest.clone())
        );
        let (first, second) = tokio::join!(manager.settled(&wanted.digest), async {
            gate.add_permits(8);
            manager.settled(&mirrored.digest).await
        });
        assert_eq!(first?, ModelAssetState::Present);
        assert_eq!(second?, ModelAssetState::Present);
        assert_eq!(origin.hits.to("/blob").len(), 1);
        assert!(origin.hits.to("/mirror").is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn a_restarted_agent_resumes_the_journaled_job() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let stall = |bytes: &[u8]| truncated(bytes, CUT, true);
        let origin =
            Origin::start(Router::new().route("/blob", first_then_ranged(bytes.clone(), stall)))
                .await;
        let state = tempfile::tempdir()?;
        let wanted = asset(&bytes, vec![origin.url("/blob")]);
        let first = manager(state.path(), &origin, None);
        first.ensure(&wanted, Some("deploy"))?;
        staged(&first, &wanted.digest, CUT as u64).await?;
        let job_id = first.jobs()[0].job_id.clone();
        first.shutdown().await;
        let journaled = first.store().with_db(|db| db.job(&wanted.digest))?;
        let journaled = journaled.context("journaled job")?;
        assert!(matches!(journaled.state, ModelAssetState::Fetching { .. }));
        drop(first);

        let second = manager(state.path(), &origin, None);
        assert_eq!(second.jobs()[0].job_id, job_id);
        assert_eq!(second.jobs()[0].operation_ids, ["deploy"]);
        assert_eq!(
            second.settled(&wanted.digest).await?,
            ModelAssetState::Present
        );
        assert_eq!(blob(&second, &wanted.digest), *bytes);
        let resumed = origin.hits.to("/blob");
        assert_eq!(
            resumed[1].range.as_deref(),
            Some(format!("bytes={CUT}-").as_str())
        );
        Ok(())
    }

    fn awaiting(bytes: usize) -> ModelAssetState {
        ModelAssetState::AwaitingPush {
            bytes: bytes as u64,
        }
    }

    /// A manager whose assets have no sources, so every byte arrives by push.
    fn push_manager(state: &Path) -> AcquisitionManager {
        let store = ModelStore::open(state, ModelStoreConfig::default()).expect("a model store");
        let fetcher = Fetcher::new(AddressPolicy::global_only()).expect("a fetcher");
        AcquisitionManager::start(Arc::new(store), fetcher, config())
            .expect("an acquisition manager")
    }

    #[tokio::test]
    async fn job_ids_are_canonical_uuids_on_the_wire() -> Result<()> {
        let state = tempfile::tempdir()?;
        let manager = push_manager(state.path());
        let wanted = asset(b"weights", Vec::new());
        let status = manager.ensure(&wanted, None)?;
        status.validate()?;
        let job_id = status.job_id.context("a job for an absent asset")?;
        flow_like_device_protocol::validate_model_job_id(&job_id)?;
        let listed = manager.jobs()[0].wire();
        listed.validate()?;
        assert_eq!(listed.job_id, job_id);
        assert_eq!(manager.job_digest(&job_id), Some(wanted.digest));
        Ok(())
    }

    #[test]
    fn listed_jobs_name_the_sources_a_controller_can_send_from() {
        let cdn = "https://cdn.flow-like.com/bits/model".to_owned();
        let snapshot = JobSnapshot {
            job_id: uuid::Uuid::new_v4().to_string(),
            asset: asset(b"weights", vec![cdn.clone(), "http://plain.test/m".into()]),
            state: failed(ModelAssetFailure::EgressBlocked),
            source_host: None,
            attempts: 3,
            bytes_per_second: None,
            operation_ids: Vec::new(),
            updated_at: 1,
        };
        let listed = snapshot.wire();
        listed.validate().expect("a valid listed job");
        assert_eq!(listed.sources, [cdn]);
    }

    #[test]
    fn a_fetching_job_names_only_a_source_host_clients_accept() -> Result<()> {
        for (source, host) in [
            (
                "https://cdn.flow-like.com/bits/m",
                Some("cdn.flow-like.com"),
            ),
            ("https://[2606:4700::1111]/m", Some("[2606:4700::1111]")),
            ("https://my_cdn.example.com/m", None),
            ("https://*.example.com/m", None),
        ] {
            let fetching = JobRecord {
                job_id: uuid::Uuid::new_v4().to_string(),
                asset: asset(b"weights", vec![source.to_owned()]),
                state: ModelAssetState::Fetching {
                    source_index: 0,
                    bytes: 3,
                },
                attempts: 1,
                operation_ids: Vec::new(),
                created_at: 1,
                updated_at: 1,
            };
            let listed = Job::restore(fetching).snapshot().wire();
            listed.validate()?;
            assert_eq!(listed.source_host.as_deref(), host, "{source}");
        }
        Ok(())
    }

    #[tokio::test]
    async fn a_trickling_source_gets_no_second_attempt() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let trickle = get({
            let bytes = Arc::clone(&bytes);
            move || {
                let bytes = Arc::clone(&bytes);
                async move { trickling(&bytes, Duration::from_millis(50)) }
            }
        });
        let routes = Router::new()
            .route("/trickle", trickle)
            .route("/blob", serving(Arc::clone(&bytes)));
        let origin = Origin::start(routes).await;
        let state = tempfile::tempdir()?;
        let store = ModelStore::open(state.path(), ModelStoreConfig::default())?;
        let fetcher = origin.fetcher().with_rate_floor(Duration::from_secs(1), 64);
        let manager = AcquisitionManager::start(Arc::new(store), fetcher, config())?;
        let wanted = asset(&bytes, vec![origin.url("/trickle"), origin.url("/blob")]);
        manager.ensure(&wanted, None)?;
        let settled =
            tokio::time::timeout(Duration::from_secs(30), manager.settled(&wanted.digest));
        let settled = settled.await.context("the trickle held the job")??;
        assert_eq!(settled, ModelAssetState::Present);
        assert_eq!(blob(&manager, &wanted.digest), *bytes);
        assert_eq!(origin.hits.to("/trickle").len(), 1);
        assert_eq!(origin.hits.to("/blob").len(), 1);
        Ok(())
    }

    /// Requests `bytes` without sources and opens its push session.
    async fn open_push(manager: &AcquisitionManager, bytes: &[u8]) -> Result<ModelAssetDescriptor> {
        let pushed = asset(bytes, Vec::new());
        manager.ensure(&pushed, None)?;
        let reason = manager.settled(&pushed.digest).await?;
        assert_eq!(reason, failed(ModelAssetFailure::NoSources));
        let early = manager.push_chunk(&pushed.digest, 0, &bytes[..10]).await;
        assert!(early.is_err(), "a chunk before the session was accepted");
        let opened = manager.begin_push(&pushed.digest, false).await?;
        assert_eq!(opened, awaiting(0));
        Ok(pushed)
    }

    #[tokio::test]
    async fn pushed_chunks_must_continue_the_received_bytes() -> Result<()> {
        let bytes = pattern(SIZE);
        let state = tempfile::tempdir()?;
        let manager = push_manager(state.path());
        let pushed = open_push(&manager, &bytes).await?;
        let digest = &pushed.digest;
        let (half, at) = (SIZE / 2, (SIZE / 2) as u64);
        let first = &bytes[..half];
        assert_eq!(manager.push_chunk(digest, 0, first).await?, awaiting(half));
        assert_eq!(manager.push_chunk(digest, 0, first).await?, awaiting(half));
        let mut altered = first.to_vec();
        altered[1] ^= 1;
        assert!(manager.push_chunk(digest, 0, &altered).await.is_err());
        let gap = manager.push_chunk(digest, at + 1, &bytes[half + 1..]).await;
        assert!(gap.is_err());
        let beyond = manager.push_chunk(digest, at - 1, &bytes[half - 1..]).await;
        assert!(beyond.is_err());
        assert_eq!(manager.state(digest)?, Some(awaiting(half)));

        let rest = &bytes[half..];
        let done = manager.push_chunk(digest, at, rest).await?;
        assert_eq!(done, ModelAssetState::Present);
        assert_eq!(blob(&manager, digest), bytes);
        let late = manager.push_chunk(digest, at, rest).await?;
        assert_eq!(late, ModelAssetState::Present);
        Ok(())
    }

    #[tokio::test]
    async fn a_pushed_digest_mismatch_restarts_the_session_from_zero() -> Result<()> {
        let bytes = pattern(SIZE + 1);
        let state = tempfile::tempdir()?;
        let manager = push_manager(state.path());
        let pushed = open_push(&manager, &bytes).await?;
        let mut wrong = bytes.clone();
        wrong[0] ^= 1;
        let error = manager.push_chunk(&pushed.digest, 0, &wrong).await;
        let error = error.unwrap_err().to_string();
        assert!(error.contains("restart from offset zero"), "{error}");
        assert_eq!(manager.state(&pushed.digest)?, Some(awaiting(0)));
        assert_eq!(manager.store().partial_len(&pushed.digest)?, 0);
        let done = manager.push_chunk(&pushed.digest, 0, &bytes).await?;
        assert_eq!(done, ModelAssetState::Present);
        Ok(())
    }

    /// Starts fetching `bytes` from a source that stalls after `CUT` bytes.
    async fn stalled_fetch(
        state: &Path,
        bytes: Arc<Vec<u8>>,
    ) -> (Origin, AcquisitionManager, ModelAssetDescriptor) {
        let stall = |bytes: &[u8]| truncated(bytes, CUT, true);
        let routes = Router::new().route("/slow", first_then_ranged(Arc::clone(&bytes), stall));
        let origin = Origin::start(routes).await;
        let manager = manager(state, &origin, None);
        let wanted = asset(&bytes, vec![origin.url("/slow")]);
        manager.ensure(&wanted, None).expect("a started job");
        let progress = staged(&manager, &wanted.digest, CUT as u64).await;
        progress.expect("bytes staged before the stall");
        (origin, manager, wanted)
    }

    #[tokio::test]
    async fn a_push_takes_over_a_running_fetch() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let state = tempfile::tempdir()?;
        let (_origin, manager, wanted) = stalled_fetch(state.path(), Arc::clone(&bytes)).await;
        let fetching = ModelAssetState::Fetching {
            source_index: 0,
            bytes: CUT as u64,
        };
        assert_eq!(manager.state(&wanted.digest)?, Some(fetching));
        let listed = manager.jobs()[0].wire();
        listed.validate()?;
        assert_eq!(listed.source_host.as_deref(), Some("127.0.0.1"));
        assert!(manager.begin_push(&wanted.digest, false).await.is_err());
        let offset = manager.begin_push(&wanted.digest, true).await?;
        assert_eq!(offset, ModelAssetState::AwaitingPush { bytes: CUT as u64 });
        let rest = &bytes[CUT..];
        let pushed = manager.push_chunk(&wanted.digest, CUT as u64, rest).await?;
        assert_eq!(pushed, ModelAssetState::Present);
        assert_eq!(blob(&manager, &wanted.digest), *bytes);
        Ok(())
    }

    #[tokio::test]
    async fn cancel_discards_the_staged_bytes() -> Result<()> {
        let state = tempfile::tempdir()?;
        let bytes = Arc::new(pattern(SIZE));
        let (_origin, manager, dropped) = stalled_fetch(state.path(), bytes).await;
        let cancelled = manager.cancel(&dropped.digest).await?;
        assert_eq!(cancelled, failed(ModelAssetFailure::Cancelled));
        assert_eq!(manager.store().partial_len(&dropped.digest)?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn an_asset_beyond_the_disk_budget_is_refused_before_any_download() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let origin = Origin::start(Router::new().route("/blob", serving(bytes.clone()))).await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, Some(SIZE as u64 - 1));
        let wanted = asset(&bytes, vec![origin.url("/blob")]);
        manager.ensure(&wanted, None)?;
        let refused = manager.settled(&wanted.digest).await?;
        assert_eq!(refused, failed(ModelAssetFailure::DiskBudget));
        assert_eq!(origin.hits.total(), 0);
        Ok(())
    }

    /// A store whose journal holds a failed job untouched for over a week, with staged bytes.
    fn store_with_stale_job(state: &Path) -> (Arc<ModelStore>, ModelAssetDescriptor) {
        let store = ModelStore::open(state, ModelStoreConfig::default()).expect("a model store");
        let source = "https://cdn.flow-like.com/bits/stale".to_owned();
        let stale = asset(b"stale", vec![source]);
        let week_ago = unix_time().expect("the clock") - SETTLED_JOB_RETENTION_SECONDS - 1;
        let record = JobRecord {
            job_id: uuid::Uuid::new_v4().to_string(),
            asset: stale.clone(),
            state: failed(ModelAssetFailure::EgressBlocked),
            attempts: 3,
            operation_ids: Vec::new(),
            created_at: week_ago,
            updated_at: week_ago,
        };
        let journaled = store.with_db(|db| db.put_job(&record));
        journaled.expect("a journaled job");
        let mut staging = store.open_partial(&stale.digest).expect("a staging file");
        staging.write_all(b"sta").expect("staged bytes");
        (Arc::new(store), stale)
    }

    #[tokio::test]
    async fn failed_jobs_untouched_for_a_week_are_dropped() -> Result<()> {
        let state = tempfile::tempdir()?;
        let (store, stale) = store_with_stale_job(state.path());
        let fetcher = Fetcher::new(AddressPolicy::global_only())?;
        let manager = AcquisitionManager::start(Arc::clone(&store), fetcher, config())?;
        assert_eq!(manager.jobs().len(), 1);
        assert_eq!(manager.collect_garbage()?.partials_removed, 0);
        assert!(manager.jobs().is_empty());
        assert!(store.with_db(|db| db.job(&stale.digest))?.is_none());
        assert_eq!(store.partial_len(&stale.digest)?, 0);
        Ok(())
    }

    /// Polls `check` until it holds, for up to ten seconds.
    async fn eventually(mut check: impl FnMut() -> Result<bool>) -> Result<()> {
        for _ in 0..1000 {
            if check()? {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        bail!("The awaited condition never held")
    }

    async fn until(
        manager: &AcquisitionManager,
        digest: &ModelAssetDigest,
        wanted: impl Fn(&ModelAssetState) -> bool,
    ) -> Result<ModelAssetState> {
        let mut reached = None;
        eventually(|| {
            reached = manager.state(digest)?.filter(&wanted);
            Ok(reached.is_some())
        })
        .await?;
        reached.context("a reached state")
    }

    /// Polls `request` once and drops it, the way a reset tunnel stream drops its request.
    fn abandon<F: Future>(request: F) {
        let mut request = std::pin::pin!(request);
        let mut context = std::task::Context::from_waker(std::task::Waker::noop());
        let polled = request.as_mut().poll(&mut context);
        assert!(polled.is_pending(), "the request finished at once");
    }

    fn job_of(manager: &AcquisitionManager, digest: &ModelAssetDigest) -> Arc<Job> {
        manager.inner.job(digest).expect("an open job")
    }

    /// Requests every asset and waits until its job settles.
    async fn settle_all(manager: &AcquisitionManager, assets: &[ModelAssetDescriptor]) {
        for wanted in assets {
            manager.ensure(wanted, None).expect("a job");
            manager
                .settled(&wanted.digest)
                .await
                .expect("a settled job");
        }
    }

    #[tokio::test]
    async fn failed_jobs_make_way_for_new_requests() -> Result<()> {
        let state = tempfile::tempdir()?;
        let manager = push_manager(state.path());
        let sourceless: Vec<_> = (0..MAX_JOBS)
            .map(|index| asset(&index.to_le_bytes(), Vec::new()))
            .collect();
        settle_all(&manager, &sourceless).await;
        let oldest = &sourceless[7].digest;
        lock!(job_of(&manager, oldest).control).updated_at = 0;
        let newest = manager.ensure(&asset(b"newest", Vec::new()), None)?;
        assert!(newest.job_id.is_some());
        assert_eq!(manager.jobs().len(), MAX_JOBS);
        assert_eq!(manager.state(oldest)?, None);
        assert!(manager.store().with_db(|db| db.job(oldest))?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn jobs_in_progress_are_never_evicted() -> Result<()> {
        let gate = Arc::new(Semaphore::new(0));
        let held = get(move || {
            let gate = Arc::clone(&gate);
            async move {
                let _open = gate.acquire().await;
                StatusCode::OK
            }
        });
        let origin = Origin::start(Router::new().route("/held", held)).await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let source = || vec![origin.url("/held")];
        for index in 0..MAX_JOBS {
            manager.ensure(&asset(&index.to_le_bytes(), source()), None)?;
        }
        let another = asset(b"another", source());
        let refused = manager.ensure(&another, None).unwrap_err();
        assert!(
            format!("{refused:#}").contains("in progress"),
            "{refused:#}"
        );
        let queued = asset(&(MAX_JOBS - 1).to_le_bytes(), source());
        let cancelled = manager.cancel(&queued.digest).await?;
        assert_eq!(cancelled, failed(ModelAssetFailure::Cancelled));
        eventually(|| Ok(manager.ensure(&another, None).is_ok())).await?;
        assert_eq!(manager.state(&queued.digest)?, None);
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_a_failed_job_removes_it_with_its_staged_bytes() -> Result<()> {
        let state = tempfile::tempdir()?;
        let manager = push_manager(state.path());
        let sourceless = asset(&pattern(SIZE), Vec::new());
        settle_all(&manager, std::slice::from_ref(&sourceless)).await;
        let digest = &sourceless.digest;
        manager.store().open_partial(digest)?.write_all(b"staged")?;
        let cancelled = manager.cancel(digest).await?;
        assert_eq!(cancelled, failed(ModelAssetFailure::Cancelled));
        assert!(manager.jobs().is_empty());
        assert!(manager.store().with_db(|db| db.job(digest))?.is_none());
        assert_eq!(manager.store().partial_len(digest)?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn a_failed_job_starts_over_with_the_size_and_sources_of_a_later_request() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let origin = Origin::start(Router::new().route("/blob", serving(bytes.clone()))).await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let wanted = asset(&bytes, vec![origin.url("/blob")]);
        let oversized = ModelAssetDescriptor {
            size: wanted.size + 1,
            ..wanted.clone()
        };
        manager.ensure(&oversized, None)?;
        let mismatch = manager.settled(&wanted.digest).await?;
        assert_eq!(mismatch, failed(ModelAssetFailure::SizeMismatch));
        let gone = (0..MODEL_ASSET_MAX_SOURCES).map(|index| origin.url(&format!("/gone/{index}")));
        let crowded = ModelAssetDescriptor {
            sources: gone.collect(),
            ..wanted.clone()
        };
        manager.ensure(&crowded, None)?;
        let missing = manager.settled(&wanted.digest).await?;
        assert!(
            matches!(missing, ModelAssetState::Failed { .. }),
            "{missing:?}"
        );
        manager.ensure(&wanted, None)?;
        let present = manager.settled(&wanted.digest).await?;
        assert_eq!(present, ModelAssetState::Present);
        assert_eq!(blob(&manager, &wanted.digest), *bytes);
        Ok(())
    }

    #[tokio::test]
    async fn a_push_finishes_verifying_after_its_caller_goes_away() -> Result<()> {
        let bytes = pattern(SIZE);
        let state = tempfile::tempdir()?;
        let manager = push_manager(state.path());
        let pushed = open_push(&manager, &bytes).await?;
        abandon(manager.push_chunk(&pushed.digest, 0, &bytes));
        until(&manager, &pushed.digest, |state| {
            *state == ModelAssetState::Present
        })
        .await?;
        assert_eq!(blob(&manager, &pushed.digest), bytes);
        Ok(())
    }

    #[tokio::test]
    async fn a_staged_copy_that_holds_every_byte_is_verified_when_a_push_opens() -> Result<()> {
        let bytes = pattern(SIZE);
        let state = tempfile::tempdir()?;
        let manager = push_manager(state.path());
        let sourceless = asset(&bytes, Vec::new());
        settle_all(&manager, std::slice::from_ref(&sourceless)).await;
        let digest = &sourceless.digest;
        let mut altered = bytes.clone();
        altered[7] ^= 1;
        manager.store().open_partial(digest)?.write_all(&altered)?;
        assert_eq!(manager.begin_push(digest, false).await?, awaiting(0));
        assert_eq!(manager.store().partial_len(digest)?, 0);
        manager.store().open_partial(digest)?.write_all(&bytes)?;
        let opened = manager.begin_push(digest, false).await?;
        assert_eq!(opened, ModelAssetState::Present);
        assert_eq!(blob(&manager, digest), bytes);
        Ok(())
    }

    /// Makes the job's push session look abandoned.
    fn abandon_push(manager: &AcquisitionManager, digest: &ModelAssetDigest) {
        let idle = Instant::now().checked_sub(PUSH_IDLE + Duration::from_secs(1));
        lock!(job_of(manager, digest).control).pushed_at = idle;
    }

    #[tokio::test]
    async fn an_abandoned_push_falls_back_to_downloading() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let missing = |_: &[u8]| StatusCode::NOT_FOUND.into_response();
        let routes = Router::new().route("/blob", first_then_ranged(bytes.clone(), missing));
        let origin = Origin::start(routes).await;
        let state = tempfile::tempdir()?;
        let manager = manager(state.path(), &origin, None);
        let wanted = asset(&bytes, vec![origin.url("/blob")]);
        manager.ensure(&wanted, None)?;
        let refused = manager.settled(&wanted.digest).await?;
        assert!(
            matches!(refused, ModelAssetState::Failed { .. }),
            "{refused:?}"
        );
        assert_eq!(manager.begin_push(&wanted.digest, true).await?, awaiting(0));
        manager.ensure(&wanted, None)?;
        assert_eq!(manager.settled(&wanted.digest).await?, awaiting(0));
        abandon_push(&manager, &wanted.digest);
        manager.ensure(&wanted, None)?;
        let present = manager.settled(&wanted.digest).await?;
        assert_eq!(present, ModelAssetState::Present);
        assert_eq!(blob(&manager, &wanted.digest), *bytes);
        Ok(())
    }

    #[tokio::test]
    async fn a_restart_downloads_a_job_that_awaited_a_push() -> Result<()> {
        let bytes = Arc::new(pattern(SIZE));
        let origin = Origin::start(Router::new().route("/blob", serving(bytes.clone()))).await;
        let state = tempfile::tempdir()?;
        let wanted = asset(&bytes, vec![origin.url("/blob")]);
        let store = ModelStore::open(state.path(), ModelStoreConfig::default())?;
        let now = unix_time()?;
        let record = JobRecord {
            job_id: uuid::Uuid::new_v4().to_string(),
            asset: wanted.clone(),
            state: awaiting(CUT),
            attempts: 1,
            operation_ids: Vec::new(),
            created_at: now,
            updated_at: now,
        };
        store.with_db(|db| db.put_job(&record))?;
        drop(store);
        let manager = manager(state.path(), &origin, None);
        let present = manager.settled(&wanted.digest).await?;
        assert_eq!(present, ModelAssetState::Present);
        Ok(())
    }

    #[tokio::test]
    async fn a_take_over_finishes_after_its_caller_goes_away() -> Result<()> {
        let state = tempfile::tempdir()?;
        let bytes = Arc::new(pattern(SIZE));
        let (_origin, manager, wanted) = stalled_fetch(state.path(), bytes).await;
        abandon(manager.begin_push(&wanted.digest, true));
        let opened = until(&manager, &wanted.digest, |state| {
            matches!(state, ModelAssetState::AwaitingPush { .. })
        })
        .await?;
        assert_eq!(opened, awaiting(CUT));
        Ok(())
    }

    #[tokio::test]
    async fn a_cancel_finishes_after_its_caller_goes_away() -> Result<()> {
        let state = tempfile::tempdir()?;
        let bytes = Arc::new(pattern(SIZE));
        let (_origin, manager, dropped) = stalled_fetch(state.path(), bytes).await;
        abandon(manager.cancel(&dropped.digest));
        let store = Arc::clone(manager.store());
        eventually(|| Ok(store.partial_len(&dropped.digest)? == 0)).await?;
        let cancelled = manager.state(&dropped.digest)?;
        assert_eq!(cancelled, Some(failed(ModelAssetFailure::Cancelled)));
        Ok(())
    }

    #[tokio::test]
    async fn settled_jobs_are_collected_periodically() -> Result<()> {
        let state = tempfile::tempdir()?;
        let (store, stale) = store_with_stale_job(state.path());
        let fetcher = Fetcher::new(AddressPolicy::global_only())?;
        let config = AcquisitionConfig {
            gc_every: Duration::from_millis(20),
            ..config()
        };
        let manager = AcquisitionManager::start(Arc::clone(&store), fetcher, config)?;
        eventually(|| Ok(manager.jobs().is_empty())).await?;
        assert!(store.with_db(|db| db.job(&stale.digest))?.is_none());
        manager.shutdown().await;
        Ok(())
    }

    #[tokio::test]
    async fn a_released_blob_is_collected_soon_under_disk_pressure() -> Result<()> {
        let state = tempfile::tempdir()?;
        let full = ModelStoreConfig {
            max_bytes: Some(1),
            ..ModelStoreConfig::default()
        };
        let store = Arc::new(ModelStore::open(state.path(), full)?);
        let weights = asset(b"weights", Vec::new());
        store.open_partial(&weights.digest)?.write_all(b"weights")?;
        store.publish(&weights)?;
        let model = AssetOwner::new(OwnerKind::HostedModel, "qwen")?;
        store.add_ref(&weights.digest, &model)?;
        let fetcher = Fetcher::new(AddressPolicy::global_only())?;
        let config = AcquisitionConfig {
            gc_after_release: Duration::from_millis(10),
            ..config()
        };
        let manager = AcquisitionManager::start(Arc::clone(&store), fetcher, config)?;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            store.path_of(&weights.digest)?.is_some(),
            "a held blob went"
        );
        assert!(store.remove_ref(&weights.digest, &model)?);
        eventually(|| Ok(store.path_of(&weights.digest)?.is_none())).await?;
        manager.shutdown().await;
        Ok(())
    }
}
