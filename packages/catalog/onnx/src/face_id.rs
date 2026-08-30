/// # face_id Nodes
/// Batteries-included face analysis (SCRFD detection + ArcFace recognition + gender/age)
/// backed by the `face_id` crate. Reuses the shared ONNX Runtime that flow-like initializes
/// globally, so these sessions inherit the process-wide execution providers.
///
/// The `face_id` crate loads its three ONNX models from local file paths only (no in-memory
/// loading), so the loader node materializes weights from a `FlowPath` cache directory: on the
/// first run it downloads the models and persists them into the cache dir; afterwards it reads
/// them straight from the `FlowPath` store.
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_catalog_core::{BoundingBox, FlowPath, NodeImage};
#[cfg(feature = "execute")]
use flow_like_storage::object_store::PutPayload;
use flow_like_types::{Result, anyhow, async_trait, json::json};
#[cfg(feature = "execute")]
use flow_like_types::{
    futures::StreamExt,
    tokio::io::{AsyncReadExt, AsyncWriteExt},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use sha2::{Digest, Sha256};
#[cfg(feature = "execute")]
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

/// Immutable default HuggingFace weights and their Git-LFS SHA-256 object IDs.
pub const DEFAULT_DETECTOR_URL: &str = "https://huggingface.co/RuteNL/SCRFD-face-detection-ONNX/resolve/3d9a1b3bc9f8a50635817929118fb9184f5bc30b/34g_gnkps.onnx";
pub const DEFAULT_DETECTOR_SHA256: &str =
    "aa19f0e7f4d120d4cf990086639ab74a0136adceaebd232e0dc4745e0cfd4257";
pub const DEFAULT_EMBEDDER_URL: &str = "https://huggingface.co/public-data/insightface/resolve/33c1063c49c785b7652d3fd529f86fa4f149392b/models/buffalo_l/w600k_r50.onnx";
pub const DEFAULT_EMBEDDER_SHA256: &str =
    "4c06341c33c2ca1f86781dab0e829f88ad5b64be9fba56e56bc9ebdefc619e43";
pub const DEFAULT_GENDER_AGE_URL: &str = "https://huggingface.co/public-data/insightface/resolve/33c1063c49c785b7652d3fd529f86fa4f149392b/models/buffalo_l/genderage.onnx";
pub const DEFAULT_GENDER_AGE_SHA256: &str =
    "4fde69b1c810857b88c64a335084f1c3fe8f01246c9a191b48c7bb756d6652fb";
const LEGACY_DETECTOR_URL: &str =
    "https://huggingface.co/RuteNL/SCRFD-face-detection-ONNX/resolve/main/34g_gnkps.onnx";
const LEGACY_EMBEDDER_URL: &str =
    "https://huggingface.co/public-data/insightface/resolve/main/models/buffalo_l/w600k_r50.onnx";
const LEGACY_GENDER_AGE_URL: &str =
    "https://huggingface.co/public-data/insightface/resolve/main/models/buffalo_l/genderage.onnx";

const MIN_DETECTOR_INPUT_SIZE: i64 = 32;
// face_id 0.4 performs NMS before returning detections. Keep the exposed search space
// conservative until the detector supports a pre-NMS candidate limit.
const MAX_DETECTOR_INPUT_SIZE: i64 = 640;
const DETECTOR_INPUT_SIZE_STEP: i64 = 32;
const MIN_SCORE_THRESHOLD: f64 = 0.25;
const MAX_IOU_THRESHOLD: f64 = 0.75;
const MAX_FACES: i64 = 100;
const DEFAULT_MAX_FACES: i64 = 100;
#[cfg(any(target_os = "android", target_os = "ios", target_os = "tvos"))]
#[cfg(feature = "execute")]
const FACE_BATCH_SIZE: usize = 4;
#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "tvos")))]
#[cfg(feature = "execute")]
const FACE_BATCH_SIZE: usize = 16;
#[cfg(feature = "execute")]
const FACE_EMBEDDING_DIMENSION: usize = 512;
#[cfg(any(target_os = "android", target_os = "ios", target_os = "tvos"))]
#[cfg(feature = "execute")]
const MAX_CONCURRENT_ANALYSES: usize = 1;
#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "tvos")))]
#[cfg(feature = "execute")]
const MAX_CONCURRENT_ANALYSES: usize = 2;
#[cfg(any(target_os = "android", target_os = "ios", target_os = "tvos"))]
#[cfg(feature = "execute")]
const MAX_CACHED_FACE_ANALYZERS: usize = 1;
#[cfg(not(any(target_os = "android", target_os = "ios", target_os = "tvos")))]
#[cfg(feature = "execute")]
const MAX_CACHED_FACE_ANALYZERS: usize = 2;
#[cfg(feature = "execute")]
const MODEL_UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;
#[cfg(all(
    feature = "execute",
    any(target_os = "android", target_os = "ios", target_os = "tvos")
))]
const MAX_NON_MULTIPART_CACHE_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(all(
    feature = "execute",
    not(any(target_os = "android", target_os = "ios", target_os = "tvos"))
))]
const MAX_NON_MULTIPART_CACHE_BYTES: u64 = 256 * 1024 * 1024;
#[cfg(all(
    feature = "execute",
    any(target_os = "android", target_os = "ios", target_os = "tvos")
))]
const MAX_MODEL_CACHE_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(all(
    feature = "execute",
    not(any(target_os = "android", target_os = "ios", target_os = "tvos"))
))]
const MAX_MODEL_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
#[cfg(all(
    any(feature = "execute", test),
    any(target_os = "android", target_os = "ios", target_os = "tvos")
))]
const MAX_SOURCE_IMAGE_PIXELS: u64 = 12_000_000;
#[cfg(all(
    any(feature = "execute", test),
    not(any(target_os = "android", target_os = "ios", target_os = "tvos"))
))]
const MAX_SOURCE_IMAGE_PIXELS: u64 = 24_000_000;

#[cfg(any(feature = "execute", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct ValidatedAnalyzerConfig {
    input_size: u32,
    score_threshold: f32,
    iou_threshold: f32,
}

#[cfg(any(feature = "execute", test))]
fn validate_analyzer_config(
    input_size: i64,
    score_threshold: f64,
    iou_threshold: f64,
) -> Result<ValidatedAnalyzerConfig> {
    if !(MIN_DETECTOR_INPUT_SIZE..=MAX_DETECTOR_INPUT_SIZE).contains(&input_size)
        || input_size % DETECTOR_INPUT_SIZE_STEP != 0
    {
        return Err(anyhow!(
            "Detector input size must be a multiple of {DETECTOR_INPUT_SIZE_STEP} between {MIN_DETECTOR_INPUT_SIZE} and {MAX_DETECTOR_INPUT_SIZE}, got {input_size}"
        ));
    }
    if !score_threshold.is_finite() || !(MIN_SCORE_THRESHOLD..=1.0).contains(&score_threshold) {
        return Err(anyhow!(
            "Score threshold must be finite and between {MIN_SCORE_THRESHOLD} and 1.0, got {score_threshold}"
        ));
    }
    if !iou_threshold.is_finite() || !(0.0..=MAX_IOU_THRESHOLD).contains(&iou_threshold) {
        return Err(anyhow!(
            "IoU threshold must be finite and between 0.0 and {MAX_IOU_THRESHOLD}, got {iou_threshold}"
        ));
    }

    Ok(ValidatedAnalyzerConfig {
        input_size: u32::try_from(input_size)
            .map_err(|_| anyhow!("Detector input size does not fit in u32: {input_size}"))?,
        score_threshold: score_threshold as f32,
        iou_threshold: iou_threshold as f32,
    })
}

#[cfg(any(feature = "execute", test))]
fn validate_max_faces(max_faces: i64) -> Result<usize> {
    if !(1..=MAX_FACES).contains(&max_faces) {
        return Err(anyhow!(
            "Maximum faces must be between 1 and {MAX_FACES}, got {max_faces}"
        ));
    }
    usize::try_from(max_faces).map_err(|_| anyhow!("Maximum faces does not fit in usize"))
}

#[cfg(any(feature = "execute", test))]
fn validate_source_image_dimensions(width: u32, height: u32) -> Result<()> {
    if width == 0 || height == 0 {
        return Err(anyhow!("Cannot analyze an empty image"));
    }
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or_else(|| anyhow!("Source image dimensions overflow"))?;
    if pixels > MAX_SOURCE_IMAGE_PIXELS {
        return Err(anyhow!(
            "Source image contains {pixels} pixels; the face analysis limit is {MAX_SOURCE_IMAGE_PIXELS}"
        ));
    }
    Ok(())
}

#[cfg(any(feature = "execute", test))]
fn validate_detector_projection(
    width: u32,
    height: u32,
    detector_input_size: (u32, u32),
) -> Result<()> {
    let (input_width, input_height) = detector_input_size;
    if input_width == 0 || input_height == 0 {
        return Err(anyhow!("Face detector input dimensions must be positive"));
    }
    let ratio = (f64::from(input_width) / f64::from(width))
        .min(f64::from(input_height) / f64::from(height));
    let projected_width = (f64::from(width) * ratio).round() as u32;
    let projected_height = (f64::from(height) * ratio).round() as u32;
    if projected_width == 0 || projected_height == 0 {
        return Err(anyhow!(
            "Source image aspect ratio is too extreme for the {input_width}x{input_height} face detector input"
        ));
    }
    Ok(())
}

