//! Owns every model process of the device: residency, memory-budget admission with LRU
//! eviction, health checks, restarts with backoff, and leases that keep an engine alive
//! while a request is in flight.

use super::{
    acquire::AcquisitionManager,
    db::{AssetOwner, HostedModelRecord, ModelOrigin, OwnerKind},
    engines::{
        self, EngineLauncher, EnginePlan, EngineProcess, MemoryEstimate, StartFailure,
        gguf::{self, GgufFacts},
        llamacpp::{self, Gauges, LlamaInputs, ModelFiles},
    },
    runtime::{InstalledRuntime, RuntimeInstaller},
    store::DEFAULT_GC_GRACE,
    system,
};
use crate::enrollment::unix_time;
use anyhow::{Context, Result, bail, ensure};
use flow_like_device_protocol::{
    HostedModel, HostedModelState, MODEL_MAX_PENDING_ASSETS, ModelAssetDescriptor,
    ModelAssetDigest, ModelAssetState, ModelAssetSummary, ModelBackend, ModelEngine,
    ModelHostFailure, ModelRuntime, ModelSettings, ModelSpec, ReleaseTarget, Residency,
    SystemFacts, validate_management_id,
};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Mutex as AsyncMutex, watch};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

const MONITOR_INTERVAL: Duration = Duration::from_secs(10);
const HEALTH_FAILURES_BEFORE_RESTART: u32 = 3;
const BACKOFF_BASE: Duration = Duration::from_secs(2);
const BACKOFF_MAX: Duration = Duration::from_secs(300);
/// An engine that stayed up this long starts its next crash with the base backoff.
const STABLE_AFTER: Duration = Duration::from_secs(600);
const KEY_FILE: &str = ".engine-key";
/// The process of a running engine and its start time, so the next agent can stop an engine a
/// crashed one left behind.
const ENGINE_PID_FILE: &str = "engine.pid";

#[derive(Clone, Debug)]
pub struct SupervisorConfig {
    /// Bytes loaded models may hold together; `None` derives it from the hardware.
    pub memory_budget: Option<u64>,
    /// Memory free for another engine; `None` reads the device's available memory at every
    /// admission.
    pub free_memory: Option<u64>,
    pub monitor_interval: Duration,
    /// A model hosted for placements goes once no placement uses it and it was unused this long.
    pub release_unused_after: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            memory_budget: None,
            free_memory: None,
            monitor_interval: MONITOR_INTERVAL,
            release_unused_after: DEFAULT_GC_GRACE,
        }
    }
}

/// Why a request cannot reach a model; the gateway maps each to an HTTP status.
#[derive(Debug)]
pub enum AcquireError {
    NotFound(String),
    /// Pinned off, unloading, or failed and waiting for its next restart.
    Unavailable {
        message: String,
        retry_after: Option<Duration>,
    },
    Failed(anyhow::Error),
}

impl std::fmt::Display for AcquireError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Self::NotFound(model) => write!(formatter, "No hosted model is called {model}"),
            Self::Unavailable { message, .. } => formatter.write_str(message),
            Self::Failed(error) => write!(formatter, "{error:#}"),
        }
    }
}

impl std::error::Error for AcquireError {}

struct Running {
    process: AsyncMutex<Option<EngineProcess>>,
    base_url: String,
    key: Zeroizing<String>,
    estimate: MemoryEstimate,
    slots: u8,
    engine: ModelEngine,
    backend: Option<ModelBackend>,
    started: Instant,
    gauges: Mutex<Gauges>,
    ram_bytes: Mutex<u64>,
    health_failures: Mutex<u32>,
}

#[derive(Default)]
struct Restart {
    failures: u32,
    retry_at: Option<Instant>,
    reason: Option<ModelHostFailure>,
}

/// What a load ended with, for everyone who waited for it.
type LoadOutcome = std::result::Result<(), String>;

struct Hosted {
    id: String,
    record: Mutex<HostedModelRecord>,
    /// Serializes loads and unloads of this model.
    transition: AsyncMutex<()>,
    state: watch::Sender<HostedModelState>,
    engine: Mutex<Option<Arc<Running>>>,
    leases: AtomicUsize,
    last_used: Mutex<Instant>,
    restart: Mutex<Restart>,
    draining: Mutex<bool>,
    /// The template probe found no tool support; starts use chatml.
    chatml: Mutex<bool>,
    /// The load in progress, which every request and command needing the model waits for.
    loading: Mutex<Option<watch::Receiver<Option<LoadOutcome>>>>,
}

impl Hosted {
    fn new(record: HostedModelRecord) -> Arc<Self> {
        Arc::new(Self {
            id: record.id.clone(),
            record: Mutex::new(record),
            transition: AsyncMutex::new(()),
            state: watch::Sender::new(HostedModelState::Stopped),
            engine: Mutex::default(),
            leases: AtomicUsize::new(0),
            last_used: Mutex::new(Instant::now()),
            restart: Mutex::default(),
            draining: Mutex::new(false),
            chatml: Mutex::new(false),
            loading: Mutex::default(),
        })
    }

    fn record(&self) -> HostedModelRecord {
        lock!(self.record).clone()
    }

    fn running(&self) -> Option<Arc<Running>> {
        lock!(self.engine).clone()
    }

    fn set_state(&self, state: HostedModelState) {
        self.state.send_replace(state);
    }

    fn idle_for(&self) -> Duration {
        lock!(self.last_used).elapsed()
    }

    /// Moves the recorded last use to `now`, never back.
    fn touch(&self, now: i64) {
        let mut record = lock!(self.record);
        record.last_used_at = record.last_used_at.max(now);
    }

    /// A model without an engine shows as stopped, unless it failed.
    fn show_stopped(&self) {
        if !matches!(*self.state.borrow(), HostedModelState::Failed { .. }) {
            self.set_state(HostedModelState::Stopped);
        }
    }

    /// Counts a failed start or a crash; the next start waits out the backoff.
    fn failed(&self, reason: ModelHostFailure) {
        let mut restart = lock!(self.restart);
        restart.failures += 1;
        restart.retry_at = Some(Instant::now() + backoff(restart.failures));
        restart.reason = Some(reason);
        drop(restart);
        self.set_state(HostedModelState::Failed { reason });
    }

    fn serve(&self, running: &Arc<Running>) {
        *lock!(self.last_used) = Instant::now();
        *lock!(self.engine) = Some(Arc::clone(running));
        self.set_state(HostedModelState::Loaded {
            ram_bytes: 0,
            vram_bytes: 0,
            slots: running.slots,
            slots_busy: 0,
        });
        lock!(self.restart).reason = None;
    }
}

/// Refuses new leases while it lives. Dropped, also by a caller that was cancelled, it lets
/// requests in again.
struct Draining(Arc<Hosted>);

impl Draining {
    fn start(hosted: &Arc<Hosted>) -> Self {
        *lock!(hosted.draining) = true;
        Self(Arc::clone(hosted))
    }
}

impl Drop for Draining {
    fn drop(&mut self) {
        *lock!(self.0.draining) = false;
    }
}

/// Keeps one engine serving while it lives; dropping it ends the request.
pub struct EngineLease {
    hosted: Arc<Hosted>,
    engine: Arc<Running>,
}

impl EngineLease {
    pub fn base_url(&self) -> &str {
        &self.engine.base_url
    }

    pub fn key(&self) -> &str {
        &self.engine.key
    }

    pub fn model_id(&self) -> &str {
        &self.hosted.id
    }
}

