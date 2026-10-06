//! On-device model hosting: content-addressed model assets and how a device acquires them.

use crate::{ProtocolError, Result, validate_artifact_relative_path};
use serde::{Deserialize, Serialize};

pub const MODEL_ASSET_MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;
pub const MODEL_ASSET_MAX_SOURCES: usize = 8;
pub const MODEL_ASSET_SOURCE_MAX_LEN: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestAlgorithm {
    Sha256,
    Blake3,
}

impl DigestAlgorithm {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha256 => "sha256",
            Self::Blake3 => "blake3",
        }
    }
}

/// Pinned before any byte is fetched; where the bytes come from never affects trust.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAssetDigest {
    pub algorithm: DigestAlgorithm,
    pub hex: String,
}

impl ModelAssetDigest {
    pub fn validate(&self) -> Result<()> {
        if self.hex.len() != 64
            || !self
                .hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(ProtocolError::Invalid("model asset digest"));
        }
        Ok(())
    }

    /// Relative location in a content-addressed store, e.g. `blake3/<hex>`.
    pub fn store_key(&self) -> String {
        format!("{}/{}", self.algorithm.as_str(), self.hex)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAssetDescriptor {
    pub digest: ModelAssetDigest,
    pub size: u64,
    /// Path the engine expects inside the model directory, e.g. `model.gguf`.
    pub file_name: String,
    /// HTTPS sources in the order the device tries them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
}

impl ModelAssetDescriptor {
    pub fn validate(&self) -> Result<()> {
        self.digest.validate()?;
        if self.size == 0 || self.size > MODEL_ASSET_MAX_BYTES {
            return Err(ProtocolError::Invalid("model asset size"));
        }
        validate_artifact_relative_path(&self.file_name)?;
        if self.sources.len() > MODEL_ASSET_MAX_SOURCES {
            return Err(ProtocolError::Invalid("too many model asset sources"));
        }
        for source in &self.sources {
            validate_model_asset_source(source)?;
        }
        Ok(())
    }
}

/// Sources are public HTTPS locations; credentials and fragments never travel to a device.
pub fn validate_model_asset_source(source: &str) -> Result<()> {
    let invalid = ProtocolError::Invalid("model asset source");
    if source.len() > MODEL_ASSET_SOURCE_MAX_LEN {
        return Err(invalid);
    }
    let url = url::Url::parse(source).map_err(|_| ProtocolError::Invalid("model asset source"))?;
    if url.scheme() != "https"
        || url.host_str().is_none_or(str::is_empty)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.query().is_some_and(|query| query != "download=true")
    {
        return Err(invalid);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAssetFailure {
    /// No source could be reached: DNS, connect or TLS failed, or the address was refused.
    EgressBlocked,
    HttpStatus,
    DigestMismatch,
    SizeMismatch,
    DiskBudget,
    NoSources,
    Cancelled,
    Io,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelAssetState {
    Queued,
    Fetching {
        source_index: u8,
        bytes: u64,
    },
    Verifying,
    Present,
    /// A controller is sending the bytes over the tunnel.
    AwaitingPush {
        bytes: u64,
    },
    Failed {
        reason: ModelAssetFailure,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        http_status: Option<u16>,
    },
}

use crate::{ProjectBitPin, validate_artifact_project_id, validate_management_id};
use std::collections::{BTreeMap, HashSet};

/// One encrypted management reply, envelope included.
pub const MODELS_REPLY_MAX_BYTES: usize = 16 * 1024;
/// `Stats` is a bulk read and answers over a tunnel data stream.
pub const MODELS_BULK_REPLY_MAX_BYTES: usize = 1024 * 1024;
pub const MODEL_MAX_ASSETS: usize = 256;
/// Digests projected into a hosted-model reply; `asset_count` names the full model size.
pub const MODEL_MAX_LISTED_ASSETS: usize = 32;
/// Leaves room for the request envelope within one encrypted management frame.
pub const MODEL_RUNTIME_MANIFEST_MAX_BYTES: usize = 15 * 1024;
pub const MODEL_DISPLAY_NAME_MAX_BYTES: usize = 128;
pub const MODEL_MIN_CTX_PER_SLOT: u32 = 256;
pub const MODEL_MAX_CTX_PER_SLOT: u32 = 1 << 20;
pub const MODEL_MAX_PARALLEL: u8 = 64;
pub const MODEL_MAX_THREADS: u16 = 1024;
pub const MODEL_MAX_GPU_LAYERS: u16 = 1024;
pub const MODEL_MIN_IDLE_UNLOAD_SECONDS: u32 = 60;
pub const MODEL_MAX_IDLE_UNLOAD_SECONDS: u32 = 7 * 86_400;
pub const MODEL_DEFAULT_IDLE_UNLOAD_SECONDS: u32 = 15 * 60;
pub const MODELS_PAGE_MAX: u16 = 32;
pub const MODEL_ENSURE_MAX_PINS: usize = 32;
pub const MODEL_MAX_PENDING_ASSETS: usize = 32;
pub const MODEL_STATS_MAX_POINTS: usize = 2_160;
pub const MODEL_STATS_MAX_CONSUMERS: usize = 64;
pub const MODEL_MAX_GPUS: usize = 8;
pub const MODEL_MAX_CPU_FEATURES: usize = 32;
pub const MODEL_MAX_RUNTIMES: usize = 8;
pub const MODEL_OVERVIEW_MAX_RECOMMENDATIONS: usize = 3;
pub const MODEL_RECOMMENDATION_MAX_PARAMS: usize = 8;
pub const MODEL_TEXT_MAX_BYTES: usize = 128;
pub const MODEL_HOST_MAX_BYTES: usize = 253;
/// The sources one listed job carries, in bytes all together, so a page stays one reply.
pub const MODEL_JOB_SOURCES_MAX_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelKind {
    Chat,
    Vision,
    Embedding,
    #[serde(rename = "systemone")]
    SystemOne,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelEngine {
    Llamacpp,
    /// macOS arm64 only.
    Mlx,
    /// Text and image embeddings in the agent's own worker; it needs no runtime pack.
    Onnx,
}

/// A runtime pack the device installs on demand.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRuntime {
    Llamacpp,
    Mlx,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelBackend {
    Cpu,
    Vulkan,
    Metal,
    Cuda,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelPooling {
    Mean,
    Cls,
    Last,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum KvCacheType {
    #[serde(rename = "f16")]
    F16,
    #[serde(rename = "q8_0")]
    Q8,
    #[serde(rename = "q4_0")]
    Q4,
}

/// `"auto"` fits layers to free GPU memory; `{"count": n}` offloads exactly n, 0 keeps the CPU.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GpuLayers {
    #[default]
    Auto,
    Count(u16),
}

impl GpuLayers {
    fn is_auto(&self) -> bool {
        *self == Self::Auto
    }
}

/// Engine settings of one hosted model. A field left out takes the device's recommended value.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ctx_per_slot: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kv_cache_type: Option<KvCacheType>,
    #[serde(default, skip_serializing_if = "GpuLayers::is_auto")]
    pub gpu_layers: GpuLayers,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threads: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flash_attn: Option<bool>,
}

impl ModelSettings {
    pub fn validate(&self) -> Result<()> {
        if self
            .ctx_per_slot
            .is_some_and(|ctx| !(MODEL_MIN_CTX_PER_SLOT..=MODEL_MAX_CTX_PER_SLOT).contains(&ctx))
        {
            return Err(ProtocolError::Invalid("model context per slot"));
        }
        if self
            .parallel
            .is_some_and(|slots| slots == 0 || slots > MODEL_MAX_PARALLEL)
        {
            return Err(ProtocolError::Invalid("model parallel slots"));
        }
        if self
            .threads
            .is_some_and(|threads| threads == 0 || threads > MODEL_MAX_THREADS)
        {
            return Err(ProtocolError::Invalid("model thread count"));
        }
        if matches!(self.gpu_layers, GpuLayers::Count(layers) if layers > MODEL_MAX_GPU_LAYERS) {
            return Err(ProtocolError::Invalid("model GPU layer count"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
pub enum Residency {
    AlwaysOn,
    OnDemand { idle_unload_after_seconds: u32 },
    PinnedOff,
}

impl Default for Residency {
    fn default() -> Self {
        Self::OnDemand {
            idle_unload_after_seconds: MODEL_DEFAULT_IDLE_UNLOAD_SECONDS,
        }
    }
}

impl Residency {
    pub fn validate(&self) -> Result<()> {
        if let Self::OnDemand {
            idle_unload_after_seconds,
        } = self
            && !(MODEL_MIN_IDLE_UNLOAD_SECONDS..=MODEL_MAX_IDLE_UNLOAD_SECONDS)
                .contains(idle_unload_after_seconds)
        {
            return Err(ProtocolError::Invalid("model idle unload delay"));
        }
        Ok(())
    }
}

/// What `Install` hosts. The device verifies every asset against its pinned digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelSpec {
    pub display_name: String,
    pub kind: ModelKind,
    pub engine: ModelEngine,
    /// In load order; split GGUF parts run first to last.
    pub assets: Vec<ModelAssetDescriptor>,
    /// File name of the asset llama.cpp loads with `--mmproj`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projector: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pooling: Option<ModelPooling>,
}

impl ModelSpec {
    pub fn validate(&self) -> Result<()> {
        validate_model_display_name(&self.display_name)?;
        validate_engine_kind(self.engine, self.kind)?;
        let names = self.validate_assets()?;
        self.validate_roles(&names)
    }

    /// The file names, once every asset is valid and none repeats a digest or a name.
    fn validate_assets(&self) -> Result<HashSet<&str>> {
        let digests = self
            .assets
            .iter()
            .map(|asset| asset.digest.clone())
            .collect::<Vec<_>>();
        validate_digests(&digests)?;
        let mut names = HashSet::new();
        for asset in &self.assets {
            asset.validate()?;
            if !names.insert(asset.file_name.as_str()) {
                return Err(ProtocolError::Invalid("duplicate model asset"));
            }
        }
        Ok(names)
    }

    fn validate_roles(&self, names: &HashSet<&str>) -> Result<()> {
        self.validate_projector(names)?;
        if self.engine == ModelEngine::Llamacpp
            && !self.assets.iter().any(|asset| self.is_gguf_weights(asset))
        {
            return Err(ProtocolError::Invalid(
                "llama.cpp model without GGUF weights",
            ));
        }
        if self.pooling.is_some() && self.kind != ModelKind::Embedding {
            return Err(ProtocolError::Invalid("pooling outside an embedding model"));
        }
        Ok(())
    }

    fn validate_projector(&self, names: &HashSet<&str>) -> Result<()> {
        let needed = self.engine == ModelEngine::Llamacpp && self.kind == ModelKind::Vision;
        let allowed = self.engine == ModelEngine::Llamacpp
            && matches!(self.kind, ModelKind::Vision | ModelKind::SystemOne);
        match self.projector.as_deref() {
            Some(name) if !allowed || !names.contains(name) => {
                Err(ProtocolError::Invalid("model projector"))
            }
            None if needed => Err(ProtocolError::Invalid(
                "llama.cpp vision model without projector",
            )),
            _ => Ok(()),
        }
    }

    fn is_gguf_weights(&self, asset: &ModelAssetDescriptor) -> bool {
        self.projector.as_deref() != Some(asset.file_name.as_str())
            && asset.file_name.to_ascii_lowercase().ends_with(".gguf")
    }
}

fn validate_engine_kind(engine: ModelEngine, kind: ModelKind) -> Result<()> {
    match (engine, kind) {
        (ModelEngine::Llamacpp, _)
        | (ModelEngine::Mlx, ModelKind::Chat | ModelKind::Vision)
        | (ModelEngine::Onnx, ModelKind::Embedding) => Ok(()),
        _ => Err(ProtocolError::Invalid(
            "model engine cannot serve this kind",
        )),
    }
}

pub fn validate_model_display_name(name: &str) -> Result<()> {
    text(name, MODEL_DISPLAY_NAME_MAX_BYTES, "model display name")
}

/// Job ids are canonical lowercase UUIDs.
pub fn validate_model_job_id(id: &str) -> Result<()> {
    let canonical = id.len() == 36
        && id.bytes().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                byte == b'-'
            } else {
                byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
            }
        });
    if !canonical {
        return Err(ProtocolError::Invalid("model job ID"));
    }
    Ok(())
}

fn text(value: &str, max: usize, what: &'static str) -> Result<()> {
    crate::proof::bounded_text(value, max).map_err(|_| ProtocolError::Invalid(what))
}

fn validate_page(after: Option<&str>, limit: u16) -> Result<()> {
    if let Some(cursor) = after {
        validate_management_id(cursor)?;
    }
    if limit == 0 || limit > MODELS_PAGE_MAX {
        return Err(ProtocolError::Invalid("model page limit"));
    }
    Ok(())
}

fn validate_digests(digests: &[ModelAssetDigest]) -> Result<()> {
    if digests.is_empty() || digests.len() > MODEL_MAX_ASSETS {
        return Err(ProtocolError::Invalid("model asset count"));
    }
    let mut seen = HashSet::new();
    for digest in digests {
        digest.validate()?;
        if !seen.insert(digest) {
            return Err(ProtocolError::Invalid("duplicate model asset"));
        }
    }
    Ok(())
}

fn validate_next(next: Option<&str>) -> Result<()> {
    next.map_or(Ok(()), validate_management_id)
}

fn default_models_page() -> u16 {
    8
}

fn default_jobs_page() -> u16 {
    16
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatsStep {
    Minute,
    Hour,
}

impl StatsStep {
    pub fn seconds(self) -> i64 {
        match self {
            Self::Minute => 60,
            Self::Hour => 3_600,
        }
    }
}

/// `ManagementCommand::Models`. Writes are journaled under the enclosing
/// `ManagementRequest.operation_id` like every other mutating command, so a retry with the
/// same operation ID returns the first answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelsRequest {
    /// Answers `ModelsOverview`. Empty braces keep unknown fields refused.
    Overview {},
    /// Answers `HostedModelPage`.
    Models {
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_models_page")]
        limit: u16,
    },
    /// Answers `ModelStats` with one value per `step` from `from` up to `to`; both are
    /// multiples of `step`.
    Stats {
        #[serde(default)]
        model_id: Option<String>,
        from: i64,
        to: i64,
        step: StatsStep,
    },
    /// Answers `ModelJobPage`.
    Jobs {
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_jobs_page")]
        limit: u16,
    },
    /// Answers `RecommendationPage`.
    Recommendations {
        #[serde(default)]
        after: Option<String>,
        #[serde(default = "default_models_page")]
        limit: u16,
    },
    /// Runs the hardware probe again; answers `SystemFacts`.
    Probe {},
    /// Hosts a model and starts or joins the acquisition of its assets; answers `ModelInstalled`.
    Install {
        model_id: String,
        model: ModelSpec,
        #[serde(default)]
        settings: ModelSettings,
        #[serde(default)]
        residency: Residency,
    },
    /// Replaces settings and residency; answers the `HostedModel`.
    Configure {
        model_id: String,
        expected_revision: u64,
        settings: ModelSettings,
        residency: Residency,
    },
    /// Answers the `HostedModel`.
    Load { model_id: String },
    /// Answers the `HostedModel`.
    Unload { model_id: String },
    /// Answers `ModelRemoved`; assets nothing else refers to become collectable.
    Remove {
        model_id: String,
        expected_revision: u64,
    },
    /// Deploy step: starts or joins the acquisition of every asset the pinned Bits of a
    /// committed project name; answers `ModelAssetSummary`.
    Ensure {
        project_id: String,
        pins: Vec<ProjectBitPin>,
    },
    /// Answers `RuntimeInstalled`.
    InstallRuntime {
        runtime: ModelRuntime,
        backend: ModelBackend,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        manifest_jws: Option<String>,
    },
    /// Answers the `RuntimeInfo`.
    RemoveRuntime {
        runtime: ModelRuntime,
        backend: ModelBackend,
    },
    /// Answers the `ModelJob`.
    CancelJob { job_id: String },
}

impl ModelsRequest {
    pub fn is_write(&self) -> bool {
        !matches!(
            self,
            Self::Overview {}
                | Self::Models { .. }
                | Self::Stats { .. }
                | Self::Jobs { .. }
                | Self::Recommendations { .. }
                | Self::Probe {}
        )
    }

    pub fn validate(&self) -> Result<()> {
        if let Self::InstallRuntime {
            manifest_jws: Some(compact),
            ..
        } = self
        {
            if compact.is_empty()
                || compact.len() > MODEL_RUNTIME_MANIFEST_MAX_BYTES
                || !compact
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            {
                return Err(ProtocolError::Invalid("runtime manifest size or encoding"));
            }
        }
        match self {
            Self::Overview {} | Self::Probe {} => Ok(()),
            Self::Models { after, limit }
            | Self::Jobs { after, limit }
            | Self::Recommendations { after, limit } => validate_page(after.as_deref(), *limit),
            Self::Stats {
                model_id,
                from,
                to,
                step,
            } => validate_stats_range(model_id.as_deref(), *from, *to, *step),
            Self::Install {
                model_id,
                model,
                settings,
                residency,
            } => {
                model.validate()?;
                validate_model_config(model_id, settings, residency)
            }
            Self::Configure {
                model_id,
                settings,
                residency,
                ..
            } => validate_model_config(model_id, settings, residency),
            Self::Load { model_id } | Self::Unload { model_id } | Self::Remove { model_id, .. } => {
                validate_management_id(model_id)
            }
            Self::Ensure { project_id, pins } => validate_ensure(project_id, pins),
            Self::InstallRuntime {
                runtime, backend, ..
            }
            | Self::RemoveRuntime { runtime, backend } => {
                if *runtime == ModelRuntime::Mlx && *backend != ModelBackend::Metal {
                    return Err(ProtocolError::Invalid("MLX runs on Metal only"));
                }
                Ok(())
            }
            Self::CancelJob { job_id } => validate_model_job_id(job_id),
        }
    }
}

fn validate_stats_range(model_id: Option<&str>, from: i64, to: i64, step: StatsStep) -> Result<()> {
    model_id.map_or(Ok(()), validate_management_id)?;
    let step = step.seconds();
    let aligned = from % step == 0 && to % step == 0;
    if from <= 0 || to <= from || !aligned || (to - from) / step > MODEL_STATS_MAX_POINTS as i64 {
        return Err(ProtocolError::Invalid("model stats range"));
    }
    Ok(())
}

fn validate_model_config(
    model_id: &str,
    settings: &ModelSettings,
    residency: &Residency,
) -> Result<()> {
    validate_management_id(model_id)?;
    settings.validate()?;
    residency.validate()
}

fn validate_ensure(project_id: &str, pins: &[ProjectBitPin]) -> Result<()> {
    validate_artifact_project_id(project_id)?;
    if pins.is_empty() || pins.len() > MODEL_ENSURE_MAX_PINS {
        return Err(ProtocolError::Invalid("model ensure pin count"));
    }
    let mut seen = HashSet::new();
    for pin in pins {
        pin.validate()?;
        if !seen.insert(&pin.bit_id) {
            return Err(ProtocolError::Invalid("duplicate model ensure pin"));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelHostFailure {
    /// An asset is missing from the store and could not be acquired again.
    AssetMissing,
    RuntimeMissing,
    /// Weights plus KV cache exceed the memory budget even after idle models were unloaded.
    InsufficientMemory,
    EngineExited,
    HealthTimeout,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostedModelState {
    /// Waits for its assets.
    Acquiring,
    Stopped,
    Loading,
    Loaded {
        ram_bytes: u64,
        vram_bytes: u64,
        slots: u8,
        slots_busy: u8,
    },
    Unloading,
    Failed {
        reason: ModelHostFailure,
    },
}

/// Replies carry the state flat, e.g. `{"id": …, "state": "loaded", "slots": 4, …}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedModel {
    pub id: String,
    pub display_name: String,
    pub kind: ModelKind,
    pub engine: ModelEngine,
    pub assets: Vec<ModelAssetDigest>,
    /// Total assets when `assets` is only the first `MODEL_MAX_LISTED_ASSETS` digests.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_count: Option<u32>,
    pub settings: ModelSettings,
    pub residency: Residency,
    /// `Configure` and `Remove` name it as `expected_revision`.
    pub revision: u64,
    #[serde(flatten)]
    pub state: HostedModelState,
}

impl HostedModel {
    pub fn validate(&self) -> Result<()> {
        validate_management_id(&self.id)?;
        validate_model_display_name(&self.display_name)?;
        validate_engine_kind(self.engine, self.kind)?;
        validate_digests(&self.assets)?;
        if self.assets.len() > MODEL_MAX_LISTED_ASSETS
            || self.asset_count.is_some_and(|count| {
                count as usize > MODEL_MAX_ASSETS || (count as usize) < self.assets.len()
            })
        {
            return Err(ProtocolError::Invalid("listed model assets"));
        }
        self.settings.validate()?;
        self.residency.validate()?;
        if let HostedModelState::Loaded {
            slots, slots_busy, ..
        } = self.state
            && slots_busy > slots
        {
            return Err(ProtocolError::Invalid("model slots"));
        }
        Ok(())
    }
}

/// Acquisition state of one asset, flat like `HostedModel`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelAssetStatus {
    pub digest: ModelAssetDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<String>,
    #[serde(flatten)]
    pub state: ModelAssetState,
}

impl ModelAssetStatus {
    pub fn validate(&self) -> Result<()> {
        self.digest.validate()?;
        self.job_id.as_deref().map_or(Ok(()), validate_model_job_id)
    }
}

/// Assets of an `Install` or `Ensure`, deduplicated by digest. `pending` lists at most
/// `MODEL_MAX_PENDING_ASSETS` of the ones not present yet; `Jobs` lists every job.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelAssetSummary {
    pub total: u32,
    pub present: u32,
    pub pending: Vec<ModelAssetStatus>,
}

impl ModelAssetSummary {
    pub fn validate(&self) -> Result<()> {
        if self.present > self.total
            || self.pending.len() > MODEL_MAX_PENDING_ASSETS
            || self.pending.len() as u64 > u64::from(self.total - self.present)
        {
            return Err(ProtocolError::Invalid("model asset summary"));
        }
        for status in &self.pending {
            status.validate()?;
            if status.state == ModelAssetState::Present {
                return Err(ProtocolError::Invalid(
                    "present model asset listed as pending",
                ));
            }
        }
        Ok(())
    }
}

/// One acquisition, shared by everything that wants the same digest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelJob {
    pub job_id: String,
    pub digest: ModelAssetDigest,
    pub size: u64,
    pub file_name: String,
    /// Host of the source being fetched, e.g. `cdn.flow-like.com`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_host: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_per_second: Option<u64>,
    /// The asset's sources in the order the device tries them, so a controller can send the
    /// file when the device can't fetch it; `listed_sources` bounds them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<String>,
    pub updated_at: i64,
    #[serde(flatten)]
    pub state: ModelAssetState,
}

impl ModelJob {
    pub fn validate(&self) -> Result<()> {
        validate_model_job_id(&self.job_id)?;
        self.digest.validate()?;
        if self.size == 0 || self.size > MODEL_ASSET_MAX_BYTES {
            return Err(ProtocolError::Invalid("model asset size"));
        }
        validate_artifact_relative_path(&self.file_name)?;
        if self
            .source_host
            .as_deref()
            .is_some_and(|host| !is_host(host))
        {
            return Err(ProtocolError::Invalid("model source host"));
        }
        if self.sources.len() > MODEL_ASSET_MAX_SOURCES
            || self.sources.iter().map(String::len).sum::<usize>() > MODEL_JOB_SOURCES_MAX_BYTES
        {
            return Err(ProtocolError::Invalid("model job sources"));
        }
        self.sources
            .iter()
            .try_for_each(|source| validate_model_asset_source(source))
    }

    /// The leading valid sources of an asset that fit `MODEL_JOB_SOURCES_MAX_BYTES`.
    pub fn listed_sources(sources: &[String]) -> Vec<String> {
        let mut budget = MODEL_JOB_SOURCES_MAX_BYTES;
        sources
            .iter()
            .filter(|source| validate_model_asset_source(source).is_ok())
            .take(MODEL_ASSET_MAX_SOURCES)
            .take_while(|source| {
                let fits = source.len() <= budget;
                budget = budget.saturating_sub(source.len());
                fits
            })
            .cloned()
            .collect()
    }

    /// The host of `source` as `source_host` names it, or `None` when the rule refuses it: a
    /// URL host may also hold characters such as `_` or `*`.
    pub fn listed_source_host(source: &str) -> Option<String> {
        let host = url::Url::parse(source).ok()?.host_str()?.to_owned();
        is_host(&host).then_some(host)
    }
}

fn is_host(host: &str) -> bool {
    (1..=MODEL_HOST_MAX_BYTES).contains(&host.len())
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.:[]".contains(&b))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInfo {
    pub runtime: ModelRuntime,
    pub backend: ModelBackend,
    /// Upstream build, e.g. `b10809`.
    pub build: String,
    pub installed: bool,
    /// Bytes on disk once installed, else the download size.
    pub size: u64,
}

impl RuntimeInfo {
    pub fn validate(&self) -> Result<()> {
        if self.build.is_empty()
            || self.build.len() > 64
            || !self
                .build
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        {
            return Err(ProtocolError::Invalid("model runtime build"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CpuFacts {
    pub brand: String,
    /// e.g. `x86_64`, `aarch64`.
    pub arch: String,
    /// e.g. `avx2`, `avx512f`, `neon`.
    pub features: Vec<String>,
    pub physical_cores: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuFacts {
    pub name: String,
    /// The backend this device would use for it.
    pub backend: ModelBackend,
    /// Unknown while only an OS probe saw the GPU.
    pub memory_total: Option<u64>,
    pub memory_free: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapacityFacts {
    pub total: u64,
    pub free: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SystemFacts {
    pub cpu: CpuFacts,
    pub ram: CapacityFacts,
    pub gpus: Vec<GpuFacts>,
    /// The volume that holds the model store.
    pub model_volume: CapacityFacts,
}

impl SystemFacts {
    pub fn validate(&self) -> Result<()> {
        self.cpu.validate()?;
        if self.gpus.len() > MODEL_MAX_GPUS {
            return Err(ProtocolError::Invalid("GPU count"));
        }
        self.gpus.iter().try_for_each(GpuFacts::validate)?;
        if self.ram.free > self.ram.total || self.model_volume.free > self.model_volume.total {
            return Err(ProtocolError::Invalid("system capacity"));
        }
        Ok(())
    }
}

impl CpuFacts {
    fn validate(&self) -> Result<()> {
        text(&self.brand, MODEL_TEXT_MAX_BYTES, "CPU brand")?;
        let features = self.features.len() <= MODEL_MAX_CPU_FEATURES
            && self.features.iter().all(|feature| is_token(feature));
        if !is_token(&self.arch) || !features {
            return Err(ProtocolError::Invalid("CPU facts"));
        }
        Ok(())
    }
}

impl GpuFacts {
    fn validate(&self) -> Result<()> {
        text(&self.name, MODEL_TEXT_MAX_BYTES, "GPU name")?;
        if self
            .memory_total
            .zip(self.memory_free)
            .is_some_and(|(total, free)| free > total)
        {
            return Err(ProtocolError::Invalid("GPU memory"));
        }
        Ok(())
    }
}

fn is_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.".contains(&b))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationCode {
    GpuUnused,
    PartialOffload,
    RequestsQueued,
    KvPressure,
    CtxTruncation,
    MemoryPressure,
    IdleResident,
    SlowTtft,
    CpuThreads,
    DiskLow,
    RuntimeOutdated,
    ContainerGpuHidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationTier {
    Now,
    Soon,
    Later,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RecommendationValue {
    Number(u64),
    Text(String),
}

/// Computed on the device so its facts never leave it; the client owns the copy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Recommendation {
    pub code: RecommendationCode,
    pub tier: RecommendationTier,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, RecommendationValue>,
    /// A `Configure`, `Unload` or `InstallRuntime` the client sends unchanged on accept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix: Option<ModelsRequest>,
}

impl Recommendation {
    pub fn validate(&self) -> Result<()> {
        self.model_id
            .as_deref()
            .map_or(Ok(()), validate_management_id)?;
        if self.params.len() > MODEL_RECOMMENDATION_MAX_PARAMS {
            return Err(ProtocolError::Invalid("recommendation parameter count"));
        }
        if !self.params.iter().all(|(key, value)| is_param(key, value)) {
            return Err(ProtocolError::Invalid("recommendation parameter"));
        }
        let Some(fix) = &self.fix else {
            return Ok(());
        };
        if !matches!(
            fix,
            ModelsRequest::Configure { .. }
                | ModelsRequest::Unload { .. }
                | ModelsRequest::InstallRuntime { .. }
        ) {
            return Err(ProtocolError::Invalid("recommendation fix"));
        }
        fix.validate()
    }
}

fn is_param(key: &str, value: &RecommendationValue) -> bool {
    is_token(key)
        && match value {
            RecommendationValue::Number(_) => true,
            RecommendationValue::Text(text) => {
                text.len() <= MODEL_TEXT_MAX_BYTES && !text.chars().any(char::is_control)
            }
        }
}

/// Headline counters, also the compact `models` part of the fleet snapshot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelsSummary {
    pub models: u16,
    pub loaded: u16,
    pub failed: u16,
    pub requests_24h: u64,
    pub tokens_24h: u64,
    pub errors_24h: u64,
    pub store_bytes: u64,
    pub store_budget_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelsOverview {
    pub observed_at: i64,
    pub system: SystemFacts,
    pub runtimes: Vec<RuntimeInfo>,
    /// The enrolled release source; a controller can forward its signed manifest offline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_manifest_url: Option<String>,
    pub summary: ModelsSummary,
    /// The most urgent ones; `Recommendations` lists all.
    pub recommendations: Vec<Recommendation>,
    /// The first hosted models by id, as many as fit; `Models` continues after `next`.
    pub models: Vec<HostedModel>,
    pub next: Option<String>,
}

impl ModelsOverview {
    pub fn validate(&self) -> Result<()> {
        self.system.validate()?;
        self.runtime_manifest_url
            .as_deref()
            .map_or(Ok(()), crate::validate_release_url)?;
        if self.runtimes.len() > MODEL_MAX_RUNTIMES
            || self.recommendations.len() > MODEL_OVERVIEW_MAX_RECOMMENDATIONS
            || self.models.len() > usize::from(MODELS_PAGE_MAX)
        {
            return Err(ProtocolError::Invalid("model overview size"));
        }
        self.runtimes.iter().try_for_each(RuntimeInfo::validate)?;
        self.recommendations
            .iter()
            .try_for_each(Recommendation::validate)?;
        self.models.iter().try_for_each(HostedModel::validate)?;
        validate_next(self.next.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostedModelPage {
    pub models: Vec<HostedModel>,
    pub next: Option<String>,
}

impl HostedModelPage {
    pub fn validate(&self) -> Result<()> {
        if self.models.len() > usize::from(MODELS_PAGE_MAX) {
            return Err(ProtocolError::Invalid("model page size"));
        }
        self.models.iter().try_for_each(HostedModel::validate)?;
        validate_next(self.next.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelJobPage {
    pub jobs: Vec<ModelJob>,
    pub next: Option<String>,
}

impl ModelJobPage {
    pub fn validate(&self) -> Result<()> {
        if self.jobs.len() > usize::from(MODELS_PAGE_MAX) {
            return Err(ProtocolError::Invalid("model job page size"));
        }
        self.jobs.iter().try_for_each(ModelJob::validate)?;
        validate_next(self.next.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecommendationPage {
    pub recommendations: Vec<Recommendation>,
    pub next: Option<String>,
}

impl RecommendationPage {
    pub fn validate(&self) -> Result<()> {
        if self.recommendations.len() > usize::from(MODELS_PAGE_MAX) {
            return Err(ProtocolError::Invalid("recommendation page size"));
        }
        self.recommendations
            .iter()
            .try_for_each(Recommendation::validate)?;
        validate_next(self.next.as_deref())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelInstalled {
    pub model: HostedModel,
    pub assets: ModelAssetSummary,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRemoved {
    pub model_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeInstalled {
    pub runtime: RuntimeInfo,
    pub asset: ModelAssetStatus,
}

/// Who called a model: the owner, a grantee through the tunnel, or a placement.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ModelConsumer {
    Owner,
    Grant { grant_id: String },
    Placement { placement_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelConsumerTotals {
    pub consumer: ModelConsumer,
    pub requests: u64,
    pub errors: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

/// Equal-length columns, one value per step. Counts and timings only, never prompt text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelStatsSeries {
    pub requests: Vec<u64>,
    pub errors: Vec<u64>,
    pub prompt_tokens: Vec<u64>,
    pub completion_tokens: Vec<u64>,
    pub cached_tokens: Vec<u64>,
    /// Time spent generating; tokens per second is `completion_tokens * 1000 / decode_ms`.
    pub decode_ms: Vec<u64>,
    pub ttft_p50_ms: Vec<Option<u32>>,
    pub ttft_p95_ms: Vec<Option<u32>>,
    pub queue_wait_p95_ms: Vec<Option<u32>>,
}

/// The `Stats` answer; the series starts at `from` and its length is the step count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelStats {
    pub model_id: Option<String>,
    pub from: i64,
    pub step: StatsStep,
    pub series: ModelStatsSeries,
    pub consumers: Vec<ModelConsumerTotals>,
}

impl ModelStats {
    pub fn validate(&self) -> Result<()> {
        self.model_id
            .as_deref()
            .map_or(Ok(()), validate_management_id)?;
        let dense = self
            .series
            .points()
            .is_some_and(|points| points <= MODEL_STATS_MAX_POINTS);
        if !dense || self.from <= 0 || self.from % self.step.seconds() != 0 {
            return Err(ProtocolError::Invalid("model stats series"));
        }
        if self.consumers.len() > MODEL_STATS_MAX_CONSUMERS {
            return Err(ProtocolError::Invalid("model stats consumer count"));
        }
        self.consumers
            .iter()
            .try_for_each(|totals| match &totals.consumer {
                ModelConsumer::Owner => Ok(()),
                ModelConsumer::Grant { grant_id: id }
                | ModelConsumer::Placement { placement_id: id } => validate_management_id(id),
            })
    }
}

impl ModelStatsSeries {
    /// The column length, when every column has it.
    fn points(&self) -> Option<usize> {
        let points = self.requests.len();
        [
            self.errors.len(),
            self.prompt_tokens.len(),
            self.completion_tokens.len(),
            self.cached_tokens.len(),
            self.decode_ms.len(),
            self.ttft_p50_ms.len(),
            self.ttft_p95_ms.len(),
            self.queue_wait_p95_ms.len(),
        ]
        .iter()
        .all(|len| *len == points)
        .then_some(points)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Blake3,
                hex: "a".repeat(64),
            },
            size: 42,
            file_name: "model.gguf".into(),
            sources: vec!["https://cdn.flow-like.com/bits/abc".into()],
        }
    }

    #[test]
    fn descriptors_require_pinned_digests_and_public_https_sources() {
        assert!(descriptor().validate().is_ok());
        assert_eq!(
            descriptor().digest.store_key(),
            format!("blake3/{}", "a".repeat(64))
        );

        let mut upper = descriptor();
        upper.digest.hex = "A".repeat(64);
        assert!(upper.validate().is_err());

        let mut empty = descriptor();
        empty.size = 0;
        assert!(empty.validate().is_err());

        let mut escape = descriptor();
        escape.file_name = "../model.gguf".into();
        assert!(escape.validate().is_err());

        for source in [
            "http://cdn.flow-like.com/bits/abc",
            "https://user:pass@cdn.flow-like.com/bits/abc",
            "https://cdn.flow-like.com/bits/abc#part",
            "https://cdn.flow-like.com/bits/abc?X-Amz-Signature=secret",
            "https://cdn.flow-like.com/bits/abc?download=true&token=secret",
            "https://cdn.flow-like.com/bits/abc?download=%74rue",
            "https://cdn.flow-like.com/bits/abc?",
            "file:///etc/passwd",
        ] {
            let mut bad = descriptor();
            bad.sources = vec![source.into()];
            assert!(bad.validate().is_err(), "{source}");
        }

        let mut many = descriptor();
        many.sources = vec!["https://huggingface.co/a".into(); MODEL_ASSET_MAX_SOURCES + 1];
        assert!(many.validate().is_err());
        let mut public_download = descriptor();
        public_download.sources[0].push_str("?download=true");
        assert!(public_download.validate().is_ok());
    }

    #[test]
    fn states_use_a_tagged_wire_shape() {
        let failed = ModelAssetState::Failed {
            reason: ModelAssetFailure::EgressBlocked,
            http_status: None,
        };
        assert_eq!(
            serde_json::to_value(&failed).unwrap(),
            serde_json::json!({"state":"failed","reason":"egress_blocked"})
        );
        assert_eq!(
            serde_json::from_value::<ModelAssetState>(
                serde_json::json!({"state":"fetching","source_index":1,"bytes":7})
            )
            .unwrap(),
            ModelAssetState::Fetching {
                source_index: 1,
                bytes: 7
            }
        );
    }

    use serde_json::json;

    const JOB: &str = "12345678-1234-1234-1234-123456789abc";

    fn digest(index: usize) -> ModelAssetDigest {
        ModelAssetDigest {
            algorithm: DigestAlgorithm::Sha256,
            hex: format!("{index:064x}"),
        }
    }

    fn asset(index: usize, file_name: &str) -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: digest(index),
            size: 4_900_000_000,
            file_name: file_name.into(),
            sources: vec![format!(
                "https://huggingface.co/org/repo/resolve/{}/{file_name}",
                "0".repeat(40)
            )],
        }
    }

    fn spec(kind: ModelKind, engine: ModelEngine, files: &[&str]) -> ModelSpec {
        ModelSpec {
            display_name: "Qwen3 8B".into(),
            kind,
            engine,
            assets: files
                .iter()
                .enumerate()
                .map(|(index, name)| asset(index, name))
                .collect(),
            projector: None,
            pooling: None,
        }
    }

    fn request(value: serde_json::Value) -> Result<ModelsRequest> {
        let request: ModelsRequest =
            serde_json::from_value(value).map_err(|_| ProtocolError::Invalid("schema"))?;
        request.validate()?;
        Ok(request)
    }

    fn settings_max() -> ModelSettings {
        ModelSettings {
            ctx_per_slot: Some(MODEL_MAX_CTX_PER_SLOT),
            parallel: Some(MODEL_MAX_PARALLEL),
            kv_cache_type: Some(KvCacheType::Q8),
            gpu_layers: GpuLayers::Count(MODEL_MAX_GPU_LAYERS),
            threads: Some(MODEL_MAX_THREADS),
            flash_attn: Some(true),
        }
    }

    fn worst_model() -> HostedModel {
        HostedModel {
            id: "m".repeat(128),
            display_name: "n".repeat(MODEL_DISPLAY_NAME_MAX_BYTES),
            kind: ModelKind::Vision,
            engine: ModelEngine::Llamacpp,
            assets: (0..MODEL_MAX_LISTED_ASSETS).map(digest).collect(),
            asset_count: Some(MODEL_MAX_ASSETS as u32),
            settings: settings_max(),
            residency: Residency::OnDemand {
                idle_unload_after_seconds: MODEL_MAX_IDLE_UNLOAD_SECONDS,
            },
            revision: u64::MAX,
            state: HostedModelState::Loaded {
                ram_bytes: u64::MAX,
                vram_bytes: u64::MAX,
                slots: u8::MAX,
                slots_busy: u8::MAX,
            },
        }
    }

    fn worst_recommendation() -> Recommendation {
        Recommendation {
            code: RecommendationCode::ContainerGpuHidden,
            tier: RecommendationTier::Later,
            model_id: Some("m".repeat(128)),
            params: (0..MODEL_RECOMMENDATION_MAX_PARAMS)
                .map(|index| {
                    (
                        format!("{index}{}", "p".repeat(31)),
                        RecommendationValue::Text("t".repeat(MODEL_TEXT_MAX_BYTES)),
                    )
                })
                .collect(),
            fix: Some(ModelsRequest::Configure {
                model_id: "m".repeat(128),
                expected_revision: u64::MAX,
                settings: settings_max(),
                residency: Residency::OnDemand {
                    idle_unload_after_seconds: MODEL_MAX_IDLE_UNLOAD_SECONDS,
                },
            }),
        }
    }

    fn worst_pending() -> ModelAssetStatus {
        ModelAssetStatus {
            digest: digest(usize::MAX),
            job_id: Some(JOB.into()),
            state: ModelAssetState::Failed {
                reason: ModelAssetFailure::DigestMismatch,
                http_status: Some(u16::MAX),
            },
        }
    }

    fn worst_overview() -> ModelsOverview {
        ModelsOverview {
            observed_at: i64::MAX,
            runtime_manifest_url: Some(format!("https://cdn.test/{}", "m".repeat(1_007))),
            system: SystemFacts {
                cpu: CpuFacts {
                    brand: "b".repeat(MODEL_TEXT_MAX_BYTES),
                    arch: "a".repeat(32),
                    features: (0..MODEL_MAX_CPU_FEATURES)
                        .map(|index| format!("{index:02}{}", "f".repeat(30)))
                        .collect(),
                    physical_cores: u16::MAX,
                },
                ram: CapacityFacts {
                    total: u64::MAX,
                    free: u64::MAX,
                },
                gpus: vec![
                    GpuFacts {
                        name: "g".repeat(MODEL_TEXT_MAX_BYTES),
                        backend: ModelBackend::Vulkan,
                        memory_total: Some(u64::MAX),
                        memory_free: Some(u64::MAX),
                    };
                    MODEL_MAX_GPUS
                ],
                model_volume: CapacityFacts {
                    total: u64::MAX,
                    free: u64::MAX,
                },
            },
            runtimes: vec![
                RuntimeInfo {
                    runtime: ModelRuntime::Llamacpp,
                    backend: ModelBackend::Vulkan,
                    build: "b".repeat(64),
                    installed: true,
                    size: u64::MAX,
                };
                MODEL_MAX_RUNTIMES
            ],
            summary: ModelsSummary {
                models: u16::MAX,
                loaded: u16::MAX,
                failed: u16::MAX,
                requests_24h: u64::MAX,
                tokens_24h: u64::MAX,
                errors_24h: u64::MAX,
                store_bytes: u64::MAX,
                store_budget_bytes: u64::MAX,
            },
            recommendations: vec![worst_recommendation(); MODEL_OVERVIEW_MAX_RECOMMENDATIONS],
            models: vec![worst_model()],
            next: Some("c".repeat(128)),
        }
    }

    fn reply_bytes(result: &impl Serialize) -> usize {
        serde_json::to_vec(&crate::ManagementResponse {
            operation_id: "o".repeat(128),
            state: "completed".into(),
            result: serde_json::to_value(result).unwrap(),
        })
        .unwrap()
        .len()
    }

    #[test]
    fn models_requests_use_their_wire_names_defaults_and_read_write_split() {
        for (value, write) in [
            (json!({"kind":"overview"}), false),
            (json!({"kind":"models"}), false),
            (json!({"kind":"jobs","after":JOB,"limit":32}), false),
            (json!({"kind":"recommendations"}), false),
            (json!({"kind":"probe"}), false),
            (
                json!({"kind":"stats","from":3_600,"to":7_200,"step":"minute"}),
                false,
            ),
            (json!({"kind":"load","model_id":"qwen3-8b"}), true),
            (json!({"kind":"unload","model_id":"qwen3-8b"}), true),
            (
                json!({"kind":"remove","model_id":"qwen3-8b","expected_revision":3}),
                true,
            ),
            (
                json!({"kind":"configure","model_id":"qwen3-8b","expected_revision":3,"settings":{},"residency":{"mode":"always_on"}}),
                true,
            ),
            (
                json!({"kind":"ensure","project_id":"invoice-ai","pins":[{"bit_id":"qwen3","metadata_sha256":"a".repeat(64)}]}),
                true,
            ),
            (
                json!({"kind":"install_runtime","runtime":"llamacpp","backend":"vulkan"}),
                true,
            ),
            (
                json!({"kind":"remove_runtime","runtime":"mlx","backend":"metal"}),
                true,
            ),
            (json!({"kind":"cancel_job","job_id":JOB}), true),
        ] {
            let parsed = request(value.clone()).unwrap_or_else(|_| panic!("{value}"));
            assert_eq!(parsed.is_write(), write, "{value}");
        }
        assert_eq!(
            serde_json::to_value(request(json!({"kind":"models"})).unwrap()).unwrap(),
            json!({"kind":"models","after":null,"limit":8})
        );
        assert_eq!(
            serde_json::to_value(request(json!({"kind":"jobs"})).unwrap()).unwrap(),
            json!({"kind":"jobs","after":null,"limit":16})
        );
        let install = ModelsRequest::Install {
            model_id: "qwen3-8b".into(),
            model: spec(ModelKind::Chat, ModelEngine::Llamacpp, &["model.gguf"]),
            settings: ModelSettings::default(),
            residency: Residency::default(),
        };
        let wire = serde_json::to_value(&install).unwrap();
        assert_eq!(wire["settings"], json!({}));
        assert_eq!(
            wire["residency"],
            json!({"mode":"on_demand","idle_unload_after_seconds":900})
        );
        assert_eq!(request(wire).unwrap(), install);
        let defaults = json!({"kind":"install","model_id":"qwen3-8b","model":serde_json::to_value(spec(ModelKind::Chat, ModelEngine::Llamacpp, &["model.gguf"])).unwrap()});
        assert_eq!(request(defaults).unwrap(), install);
    }

    #[test]
    fn models_requests_refuse_unknown_fields_and_out_of_bound_values() {
        for value in [
            json!({"kind":"overview","placement_id":"api"}),
            json!({"kind":"models","limit":0}),
            json!({"kind":"models","limit":33}),
            json!({"kind":"models","after":"../x"}),
            json!({"kind":"stats","from":0,"to":60,"step":"minute"}),
            json!({"kind":"stats","from":120,"to":60,"step":"minute"}),
            json!({"kind":"stats","from":90,"to":180,"step":"minute"}),
            json!({"kind":"stats","from":3_600,"to":3_600 + 2_161 * 3_600,"step":"hour"}),
            json!({"kind":"stats","from":60,"to":120,"step":"second"}),
            json!({"kind":"load","model_id":""}),
            json!({"kind":"cancel_job","job_id":"12345678-1234-1234-1234-123456789ABC"}),
            json!({"kind":"install_runtime","runtime":"mlx","backend":"vulkan"}),
            json!({"kind":"ensure","project_id":"invoice-ai","pins":[]}),
            json!({"kind":"ensure","project_id":"..","pins":[{"bit_id":"a","metadata_sha256":"a".repeat(64)}]}),
            json!({"kind":"ensure","project_id":"p","pins":[{"bit_id":"a","metadata_sha256":"a".repeat(64)},{"bit_id":"a","metadata_sha256":"b".repeat(64)}]}),
            json!({"kind":"configure","model_id":"m","expected_revision":1,"settings":{"ctx_per_slot":128},"residency":{"mode":"always_on"}}),
            json!({"kind":"configure","model_id":"m","expected_revision":1,"settings":{"parallel":65},"residency":{"mode":"always_on"}}),
            json!({"kind":"configure","model_id":"m","expected_revision":1,"settings":{"threads":0},"residency":{"mode":"always_on"}}),
            json!({"kind":"configure","model_id":"m","expected_revision":1,"settings":{"gpu_layers":{"count":1025}},"residency":{"mode":"always_on"}}),
            json!({"kind":"configure","model_id":"m","expected_revision":1,"settings":{"seed":7},"residency":{"mode":"always_on"}}),
            json!({"kind":"configure","model_id":"m","expected_revision":1,"settings":{},"residency":{"mode":"on_demand","idle_unload_after_seconds":59}}),
            json!({"kind":"configure","model_id":"m","expected_revision":1,"settings":{},"residency":{"mode":"on_demand"}}),
            json!({"kind":"probe","full":true}),
        ] {
            assert!(request(value.clone()).is_err(), "{value}");
        }
        let pins = (0..=MODEL_ENSURE_MAX_PINS)
            .map(|index| json!({"bit_id":format!("bit-{index}"),"metadata_sha256":"a".repeat(64)}))
            .collect::<Vec<_>>();
        assert!(request(json!({"kind":"ensure","project_id":"p","pins":pins})).is_err());
    }

    #[test]
    fn forwarded_runtime_manifest_fits_the_management_envelope_and_is_bounded() {
        let install = |compact: String| ModelsRequest::InstallRuntime {
            runtime: ModelRuntime::Llamacpp,
            backend: ModelBackend::Cpu,
            manifest_jws: Some(compact),
        };
        let maximum = install("a".repeat(MODEL_RUNTIME_MANIFEST_MAX_BYTES));
        assert!(maximum.validate().is_ok());
        let request = crate::ManagementRequest {
            operation_id: "o".repeat(128),
            device_id: "d".repeat(128),
            issued_at: i64::MAX - 1,
            expires_at: i64::MAX,
            command: crate::ManagementCommand::Models { request: maximum },
        };
        assert!(serde_json::to_vec(&request).unwrap().len() <= crate::TUNNEL_MAX_DATA);
        for invalid in [
            String::new(),
            "a".repeat(MODEL_RUNTIME_MANIFEST_MAX_BYTES + 1),
            "a.b.c\n".into(),
            "a/b/c".into(),
        ] {
            assert!(install(invalid).validate().is_err());
        }
    }

    #[test]
    fn model_specs_bind_engine_kind_projector_and_pooling() {
        let install = |model: ModelSpec| {
            ModelsRequest::Install {
                model_id: "model".into(),
                model,
                settings: ModelSettings::default(),
                residency: Residency::AlwaysOn,
            }
            .validate()
        };
        assert!(
            install(spec(
                ModelKind::Chat,
                ModelEngine::Llamacpp,
                &["m-00001-of-00002.gguf", "m-00002-of-00002.gguf"]
            ))
            .is_ok()
        );
        assert!(
            install(spec(
                ModelKind::Chat,
                ModelEngine::Mlx,
                &["config.json", "model.safetensors"]
            ))
            .is_ok()
        );
        assert!(
            install(spec(
                ModelKind::Embedding,
                ModelEngine::Onnx,
                &["model.onnx", "tokenizer.json"]
            ))
            .is_ok()
        );

        let mut vision = spec(
            ModelKind::Vision,
            ModelEngine::Llamacpp,
            &["model.gguf", "mmproj.gguf"],
        );
        assert!(install(vision.clone()).is_err());
        vision.projector = Some("mmproj.gguf".into());
        assert!(install(vision.clone()).is_ok());
        vision.projector = Some("other.gguf".into());
        assert!(install(vision).is_err());

        let mut projector_only = spec(ModelKind::Vision, ModelEngine::Llamacpp, &["mmproj.gguf"]);
        projector_only.projector = Some("mmproj.gguf".into());
        assert!(install(projector_only).is_err());

        let mut chat_projector = spec(
            ModelKind::Chat,
            ModelEngine::Llamacpp,
            &["model.gguf", "mmproj.gguf"],
        );
        chat_projector.projector = Some("mmproj.gguf".into());
        assert!(install(chat_projector).is_err());

        let mut embedding = spec(ModelKind::Embedding, ModelEngine::Llamacpp, &["embed.gguf"]);
        embedding.pooling = Some(ModelPooling::Cls);
        assert!(install(embedding).is_ok());
        let mut pooled_chat = spec(ModelKind::Chat, ModelEngine::Llamacpp, &["model.gguf"]);
        pooled_chat.pooling = Some(ModelPooling::Mean);
        assert!(install(pooled_chat).is_err());

        for (kind, engine, files) in [
            (ModelKind::Chat, ModelEngine::Onnx, &["model.onnx"][..]),
            (
                ModelKind::Embedding,
                ModelEngine::Mlx,
                &["model.safetensors"][..],
            ),
            (
                ModelKind::Chat,
                ModelEngine::Llamacpp,
                &["model.safetensors"][..],
            ),
            (ModelKind::Chat, ModelEngine::Llamacpp, &[][..]),
        ] {
            assert!(
                install(spec(kind, engine, files)).is_err(),
                "{kind:?} {engine:?} {files:?}"
            );
        }
        let mut duplicate = spec(
            ModelKind::Chat,
            ModelEngine::Llamacpp,
            &["a.gguf", "b.gguf"],
        );
        duplicate.assets[1].digest = duplicate.assets[0].digest.clone();
        assert!(install(duplicate).is_err());
        let mut renamed = spec(
            ModelKind::Chat,
            ModelEngine::Llamacpp,
            &["a.gguf", "b.gguf"],
        );
        renamed.assets[1].file_name = "a.gguf".into();
        assert!(install(renamed).is_err());
        let names = (0..=MODEL_MAX_ASSETS)
            .map(|index| format!("m-{index}.gguf"))
            .collect::<Vec<_>>();
        let names = names.iter().map(String::as_str).collect::<Vec<_>>();
        assert!(
            install(spec(
                ModelKind::Chat,
                ModelEngine::Llamacpp,
                &names[..MODEL_MAX_ASSETS]
            ))
            .is_ok()
        );
        assert!(install(spec(ModelKind::Chat, ModelEngine::Llamacpp, &names)).is_err());
        let mut unnamed = spec(ModelKind::Chat, ModelEngine::Llamacpp, &["model.gguf"]);
        unnamed.display_name = " padded".into();
        assert!(install(unnamed).is_err());
    }

    #[test]
    fn systemone_is_a_native_llama_kind_with_an_optional_projector() {
        let mut model = spec(ModelKind::SystemOne, ModelEngine::Llamacpp, &["laya.gguf"]);
        assert_eq!(serde_json::to_value(model.kind).unwrap(), "systemone");
        assert!(model.validate().is_ok());
        model.engine = ModelEngine::Mlx;
        assert!(model.validate().is_err());
        model.engine = ModelEngine::Onnx;
        assert!(model.validate().is_err());
        let mut vision = spec(
            ModelKind::SystemOne,
            ModelEngine::Llamacpp,
            &["clef.gguf", "mmproj.gguf"],
        );
        vision.projector = Some("mmproj.gguf".into());
        assert!(vision.validate().is_ok());
        vision.projector = Some("missing.gguf".into());
        assert!(vision.validate().is_err());
    }

    #[test]
    fn settings_residency_and_gpu_layers_have_compact_wire_shapes() {
        assert_eq!(
            serde_json::to_value(ModelSettings::default()).unwrap(),
            json!({})
        );
        let settings = ModelSettings {
            kv_cache_type: Some(KvCacheType::Q8),
            gpu_layers: GpuLayers::Count(0),
            ..ModelSettings::default()
        };
        let wire = json!({"kv_cache_type":"q8_0","gpu_layers":{"count":0}});
        assert_eq!(serde_json::to_value(&settings).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<ModelSettings>(wire).unwrap(),
            settings
        );
        assert_eq!(
            serde_json::from_value::<ModelSettings>(json!({"gpu_layers":"auto"})).unwrap(),
            ModelSettings::default()
        );
        assert!(serde_json::from_value::<GpuLayers>(json!({"count":1,"extra":2})).is_err());
        assert_eq!(
            serde_json::to_value(KvCacheType::F16).unwrap(),
            json!("f16")
        );
        assert_eq!(
            serde_json::to_value(KvCacheType::Q4).unwrap(),
            json!("q4_0")
        );
        for residency in [
            Residency::AlwaysOn,
            Residency::PinnedOff,
            Residency::default(),
        ] {
            let wire = serde_json::to_value(residency).unwrap();
            assert_eq!(
                serde_json::from_value::<Residency>(wire).unwrap(),
                residency
            );
        }
        assert_eq!(
            serde_json::to_value(Residency::PinnedOff).unwrap(),
            json!({"mode":"pinned_off"})
        );
    }

    #[test]
    fn replies_carry_model_and_asset_states_flat() {
        let model = HostedModel {
            id: "qwen3-8b".into(),
            display_name: "Qwen3 8B".into(),
            kind: ModelKind::Chat,
            engine: ModelEngine::Llamacpp,
            assets: vec![digest(1)],
            asset_count: None,
            settings: ModelSettings::default(),
            residency: Residency::AlwaysOn,
            revision: 2,
            state: HostedModelState::Loaded {
                ram_bytes: 1,
                vram_bytes: 2,
                slots: 4,
                slots_busy: 1,
            },
        };
        let wire = serde_json::to_value(&model).unwrap();
        assert_eq!(wire["state"], "loaded");
        assert_eq!(wire["slots_busy"], 1);
        assert_eq!(serde_json::from_value::<HostedModel>(wire).unwrap(), model);
        model.validate().unwrap();
        let busy = HostedModel {
            state: HostedModelState::Loaded {
                ram_bytes: 1,
                vram_bytes: 2,
                slots: 1,
                slots_busy: 2,
            },
            ..model.clone()
        };
        assert!(busy.validate().is_err());
        let failed = HostedModel {
            state: HostedModelState::Failed {
                reason: ModelHostFailure::InsufficientMemory,
            },
            ..model
        };
        let wire = serde_json::to_value(&failed).unwrap();
        assert_eq!(
            (&wire["state"], &wire["reason"]),
            (&json!("failed"), &json!("insufficient_memory"))
        );
        assert_eq!(serde_json::from_value::<HostedModel>(wire).unwrap(), failed);

        let status = ModelAssetStatus {
            digest: digest(1),
            job_id: Some(JOB.into()),
            state: ModelAssetState::Fetching {
                source_index: 1,
                bytes: 7,
            },
        };
        let wire = serde_json::to_value(&status).unwrap();
        assert_eq!(
            wire,
            json!({"digest":serde_json::to_value(digest(1)).unwrap(),"job_id":JOB,"state":"fetching","source_index":1,"bytes":7})
        );
        assert_eq!(
            serde_json::from_value::<ModelAssetStatus>(wire).unwrap(),
            status
        );
        assert!(
            serde_json::from_value::<ModelAssetStatus>(
                json!({"digest":serde_json::to_value(digest(1)).unwrap(),"bytes":7})
            )
            .is_err()
        );
        let worst = worst_model();
        assert_eq!(
            serde_json::from_value::<HostedModel>(serde_json::to_value(&worst).unwrap()).unwrap(),
            worst
        );

        let job = ModelJob {
            job_id: JOB.into(),
            digest: digest(1),
            size: 42,
            file_name: "model.gguf".into(),
            source_host: Some("cdn.flow-like.com".into()),
            bytes_per_second: Some(1_000_000),
            sources: vec!["https://cdn.flow-like.com/bits/model".into()],
            updated_at: 100,
            state: ModelAssetState::AwaitingPush { bytes: 0 },
        };
        job.validate().unwrap();
        let wire = serde_json::to_value(&job).unwrap();
        assert_eq!(wire["state"], "awaiting_push");
        assert_eq!(
            wire["sources"],
            json!(["https://cdn.flow-like.com/bits/model"])
        );
        assert_eq!(serde_json::from_value::<ModelJob>(wire).unwrap(), job);
        let unlisted = ModelJob {
            sources: vec![],
            ..job.clone()
        };
        let wire = serde_json::to_value(&unlisted).unwrap();
        assert!(wire.get("sources").is_none());
        assert_eq!(serde_json::from_value::<ModelJob>(wire).unwrap(), unlisted);
        for host in ["", "cdn.flow-like.com/bits", "cdn flow"] {
            let bad = ModelJob {
                source_host: Some(host.into()),
                ..job.clone()
            };
            assert!(bad.validate().is_err(), "{host}");
        }
        let long = |byte: &str| format!("https://h.test/{}", byte.repeat(2_000));
        for sources in [
            vec!["http://cdn.flow-like.com/bits/model".to_owned()],
            vec!["https://h.test/m".to_owned(); MODEL_ASSET_MAX_SOURCES + 1],
            vec![long("a"), long("b"), long("c")],
        ] {
            let bad = ModelJob {
                sources,
                ..job.clone()
            };
            assert!(bad.validate().is_err());
        }
        let offered = vec![
            "http://plain.test/m".to_owned(),
            long("a"),
            long("b"),
            long("c"),
            "https://h.test/small".to_owned(),
        ];
        let listed = ModelJob::listed_sources(&offered);
        assert_eq!(listed, [long("a"), long("b")]);
        let listed = ModelJob {
            sources: listed,
            ..job.clone()
        };
        listed.validate().unwrap();

        let summary = ModelAssetSummary {
            total: 2,
            present: 1,
            pending: vec![status],
        };
        summary.validate().unwrap();
        let overfull = ModelAssetSummary {
            present: 2,
            ..summary.clone()
        };
        assert!(overfull.validate().is_err());
        let present = ModelAssetSummary {
            pending: vec![ModelAssetStatus {
                digest: digest(2),
                job_id: None,
                state: ModelAssetState::Present,
            }],
            ..summary
        };
        assert!(present.validate().is_err());
    }

    #[test]
    fn recommendations_fix_only_with_bounded_writes() {
        let mut recommendation = worst_recommendation();
        recommendation.validate().unwrap();
        let wire = serde_json::to_value(&recommendation).unwrap();
        assert_eq!(wire["fix"]["kind"], "configure");
        assert_eq!(
            serde_json::from_value::<Recommendation>(wire).unwrap(),
            recommendation
        );
        recommendation.fix = Some(ModelsRequest::InstallRuntime {
            runtime: ModelRuntime::Llamacpp,
            backend: ModelBackend::Vulkan,
            manifest_jws: None,
        });
        recommendation.validate().unwrap();
        recommendation.fix = Some(ModelsRequest::Remove {
            model_id: "m".into(),
            expected_revision: 1,
        });
        assert!(recommendation.validate().is_err());
        recommendation.fix = None;
        recommendation
            .params
            .insert("Upper".into(), RecommendationValue::Number(1));
        assert!(recommendation.validate().is_err());
        let numeric = Recommendation {
            code: RecommendationCode::CpuThreads,
            tier: RecommendationTier::Soon,
            model_id: None,
            params: [("physical_cores".to_string(), RecommendationValue::Number(8))].into(),
            fix: None,
        };
        assert_eq!(
            serde_json::to_value(&numeric).unwrap(),
            json!({"code":"cpu_threads","tier":"soon","params":{"physical_cores":8}})
        );
        assert!(
            serde_json::from_value::<Recommendation>(
                json!({"code":"cpu_threads","tier":"soon","params":{"cores":-1}})
            )
            .is_err()
        );
    }

    #[test]
    fn worst_case_replies_fit_one_encrypted_message() {
        let overview = worst_overview();
        overview.validate().unwrap();
        assert!(
            reply_bytes(&overview) <= MODELS_REPLY_MAX_BYTES,
            "overview with one model: {} bytes",
            reply_bytes(&overview)
        );
        let model_page = HostedModelPage {
            models: vec![worst_model()],
            next: Some("c".repeat(128)),
        };
        model_page.validate().unwrap();
        assert!(reply_bytes(&model_page) <= MODELS_REPLY_MAX_BYTES);
        let installed = ModelInstalled {
            model: worst_model(),
            assets: ModelAssetSummary {
                total: u32::MAX,
                present: 0,
                pending: vec![worst_pending(); MODEL_MAX_PENDING_ASSETS],
            },
        };
        installed.assets.validate().unwrap();
        assert!(
            reply_bytes(&installed) <= MODELS_REPLY_MAX_BYTES,
            "install answer: {} bytes",
            reply_bytes(&installed)
        );
        let job = ModelJob {
            job_id: JOB.into(),
            digest: digest(usize::MAX),
            size: MODEL_ASSET_MAX_BYTES,
            file_name: format!("{}/{}", "d".repeat(255), "f".repeat(255)),
            source_host: Some("h".repeat(MODEL_HOST_MAX_BYTES)),
            bytes_per_second: Some(u64::MAX),
            sources: (0..4)
                .map(|index| format!("https://h.test/{index}/{}", "s".repeat(1_007)))
                .collect(),
            updated_at: i64::MIN,
            state: worst_pending().state,
        };
        assert_eq!(
            job.sources.iter().map(String::len).sum::<usize>(),
            MODEL_JOB_SOURCES_MAX_BYTES
        );
        job.validate().unwrap();
        assert!(
            reply_bytes(&ModelJobPage {
                jobs: vec![job],
                next: Some(JOB.into())
            }) <= MODELS_REPLY_MAX_BYTES
        );
        let recommendations = RecommendationPage {
            recommendations: vec![worst_recommendation(); 8],
            next: Some("c".repeat(128)),
        };
        recommendations.validate().unwrap();
        assert!(reply_bytes(&recommendations) <= MODELS_REPLY_MAX_BYTES);
    }

    #[test]
    fn stats_are_dense_bounded_columns() {
        let points = MODEL_STATS_MAX_POINTS;
        let stats = ModelStats {
            model_id: Some("m".repeat(128)),
            from: 3_600,
            step: StatsStep::Hour,
            series: ModelStatsSeries {
                requests: vec![u64::MAX; points],
                errors: vec![u64::MAX; points],
                prompt_tokens: vec![u64::MAX; points],
                completion_tokens: vec![u64::MAX; points],
                cached_tokens: vec![u64::MAX; points],
                decode_ms: vec![u64::MAX; points],
                ttft_p50_ms: vec![Some(u32::MAX); points],
                ttft_p95_ms: vec![Some(u32::MAX); points],
                queue_wait_p95_ms: vec![Some(u32::MAX); points],
            },
            consumers: vec![
                ModelConsumerTotals {
                    consumer: ModelConsumer::Placement {
                        placement_id: "p".repeat(128),
                    },
                    requests: u64::MAX,
                    errors: u64::MAX,
                    prompt_tokens: u64::MAX,
                    completion_tokens: u64::MAX,
                };
                MODEL_STATS_MAX_CONSUMERS
            ],
        };
        stats.validate().unwrap();
        assert!(reply_bytes(&stats) <= MODELS_BULK_REPLY_MAX_BYTES);
        let wire = serde_json::to_value(&stats.consumers[0].consumer).unwrap();
        assert_eq!(
            wire,
            json!({"kind":"placement","placement_id":"p".repeat(128)})
        );

        let mut ragged = stats.clone();
        ragged.series.decode_ms.pop();
        assert!(ragged.validate().is_err());
        let mut unaligned = stats.clone();
        unaligned.from = 3_601;
        assert!(unaligned.validate().is_err());
        let mut crowded = stats;
        crowded.consumers.push(crowded.consumers[0].clone());
        assert!(crowded.validate().is_err());
    }

    #[test]
    fn shared_fixture_replies_validate_and_keep_their_wire_shape() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/models-v1.json")).unwrap();
        fn same<T: Serialize + serde::de::DeserializeOwned>(value: &serde_json::Value) -> T {
            let parsed: T = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(&serde_json::to_value(&parsed).unwrap(), value);
            parsed
        }
        same::<ModelsOverview>(&fixture["overview"])
            .validate()
            .unwrap();
        same::<ModelJobPage>(&fixture["jobs"]).validate().unwrap();
        let installed = same::<ModelInstalled>(&fixture["installed"]);
        installed.model.validate().unwrap();
        installed.assets.validate().unwrap();
        same::<ModelStats>(&fixture["stats"]).validate().unwrap();
    }

    #[test]
    fn system_facts_and_runtimes_are_bounded() {
        let overview = worst_overview();
        let mut system = overview.system.clone();
        system.validate().unwrap();
        system.cpu.features.push("extra".into());
        assert!(system.validate().is_err());
        let mut system = overview.system.clone();
        system.gpus[0].memory_free = Some(1);
        system.gpus[0].memory_total = Some(0);
        assert!(system.validate().is_err());
        let mut system = overview.system.clone();
        system.cpu.arch = "x86-64".into();
        assert!(system.validate().is_err());
        let mut system = overview.system;
        system.ram.total = 0;
        assert!(system.validate().is_err());
        let mut runtime = overview.runtimes[0].clone();
        runtime.build = "b10809 cuda".into();
        assert!(runtime.validate().is_err());
    }
}