#[cfg(any(feature = "execute", test))]
fn validate_model_cache_dir(cache_dir: &FlowPath) -> Result<()> {
    if cache_dir.path.trim().trim_matches('/').is_empty() {
        return Err(anyhow!(
            "Face models require a non-empty cache directory prefix; using a store root would make cache quota checks scan the entire store"
        ));
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn validate_model_set_size(model_sizes: [u64; 3]) -> Result<u64> {
    let total = model_sizes.into_iter().try_fold(0u64, |total, size| {
        total
            .checked_add(size)
            .ok_or_else(|| anyhow!("Combined face model size overflow"))
    })?;
    if total > MAX_MODEL_CACHE_BYTES {
        return Err(anyhow!(
            "Combined face models require {total} bytes, exceeding this target's {MAX_MODEL_CACHE_BYTES} byte cache quota"
        ));
    }
    Ok(total)
}

fn migrate_legacy_url_default(node: &mut Node, pin_name: &str, legacy: &str, pinned: &str) {
    let Some(pin) = node.get_pin_mut_by_name(pin_name) else {
        return;
    };
    let is_legacy = pin
        .default_value
        .as_deref()
        .and_then(|value| flow_like_types::json::from_slice::<String>(value).ok())
        .is_some_and(|value| value == legacy);
    if is_legacy {
        pin.default_value = flow_like_types::json::to_vec(pinned).ok();
    }
}

/// Handle to a cached `FaceAnalyzer` living in the execution context cache.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NodeFaceAnalyzer {
    /// Cache ID for the analyzer
    pub analyzer_ref: String,
    /// Per-call detector confidence threshold (sessions are shared across threshold variants)
    #[serde(default = "default_score_threshold")]
    #[schemars(default = "default_score_threshold")]
    pub score_threshold: f32,
    /// Per-call detector NMS threshold (sessions are shared across threshold variants)
    #[serde(default = "default_iou_threshold")]
    #[schemars(default = "default_iou_threshold")]
    pub iou_threshold: f32,
}

fn default_score_threshold() -> f32 {
    0.5
}

fn default_iou_threshold() -> f32 {
    0.4
}

/// One analyzed face: absolute-pixel geometry plus identity/attribute outputs.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct FaceIdResult {
    /// Face bounding box in pixels (shared object-detection type; carries the detection score)
    pub bbox: BoundingBox,
    /// 5-point landmarks in pixels [[x, y], ...], if the detector produced them
    pub landmarks: Option<Vec<[f32; 2]>>,
    /// 512-dimensional, L2-normalized identity embedding
    pub embedding: Vec<f32>,
    /// Estimated gender ("Male" / "Female")
    pub gender: String,
    /// Estimated age in years
    pub age: u8,
}

#[cfg(feature = "execute")]
type FaceAnalyzerCell =
    Arc<flow_like_types::tokio::sync::OnceCell<Arc<face_id::analyzer::FaceAnalyzer>>>;

/// Cache entry that keeps an analyzer alive only while a node still holds it.
#[cfg(feature = "execute")]
type WeakFaceAnalyzerCell =
    Weak<flow_like_types::tokio::sync::OnceCell<Arc<face_id::analyzer::FaceAnalyzer>>>;

/// Analyzer reference keyed by the model reference it was loaded from.
#[cfg(feature = "execute")]
type FaceAnalyzerRegistry = Mutex<HashMap<String, WeakFaceAnalyzerCell>>;

#[cfg(feature = "execute")]
pub struct NodeFaceAnalyzerWrapper {
    analyzer: FaceAnalyzerCell,
    analysis_limit: Arc<flow_like_types::tokio::sync::Semaphore>,
    claimed: AtomicBool,
    loaders: AtomicUsize,
}

#[cfg(feature = "execute")]
struct AnalysisPermits {
    _analyzer: flow_like_types::tokio::sync::OwnedSemaphorePermit,
    _global: flow_like_types::tokio::sync::OwnedSemaphorePermit,
    _cell: FaceAnalyzerCell,
}

#[cfg(feature = "execute")]
struct AnalyzerSlotGuard {
    cache: Arc<
        flow_like_types::tokio::sync::RwLock<
            ahash::AHashMap<String, Arc<dyn flow_like_types::Cacheable>>,
        >,
    >,
    key: String,
    entry: Option<Arc<dyn flow_like_types::Cacheable>>,
    armed: bool,
}

#[cfg(feature = "execute")]
impl AnalyzerSlotGuard {
    fn new(
        context: &ExecutionContext,
        key: &str,
        entry: Arc<dyn flow_like_types::Cacheable>,
    ) -> Result<Self> {
        let wrapper = entry
            .as_any()
            .downcast_ref::<NodeFaceAnalyzerWrapper>()
            .ok_or_else(|| anyhow!("Face analyzer cache entry changed type"))?;
        wrapper.loaders.fetch_add(1, Ordering::AcqRel);
        Ok(Self {
            cache: context.cache.clone(),
            key: key.to_string(),
            entry: Some(entry),
            armed: true,
        })
    }

    fn entry(&self) -> &Arc<dyn flow_like_types::Cacheable> {
        self.entry
            .as_ref()
            .expect("face analyzer slot guard was already disarmed")
    }

    fn claim(&mut self) -> Result<()> {
        let wrapper = self
            .entry
            .as_ref()
            .expect("face analyzer slot guard was already disarmed")
            .as_any()
            .downcast_ref::<NodeFaceAnalyzerWrapper>()
            .ok_or_else(|| anyhow!("Face analyzer cache entry changed type"))?;
        wrapper.claimed.store(true, Ordering::Release);
        wrapper.loaders.fetch_sub(1, Ordering::AcqRel);
        self.armed = false;
        Ok(())
    }
}

#[cfg(feature = "execute")]
impl Drop for AnalyzerSlotGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let entry = self
            .entry
            .take()
            .expect("face analyzer slot guard was already disarmed");
        let Some(wrapper) = entry.as_any().downcast_ref::<NodeFaceAnalyzerWrapper>() else {
            return;
        };
        if wrapper.loaders.fetch_sub(1, Ordering::AcqRel) != 1
            || wrapper.claimed.load(Ordering::Acquire)
        {
            return;
        }
        let cache = self.cache.clone();
        let key = self.key.clone();
        if let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() {
            std::mem::drop(runtime.spawn(async move {
                let mut cache = cache.write().await;
                let is_current = cache
                    .get(&key)
                    .is_some_and(|current| Arc::ptr_eq(current, &entry));
                let Some(wrapper) = entry.as_any().downcast_ref::<NodeFaceAnalyzerWrapper>() else {
                    return;
                };
                if is_current
                    && !wrapper.claimed.load(Ordering::Acquire)
                    && wrapper.loaders.load(Ordering::Acquire) == 0
                {
                    wrapper.analysis_limit.close();
                    cache.remove(&key);
                }
            }));
        }
    }
}

#[cfg(feature = "execute")]
fn global_analysis_limit() -> Arc<flow_like_types::tokio::sync::Semaphore> {
    static LIMIT: OnceLock<Arc<flow_like_types::tokio::sync::Semaphore>> = OnceLock::new();
    LIMIT
        .get_or_init(|| {
            let permits = if cfg!(any(
                target_os = "android",
                target_os = "ios",
                target_os = "tvos"
            )) {
                1
            } else {
                std::env::var("FLOW_LIKE_FACE_ANALYSIS_CONCURRENCY")
                    .ok()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|value| (1..=MAX_CONCURRENT_ANALYSES).contains(value))
                    .unwrap_or(1)
            };
            Arc::new(flow_like_types::tokio::sync::Semaphore::new(permits))
        })
        .clone()
}

#[cfg(feature = "execute")]
fn shared_analyzer_cell(analyzer_ref: &str) -> Result<FaceAnalyzerCell> {
    static ANALYZERS: OnceLock<FaceAnalyzerRegistry> = OnceLock::new();

    let mut analyzers = ANALYZERS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow!("Shared face analyzer registry was poisoned"))?;
    analyzers.retain(|_, analyzer| analyzer.strong_count() > 0);
    if let Some(analyzer) = analyzers.get(analyzer_ref).and_then(Weak::upgrade) {
        return Ok(analyzer);
    }
    if analyzers.len() >= MAX_CACHED_FACE_ANALYZERS {
        return Err(anyhow!(
            "The process-wide face analyzer limit ({MAX_CACHED_FACE_ANALYZERS}) was reached; unload an analyzer before loading another configuration"
        ));
    }
    let analyzer = Arc::new(flow_like_types::tokio::sync::OnceCell::new());
    analyzers.insert(analyzer_ref.to_string(), Arc::downgrade(&analyzer));
    Ok(analyzer)
}

#[cfg(feature = "execute")]
impl flow_like_types::Cacheable for NodeFaceAnalyzerWrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl NodeFaceAnalyzer {
    #[cfg(feature = "execute")]
    async fn get_or_insert_slot(
        ctx: &mut ExecutionContext,
        analyzer_ref: &str,
    ) -> Result<(FaceAnalyzerCell, AnalyzerSlotGuard)> {
        let mut cache = ctx.cache.write().await;
        cache.retain(|key, entry| {
            let Some(wrapper) = entry.as_any().downcast_ref::<NodeFaceAnalyzerWrapper>() else {
                return true;
            };
            let abandoned = key != analyzer_ref
                && !wrapper.claimed.load(Ordering::Acquire)
                && wrapper.loaders.load(Ordering::Acquire) == 0;
            if abandoned {
                wrapper.analysis_limit.close();
            }
            !abandoned
        });
        if let Some(entry) = cache.get(analyzer_ref).cloned() {
            let wrapper = entry
                .as_any()
                .downcast_ref::<NodeFaceAnalyzerWrapper>()
                .ok_or_else(|| anyhow!("Face analyzer cache key is occupied by another type"))?;
            let analyzer = wrapper.analyzer.clone();
            let slot_guard = AnalyzerSlotGuard::new(ctx, analyzer_ref, entry)?;
            return Ok((analyzer, slot_guard));
        }
        let cached_analyzers = cache
            .values()
            .filter(|entry| entry.as_any().is::<NodeFaceAnalyzerWrapper>())
            .count();
        if cached_analyzers >= MAX_CACHED_FACE_ANALYZERS {
            return Err(anyhow!(
                "The face analyzer cache limit ({MAX_CACHED_FACE_ANALYZERS}) was reached; unload an analyzer before loading another configuration"
            ));
        }

        let analyzer = shared_analyzer_cell(analyzer_ref)?;
        let entry: Arc<dyn flow_like_types::Cacheable> = Arc::new(NodeFaceAnalyzerWrapper {
            analyzer: analyzer.clone(),
            analysis_limit: Arc::new(flow_like_types::tokio::sync::Semaphore::new(
                MAX_CONCURRENT_ANALYSES,
            )),
            claimed: AtomicBool::new(false),
            loaders: AtomicUsize::new(0),
        });
        cache.insert(analyzer_ref.to_string(), entry.clone());
        let slot_guard = AnalyzerSlotGuard::new(ctx, analyzer_ref, entry)?;
        Ok((analyzer, slot_guard))
    }