impl Drop for EngineLease {
    fn drop(&mut self) {
        *lock!(self.hosted.last_used) = Instant::now();
        self.hosted.leases.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Counts a request against a model until the lease takes over or it is dropped.
struct PendingLease(Option<Arc<Hosted>>);

impl PendingLease {
    fn new(hosted: &Arc<Hosted>) -> Self {
        hosted.leases.fetch_add(1, Ordering::SeqCst);
        Self(Some(Arc::clone(hosted)))
    }

    fn into_lease(mut self, engine: Arc<Running>) -> EngineLease {
        let hosted = self.0.take().expect("a pending lease holds its model");
        EngineLease { hosted, engine }
    }
}

impl Drop for PendingLease {
    fn drop(&mut self) {
        if let Some(hosted) = &self.0 {
            hosted.leases.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

struct Inner {
    acquisition: AcquisitionManager,
    runtimes: Arc<RuntimeInstaller>,
    launcher: Arc<dyn EngineLauncher>,
    target: ReleaseTarget,
    facts: Mutex<SystemFacts>,
    budget: Mutex<u64>,
    run_root: PathBuf,
    client: reqwest::Client,
    models: Mutex<HashMap<String, Arc<Hosted>>>,
    /// Serializes admission so two loads never both count the same free memory.
    admission: AsyncMutex<()>,
    /// Bytes admitted for engines that are starting and serve nothing yet, by model.
    starting: Mutex<HashMap<String, u64>>,
    config: SupervisorConfig,
}

/// Holds the memory admitted for a starting engine until the engine serves or its start ends.
struct Reserved {
    inner: Arc<Inner>,
    model_id: String,
}

impl Drop for Reserved {
    fn drop(&mut self) {
        lock!(self.inner.starting).remove(&self.model_id);
    }
}

/// What a new engine may take: what the budget leaves, and no more than the device has free.
fn room(budget: u64, reserved: u64, free: u64) -> u64 {
    budget.saturating_sub(reserved).min(free)
}

/// Whether an engine keeps its weights in discrete GPU memory rather than RAM.
fn on_discrete_gpu(backend: Option<ModelBackend>) -> bool {
    matches!(backend, Some(ModelBackend::Vulkan | ModelBackend::Cuda))
}

#[derive(Clone)]
pub struct Supervisor {
    inner: Arc<Inner>,
}

fn wire(record: &HostedModelRecord, state: HostedModelState) -> HostedModel {
    HostedModel {
        id: record.id.clone(),
        display_name: record.spec.display_name.clone(),
        kind: record.spec.kind,
        engine: record.spec.engine,
        assets: record
            .spec
            .assets
            .iter()
            .map(|asset| asset.digest.clone())
            .collect(),
        settings: record.settings.clone(),
        residency: record.residency,
        revision: record.revision,
        state,
    }
}

/// A load step's result; a failure names the reason the model shows.
type Loaded<T> = std::result::Result<T, (ModelHostFailure, anyhow::Error)>;

struct Prepared {
    files: ModelFiles,
    header: GgufFacts,
    runtime: Option<InstalledRuntime>,
}

fn failure(
    reason: ModelHostFailure,
) -> impl FnOnce(anyhow::Error) -> (ModelHostFailure, anyhow::Error) {
    move |error| (reason, error)
}

fn start_failure(failure: &StartFailure) -> ModelHostFailure {
    match failure {
        StartFailure::Exited(_) => ModelHostFailure::EngineExited,
        StartFailure::TimedOut => ModelHostFailure::HealthTimeout,
    }
}

/// A lease on the running engine; `None` when it has to load first. Pinned-off models and
/// failed ones inside their backoff refuse.
fn lease(hosted: &Arc<Hosted>) -> std::result::Result<Option<EngineLease>, AcquireError> {
    if hosted.record().residency == Residency::PinnedOff {
        return Err(AcquireError::Unavailable {
            message: format!("Model {} is pinned off on this device", hosted.id),
            retry_after: None,
        });
    }
    let pending = PendingLease::new(hosted);
    let engine = (!*lock!(hosted.draining))
        .then(|| hosted.running())
        .flatten();
    if let Some(engine) = engine {
        *lock!(hosted.last_used) = Instant::now();
        return Ok(Some(pending.into_lease(engine)));
    }
    drop(pending);
    let retry_at = lock!(hosted.restart).retry_at;
    let wait = retry_at
        .and_then(|at| at.checked_duration_since(Instant::now()))
        .filter(|wait| !wait.is_zero());
    match wait {
        Some(wait) => Err(AcquireError::Unavailable {
            message: format!("Model {} failed and restarts shortly", hosted.id),
            retry_after: Some(wait),
        }),
        None => Ok(None),
    }
}

/// Run directories of an earlier agent hold links and keys nothing uses any more, and the
/// record of each engine that agent may have left running.
fn clear_run_root(run_root: &Path) -> Result<()> {
    super::runtime::private_directory(run_root)?;
    for entry in std::fs::read_dir(run_root)? {
        let path = entry?.path();
        stop_stale_engine(&path);
        if let Err(error) = std::fs::remove_dir_all(&path) {
            tracing::warn!(
                "Remove the stale model run directory {}: {error}",
                path.display()
            );
        }
    }
    Ok(())
}

fn engine_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("Build the engine HTTP client")
}

fn validate_config(settings: &ModelSettings, residency: &Residency) -> Result<()> {
    settings.validate()?;
    residency.validate()?;
    Ok(())
}

fn validate_install(
    model_id: &str,
    spec: &ModelSpec,
    settings: &ModelSettings,
    residency: &Residency,
) -> Result<()> {
    validate_management_id(model_id)?;
    spec.validate()?;
    validate_config(settings, residency)
}

fn new_record(
    model_id: &str,
    spec: &ModelSpec,
    settings: ModelSettings,
    residency: Residency,
    origin: ModelOrigin,
) -> Result<HostedModelRecord> {
    let now = unix_time()?;
    Ok(HostedModelRecord {
        id: model_id.to_owned(),
        spec: spec.clone(),
        settings,
        residency,
        revision: 1,
        origin,
        created_at: now,
        updated_at: now,
        last_used_at: now,
    })
}

fn backoff(failures: u32) -> Duration {
    BACKOFF_BASE
        .saturating_mul(1 << failures.saturating_sub(1).min(16))
        .min(BACKOFF_MAX)
}

/// The run directory of a model; ids may hold dots, so the name is a hash.
fn run_dir_name(model_id: &str) -> String {
    blake3::hash(model_id.as_bytes()).to_hex()[..32].to_owned()
}

fn sorted_digests(spec: &ModelSpec) -> Vec<ModelAssetDigest> {
    let mut digests: Vec<_> = spec
        .assets
        .iter()
        .map(|asset| asset.digest.clone())
        .collect();
    digests.sort();
    digests
}

/// The file a vision model loads as its projector.
fn projector_digest(spec: &ModelSpec) -> Option<&ModelAssetDigest> {
    let name = spec.projector.as_deref()?;
    let projector = spec.assets.iter().find(|asset| asset.file_name == name)?;
    Some(&projector.digest)
}

/// Whether `hosted` serves what `wanted` asks for: the same files with the same engine and
/// kind, the same pooling, and the same file as projector.
fn loads_alike(hosted: &ModelSpec, wanted: &ModelSpec, digests: &[ModelAssetDigest]) -> bool {
    hosted.engine == wanted.engine
        && hosted.kind == wanted.kind
        && hosted.pooling == wanted.pooling
        && projector_digest(hosted) == projector_digest(wanted)
        && sorted_digests(hosted) == digests
}

impl Supervisor {
    pub fn start(
        state_dir: &Path,
        acquisition: AcquisitionManager,
        runtimes: Arc<RuntimeInstaller>,
        launcher: Arc<dyn EngineLauncher>,
        facts: SystemFacts,
        config: SupervisorConfig,
        cancel: CancellationToken,
    ) -> Result<Self> {
        let run_root = state_dir.join("models").join("run");
        clear_run_root(&run_root)?;
        let budget = config
            .memory_budget
            .unwrap_or_else(|| system::memory_budget(&facts));
        let client = engine_client()?;
        let records = acquisition.store().with_db(|db| db.hosted_models())?;
        let models = records
            .into_iter()
            .map(|record| (record.id.clone(), Hosted::new(record)))
            .collect();
        let supervisor = Self {
            inner: Arc::new(Inner {
                acquisition,
                runtimes,
                launcher,
                target: ReleaseTarget::current()?,
                facts: Mutex::new(facts),
                budget: Mutex::new(budget),
                run_root,
                client,
                models: Mutex::new(models),
                admission: AsyncMutex::new(()),
                starting: Mutex::default(),
                config,
            }),
        };
        tokio::spawn(supervisor.clone().monitor(cancel));
        Ok(supervisor)
    }

    pub fn facts(&self) -> SystemFacts {
        lock!(self.inner.facts).clone()
    }

    pub fn set_facts(&self, facts: SystemFacts) {
        if self.inner.config.memory_budget.is_none() {
            *lock!(self.inner.budget) = system::memory_budget(&facts);
        }
        *lock!(self.inner.facts) = facts;
    }

    pub fn memory_budget(&self) -> u64 {
        *lock!(self.inner.budget)
    }

    fn hosted(&self, model_id: &str) -> Option<Arc<Hosted>> {
        lock!(self.inner.models).get(model_id).cloned()
    }

    fn all(&self) -> Vec<Arc<Hosted>> {
        let mut models: Vec<_> = lock!(self.inner.models).values().cloned().collect();
        models.sort_by(|left, right| left.id.cmp(&right.id));
        models
    }

    pub fn model(&self, model_id: &str) -> Option<HostedModel> {
        self.hosted(model_id)
            .map(|hosted| wire(&hosted.record(), self.observed(&hosted)))
    }

    /// Every hosted model by id with its state.
    pub fn models(&self) -> Vec<HostedModel> {
        self.all()
            .iter()
            .map(|hosted| wire(&hosted.record(), self.observed(hosted)))
            .collect()
    }

    /// The published state, with the live memory and slot use of a loaded engine.
    fn observed(&self, hosted: &Hosted) -> HostedModelState {
        let state = hosted.state.borrow().clone();
        let (HostedModelState::Loaded { .. }, Some(running)) = (&state, hosted.running()) else {
            return state;
        };
        let gauges = *lock!(running.gauges);
        let slots_busy = gauges
            .slots_busy
            .unwrap_or(0)
            .max(u8::try_from(hosted.leases.load(Ordering::SeqCst)).unwrap_or(u8::MAX))
            .min(running.slots);
        let vram_bytes = match running.backend {
            Some(ModelBackend::Vulkan | ModelBackend::Cuda) => {
                running.estimate.weights + running.estimate.kv_cache
            }
            _ => 0,
        };
        HostedModelState::Loaded {
            ram_bytes: *lock!(running.ram_bytes),
            vram_bytes,
            slots: running.slots,
            slots_busy,
        }
    }

    pub fn in_flight(&self, model_id: &str) -> usize {
        self.hosted(model_id)
            .map_or(0, |hosted| hosted.leases.load(Ordering::SeqCst))
    }

    pub fn gauges(&self, model_id: &str) -> Option<Gauges> {
        self.hosted(model_id)?
            .running()
            .map(|running| *lock!(running.gauges))
    }

    /// Hosts a new model and starts or joins the acquisition of its assets. Repeating the
    /// same install answers the existing model.
    pub fn install(
        &self,
        model_id: &str,
        spec: ModelSpec,
        settings: ModelSettings,
        residency: Residency,
        origin: ModelOrigin,
    ) -> Result<(HostedModel, ModelAssetSummary)> {
        validate_install(model_id, &spec, &settings, &residency)?;
        self.check_engine(model_id, &spec)?;
        match self.hosted(model_id) {
            Some(existing) => ensure!(
                existing.record().spec == spec,
                "Install model {model_id}: a different model already uses this id"
            ),
            None => self.register(new_record(model_id, &spec, settings, residency, origin)?)?,
        }
        let summary = self.ensure_assets(&spec.assets, None)?;
        let model = self
            .model(model_id)
            .context("The installed model vanished")?;
        Ok((model, summary))
    }

    /// MLX models run on Apple-silicon Macs only, from a directory MLX can load.
    fn check_engine(&self, model_id: &str, spec: &ModelSpec) -> Result<()> {
        if spec.engine != ModelEngine::Mlx {
            return Ok(());
        }
        ensure!(
            self.inner.target == ReleaseTarget::MacosAarch64,
            "Install model {model_id}: MLX models run on Apple-silicon Macs, not on {}",
            self.inner.target.triple()
        );
        engines::mlx::check_layout(spec).with_context(|| format!("Install model {model_id}"))
    }

    /// Persists a new hosted model with a reference on each of its assets.
    fn register(&self, record: HostedModelRecord) -> Result<()> {
        let store = self.inner.acquisition.store();
        let owner = AssetOwner::new(OwnerKind::HostedModel, &record.id)?;
        for asset in &record.spec.assets {
            store.add_ref(&asset.digest, &owner)?;
        }
        store.with_db(|db| db.put_hosted_model(&record))?;
        lock!(self.inner.models).insert(record.id.clone(), Hosted::new(record));
        Ok(())
    }

    /// A hosted model serving exactly these assets with this engine, created on demand with
    /// the device's defaults; placements share the user's tuned instance this way. The model
    /// counts as used, so it is not released as unused while the placement starts on it.
    pub fn ensure_hosted(&self, spec: ModelSpec) -> Result<String> {
        spec.validate()?;
        let now = unix_time()?;
        if let Some(found) = self.serving(&spec, now) {
            let _ = self
                .inner
                .acquisition
                .store()
                .with_db(|db| db.touch_hosted_model(&found, now));
            return Ok(found);
        }
        let mut hash = blake3::Hasher::new();
        hash.update(serde_json::to_string(&spec)?.as_bytes());
        let model_id = format!("auto-{}", &hash.finalize().to_hex()[..16]);
        self.install(
            &model_id,
            spec,
            ModelSettings::default(),
            Residency::default(),
            ModelOrigin::Placement,
        )?;
        Ok(model_id)
    }

    /// The hosted model with exactly these files, loaded the same way, marked used at `now`
    /// under the registry lock that `release_if_unused` takes too.
    fn serving(&self, spec: &ModelSpec, now: i64) -> Option<String> {
        let wanted = sorted_digests(spec);
        let models = lock!(self.inner.models);
        let found = models
            .values()
            .filter(|hosted| loads_alike(&hosted.record().spec, spec, &wanted))
            .min_by(|left, right| left.id.cmp(&right.id))?;
        found.touch(now);
        Some(found.id.clone())
    }

    fn ensure_assets(
        &self,
        assets: &[ModelAssetDescriptor],
        operation_id: Option<&str>,
    ) -> Result<ModelAssetSummary> {
        let mut summary = ModelAssetSummary {
            total: u32::try_from(assets.len()).unwrap_or(u32::MAX),
            present: 0,
            pending: Vec::new(),
        };
        for asset in assets {
            let status = self.inner.acquisition.ensure(asset, operation_id)?;
            if status.state == ModelAssetState::Present {
                summary.present += 1;
            } else if summary.pending.len() < MODEL_MAX_PENDING_ASSETS {
                summary.pending.push(status);
            }
        }
        Ok(summary)
    }

    pub async fn configure(
        &self,
        model_id: &str,
        expected_revision: u64,
        settings: ModelSettings,
        residency: Residency,
    ) -> Result<HostedModel> {
        validate_config(&settings, &residency)?;
        let hosted = self
            .hosted(model_id)
            .with_context(|| format!("Configure model {model_id}: it is not hosted"))?;
        let restart = self.revise(&hosted, expected_revision, settings, residency)?;
        *lock!(hosted.restart) = Restart::default();
        if residency == Residency::PinnedOff || (restart && hosted.running().is_some()) {
            self.drain_and_stop(&hosted).await;
        }
        Ok(wire(&hosted.record(), self.observed(&hosted)))
    }

    /// Stores the next revision; answers whether a running engine must restart for it. A model
    /// hosted for placements becomes the user's once they configure it, so it stays hosted when
    /// no placement uses it any more.
    fn revise(
        &self,
        hosted: &Hosted,
        expected_revision: u64,
        settings: ModelSettings,
        residency: Residency,
    ) -> Result<bool> {
        let mut record = lock!(hosted.record);
        ensure!(
            record.revision == expected_revision,
            "Configure model {}: it is at revision {}, not {expected_revision}",
            hosted.id,
            record.revision
        );
        let restart = record.settings != settings;
        record.settings = settings;
        record.residency = residency;
        record.origin = ModelOrigin::User;
        record.revision += 1;
        record.updated_at = unix_time()?;
        self.inner
            .acquisition
            .store()
            .with_db(|db| db.put_hosted_model(&record))?;
        Ok(restart)
    }

    /// Starts loading in the background; the answer shows `loading`.
    pub fn load(&self, model_id: &str) -> Result<HostedModel> {
        let hosted = self
            .hosted(model_id)
            .with_context(|| format!("Load model {model_id}: it is not hosted"))?;
        ensure!(
            hosted.record().residency != Residency::PinnedOff,
            "Load model {model_id}: it is pinned off"
        );
        *lock!(hosted.restart) = Restart::default();
        if hosted.running().is_none() {
            hosted.set_state(HostedModelState::Loading);
            self.start_load(&hosted);
        }
        Ok(wire(&hosted.record(), self.observed(&hosted)))
    }

    /// Lets in-flight requests finish, refusing new ones, then stops the engine.
    pub async fn unload(&self, model_id: &str) -> Result<HostedModel> {
        let hosted = self
            .hosted(model_id)
            .with_context(|| format!("Unload model {model_id}: it is not hosted"))?;
        self.drain_and_stop(&hosted).await;
        Ok(wire(&hosted.record(), self.observed(&hosted)))
    }

    pub async fn remove(&self, model_id: &str, expected_revision: u64) -> Result<()> {
        let hosted = self
            .hosted(model_id)
            .with_context(|| format!("Remove model {model_id}: it is not hosted"))?;
        let record = hosted.record();
        ensure!(
            record.revision == expected_revision,
            "Remove model {model_id}: it is at revision {}, not {expected_revision}",
            record.revision
        );
        self.drain_and_stop(&hosted).await;
        lock!(self.inner.models).remove(model_id);
        self.forget(&record)
    }

    /// Whether a loaded model runs from this runtime slot.
    pub fn runtime_in_use(&self, runtime: ModelRuntime, backend: ModelBackend) -> Option<String> {
        let engine = match runtime {
            ModelRuntime::Llamacpp => ModelEngine::Llamacpp,
            ModelRuntime::Mlx => ModelEngine::Mlx,
        };
        self.all().into_iter().find_map(|hosted| {
            hosted
                .running()
                .filter(|running| running.engine == engine && running.backend == Some(backend))
                .map(|_| hosted.id.clone())
        })
    }

    /// New requests wait for the stop, requests in flight finish first.
    async fn drain_and_stop(&self, hosted: &Arc<Hosted>) {
        let _transition = hosted.transition.lock().await;
        let _draining = Draining::start(hosted);
        while hosted.leases.load(Ordering::SeqCst) > 0 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        self.stop_engine(hosted).await;
    }

    /// Stops the engine unless a request holds it. The caller holds the transition lock.
    /// `acquire` raises the lease count before it reads `draining`, and this sets `draining`
    /// before it reads the count, so a request either sees the drain or keeps the engine.
    async fn stop_if_idle(&self, hosted: &Arc<Hosted>) -> bool {
        let _draining = Draining::start(hosted);
        let idle = hosted.leases.load(Ordering::SeqCst) == 0;
        if idle {
            self.stop_engine(hosted).await;
        }
        idle
    }

    async fn stop_engine(&self, hosted: &Hosted) {
        let Some(running) = lock!(hosted.engine).take() else {
            hosted.show_stopped();
            return;
        };
        hosted.set_state(HostedModelState::Unloading);
        if let Some(process) = running.process.lock().await.take() {
            process.stop().await;
        }
        let dir = self.inner.run_root.join(run_dir_name(&hosted.id));
        if let Err(error) = std::fs::remove_dir_all(&dir)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("Remove the run directory of {}: {error}", hosted.id);
        }
        if let Ok(now) = unix_time() {
            hosted.touch(now);
            let _ = self
                .inner
                .acquisition
                .store()
                .with_db(|db| db.touch_hosted_model(&hosted.id, now));
        }
        hosted.set_state(HostedModelState::Stopped);
    }

    /// A lease on the model's engine, loading it first when needed. The request counts as
    /// in flight while it holds the lease, so the model is never unloaded under it.
    pub async fn acquire(&self, model_id: &str) -> std::result::Result<EngineLease, AcquireError> {
        loop {
            let hosted = self
                .hosted(model_id)
                .ok_or_else(|| AcquireError::NotFound(model_id.to_owned()))?;
            if let Some(lease) = lease(&hosted)? {
                return Ok(lease);
            }
            self.loaded(&hosted).await.map_err(AcquireError::Failed)?;
        }
    }

    /// Waits for the model's load, which starts now unless one runs.
    async fn loaded(&self, hosted: &Arc<Hosted>) -> Result<()> {
        let mut load = self.start_load(hosted);
        let outcome = load
            .wait_for(Option::is_some)
            .await
            .map(|outcome| outcome.clone())
            .map_err(|_| anyhow::anyhow!("The load of model {} stopped", hosted.id))?;
        outcome.unwrap_or(Ok(())).map_err(anyhow::Error::msg)
    }

    /// Starts loading the model unless a load runs already. The load runs on its own task, so a
    /// request that gives up never cancels it, nor the evictions made for it.
    fn start_load(&self, hosted: &Arc<Hosted>) -> watch::Receiver<Option<LoadOutcome>> {
        let mut loading = lock!(hosted.loading);
        if let Some(running) = loading.as_ref().filter(|load| load.has_changed().is_ok()) {
            return running.clone();
        }
        let (sender, load) = watch::channel(None);
        *loading = Some(load.clone());
        drop(loading);
        let supervisor = self.clone();
        let hosted = Arc::clone(hosted);
        tokio::spawn(async move {
            let outcome = supervisor.ensure_loaded(&hosted).await.map(drop);
            if let Err(error) = &outcome {
                tracing::warn!(model = %hosted.id, "Load the model: {error:#}");
            }
            sender.send_replace(Some(outcome.map_err(|error| format!("{error:#}"))));
            *lock!(hosted.loading) = None;
        });
        load
    }

    /// Waits for any running transition, then loads the engine unless it runs.
    async fn ensure_loaded(&self, hosted: &Arc<Hosted>) -> Result<Arc<Running>> {
        let _transition = hosted.transition.lock().await;
        if let Some(running) = hosted.running() {
            return Ok(running);
        }
        if let Err(refused) = self.still_wanted(hosted) {
            hosted.show_stopped();
            return Err(refused);
        }
        match self.load_engine(hosted).await {
            Ok((running, reserved)) => {
                hosted.serve(&running);
                drop(reserved);
                Ok(running)
            }
            Err((reason, error)) => {
                hosted.failed(reason);
                Err(error)
            }
        }
    }

    /// A model pinned off or removed while its load waited for the transition stays stopped.
    fn still_wanted(&self, hosted: &Arc<Hosted>) -> Result<()> {
        let current = self
            .hosted(&hosted.id)
            .is_some_and(|current| Arc::ptr_eq(&current, hosted));
        ensure!(current, "Model {} is no longer hosted", hosted.id);
        ensure!(
            hosted.record().residency != Residency::PinnedOff,
            "Model {} is pinned off on this device",
            hosted.id
        );
        Ok(())
    }

    /// The running engine and the memory admitted for it, which the caller holds until the
    /// model serves from the engine.
    async fn load_engine(&self, hosted: &Arc<Hosted>) -> Loaded<(Arc<Running>, Reserved)> {
        let record = hosted.record();
        let prepared = self.prepare(hosted, &record).await?;
        let backend = prepared
            .runtime
            .as_ref()
            .map(|runtime| runtime.record.backend);
        let discrete = on_discrete_gpu(backend);
        loop {
            let plan = self
                .plan(&record, &prepared, *lock!(hosted.chatml))
                .map_err(failure(ModelHostFailure::RuntimeMissing))?;
            let reserved = self
                .reserve(&hosted.id, plan.estimate.total(), discrete)
                .await
                .map_err(failure(ModelHostFailure::InsufficientMemory))?;
            let started = self.spawn(&record, plan, prepared.runtime.as_ref()).await;
            match started {
                Ok(running) if self.wants_chatml(hosted, &running).await => {
                    self.restart_with_chatml(hosted, &running).await;
                }
                Ok(running) => return Ok((running, reserved)),
                Err((failure, error)) => return Err((start_failure(&failure), error)),
            }
        }
    }

    /// Assets, runtime and the run directory of a model, in that order.
    async fn prepare(&self, hosted: &Hosted, record: &HostedModelRecord) -> Loaded<Prepared> {
        hosted.set_state(HostedModelState::Acquiring);
        let blobs = self
            .assets_present(&record.spec)
            .await
            .map_err(failure(ModelHostFailure::AssetMissing))?;
        hosted.set_state(HostedModelState::Loading);
        let runtime = self
            .runtime_for(record)
            .await
            .map_err(failure(ModelHostFailure::RuntimeMissing))?;
        let files = self
            .prepare_files(record, &blobs)
            .map_err(failure(ModelHostFailure::AssetMissing))?;
        let header = match record.spec.engine {
            ModelEngine::Llamacpp => llamacpp::weights_file(&record.spec)
                .map(|name| gguf::read(&files.path(name)).unwrap_or_default())
                .unwrap_or_default(),
            ModelEngine::Mlx => engines::mlx::config_facts(&files.dir),
            ModelEngine::Onnx => GgufFacts::default(),
        };
        Ok(Prepared {
            files,
            header,
            runtime,
        })
    }

    async fn runtime_for(&self, record: &HostedModelRecord) -> Result<Option<InstalledRuntime>> {
        match record.spec.engine {
            ModelEngine::Llamacpp => {
                let backend = system::recommended_backend(self.inner.target, &self.facts().gpus);
                self.pack(ModelRuntime::Llamacpp, backend).await.map(Some)
            }
            ModelEngine::Mlx => self
                .pack(ModelRuntime::Mlx, ModelBackend::Metal)
                .await
                .map(Some),
            ModelEngine::Onnx => Ok(None),
        }
    }

    async fn restart_with_chatml(&self, hosted: &Hosted, running: &Running) {
        if let Some(process) = running.process.lock().await.take() {
            process.stop().await;
        }
        *lock!(hosted.chatml) = true;
    }

    /// The blob of each asset, acquiring missing ones download-first.
    async fn assets_present(&self, spec: &ModelSpec) -> Result<Vec<PathBuf>> {
        let mut blobs = Vec::new();
        for asset in &spec.assets {
            blobs.push(self.asset_present(spec, asset).await?);
        }
        Ok(blobs)
    }

    async fn asset_present(
        &self,
        spec: &ModelSpec,
        asset: &ModelAssetDescriptor,
    ) -> Result<PathBuf> {
        let acquisition = &self.inner.acquisition;
        let mut state = acquisition.ensure(asset, None)?.state;
        if state != ModelAssetState::Present {
            state = acquisition.settled(&asset.digest).await?;
        }
        ensure!(
            state == ModelAssetState::Present,
            "Asset {} of model {} is missing and could not be acquired ({state:?})",
            asset.file_name,
            spec.display_name
        );
        acquisition
            .store()
            .path_of(&asset.digest)?
            .with_context(|| {
                format!(
                    "Asset {} of model {} left the store",
                    asset.file_name, spec.display_name
                )
            })
    }

    /// The best installed pack of `runtime`, else its `backend` pack installed now.
    async fn pack(&self, runtime: ModelRuntime, backend: ModelBackend) -> Result<InstalledRuntime> {
        if let Some(installed) = self
            .inner
            .runtimes
            .installed_of(runtime)?
            .into_iter()
            .next()
        {
            return Ok(installed);
        }
        self.inner.runtimes.install(runtime, backend).await?;
        self.inner.runtimes.wait(runtime, backend).await
    }

    /// Links each asset under the name the engine expects; the key comes with the plan.
    fn prepare_files(&self, record: &HostedModelRecord, blobs: &[PathBuf]) -> Result<ModelFiles> {
        let dir = self.inner.run_root.join(run_dir_name(&record.id));
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        super::runtime::private_directory(&dir)?;
        for (asset, blob) in record.spec.assets.iter().zip(blobs) {
            link_asset(&dir.join(&asset.file_name), blob).with_context(|| {
                format!("Link asset {} of model {}", asset.file_name, record.id)
            })?;
        }
        Ok(ModelFiles {
            dir,
            blobs: blobs.to_vec(),
            sizes: record.spec.assets.iter().map(|asset| asset.size).sum(),
        })
    }

    fn plan(
        &self,
        record: &HostedModelRecord,
        prepared: &Prepared,
        chatml: bool,
    ) -> Result<EnginePlan> {
        match (record.spec.engine, &prepared.runtime) {
            (ModelEngine::Llamacpp, Some(runtime)) => {
                self.llama_plan(record, prepared, runtime, chatml)
            }
            (ModelEngine::Mlx, Some(runtime)) => engines::mlx::plan(
                &record.id,
                &record.spec,
                &record.settings,
                &prepared.header,
                &prepared.files,
                runtime,
            ),
            #[cfg(feature = "runtime")]
            (ModelEngine::Onnx, _) => {
                engines::onnx::plan(&record.id, &record.spec, &prepared.files)
            }
            (engine, _) => bail!("Model {}: no engine serves {engine:?} here", record.id),
        }
    }

    fn llama_plan(
        &self,
        record: &HostedModelRecord,
        prepared: &Prepared,
        runtime: &InstalledRuntime,
        chatml: bool,
    ) -> Result<EnginePlan> {
        let effective = llamacpp::effective(
            &record.settings,
            record.spec.kind,
            &prepared.header,
            prepared.files.sizes,
            runtime.record.backend,
            &self.facts(),
            self.memory_budget(),
        );
        let key_file = prepared.files.dir.join(KEY_FILE);
        let plan = llamacpp::plan(LlamaInputs {
            model_id: &record.id,
            spec: &record.spec,
            effective,
            header: &prepared.header,
            files: &prepared.files,
            runtime,
            key_file: &key_file,
            chatml,
        })?;
        write_private(
            &key_file,
            &Zeroizing::new(format!("{}\n", plan.key.as_str())),
        )?;
        Ok(plan)
    }

    /// Admits a starting engine of `needed` bytes and holds them for it. The admission lock
    /// covers the decision and its evictions only, so a slow start never holds up other loads.
    async fn reserve(&self, model_id: &str, needed: u64, discrete: bool) -> Result<Reserved> {
        let _admission = self.inner.admission.lock().await;
        self.admit(model_id, needed, discrete).await?;
        lock!(self.inner.starting).insert(model_id.to_owned(), needed);
        Ok(Reserved {
            inner: Arc::clone(&self.inner),
            model_id: model_id.to_owned(),
        })
    }

    /// Frees memory for `needed` bytes by unloading idle on-demand models, least recently
    /// used first, and refuses when even that is not enough. The engines of other models count
    /// against the budget by their estimates, and the device's free memory is read on every
    /// pass, so memory other programs use counts too.
    async fn admit(&self, model_id: &str, needed: u64, discrete: bool) -> Result<()> {
        let budget = self.memory_budget();
        ensure!(
            needed <= budget,
            "Model {model_id} needs {needed} bytes, more than this device's model memory budget of {budget}"
        );
        loop {
            let others = self.loaded_besides(model_id);
            let starting: u64 = lock!(self.inner.starting)
                .iter()
                .filter(|(id, _)| id.as_str() != model_id)
                .map(|(_, bytes)| bytes)
                .sum();
            let reserved: u64 = others
                .iter()
                .map(|(_, running)| running.estimate.total())
                .sum::<u64>()
                .saturating_add(starting);
            let free = self.free_memory(discrete, &others).saturating_sub(starting);
            if needed <= room(budget, reserved, free) {
                return Ok(());
            }
            if !self.evict_one(model_id, others).await {
                bail!(
                    "Model {model_id} needs {needed} bytes, but loaded and starting models hold {reserved} of the {budget} byte budget, the device has {free} bytes free, and none is idle"
                );
            }
        }
    }

    /// The loaded engines of every other model.
    fn loaded_besides(&self, model_id: &str) -> Vec<(Arc<Hosted>, Arc<Running>)> {
        self.all()
            .into_iter()
            .filter(|hosted| hosted.id != model_id)
            .filter_map(|hosted| hosted.running().map(|running| (hosted, running)))
            .collect()
    }

    /// Memory a new engine finds free now: available RAM, plus the discrete GPU memory the
    /// loaded engines leave when the new engine runs on such a GPU.
    fn free_memory(&self, discrete: bool, loaded: &[(Arc<Hosted>, Arc<Running>)]) -> u64 {
        let ram = self
            .inner
            .config
            .free_memory
            .unwrap_or_else(|| system::ram().free);
        if !discrete {
            return ram;
        }
        let held = loaded
            .iter()
            .filter(|(_, running)| on_discrete_gpu(running.backend))
            .map(|(_, running)| {
                running
                    .estimate
                    .weights
                    .saturating_add(running.estimate.kv_cache)
            })
            .fold(0, u64::saturating_add);
        let gpus = system::discrete_memory(&self.facts());
        ram.saturating_add(gpus.saturating_sub(held))
    }

    /// Unloads the least recently used idle on-demand model; false when none could go.
    async fn evict_one(&self, model_id: &str, loaded: Vec<(Arc<Hosted>, Arc<Running>)>) -> bool {
        let mut victims: Vec<_> = loaded
            .into_iter()
            .map(|(hosted, _)| hosted)
            .filter(|hosted| {
                matches!(hosted.record().residency, Residency::OnDemand { .. })
                    && hosted.leases.load(Ordering::SeqCst) == 0
            })
            .collect();
        victims.sort_by_key(|hosted| std::cmp::Reverse(hosted.idle_for()));
        for victim in victims {
            let Ok(_transition) = victim.transition.try_lock() else {
                continue;
            };
            if self.stop_if_idle(&victim).await {
                tracing::info!(model = %victim.id, "Unloaded the least recently used model to make room for {model_id}");
                return true;
            }
        }
        false
    }

    async fn spawn(
        &self,
        record: &HostedModelRecord,
        plan: EnginePlan,
        runtime: Option<&InstalledRuntime>,
    ) -> std::result::Result<Arc<Running>, (StartFailure, anyhow::Error)> {
        let model_id = record.id.as_str();
        let mut process =
            EngineProcess::spawn(self.inner.launcher.as_ref(), &plan.launch, model_id)
                .await
                .map_err(|error| (StartFailure::Exited(exit_failure()), error))?;
        let dir = self.inner.run_root.join(run_dir_name(model_id));
        if let Some(pid) = process.id()
            && let Err(error) = record_engine(&dir, pid)
        {
            tracing::warn!(model = %model_id, "Record the engine process: {error:#}");
        }
        let timeout = engines::ready_timeout(plan.estimate.weights);
        let ready = process
            .wait_ready(&self.inner.client, &plan.key, timeout)
            .await;
        let base_url = match ready {
            Ok(base_url) => base_url,
            Err(failure) => {
                let output = process.output();
                process.stop().await;
                let error = engines::start_error(model_id, &failure, &output);
                return Err((failure, error));
            }
        };
        let gauges = Gauges {
            slots: Some(plan.slots),
            ..Gauges::default()
        };
        Ok(Arc::new(Running {
            base_url,
            process: AsyncMutex::new(Some(process)),
            key: plan.key,
            estimate: plan.estimate,
            slots: plan.slots,
            engine: record.spec.engine,
            backend: runtime.map(|runtime| runtime.record.backend),
            started: Instant::now(),
            gauges: Mutex::new(gauges),
            ram_bytes: Mutex::new(0),
            health_failures: Mutex::new(0),
        }))
    }

    /// A chat model without a tool-capable template restarts once with chatml.
    async fn wants_chatml(&self, hosted: &Hosted, running: &Running) -> bool {
        let record = hosted.record();
        if *lock!(hosted.chatml)
            || record.spec.engine != ModelEngine::Llamacpp
            || record.spec.kind != flow_like_device_protocol::ModelKind::Chat
            || record.spec.projector.is_some()
        {
            return false;
        }
        self.props(running)
            .await
            .is_some_and(|props| !llamacpp::props_support_tools(&props))
    }

    /// `/props` of a llama.cpp engine; `None` when it does not answer one.
    async fn props(&self, running: &Running) -> Option<serde_json::Value> {
        let response = self
            .inner
            .client
            .get(format!("{}/props", running.base_url))
            .bearer_auth(running.key.as_str())
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .ok()?;
        if !response.status().is_success() {
            return None;
        }
        response.json().await.ok()
    }

    async fn monitor(self, cancel: CancellationToken) {
        let mut tick = tokio::time::interval(self.inner.config.monitor_interval);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                () = cancel.cancelled() => break,
                _ = tick.tick() => {}
            }
            self.check(Instant::now()).await;
        }
        for hosted in self.all() {
            self.stop_engine(&hosted).await;
        }
    }

    /// One monitor pass: health and gauges of loaded engines, restarts after crashes,
    /// idle unloads, always-on loads, and models no placement uses any more.
    pub async fn check(&self, now: Instant) {
        for hosted in self.all() {
            if let Some(running) = hosted.running() {
                self.check_running(&hosted, &running).await;
            }
            self.reap_idle(&hosted, now).await;
            self.keep_resident(&hosted, now);
        }
        self.release_unused();
    }

    async fn check_running(&self, hosted: &Arc<Hosted>, running: &Arc<Running>) {
        let exited = {
            let mut process = running.process.lock().await;
            let status = process.as_mut().and_then(EngineProcess::exited);
            status.map(|status| {
                let output = process.as_ref().map(EngineProcess::output);
                (status, output.unwrap_or_default())
            })
        };
        if let Some((status, output)) = exited {
            tracing::warn!(model = %hosted.id, "The engine exited ({status}): {output}");
            self.crashed(hosted, running, ModelHostFailure::EngineExited)
                .await;
            return;
        }
        let healthy = self
            .inner
            .client
            .get(format!("{}/health", running.base_url))
            .bearer_auth(running.key.as_str())
            .timeout(Duration::from_secs(5))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success());
        let failures = {
            let mut failures = lock!(running.health_failures);
            *failures = if healthy { 0 } else { *failures + 1 };
            *failures
        };
        if failures >= HEALTH_FAILURES_BEFORE_RESTART {
            tracing::warn!(model = %hosted.id, "The engine failed {failures} health checks");
            self.crashed(hosted, running, ModelHostFailure::HealthTimeout)
                .await;
            return;
        }
        if running.started.elapsed() > STABLE_AFTER {
            lock!(hosted.restart).failures = 0;
        }
        self.scrape(running).await;
    }

    /// A request could not reach the model's engine: checks it now, not at the next pass.
    pub fn engine_unreachable(&self, model_id: &str) {
        let Some(hosted) = self.hosted(model_id) else {
            return;
        };
        let Some(running) = hosted.running() else {
            return;
        };
        let supervisor = self.clone();
        tokio::spawn(async move { supervisor.check_running(&hosted, &running).await });
    }

    /// Counts one failure per engine, however many checks noticed it.
    async fn crashed(
        &self,
        hosted: &Arc<Hosted>,
        running: &Arc<Running>,
        reason: ModelHostFailure,
    ) {
        let _transition = hosted.transition.lock().await;
        if !hosted
            .running()
            .is_some_and(|current| Arc::ptr_eq(&current, running))
        {
            return;
        }
        self.stop_engine(hosted).await;
        hosted.failed(reason);
    }

    async fn scrape(&self, running: &Running) {
        let pid = running
            .process
            .lock()
            .await
            .as_ref()
            .and_then(EngineProcess::id);
        if let Some(pid) = pid {
            *lock!(running.ram_bytes) = resident_bytes(pid);
        }
        if running.engine != ModelEngine::Llamacpp {
            return;
        }
        let get = |path: &str| {
            self.inner
                .client
                .get(format!("{}{path}", running.base_url))
                .bearer_auth(running.key.as_str())
                .timeout(Duration::from_secs(5))
                .send()
        };
        let mut gauges = *lock!(running.gauges);
        if let Ok(response) = get("/metrics").await
            && let Ok(text) = response.text().await
        {
            llamacpp::parse_metrics(&text, &mut gauges);
        }
        if let Ok(response) = get("/slots").await
            && let Ok(slots) = response.json::<serde_json::Value>().await
        {
            llamacpp::parse_slots(&slots, &mut gauges);
        }
        *lock!(running.gauges) = gauges;
    }