    #[cfg(feature = "execute")]
    async fn get_analyzer(
        &self,
        ctx: &mut ExecutionContext,
    ) -> Result<(Arc<face_id::analyzer::FaceAnalyzer>, AnalysisPermits)> {
        let cached = ctx
            .cache
            .read()
            .await
            .get(&self.analyzer_ref)
            .cloned()
            .ok_or_else(|| anyhow!("Face analyzer not found in cache!"))?;
        let wrapper = cached
            .as_any()
            .downcast_ref::<NodeFaceAnalyzerWrapper>()
            .ok_or_else(|| anyhow!("Could not downcast to NodeFaceAnalyzerWrapper"))?;
        let analyzer = wrapper
            .analyzer
            .get()
            .cloned()
            .ok_or_else(|| anyhow!("Face analyzer is still loading"))?;
        let global_permit = global_analysis_limit()
            .acquire_owned()
            .await
            .map_err(|_| anyhow!("Global face analysis limiter was closed"))?;
        let analyzer_permit = wrapper
            .analysis_limit
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| anyhow!("Face analyzer concurrency limiter was closed"))?;
        Ok((
            analyzer,
            AnalysisPermits {
                _analyzer: analyzer_permit,
                _global: global_permit,
                _cell: wrapper.analyzer.clone(),
            },
        ))
    }

    #[cfg(feature = "execute")]
    async fn unload(&self, ctx: &mut ExecutionContext) -> bool {
        let mut cache = ctx.cache.write().await;
        let Some(entry) = cache.get(&self.analyzer_ref) else {
            return false;
        };
        let Some(wrapper) = entry.as_any().downcast_ref::<NodeFaceAnalyzerWrapper>() else {
            return false;
        };
        wrapper.analysis_limit.close();
        cache.remove(&self.analyzer_ref).is_some()
    }
}

#[cfg(feature = "execute")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModelRole {
    Detector,
    Embedder,
    GenderAge,
}

#[cfg(feature = "execute")]
impl ModelRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Detector => "detector",
            Self::Embedder => "embedder",
            Self::GenderAge => "gender-age",
        }
    }

    fn max_bytes(self) -> u64 {
        match self {
            Self::Detector => 128 * 1024 * 1024,
            Self::Embedder => 512 * 1024 * 1024,
            Self::GenderAge => 64 * 1024 * 1024,
        }
    }
}

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
struct ModelSpec {
    role: ModelRole,
    url: reqwest::Url,
    expected_sha256: String,
}

#[cfg(feature = "execute")]
impl ModelSpec {
    fn new(role: ModelRole, url: &str, expected_sha256: &str) -> Result<Self> {
        let mut url = reqwest::Url::parse(url)
            .map_err(|e| anyhow!("Invalid {} model URL: {e}", role.as_str()))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow!(
                "{} model URL must use http or https",
                role.as_str()
            ));
        }
        url.set_fragment(None);

        let expected_sha256 = expected_sha256.trim().to_ascii_lowercase();
        if expected_sha256.len() != 64
            || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(anyhow!(
                "{} model SHA-256 must contain exactly 64 hexadecimal characters",
                role.as_str()
            ));
        }

        Ok(Self {
            role,
            url,
            expected_sha256,
        })
    }

    fn cache_file_name(&self) -> String {
        let mut hasher = Sha256::new();
        hash_field(&mut hasher, b"flowlike-face-model-cache-v3");
        hash_field(&mut hasher, self.role.as_str().as_bytes());
        hash_field(&mut hasher, self.expected_sha256.as_bytes());
        format!(
            "face-id-{}-{}.onnx",
            self.role.as_str(),
            hex::encode(hasher.finalize())
        )
    }
}

#[cfg(feature = "execute")]
fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(feature = "execute")]
fn analyzer_cache_key(
    specs: &[ModelSpec; 3],
    config: ValidatedAnalyzerConfig,
    active_providers: &[String],
) -> String {
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, b"flowlike-face-analyzer-v3");
    for spec in specs {
        hash_field(&mut hasher, spec.role.as_str().as_bytes());
        hash_field(&mut hasher, spec.expected_sha256.as_bytes());
    }
    hash_field(&mut hasher, &config.input_size.to_be_bytes());
    for provider in active_providers {
        hash_field(&mut hasher, provider.as_bytes());
    }
    format!("face-analyzer:{}", hex::encode(hasher.finalize()))
}

#[cfg(feature = "execute")]
fn child_flow_path(cache_dir: &FlowPath, file_name: &str) -> FlowPath {
    let mut path = cache_dir.clone();
    let parent = cache_dir.path.trim_end_matches('/');
    path.path = if parent.is_empty() {
        file_name.to_string()
    } else {
        format!("{parent}/{file_name}")
    };
    path
}

#[cfg(feature = "execute")]
fn model_materialization_lock(
    cache_path: &FlowPath,
) -> Result<Arc<flow_like_types::tokio::sync::Mutex<()>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>> =
        OnceLock::new();

    let mut hasher = Sha256::new();
    let normalized_path = flow_like_storage::Path::from(cache_path.path.clone());
    hash_field(&mut hasher, cache_path.store_ref.as_bytes());
    hash_field(&mut hasher, normalized_path.as_ref().as_bytes());
    let key = hex::encode(hasher.finalize());
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow!("Face model materialization lock registry was poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(flow_like_types::tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

#[cfg(feature = "execute")]
fn model_cache_write_lock(
    cache_path: &FlowPath,
) -> Result<Arc<flow_like_types::tokio::sync::Mutex<()>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>> =
        OnceLock::new();

    let normalized_path = flow_like_storage::Path::from(cache_path.path.clone());
    let parent = normalized_path
        .as_ref()
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, cache_path.store_ref.as_bytes());
    hash_field(&mut hasher, parent.as_bytes());
    let key = hex::encode(hasher.finalize());
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow!("Face model cache write lock registry was poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(flow_like_types::tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

#[cfg(feature = "execute")]
enum ModelCacheAction {
    Persist(FlowPath),
    Promote {
        cache_path: FlowPath,
        source_etag: Option<String>,
    },
}

#[cfg(feature = "execute")]
struct MaterializedModel {
    cache_action: Option<ModelCacheAction>,
    _materialization_guard: flow_like_types::tokio::sync::OwnedMutexGuard<()>,
}

#[cfg(feature = "execute")]
enum CachedModelLookup {
    Miss,
    Hit(Option<ModelCacheAction>),
}

#[cfg(feature = "execute")]
async fn stream_cached_model(
    result: flow_like_storage::object_store::GetResult,
    spec: &ModelSpec,
    destination: &Path,
) -> Result<(bool, Option<String>)> {
    if result.meta.size > spec.role.max_bytes() {
        return Ok((false, result.meta.e_tag));
    }
    let source_etag = result.meta.e_tag.clone();
    let mut output = flow_like_types::tokio::fs::File::create(destination).await?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut stream = result.into_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("Failed to read cached face model: {e}"))?;
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow!("Cached face model size overflow"))?;
        if total > spec.role.max_bytes() {
            return Ok((false, source_etag));
        }
        hasher.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    Ok((
        hex::encode(hasher.finalize()) == spec.expected_sha256,
        source_etag,
    ))
}

#[cfg(feature = "execute")]
async fn try_materialize_cached_model(
    context: &mut ExecutionContext,
    cache_dir: &FlowPath,
    spec: &ModelSpec,
    destination: &Path,
) -> Result<CachedModelLookup> {
    let cache_path = child_flow_path(cache_dir, &spec.cache_file_name());
    let (result, dirty) = cache_path.get_cached_file(context).await?;
    let Some(result) = result else {
        return Ok(CachedModelLookup::Miss);
    };
    let (valid, source_etag) = stream_cached_model(result, spec, destination).await?;
    if valid {
        let action = dirty.then_some(ModelCacheAction::Promote {
            cache_path,
            source_etag,
        });
        return Ok(CachedModelLookup::Hit(action));
    }

    if !dirty {
        // A matching ETag does not guarantee the local cache bytes are intact. Fall back
        // to the primary object before requiring network access to the model URL.
        let runtime = cache_path.to_runtime(context).await?;
        if let Ok(primary) = runtime.store.as_generic().get(&runtime.path).await {
            let (valid, source_etag) = stream_cached_model(primary, spec, destination).await?;
            if valid {
                return Ok(CachedModelLookup::Hit(Some(ModelCacheAction::Promote {
                    cache_path,
                    source_etag,
                })));
            }
        }
    }
    Ok(CachedModelLookup::Miss)
}

#[cfg(feature = "execute")]
async fn materialize_model(
    context: &mut ExecutionContext,
    cache_dir: &FlowPath,
    client: &reqwest::Client,
    spec: &ModelSpec,
    destination: &Path,
) -> Result<MaterializedModel> {
    let cache_path = child_flow_path(cache_dir, &spec.cache_file_name());
    let materialization_lock = model_materialization_lock(&cache_path)?;
    let materialization_guard = materialization_lock.lock_owned().await;

    match try_materialize_cached_model(context, cache_dir, spec, destination).await {
        Ok(CachedModelLookup::Hit(cache_action)) => {
            return Ok(MaterializedModel {
                cache_action,
                _materialization_guard: materialization_guard,
            });
        }
        Ok(CachedModelLookup::Miss) => context.log_message(
            &format!(
                "Cached {} face model is missing or invalid; downloading a verified copy",
                spec.role.as_str()
            ),
            flow_like::flow::execution::LogLevel::Info,
        ),
        Err(error) => context.log_message(
            &format!(
                "Failed to read cached {} face model; downloading it again: {error}",
                spec.role.as_str()
            ),
            flow_like::flow::execution::LogLevel::Warn,
        ),
    }

    let mut response = client
        .get(spec.url.clone())
        .send()
        .await
        .map_err(|e| anyhow!("Failed to download {} face model: {e}", spec.role.as_str()))?
        .error_for_status()
        .map_err(|e| anyhow!("Failed to download {} face model: {e}", spec.role.as_str()))?;
    if response
        .content_length()
        .is_some_and(|length| length > spec.role.max_bytes())
    {
        return Err(anyhow!(
            "{} face model exceeds the {} byte size limit",
            spec.role.as_str(),
            spec.role.max_bytes()
        ));
    }

    let mut output = flow_like_types::tokio::fs::File::create(destination).await?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow!("Failed to read {} model body: {e}", spec.role.as_str()))?
    {
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow!("Downloaded face model size overflow"))?;
        if total > spec.role.max_bytes() {
            return Err(anyhow!(
                "{} face model exceeds the {} byte size limit",
                spec.role.as_str(),
                spec.role.max_bytes()
            ));
        }
        hasher.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;

    let actual_sha256 = hex::encode(hasher.finalize());
    if actual_sha256 != spec.expected_sha256 {
        return Err(anyhow!(
            "{} face model SHA-256 mismatch: expected {}, got {actual_sha256}",
            spec.role.as_str(),
            spec.expected_sha256
        ));
    }

    Ok(MaterializedModel {
        cache_action: Some(ModelCacheAction::Persist(cache_path)),
        _materialization_guard: materialization_guard,
    })
}

#[cfg(feature = "execute")]
async fn persist_model_streaming(
    context: &mut ExecutionContext,
    cache_path: &FlowPath,
    source: &Path,
    protected_paths: &[flow_like_storage::Path],
) -> Result<()> {
    let cache_write_lock = model_cache_write_lock(cache_path)?;
    let _cache_write_guard = cache_write_lock.lock_owned().await;
    let runtime = cache_path.to_runtime(context).await?;
    let incoming_size = flow_like_types::tokio::fs::metadata(source).await?.len();
    enforce_model_cache_quota(&runtime, incoming_size, protected_paths).await?;
    let result = upload_model_file(runtime.store.as_generic(), &runtime.path, source).await?;
    if let Some(cache_store) = runtime.cache_store {
        let cache_store = cache_store.as_generic();
        upload_model_file(cache_store.clone(), &runtime.path, source).await?;
        write_cache_etag(cache_store, &runtime.path, result.e_tag).await?;
    }
    Ok(())
}

#[cfg(feature = "execute")]
async fn enforce_model_cache_quota(
    runtime: &flow_like_catalog_core::FlowPathRuntime,
    incoming_size: u64,
    protected_paths: &[flow_like_storage::Path],
) -> Result<()> {
    if incoming_size > MAX_MODEL_CACHE_BYTES {
        return Err(anyhow!(
            "Face model exceeds the {MAX_MODEL_CACHE_BYTES} byte cache quota"
        ));
    }

    let destination = runtime.path.as_ref();
    let parent = destination
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let prefix = flow_like_storage::Path::from(format!("{parent}/"));
    let primary_store = runtime.store.as_generic();
    let mut listing = primary_store.list(Some(&prefix));
    let mut cached_models = Vec::new();
    let mut cached_bytes = 0u64;
    while let Some(object) = listing.next().await {
        let object = object.map_err(|e| anyhow!("Failed to inspect face model cache: {e}"))?;
        if object.location != runtime.path && is_managed_face_model_path(&object.location, parent) {
            cached_bytes = cached_bytes
                .checked_add(object.size)
                .ok_or_else(|| anyhow!("Face model cache size overflow"))?;
            if !protected_paths.contains(&object.location) {
                cached_models.push(object);
            }
        }
    }

    let mut projected = cached_bytes
        .checked_add(incoming_size)
        .ok_or_else(|| anyhow!("Face model cache size overflow"))?;
    if projected <= MAX_MODEL_CACHE_BYTES {
        return Ok(());
    }

    cached_models.sort_unstable_by_key(|object| object.last_modified);
    for object in cached_models {
        primary_store
            .delete(&object.location)
            .await
            .map_err(|e| anyhow!("Failed to evict cached face model: {e}"))?;
        if let Some(cache_store) = &runtime.cache_store {
            let cache_store = cache_store.as_generic();
            let _ = cache_store.delete(&object.location).await;
            let _ = cache_store
                .delete(&model_cache_etag_path(&object.location))
                .await;
        }
        projected = projected.saturating_sub(object.size);
        if projected <= MAX_MODEL_CACHE_BYTES {
            break;
        }
    }

    if projected > MAX_MODEL_CACHE_BYTES {
        return Err(anyhow!(
            "Could not free enough space within the {MAX_MODEL_CACHE_BYTES} byte face model cache quota"
        ));
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn is_managed_face_model_path(path: &flow_like_storage::Path, expected_parent: &str) -> bool {
    let path = path.as_ref();
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    if parent != expected_parent {
        return false;
    }
    let Some(hash) = [
        "face-id-detector-",
        "face-id-embedder-",
        "face-id-gender-age-",
    ]
    .into_iter()
    .find_map(|prefix| name.strip_prefix(prefix))
    .and_then(|name| name.strip_suffix(".onnx")) else {
        return false;
    };
    hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(feature = "execute")]
async fn promote_model_to_cache(
    context: &mut ExecutionContext,
    cache_path: &FlowPath,
    source: &Path,
    source_etag: Option<String>,
) -> Result<()> {
    let runtime = cache_path.to_runtime(context).await?;
    let Some(cache_store) = runtime.cache_store else {
        return Ok(());
    };
    let cache_store = cache_store.as_generic();
    upload_model_file(cache_store.clone(), &runtime.path, source).await?;
    write_cache_etag(cache_store, &runtime.path, source_etag).await
}

#[cfg(feature = "execute")]
struct MultipartAbortGuard {
    upload: Option<Box<dyn flow_like_storage::object_store::MultipartUpload>>,
}

#[cfg(feature = "execute")]
impl MultipartAbortGuard {
    fn new(upload: Box<dyn flow_like_storage::object_store::MultipartUpload>) -> Self {
        Self {
            upload: Some(upload),
        }
    }

    fn upload_mut(&mut self) -> &mut dyn flow_like_storage::object_store::MultipartUpload {
        self.upload
            .as_deref_mut()
            .expect("multipart upload guard was already disarmed")
    }

    async fn abort(&mut self) -> Option<flow_like_storage::object_store::Error> {
        let mut upload = self.upload.take()?;
        match flow_like_types::tokio::spawn(async move { upload.abort().await }).await {
            Ok(result) => result.err(),
            Err(error) => Some(error.into()),
        }
    }

    fn disarm(&mut self) {
        self.upload = None;
    }
}

#[cfg(feature = "execute")]
impl Drop for MultipartAbortGuard {
    fn drop(&mut self) {
        let Some(mut upload) = self.upload.take() else {
            return;
        };
        if let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() {
            // Cancellation can drop this async function at any await. Detach cleanup so
            // S3/GCS multipart parts are not orphaned when that happens.
            std::mem::drop(runtime.spawn(async move {
                let _ = upload.abort().await;
            }));
        }
    }
}

#[cfg(feature = "execute")]
async fn upload_model_file(
    store: Arc<dyn flow_like_storage::object_store::ObjectStore>,
    destination: &flow_like_storage::Path,
    source: &Path,
) -> Result<flow_like_storage::object_store::PutResult> {
    let mut input = flow_like_types::tokio::fs::File::open(source).await?;
    let upload = match store.put_multipart(destination).await {
        Ok(upload) => upload,
        Err(multipart_error) => {
            if !matches!(
                &multipart_error,
                flow_like_storage::object_store::Error::NotSupported { .. }
                    | flow_like_storage::object_store::Error::NotImplemented
            ) {
                return Err(anyhow!(
                    "Failed to start multipart model cache upload: {multipart_error}"
                ));
            }
            let size = input.metadata().await?.len();
            if size > MAX_NON_MULTIPART_CACHE_BYTES {
                return Err(anyhow!(
                    "Model cache store lacks multipart uploads and the {size} byte model exceeds the {MAX_NON_MULTIPART_CACHE_BYTES} byte fallback limit"
                ));
            }
            let mut bytes = Vec::with_capacity(size as usize);
            input.read_to_end(&mut bytes).await?;
            return store
                .put(destination, PutPayload::from(bytes))
                .await
                .map_err(|put_error| {
                    anyhow!(
                        "Multipart upload is unavailable ({multipart_error}); bounded fallback upload failed: {put_error}"
                    )
                });
        }
    };
    let mut upload = MultipartAbortGuard::new(upload);

    loop {
        let mut chunk = vec![0u8; MODEL_UPLOAD_CHUNK_BYTES];
        let read = match input.read(&mut chunk).await {
            Ok(read) => read,
            Err(error) => {
                let abort_error = upload.abort().await;
                return Err(upload_error_with_cleanup(
                    "Failed to read model cache source",
                    error,
                    abort_error,
                ));
            }
        };
        if read == 0 {
            break;
        }
        chunk.truncate(read);
        if let Err(error) = upload.upload_mut().put_part(PutPayload::from(chunk)).await {
            let abort_error = upload.abort().await;
            return Err(upload_error_with_cleanup(
                "Failed to upload model cache chunk",
                error,
                abort_error,
            ));
        }
    }

    match upload.upload_mut().complete().await {
        Ok(result) => {
            upload.disarm();
            Ok(result)
        }
        Err(error) => {
            let abort_error = upload.abort().await;
            Err(upload_error_with_cleanup(
                "Failed to complete model cache upload",
                error,
                abort_error,
            ))
        }
    }
}

#[cfg(feature = "execute")]
fn upload_error_with_cleanup(
    operation: &str,
    error: impl std::fmt::Display,
    abort_error: Option<impl std::fmt::Display>,
) -> flow_like_types::Error {
    anyhow!(
        "{operation}: {error}{}",
        abort_error
            .map(|abort| format!("; upload cleanup also failed: {abort}"))
            .unwrap_or_default()
    )
}

#[cfg(feature = "execute")]
fn model_cache_etag_path(path: &flow_like_storage::Path) -> flow_like_storage::Path {
    let extension = path.extension().unwrap_or_default().to_string();
    let raw_path = path.as_ref();
    let suffix = format!(".{extension}");
    let base_path = if extension.is_empty() {
        raw_path
    } else {
        raw_path.strip_suffix(&suffix).unwrap_or(raw_path)
    };
    flow_like_storage::Path::from(format!("{base_path}.s3flowEtag"))
}

#[cfg(feature = "execute")]
async fn write_cache_etag(
    cache_store: Arc<dyn flow_like_storage::object_store::ObjectStore>,
    path: &flow_like_storage::Path,
    etag: Option<String>,
) -> Result<()> {
    let Some(etag) = etag else {
        return Ok(());
    };
    let etag_path = model_cache_etag_path(path);
    cache_store
        .put(&etag_path, PutPayload::from(etag))
        .await
        .map_err(|e| anyhow!("Failed to write model cache ETag: {e}"))?;
    Ok(())
}

#[cfg(feature = "execute")]
async fn apply_model_cache_action(
    context: &mut ExecutionContext,
    action: ModelCacheAction,
    source: &Path,
    protected_paths: &[flow_like_storage::Path],
) -> Result<()> {
    match action {
        ModelCacheAction::Persist(cache_path) => {
            persist_model_streaming(context, &cache_path, source, protected_paths).await
        }
        ModelCacheAction::Promote {
            cache_path,
            source_etag,
        } => promote_model_to_cache(context, &cache_path, source, source_etag).await,
    }
}

#[cfg(feature = "execute")]
async fn build_face_analyzer(
    context: &mut ExecutionContext,
    cache_dir: &FlowPath,
    specs: &[ModelSpec; 3],
    config: ValidatedAnalyzerConfig,
) -> Result<Arc<face_id::analyzer::FaceAnalyzer>> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(30 * 60))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| anyhow!("Failed to create face model download client: {e}"))?;

    let temp_dir = tempfile::Builder::new()
        .prefix("flowlike-faceid-")
        .tempdir()
        .map_err(|e| anyhow!("Failed to create temp dir for face models: {e}"))?;
    let detector_path = temp_dir.path().join("detector.onnx");
    let embedder_path = temp_dir.path().join("embedder.onnx");
    let gender_age_path = temp_dir.path().join("genderage.onnx");

    let detector_model =
        materialize_model(context, cache_dir, &client, &specs[0], &detector_path).await?;
    let embedder_model =
        materialize_model(context, cache_dir, &client, &specs[1], &embedder_path).await?;
    let gender_age_model =
        materialize_model(context, cache_dir, &client, &specs[2], &gender_age_path).await?;

    validate_model_set_size([
        flow_like_types::tokio::fs::metadata(&detector_path)
            .await?
            .len(),
        flow_like_types::tokio::fs::metadata(&embedder_path)
            .await?
            .len(),
        flow_like_types::tokio::fs::metadata(&gender_age_path)
            .await?
            .len(),
    ])?;
    let protected_cache_paths: [flow_like_storage::Path; 3] = std::array::from_fn(|index| {
        flow_like_storage::Path::from(
            child_flow_path(cache_dir, &specs[index].cache_file_name()).path,
        )
    });

    let build_detector_path = detector_path.clone();
    let build_embedder_path = embedder_path.clone();
    let build_gender_age_path = gender_age_path.clone();
    let execution_providers = super::execution_providers::session_execution_providers(true)?;
    let (analyzer, temp_dir) = flow_like_types::tokio::task::spawn_blocking(move || {
        let analyzer = face_id::analyzer::FaceAnalyzer::builder(
            build_detector_path,
            build_embedder_path,
            build_gender_age_path,
        )
        .detector_input_size((config.input_size, config.input_size))
        .detector_score_threshold(config.score_threshold)
        .detector_iou_threshold(config.iou_threshold)
        .with_execution_providers(&execution_providers)
        .build();
        (analyzer, temp_dir)
    })
    .await
    .map_err(|e| anyhow!("Face analyzer build task panicked: {e}"))?;
    let analyzer = analyzer.map_err(|e| anyhow!("Failed to build face analyzer: {e}"))?;

    let MaterializedModel {
        cache_action: detector_action,
        _materialization_guard: detector_guard,
    } = detector_model;
    let MaterializedModel {
        cache_action: embedder_action,
        _materialization_guard: embedder_guard,
    } = embedder_model;
    let MaterializedModel {
        cache_action: gender_age_action,
        _materialization_guard: gender_age_guard,
    } = gender_age_model;
    let model_actions = [
        (
            detector_action,
            detector_path,
            specs[0].role,
            protected_cache_paths[0].clone(),
        ),
        (
            embedder_action,
            embedder_path,
            specs[1].role,
            protected_cache_paths[1].clone(),
        ),
        (
            gender_age_action,
            gender_age_path,
            specs[2].role,
            protected_cache_paths[2].clone(),
        ),
    ];
    let mut protected_cache_paths: Vec<_> = model_actions
        .iter()
        .filter(|(action, _, _, _)| !matches!(action.as_ref(), Some(ModelCacheAction::Persist(_))))
        .map(|(_, _, _, cache_path)| cache_path.clone())
        .collect();
    for (action, source, role, cache_path) in model_actions {
        if let Some(action) = action {
            let protect_after_write = matches!(&action, ModelCacheAction::Persist(_));
            match apply_model_cache_action(context, action, &source, &protected_cache_paths).await {
                Ok(()) if protect_after_write => protected_cache_paths.push(cache_path),
                Ok(()) => {}
                Err(error) => {
                    context.log_message(
                        &format!(
                            "Failed to persist verified {} face model: {error}",
                            role.as_str()
                        ),
                        flow_like::flow::execution::LogLevel::Warn,
                    );
                }
            }
        }
    }
    drop((detector_guard, embedder_guard, gender_age_guard));

    match flow_like_types::tokio::task::spawn_blocking(move || temp_dir.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::warn!(%error, "failed to remove temporary face-model directory"),
        Err(error) => tracing::warn!(%error, "temporary face-model cleanup task panicked"),
    }

    Ok(Arc::new(analyzer))
}