    /// Unloads an on-demand model idle past its delay; never one serving a request.
    async fn reap_idle(&self, hosted: &Arc<Hosted>, now: Instant) {
        let Residency::OnDemand {
            idle_unload_after_seconds,
        } = hosted.record().residency
        else {
            return;
        };
        let idle_since = *lock!(hosted.last_used);
        let idle = now.saturating_duration_since(idle_since);
        if hosted.running().is_none()
            || hosted.leases.load(Ordering::SeqCst) > 0
            || idle < Duration::from_secs(u64::from(idle_unload_after_seconds))
        {
            return;
        }
        let _transition = hosted.transition.lock().await;
        if *lock!(hosted.last_used) == idle_since && self.stop_if_idle(hosted).await {
            tracing::info!(model = %hosted.id, "Unloaded the model after {}s idle", idle.as_secs());
        }
    }

    /// Always-on models load at start and come back after a crash once their backoff ends.
    fn keep_resident(&self, hosted: &Arc<Hosted>, now: Instant) {
        if hosted.record().residency != Residency::AlwaysOn
            || hosted.running().is_some()
            || lock!(hosted.restart)
                .retry_at
                .is_some_and(|retry_at| retry_at > now)
            || hosted.transition.try_lock().is_err()
        {
            return;
        }
        self.start_load(hosted);
    }

    /// Models hosted for placements go once no placement references all of their files and
    /// they were unused for a while; the store then keeps the files for its own grace period.
    fn release_unused(&self) {
        let Ok(now) = unix_time() else {
            return;
        };
        let unused: Vec<_> = self
            .all()
            .into_iter()
            .filter(|hosted| self.unused(hosted, now))
            .collect();
        if unused.is_empty() {
            return;
        }
        let referenced = match self.placement_references() {
            Ok(referenced) => referenced,
            Err(error) => {
                tracing::warn!("Read which model files placements use: {error:#}");
                return;
            }
        };
        for hosted in unused {
            let files = hosted.record().spec.assets;
            let used = referenced
                .values()
                .any(|digests| files.iter().all(|asset| digests.contains(&asset.digest)));
            if used {
                continue;
            }
            match self.release_if_unused(&hosted.id, now) {
                Ok(true) => {
                    tracing::info!(model = %hosted.id, "Removed a model hosted for placements that no longer use it")
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(model = %hosted.id, "Remove a model no placement uses: {error:#}")
                }
            }
        }
    }

    /// Hosted for placements, neither loaded nor loading, and unused for the release delay.
    fn unused(&self, hosted: &Hosted, now: i64) -> bool {
        let record = hosted.record();
        let delay = self.inner.config.release_unused_after.as_secs();
        record.origin == ModelOrigin::Placement
            && hosted.running().is_none()
            && lock!(hosted.loading).is_none()
            && hosted.leases.load(Ordering::SeqCst) == 0
            && now.saturating_sub(record.last_used_at) >= i64::try_from(delay).unwrap_or(i64::MAX)
    }