#[crate::register_node]
#[derive(Default)]
pub struct LoadFaceAnalyzerNode {}

impl LoadFaceAnalyzerNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LoadFaceAnalyzerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "face_id_load_analyzer",
            "Load Face Analyzer",
            "Load a face_id analyzer (SCRFD detector + ArcFace embedder + gender/age). Weights are verified and cached when a session identity is first built; equivalent analyzers reuse process-wide sessions.",
            "AI/ML/ONNX/Face",
        );
        node.set_flowscript_name("onnx", "faceIdLoadAnalyzer");
        node.set_version(2);
        node.add_icon("/flow/icons/find_model.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "cache_dir",
            "Cache Dir",
            "FlowPath used when this analyzer identity needs to build its ONNX sessions. If it is already resident, an alternate cache directory is not populated.",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "detector_url",
            "Detector URL",
            "Immutable SCRFD detector weights URL",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_DETECTOR_URL)));

        node.add_input_pin(
            "detector_sha256",
            "Detector SHA-256",
            "Required SHA-256 checksum for the detector weights",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_DETECTOR_SHA256)));

        node.add_input_pin(
            "embedder_url",
            "Embedder URL",
            "Immutable ArcFace recognition weights URL",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_EMBEDDER_URL)));

        node.add_input_pin(
            "embedder_sha256",
            "Embedder SHA-256",
            "Required SHA-256 checksum for the recognition weights",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_EMBEDDER_SHA256)));

        node.add_input_pin(
            "gender_age_url",
            "Gender/Age URL",
            "Immutable gender & age estimation weights URL",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_GENDER_AGE_URL)));

        node.add_input_pin(
            "gender_age_sha256",
            "Gender/Age SHA-256",
            "Required SHA-256 checksum for the gender & age weights",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_GENDER_AGE_SHA256)));

        node.add_input_pin(
            "input_size",
            "Detector Input Size",
            "Square detector input size",
            VariableType::Integer,
        )
        .set_options(
            PinOptions::new()
                .set_range((
                    MIN_DETECTOR_INPUT_SIZE as f64,
                    MAX_DETECTOR_INPUT_SIZE as f64,
                ))
                .set_step(DETECTOR_INPUT_SIZE_STEP as f64)
                .build(),
        )
        .set_default_value(Some(json!(640)));

        node.add_input_pin(
            "score_threshold",
            "Score Threshold",
            "Detector confidence threshold",
            VariableType::Float,
        )
        .set_options(
            PinOptions::new()
                .set_range((MIN_SCORE_THRESHOLD, 1.0))
                .set_step(0.01)
                .build(),
        )
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "iou_threshold",
            "IoU Threshold",
            "Detector non-maximum-suppression IoU threshold",
            VariableType::Float,
        )
        .set_options(
            PinOptions::new()
                .set_range((0.0, MAX_IOU_THRESHOLD))
                .set_step(0.01)
                .build(),
        )
        .set_default_value(Some(json!(0.4)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "analyzer",
            "Analyzer",
            "Cached face analyzer handle",
            VariableType::Struct,
        )
        .set_schema::<NodeFaceAnalyzer>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let cache_dir: FlowPath = context.evaluate_pin("cache_dir").await?;
            validate_model_cache_dir(&cache_dir)?;
            let detector_url: String = context.evaluate_pin("detector_url").await?;
            let detector_sha256: String = context.evaluate_pin("detector_sha256").await?;
            let embedder_url: String = context.evaluate_pin("embedder_url").await?;
            let embedder_sha256: String = context.evaluate_pin("embedder_sha256").await?;
            let gender_age_url: String = context.evaluate_pin("gender_age_url").await?;
            let gender_age_sha256: String = context.evaluate_pin("gender_age_sha256").await?;
            let input_size: i64 = context.evaluate_pin("input_size").await?;
            let score_threshold: f64 = context.evaluate_pin("score_threshold").await?;
            let iou_threshold: f64 = context.evaluate_pin("iou_threshold").await?;

            let config = validate_analyzer_config(input_size, score_threshold, iou_threshold)?;
            let specs = [
                ModelSpec::new(ModelRole::Detector, &detector_url, &detector_sha256)?,
                ModelSpec::new(ModelRole::Embedder, &embedder_url, &embedder_sha256)?,
                ModelSpec::new(ModelRole::GenderAge, &gender_age_url, &gender_age_sha256)?,
            ];

            // Face ID inherits the Apple/Android environment providers. Its fork also applies
            // DirectML's mandatory session options, so Windows receives the complete shared
            // provider order as well.
            let ep_info = super::execution_providers::ensure_ort_initialized()?;
            let analyzer_ref = analyzer_cache_key(&specs, config, &ep_info.active_providers);
            let (cell, mut slot_guard) =
                NodeFaceAnalyzer::get_or_insert_slot(context, &analyzer_ref).await?;
            cell.get_or_try_init(|| build_face_analyzer(context, &cache_dir, &specs, config))
                .await?;

            let still_registered = context
                .cache
                .read()
                .await
                .get(&analyzer_ref)
                .is_some_and(|current| Arc::ptr_eq(current, slot_guard.entry()));
            if !still_registered {
                return Err(anyhow!(
                    "Face analyzer was unloaded while it was being built"
                ));
            }

            let handle = NodeFaceAnalyzer {
                analyzer_ref,
                score_threshold: config.score_threshold,
                iou_threshold: config.iou_threshold,
            };
            context.set_pin_value("analyzer", json!(handle)).await?;
            context.activate_exec_pin("exec_out").await?;
            slot_guard.claim()?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!(
            "Face analysis requires the 'execute' feature. Rebuild with --features execute"
        ))
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        migrate_legacy_url_default(
            node,
            "detector_url",
            LEGACY_DETECTOR_URL,
            DEFAULT_DETECTOR_URL,
        );
        migrate_legacy_url_default(
            node,
            "embedder_url",
            LEGACY_EMBEDDER_URL,
            DEFAULT_EMBEDDER_URL,
        );
        migrate_legacy_url_default(
            node,
            "gender_age_url",
            LEGACY_GENDER_AGE_URL,
            DEFAULT_GENDER_AGE_URL,
        );
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct UnloadFaceAnalyzerNode {}

impl UnloadFaceAnalyzerNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UnloadFaceAnalyzerNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "face_id_unload_analyzer",
            "Unload Face Analyzer",
            "Release a cached face analyzer and its three ONNX sessions. Equivalent analyzer handles share the same cache entry and are invalidated together.",
            "AI/ML/ONNX/Face",
        );
        node.set_flowscript_name("onnx", "faceIdUnloadAnalyzer");
        node.set_version(1);
        node.add_icon("/flow/icons/find_model.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "analyzer",
            "Analyzer",
            "Face analyzer handle to unload",
            VariableType::Struct,
        )
        .set_schema::<NodeFaceAnalyzer>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );
        node.add_output_pin(
            "success",
            "Success",
            "Whether a face analyzer cache entry was removed",
            VariableType::Boolean,
        );
        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;
            let analyzer: NodeFaceAnalyzer = context.evaluate_pin("analyzer").await?;
            let success = analyzer.unload(context).await;
            context.set_pin_value("success", json!(success)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!(
            "Face analysis requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
}

#[cfg(feature = "execute")]
fn face_result_from_parts(
    detection: face_id::detector::DetectedFace,
    embedding: Vec<f32>,
    gender: face_id::gender_age::Gender,
    age: u8,
    width: u32,
    height: u32,
) -> Result<FaceIdResult> {
    if width == 0 || height == 0 {
        return Err(anyhow!("Cannot analyze an empty image"));
    }
    if embedding.len() != FACE_EMBEDDING_DIMENSION {
        return Err(anyhow!(
            "Face embedder output dimension mismatch: expected {FACE_EMBEDDING_DIMENSION}, got {}",
            embedding.len()
        ));
    }
    if !embedding.iter().all(|value| value.is_finite()) {
        return Err(anyhow!("Face embedder returned non-finite values"));
    }

    let face_id::detector::DetectedFace {
        bbox: source_bbox,
        landmarks,
        score,
    } = detection;
    if ![
        source_bbox.x1,
        source_bbox.y1,
        source_bbox.x2,
        source_bbox.y2,
        score,
    ]
    .into_iter()
    .all(f32::is_finite)
    {
        return Err(anyhow!("Face detector returned non-finite coordinates"));
    }

    let w = width as f32;
    let h = height as f32;
    let mut bbox = BoundingBox {
        x1: source_bbox.x1,
        y1: source_bbox.y1,
        x2: source_bbox.x2,
        y2: source_bbox.y2,
        score,
        class_name: Some("face".to_string()),
        ..Default::default()
    };
    bbox.scale(w, h);

    let landmarks = match landmarks {
        Some(points) => {
            if points.len() != 5 {
                return Err(anyhow!(
                    "Face detector returned {} landmarks; expected 5",
                    points.len()
                ));
            }
            let mut scaled = Vec::with_capacity(points.len());
            for (x, y) in points {
                if !x.is_finite() || !y.is_finite() {
                    return Err(anyhow!("Face detector returned non-finite landmarks"));
                }
                scaled.push([x * w, y * h]);
            }
            Some(scaled)
        }
        None => None,
    };

    Ok(FaceIdResult {
        bbox,
        landmarks,
        embedding,
        gender: match gender {
            face_id::gender_age::Gender::Female => "Female".to_string(),
            face_id::gender_age::Gender::Male => "Male".to_string(),
        },
        age,
    })
}

#[cfg(feature = "execute")]
fn attribute_crop_bounded(
    image: &flow_like_types::image::RgbImage,
    bbox: &face_id::detector::BoundingBox,
    output_size: u32,
) -> Result<flow_like_types::image::RgbImage> {
    use flow_like_types::image::imageops::{FilterType, crop_imm, overlay, resize};

    if output_size == 0 {
        return Err(anyhow!("Attribute crop output size must be positive"));
    }
    let (image_width, image_height) = image.dimensions();
    if image_width == 0 || image_height == 0 {
        return Err(anyhow!("Cannot crop an empty image"));
    }

    let bbox = bbox.scale(image_width, image_height);
    let side = bbox.width().max(bbox.height()) * 1.5;
    let center_x = bbox.x1 + bbox.width() / 2.0;
    let center_y = bbox.y1 + bbox.height() / 2.0;
    if !side.is_finite() || side <= 0.0 || !center_x.is_finite() || !center_y.is_finite() {
        return Err(anyhow!("Face detector returned an invalid attribute crop"));
    }

    // Resample only the in-bounds intersection directly into the fixed 96x96 result.
    // The upstream helper first allocates a square canvas proportional to the expanded
    // bounding box, which can be hundreds of MiB for large images or out-of-frame boxes.
    let left = center_x - side / 2.0;
    let top = center_y - side / 2.0;
    let right = center_x + side / 2.0;
    let bottom = center_y + side / 2.0;
    let source_left = left.max(0.0).min(image_width as f32);
    let source_top = top.max(0.0).min(image_height as f32);
    let source_right = right.max(0.0).min(image_width as f32);
    let source_bottom = bottom.max(0.0).min(image_height as f32);
    let mut output = flow_like_types::image::RgbImage::new(output_size, output_size);
    if source_right <= source_left || source_bottom <= source_top {
        return Ok(output);
    }

    let source_x = source_left.floor() as u32;
    let source_y = source_top.floor() as u32;
    let source_x2 = (source_right.ceil() as u32).min(image_width);
    let source_y2 = (source_bottom.ceil() as u32).min(image_height);
    let source_width = source_x2.saturating_sub(source_x);
    let source_height = source_y2.saturating_sub(source_y);
    if source_width == 0 || source_height == 0 {
        return Ok(output);
    }

    let output_scale = output_size as f32 / side;
    let destination_x = (((source_x as f32 - left) * output_scale).floor() as i64)
        .clamp(0, i64::from(output_size)) as u32;
    let destination_y = (((source_y as f32 - top) * output_scale).floor() as i64)
        .clamp(0, i64::from(output_size)) as u32;
    let destination_x2 = (((source_x2 as f32 - left) * output_scale).ceil() as i64)
        .clamp(0, i64::from(output_size)) as u32;
    let destination_y2 = (((source_y2 as f32 - top) * output_scale).ceil() as i64)
        .clamp(0, i64::from(output_size)) as u32;
    let destination_width = destination_x2.saturating_sub(destination_x);
    let destination_height = destination_y2.saturating_sub(destination_y);
    if destination_width == 0 || destination_height == 0 {
        return Ok(output);
    }

    let source = crop_imm(image, source_x, source_y, source_width, source_height);
    let resized = resize(
        &*source,
        destination_width,
        destination_height,
        FilterType::Triangle,
    );
    overlay(
        &mut output,
        &resized,
        i64::from(destination_x),
        i64::from(destination_y),
    );
    Ok(output)
}

#[cfg(feature = "execute")]
struct PreparedFace {
    detection: face_id::detector::DetectedFace,
    embedding_crop: flow_like_types::image::RgbImage,
    attribute_crop: flow_like_types::image::RgbImage,
}

#[cfg(feature = "execute")]
struct PreparedFaces {
    width: u32,
    height: u32,
    faces: Vec<PreparedFace>,
}

#[cfg(feature = "execute")]
fn prepare_faces_bounded(
    analyzer: &face_id::analyzer::FaceAnalyzer,
    image: &flow_like_types::image::DynamicImage,
    max_faces: usize,
    score_threshold: f32,
    iou_threshold: f32,
) -> Result<PreparedFaces> {
    use flow_like_types::image::GenericImageView;

    let (width, height) = image.dimensions();
    validate_source_image_dimensions(width, height)?;
    let mut detector = analyzer
        .detector
        .lock()
        .map_err(|_| anyhow!("Face detector mutex was poisoned"))?;
    validate_detector_projection(width, height, detector.config.input_size)?;
    detector.config.score_threshold = score_threshold;
    detector.config.iou_threshold = iou_threshold;
    let mut detections = detector
        .detect(image)
        .map_err(|e| anyhow!("Face detection failed: {e}"))?;
    drop(detector);
    detections.truncate(max_faces);
    if detections.is_empty() {
        return Ok(PreparedFaces {
            width,
            height,
            faces: Vec::new(),
        });
    }

    let converted_rgb;
    let rgb_image = match image.as_rgb8() {
        Some(rgb_image) => rgb_image,
        None => {
            converted_rgb = image.to_rgb8();
            &converted_rgb
        }
    };

    let mut faces = Vec::with_capacity(detections.len());
    for detection in detections {
        let bbox = &detection.bbox;
        let coordinates = [bbox.x1, bbox.y1, bbox.x2, bbox.y2];
        if !coordinates.into_iter().all(f32::is_finite)
            || coordinates
                .into_iter()
                .any(|coordinate| !(-1.0..=2.0).contains(&coordinate))
            || bbox.width() <= 0.0
            || bbox.height() <= 0.0
            || !detection.score.is_finite()
            || !(0.0..=1.0).contains(&detection.score)
        {
            return Err(anyhow!("Face detector returned an invalid bounding box"));
        }
        let landmarks = detection
            .landmarks
            .as_ref()
            .ok_or_else(|| anyhow!("Face detector did not return landmarks"))?;
        if landmarks.len() != 5 {
            return Err(anyhow!(
                "Face detector returned {} landmarks; expected 5",
                landmarks.len()
            ));
        }
        if landmarks.iter().any(|(x, y)| {
            !x.is_finite()
                || !y.is_finite()
                || !(-1.0..=2.0).contains(x)
                || !(-1.0..=2.0).contains(y)
        }) {
            return Err(anyhow!("Face detector returned invalid landmarks"));
        }
        let landmarks: [(f32, f32); 5] = landmarks
            .iter()
            .map(|&(x, y)| (x * width as f32, y * height as f32))
            .collect::<Vec<_>>()
            .try_into()
            .map_err(|_| anyhow!("Face landmarks were not 5-point keypoints"))?;
        faces.push(PreparedFace {
            embedding_crop: face_id::face_align::norm_crop(rgb_image, &landmarks, 112),
            attribute_crop: attribute_crop_bounded(rgb_image, &detection.bbox, 96)?,
            detection,
        });
    }

    Ok(PreparedFaces {
        width,
        height,
        faces,
    })
}

#[cfg(feature = "execute")]
fn analyze_prepared_faces(
    analyzer: &face_id::analyzer::FaceAnalyzer,
    prepared: PreparedFaces,
) -> Result<Vec<FaceIdResult>> {
    let PreparedFaces {
        width,
        height,
        faces: prepared_faces,
    } = prepared;
    let mut faces = Vec::with_capacity(prepared_faces.len());
    let mut prepared_faces = prepared_faces.into_iter();
    loop {
        let batch: Vec<_> = prepared_faces.by_ref().take(FACE_BATCH_SIZE).collect();
        if batch.is_empty() {
            break;
        }

        let expected = batch.len();
        let mut detections = Vec::with_capacity(expected);
        let mut embedding_crops = Vec::with_capacity(expected);
        let mut attribute_crops = Vec::with_capacity(expected);
        for prepared in batch {
            detections.push(prepared.detection);
            embedding_crops.push(prepared.embedding_crop);
            attribute_crops.push(prepared.attribute_crop);
        }

        let embeddings = analyzer
            .embedder
            .lock()
            .map_err(|_| anyhow!("Face embedder mutex was poisoned"))?
            .compute_embeddings_batch(&embedding_crops)
            .map_err(|e| anyhow!("Face embedding failed: {e}"))?;
        if embeddings.len() != expected {
            return Err(anyhow!(
                "Face embedder batch mismatch: expected {expected}, got {}",
                embeddings.len()
            ));
        }
        for embedding in &embeddings {
            if embedding.len() != FACE_EMBEDDING_DIMENSION {
                return Err(anyhow!(
                    "Face embedder output dimension mismatch: expected {FACE_EMBEDDING_DIMENSION}, got {}",
                    embedding.len()
                ));
            }
        }

        let attributes = analyzer
            .gender_age
            .lock()
            .map_err(|_| anyhow!("Gender/age estimator mutex was poisoned"))?
            .estimate_batch(&attribute_crops)
            .map_err(|e| anyhow!("Gender/age estimation failed: {e}"))?;
        if attributes.len() != expected {
            return Err(anyhow!(
                "Gender/age batch mismatch: expected {expected}, got {}",
                attributes.len()
            ));
        }

        for ((detection, embedding), attributes) in
            detections.into_iter().zip(embeddings).zip(attributes)
        {
            faces.push(face_result_from_parts(
                detection,
                embedding,
                attributes.gender,
                attributes.age,
                width,
                height,
            )?);
        }
    }

    Ok(faces)
}

#[crate::register_node]
#[derive(Default)]
pub struct AnalyzeFacesNode {}

impl AnalyzeFacesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AnalyzeFacesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "face_id_analyze",
            "Analyze Faces",
            "Detect faces and extract embeddings, gender and age using a face_id analyzer",
            "AI/ML/ONNX/Face",
        );
        node.set_flowscript_name("onnx", "faceIdAnalyze");
        node.set_version(2);
        node.add_icon("/flow/icons/face.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "analyzer",
            "Analyzer",
            "Face analyzer handle",
            VariableType::Struct,
        )
        .set_schema::<NodeFaceAnalyzer>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("image", "Image", "Input Image", VariableType::Struct)
            .set_schema::<NodeImage>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "max_faces",
            "Max Faces",
            "Maximum number of faces to embed and analyze",
            VariableType::Integer,
        )
        .set_options(
            PinOptions::new()
                .set_range((1.0, MAX_FACES as f64))
                .set_step(1.0)
                .build(),
        )
        .set_default_value(Some(json!(DEFAULT_MAX_FACES)));

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("faces", "Faces", "Analyzed faces", VariableType::Struct)
            .set_schema::<FaceIdResult>()
            .set_value_type(ValueType::Array)
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of detected faces",
            VariableType::Integer,
        );

        node
    }

    #[allow(unused_variables)]
    async fn run(&self, context: &mut ExecutionContext) -> Result<()> {
        #[cfg(feature = "execute")]
        {
            context.deactivate_exec_pin("exec_out").await?;

            let analyzer_ref: NodeFaceAnalyzer = context.evaluate_pin("analyzer").await?;
            let image: NodeImage = context.evaluate_pin("image").await?;
            let max_faces: i64 = context.evaluate_pin("max_faces").await?;
            let max_faces = validate_max_faces(max_faces)?;
            let thresholds = validate_analyzer_config(
                MIN_DETECTOR_INPUT_SIZE,
                f64::from(analyzer_ref.score_threshold),
                f64::from(analyzer_ref.iou_threshold),
            )?;

            let img_wrapper = image.get_image(context).await?;
            let image_guard = img_wrapper.lock_owned().await;
            let (analyzer, analysis_permit) = analyzer_ref.get_analyzer(context).await?;
            let analyzer_for_preparation = analyzer.clone();
            let (prepared, analysis_permit) =
                flow_like_types::tokio::task::spawn_blocking(move || {
                    let prepared = prepare_faces_bounded(
                        &analyzer_for_preparation,
                        &image_guard,
                        max_faces,
                        thresholds.score_threshold,
                        thresholds.iou_threshold,
                    )?;
                    Ok::<_, flow_like_types::Error>((prepared, analysis_permit))
                })
                .await
                .map_err(|e| anyhow!("Face preparation task panicked: {e}"))??;
            let faces = flow_like_types::tokio::task::spawn_blocking(move || {
                let _analysis_permit = analysis_permit;
                analyze_prepared_faces(&analyzer, prepared)
            })
            .await
            .map_err(|e| anyhow!("Face analysis task panicked: {e}"))??;

            let count = faces.len() as i64;
            context.set_pin_value("faces", json!(faces)).await?;
            context.set_pin_value("count", json!(count)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!(
            "Face analysis requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyzer_config_accepts_safe_values() {
        for (input_size, score, iou) in [
            (MIN_DETECTOR_INPUT_SIZE, MIN_SCORE_THRESHOLD, 0.0),
            (MAX_DETECTOR_INPUT_SIZE, 1.0, MAX_IOU_THRESHOLD),
            (640, 0.5, 0.4),
        ] {
            let config = validate_analyzer_config(input_size, score, iou).unwrap();
            assert_eq!(config.input_size, input_size as u32);
            assert_eq!(config.score_threshold, score as f32);
            assert_eq!(config.iou_threshold, iou as f32);
        }
    }

    #[test]
    fn analyzer_config_rejects_unsafe_values() {
        for input_size in [-1, 0, 33, MAX_DETECTOR_INPUT_SIZE + 32, 4_294_967_296] {
            assert!(validate_analyzer_config(input_size, 0.5, 0.4).is_err());
        }
        for score in [
            f64::NAN,
            f64::INFINITY,
            -1.0,
            MIN_SCORE_THRESHOLD - 0.01,
            1.01,
        ] {
            assert!(validate_analyzer_config(640, score, 0.4).is_err());
        }
        for iou in [
            f64::NAN,
            f64::INFINITY,
            -0.01,
            MAX_IOU_THRESHOLD + 0.01,
            2.0,
        ] {
            assert!(validate_analyzer_config(640, 0.5, iou).is_err());
        }
    }

    #[test]
    fn legacy_default_urls_are_migrated_without_overwriting_custom_urls() {
        let mut node = LoadFaceAnalyzerNode::new().get_node();
        node.get_pin_mut_by_name("detector_url")
            .unwrap()
            .default_value = flow_like_types::json::to_vec(LEGACY_DETECTOR_URL).ok();
        node.get_pin_mut_by_name("embedder_url")
            .unwrap()
            .default_value = flow_like_types::json::to_vec("https://example.com/custom.onnx").ok();

        migrate_legacy_url_default(
            &mut node,
            "detector_url",
            LEGACY_DETECTOR_URL,
            DEFAULT_DETECTOR_URL,
        );
        migrate_legacy_url_default(
            &mut node,
            "embedder_url",
            LEGACY_EMBEDDER_URL,
            DEFAULT_EMBEDDER_URL,
        );

        let detector: String = flow_like_types::json::from_slice(
            node.get_pin_by_name("detector_url")
                .unwrap()
                .default_value
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        let embedder: String = flow_like_types::json::from_slice(
            node.get_pin_by_name("embedder_url")
                .unwrap()
                .default_value
                .as_deref()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(detector, DEFAULT_DETECTOR_URL);
        assert_eq!(embedder, "https://example.com/custom.onnx");
    }

    #[test]
    fn maximum_faces_is_bounded() {
        assert_eq!(validate_max_faces(1).unwrap(), 1);
        assert_eq!(validate_max_faces(MAX_FACES).unwrap(), MAX_FACES as usize);
        assert!(validate_max_faces(0).is_err());
        assert!(validate_max_faces(MAX_FACES + 1).is_err());
    }

    #[test]
    fn source_image_dimensions_are_bounded() {
        assert!(validate_source_image_dimensions(1, 1).is_ok());
        assert!(validate_source_image_dimensions(0, 1).is_err());
        assert!(validate_source_image_dimensions(1, 0).is_err());
        assert!(validate_source_image_dimensions((MAX_SOURCE_IMAGE_PIXELS + 1) as u32, 1).is_err());
    }

    #[test]
    fn detector_projection_rejects_zero_sized_resizes() {
        assert!(validate_detector_projection(1920, 1080, (640, 640)).is_ok());
        assert!(validate_detector_projection(24_000_000, 1, (640, 640)).is_err());
        assert!(validate_detector_projection(1, 24_000_000, (640, 640)).is_err());
    }

    #[test]
    fn model_cache_requires_a_scoped_directory() {
        assert!(
            validate_model_cache_dir(&FlowPath::new(
                "face-models".to_string(),
                "store".to_string(),
                None,
            ))
            .is_ok()
        );
        for path in ["", "/", "///", " / "] {
            assert!(
                validate_model_cache_dir(&FlowPath::new(
                    path.to_string(),
                    "store".to_string(),
                    None,
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn legacy_analyzer_handles_receive_threshold_defaults() {
        let analyzer: NodeFaceAnalyzer = flow_like_types::json::from_value(json!({
            "analyzer_ref": "legacy"
        }))
        .unwrap();
        assert_eq!(analyzer.score_threshold, default_score_threshold());
        assert_eq!(analyzer.iou_threshold, default_iou_threshold());
    }

    #[cfg(feature = "execute")]
    fn fake_sha(byte: char) -> String {
        std::iter::repeat_n(byte, 64).collect()
    }

    #[cfg(feature = "execute")]
    #[test]
    fn analyzer_cells_are_shared_process_wide_by_session_identity() {
        let first = shared_analyzer_cell("face-analyzer:test-shared-cell").unwrap();
        let second = shared_analyzer_cell("face-analyzer:test-shared-cell").unwrap();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[cfg(feature = "execute")]
    #[test]
    fn model_cache_names_use_role_and_verified_content_identity() {
        let sha_a = fake_sha('a');
        let sha_b = fake_sha('b');
        let detector = ModelSpec::new(
            ModelRole::Detector,
            "https://one.example/models/model.onnx",
            &sha_a,
        )
        .unwrap();
        let same_basename = ModelSpec::new(
            ModelRole::Detector,
            "https://two.example/models/model.onnx",
            &sha_a,
        )
        .unwrap();
        let other_role = ModelSpec::new(
            ModelRole::Embedder,
            "https://one.example/models/model.onnx",
            &sha_a,
        )
        .unwrap();
        let other_checksum = ModelSpec::new(
            ModelRole::Detector,
            "https://one.example/models/model.onnx",
            &sha_b,
        )
        .unwrap();

        assert_eq!(detector.cache_file_name(), same_basename.cache_file_name());
        assert_ne!(detector.cache_file_name(), other_role.cache_file_name());
        assert_ne!(detector.cache_file_name(), other_checksum.cache_file_name());
        assert!(detector.cache_file_name().ends_with(".onnx"));
        assert!(!detector.cache_file_name().contains("example"));
    }

    #[cfg(feature = "execute")]
    #[test]
    fn model_cache_locks_normalize_equivalent_object_paths() {
        let canonical = FlowPath::new("models/face.onnx".to_string(), "store".to_string(), None);
        let aliased = FlowPath::new("/models//face.onnx/".to_string(), "store".to_string(), None);

        assert!(Arc::ptr_eq(
            &model_materialization_lock(&canonical).unwrap(),
            &model_materialization_lock(&aliased).unwrap(),
        ));
        assert!(Arc::ptr_eq(
            &model_cache_write_lock(&canonical).unwrap(),
            &model_cache_write_lock(&aliased).unwrap(),
        ));
    }

    #[cfg(feature = "execute")]
    #[test]
    fn combined_model_set_must_fit_the_target_cache_quota() {
        assert_eq!(
            validate_model_set_size([MAX_MODEL_CACHE_BYTES, 0, 0]).unwrap(),
            MAX_MODEL_CACHE_BYTES
        );
        assert!(validate_model_set_size([MAX_MODEL_CACHE_BYTES, 1, 0]).is_err());
        assert!(validate_model_set_size([u64::MAX, 1, 0]).is_err());
    }

    #[cfg(feature = "execute")]
    #[test]
    fn cache_gc_only_recognizes_generated_files_in_the_exact_directory() {
        let hash = fake_sha('a');
        let managed = flow_like_storage::Path::from(format!("models/face-id-detector-{hash}.onnx"));
        let nested =
            flow_like_storage::Path::from(format!("models/nested/face-id-detector-{hash}.onnx"));
        let user_file = flow_like_storage::Path::from("models/face-id-detector-user.onnx");

        assert!(is_managed_face_model_path(&managed, "models"));
        assert!(!is_managed_face_model_path(&nested, "models"));
        assert!(!is_managed_face_model_path(&user_file, "models"));
    }

    #[cfg(feature = "execute")]
    #[test]
    fn cache_etag_path_only_removes_the_final_extension() {
        let path = flow_like_storage::Path::from("models.onnx/face-id-detector-a.onnx");
        assert_eq!(
            model_cache_etag_path(&path),
            flow_like_storage::Path::from("models.onnx/face-id-detector-a.s3flowEtag")
        );
    }

    #[cfg(feature = "execute")]
    #[test]
    fn model_specs_require_http_and_valid_sha256() {
        assert!(
            ModelSpec::new(
                ModelRole::Detector,
                "file:///tmp/model.onnx",
                &fake_sha('a')
            )
            .is_err()
        );
        assert!(
            ModelSpec::new(ModelRole::Detector, "https://example.com/model.onnx", "abc").is_err()
        );

        let spec = ModelSpec::new(
            ModelRole::Detector,
            "https://example.com/model.onnx#ignored",
            &fake_sha('A'),
        )
        .unwrap();
        assert_eq!(spec.url.as_str(), "https://example.com/model.onnx");
        assert_eq!(spec.expected_sha256, fake_sha('a'));
    }

    #[cfg(feature = "execute")]
    #[test]
    fn analyzer_cache_key_changes_with_inputs() {
        let specs = [
            ModelSpec::new(
                ModelRole::Detector,
                "https://example.com/d.onnx",
                &fake_sha('a'),
            )
            .unwrap(),
            ModelSpec::new(
                ModelRole::Embedder,
                "https://example.com/e.onnx",
                &fake_sha('b'),
            )
            .unwrap(),
            ModelSpec::new(
                ModelRole::GenderAge,
                "https://example.com/g.onnx",
                &fake_sha('c'),
            )
            .unwrap(),
        ];
        let config = validate_analyzer_config(640, 0.5, 0.4).unwrap();
        let providers = vec!["CoreML".to_string(), "CPU".to_string()];
        let key = analyzer_cache_key(&specs, config, &providers);

        assert_eq!(key, analyzer_cache_key(&specs, config, &providers));
        assert_ne!(
            key,
            analyzer_cache_key(
                &specs,
                validate_analyzer_config(320, 0.5, 0.4).unwrap(),
                &providers,
            )
        );
        assert_ne!(
            key,
            analyzer_cache_key(&specs, config, &["CPU".to_string()])
        );
        assert_eq!(
            key,
            analyzer_cache_key(
                &specs,
                validate_analyzer_config(640, 0.6, 0.4).unwrap(),
                &providers,
            )
        );
        assert_eq!(
            key,
            analyzer_cache_key(
                &specs,
                validate_analyzer_config(640, 0.5, 0.5).unwrap(),
                &providers,
            )
        );

        let mut same_content_at_new_url = specs.clone();
        same_content_at_new_url[0] = ModelSpec::new(
            ModelRole::Detector,
            "https://rotated.example/d.onnx?signature=new",
            &fake_sha('a'),
        )
        .unwrap();
        assert_eq!(
            key,
            analyzer_cache_key(&same_content_at_new_url, config, &providers)
        );

        let mut changed_content = specs.clone();
        changed_content[0] = ModelSpec::new(
            ModelRole::Detector,
            "https://example.com/d.onnx",
            &fake_sha('d'),
        )
        .unwrap();
        assert_ne!(
            key,
            analyzer_cache_key(&changed_content, config, &providers)
        );
    }

    #[cfg(feature = "execute")]
    #[test]
    fn attribute_crop_is_fixed_size_and_pads_out_of_frame_regions() {
        use flow_like_types::image::{Rgb, RgbImage};

        let image = RgbImage::from_pixel(64, 32, Rgb([255, 255, 255]));
        let full_frame = face_id::detector::BoundingBox {
            x1: 0.0,
            y1: 0.0,
            x2: 1.0,
            y2: 1.0,
        };
        let out_of_frame = face_id::detector::BoundingBox {
            x1: -1.0,
            y1: -1.0,
            x2: 2.0,
            y2: 2.0,
        };

        let full_crop = attribute_crop_bounded(&image, &full_frame, 96).unwrap();
        let padded_crop = attribute_crop_bounded(&image, &out_of_frame, 96).unwrap();
        assert_eq!(full_crop.dimensions(), (96, 96));
        assert_eq!(padded_crop.dimensions(), (96, 96));
        assert_eq!(*padded_crop.get_pixel(0, 0), Rgb([0, 0, 0]));
        assert_eq!(*padded_crop.get_pixel(48, 48), Rgb([255, 255, 255]));
    }

    #[cfg(feature = "execute")]
    #[test]
    fn face_results_scale_coordinates_and_move_embeddings() {
        let detection = face_id::detector::DetectedFace {
            bbox: face_id::detector::BoundingBox {
                x1: 0.1,
                y1: 0.2,
                x2: 0.6,
                y2: 0.8,
            },
            landmarks: Some(vec![(0.1, 0.2); 5]),
            score: 0.9,
        };
        let result = face_result_from_parts(
            detection,
            vec![0.0; FACE_EMBEDDING_DIMENSION],
            face_id::gender_age::Gender::Female,
            42,
            200,
            100,
        )
        .unwrap();

        assert!((result.bbox.x1 - 20.0).abs() < 0.001);
        assert!((result.bbox.y1 - 20.0).abs() < 0.001);
        assert!((result.bbox.x2 - 120.0).abs() < 0.001);
        assert!((result.bbox.y2 - 80.0).abs() < 0.001);
        assert_eq!(result.bbox.class_name.as_deref(), Some("face"));
        assert_eq!(result.landmarks.unwrap(), vec![[20.0, 20.0]; 5]);
        assert_eq!(result.embedding.len(), FACE_EMBEDDING_DIMENSION);
        assert_eq!(result.gender, "Female");
        assert_eq!(result.age, 42);
    }

    #[cfg(feature = "execute")]
    #[test]
    fn face_results_enforce_embedding_contract() {
        let detection = face_id::detector::DetectedFace {
            bbox: face_id::detector::BoundingBox {
                x1: 0.0,
                y1: 0.0,
                x2: 1.0,
                y2: 1.0,
            },
            landmarks: None,
            score: 1.0,
        };
        assert!(
            face_result_from_parts(
                detection,
                vec![0.0; FACE_EMBEDDING_DIMENSION - 1],
                face_id::gender_age::Gender::Male,
                30,
                100,
                100,
            )
            .is_err()
        );
    }
}