    /// The model-store files each placement references, by placement.
    fn placement_references(&self) -> Result<HashMap<String, HashSet<ModelAssetDigest>>> {
        let references = self
            .inner
            .acquisition
            .store()
            .with_db(|db| db.refs_of_kind(OwnerKind::Placement))?;
        let mut by_placement: HashMap<String, HashSet<ModelAssetDigest>> = HashMap::new();
        for (digest, owner) in references {
            by_placement.entry(owner.id).or_default().insert(digest);
        }
        Ok(by_placement)
    }

    /// Removes a model while it is still unused. `ensure_hosted` marks the model it answers as
    /// used under the same lock, so a placement is never handed a model this removes.
    fn release_if_unused(&self, model_id: &str, now: i64) -> Result<bool> {
        let mut models = lock!(self.inner.models);
        let Some(hosted) = models
            .get(model_id)
            .filter(|hosted| self.unused(hosted, now))
        else {
            return Ok(false);
        };
        let record = hosted.record();
        models.remove(model_id);
        self.forget(&record)?;
        Ok(true)
    }

    /// Deletes a hosted model's record and its references on its files.
    fn forget(&self, record: &HostedModelRecord) -> Result<()> {
        let store = self.inner.acquisition.store();
        store.with_db(|db| db.delete_hosted_model(&record.id))?;
        let owner = AssetOwner::new(OwnerKind::HostedModel, &record.id)?;
        for asset in &record.spec.assets {
            store.remove_ref(&asset.digest, &owner)?;
        }
        Ok(())
    }
}

fn link_asset(link: &Path, blob: &Path) -> Result<()> {
    if let Some(parent) = link.parent() {
        super::runtime::private_directory(parent)?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(blob, link)?;
        Ok(())
    }
    #[cfg(not(unix))]
    bail!("Linking {} needs Unix", blob.display())
}

/// Replaces the file an earlier start of the same model wrote, readable by the agent only.
fn write_private(path: &Path, text: &str) -> Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(error).with_context(|| format!("Replace {}", path.display()));
        }
        _ => {}
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("Write {}", path.display()))?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    Ok(())
}

/// When a process started, in seconds since the epoch; `None` once it is gone.
fn process_started(pid: u32) -> Option<u64> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let pid = Pid::from_u32(pid);
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system.process(pid).map(sysinfo::Process::start_time)
}

/// Records the engine's process with its start time, which a reused pid does not share.
fn record_engine(dir: &Path, pid: u32) -> Result<()> {
    let started = process_started(pid).context("Read the start time of the engine")?;
    write_private(&dir.join(ENGINE_PID_FILE), &format!("{pid} {started}\n"))
}

fn engine_record(text: &str) -> Option<(u32, u64)> {
    let (pid, started) = text.trim().split_once(' ')?;
    Some((pid.parse().ok()?, started.parse().ok()?))
}

/// Stops the engine an earlier agent recorded in `dir` while it still runs: on macOS nothing
/// ends an engine with its agent, and its model would not fit a second time.
fn stop_stale_engine(dir: &Path) {
    let recorded = std::fs::read_to_string(dir.join(ENGINE_PID_FILE)).ok();
    let Some((pid, started)) = recorded.as_deref().and_then(engine_record) else {
        return;
    };
    if process_started(pid) != Some(started) {
        return;
    }
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: the pid runs the engine recorded with this start time, not a reused pid.
        unsafe { libc::kill(pid, libc::SIGKILL) };
        tracing::warn!(
            pid,
            "Stopped a model engine an earlier run of the agent left running"
        );
    }
}

fn exit_failure() -> std::process::ExitStatus {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(1 << 8)
    }
    #[cfg(not(unix))]
    {
        std::process::Command::new("false")
            .status()
            .unwrap_or_default()
    }
}

/// Resident bytes of an engine and every process below it: the engine sandbox runs the engine
/// as a grandchild, and the MLX worker runs the helper that holds the weights.
fn resident_bytes(pid: u32) -> u64 {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_memory(),
    );
    let processes: Vec<_> = system
        .processes()
        .values()
        .map(|process| {
            let parent = process.parent().map(|parent| parent.as_u32());
            (process.pid().as_u32(), parent, process.memory())
        })
        .collect();
    tree_total(&processes, pid)
}

/// The memory of `root` and its descendants among `(pid, parent, memory)` rows.
fn tree_total(processes: &[(u32, Option<u32>, u64)], root: u32) -> u64 {
    let mut total = 0;
    let mut pending = vec![root];
    let mut seen = HashSet::new();
    while let Some(pid) = pending.pop() {
        if !seen.insert(pid) {
            continue;
        }
        for &(process, parent, memory) in processes {
            if process == pid {
                total += memory;
            }
            if parent == Some(pid) {
                pending.push(process);
            }
        }
    }
    total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_engine_fits_both_the_budget_and_the_free_memory() {
        let gib = 1 << 30;
        assert_eq!(
            room(12 * gib, 2 * gib, 20 * gib),
            10 * gib,
            "the budget binds"
        );
        assert_eq!(
            room(12 * gib, 2 * gib, 3 * gib),
            3 * gib,
            "other programs hold memory"
        );
        assert_eq!(room(12 * gib, 13 * gib, 20 * gib), 0);
        assert!(on_discrete_gpu(Some(ModelBackend::Cuda)));
        assert!(!on_discrete_gpu(Some(ModelBackend::Metal)) && !on_discrete_gpu(None));
    }

    #[test]
    fn engine_memory_counts_the_processes_below_the_engine() {
        let rows = [
            (10, Some(1), 5),
            (11, Some(10), 7),
            (12, Some(11), 100),
            (13, Some(1), 1_000),
            (14, Some(14), 3),
        ];
        assert_eq!(tree_total(&rows, 10), 112);
        assert_eq!(tree_total(&rows, 13), 1_000);
        assert_eq!(tree_total(&rows, 14), 3);
        assert_eq!(tree_total(&rows, 99), 0);
        assert!(resident_bytes(std::process::id()) > 0);
    }

    fn exits_within(child: &mut std::process::Child, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    /// A process standing in for an engine an earlier agent left behind, and its run directory.
    fn left_behind(root: &Path, name: &str) -> (std::process::Child, PathBuf) {
        let dir = root.join(name);
        std::fs::create_dir(&dir).expect("a run directory");
        let process = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .expect("a process");
        (process, dir)
    }

    #[cfg(unix)]
    #[test]
    fn a_new_supervisor_stops_the_engines_an_earlier_agent_left_running() {
        let root = tempfile::tempdir().expect("a run root");
        let (mut left, recorded) = left_behind(root.path(), "left");
        let (mut reused, misrecorded) = left_behind(root.path(), "reused");
        record_engine(&recorded, left.id()).expect("a record");
        let started = process_started(reused.id()).expect("a running process");
        let other_start = format!("{} {}\n", reused.id(), started + 1);
        write_private(&misrecorded.join(ENGINE_PID_FILE), &other_start).expect("a record");
        clear_run_root(root.path()).expect("a cleared run root");
        let stopped = exits_within(&mut left, Duration::from_secs(5));
        let untouched = matches!(reused.try_wait(), Ok(None));
        for child in [&mut left, &mut reused] {
            let _ = child.kill();
            let _ = child.wait();
        }
        assert!(stopped, "the recorded engine still runs");
        assert!(untouched, "a process that started later is not the engine");
        let remaining = std::fs::read_dir(root.path()).map(Iterator::count);
        assert_eq!(remaining.ok(), Some(0));
    }
}
