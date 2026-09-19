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
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use sha2::{Digest, Sha256};
#[cfg(feature = "execute")]
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[cfg(feature = "execute")]
use super::model_cache::{self, FACE_ID_MODELS, ModelSpec, hash_field};

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
fn face_model_spec(role: ModelRole, url: &str, expected_sha256: &str) -> Result<ModelSpec> {
    ModelSpec::new(
        &FACE_ID_MODELS,
        role.as_str(),
        role.max_bytes(),
        url,
        expected_sha256,
    )
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
        hash_field(&mut hasher, spec.role().as_bytes());
        hash_field(&mut hasher, spec.expected_sha256().as_bytes());
    }
    hash_field(&mut hasher, &config.input_size.to_be_bytes());
    for provider in active_providers {
        hash_field(&mut hasher, provider.as_bytes());
    }
    format!("face-analyzer:{}", hex::encode(hasher.finalize()))
}

#[cfg(feature = "execute")]
async fn build_face_analyzer(
    context: &mut ExecutionContext,
    cache_dir: &FlowPath,
    specs: &[ModelSpec; 3],
    config: ValidatedAnalyzerConfig,
) -> Result<Arc<face_id::analyzer::FaceAnalyzer>> {
    let execution_providers = super::execution_providers::session_execution_providers(true)?;
    let analyzer = model_cache::with_verified_models(
        context,
        cache_dir,
        specs,
        "flowlike-faceid-",
        move |paths| {
            let [detector, embedder, gender_age]: [PathBuf; 3] =
                paths.try_into().map_err(|paths: Vec<PathBuf>| {
                    anyhow!("Expected 3 face model paths, got {}", paths.len())
                })?;
            face_id::analyzer::FaceAnalyzer::builder(detector, embedder, gender_age)
                .detector_input_size((config.input_size, config.input_size))
                .detector_score_threshold(config.score_threshold)
                .detector_iou_threshold(config.iou_threshold)
                .with_execution_providers(&execution_providers)
                .build()
                .map_err(|e| anyhow!("Failed to build face analyzer: {e}"))
        },
    )
    .await?;
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
            model_cache::validate_model_cache_dir(&cache_dir, FACE_ID_MODELS.label)?;
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
                face_model_spec(ModelRole::Detector, &detector_url, &detector_sha256)?,
                face_model_spec(ModelRole::Embedder, &embedder_url, &embedder_sha256)?,
                face_model_spec(ModelRole::GenderAge, &gender_age_url, &gender_age_sha256)?,
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
        let detector = face_model_spec(
            ModelRole::Detector,
            "https://one.example/models/model.onnx",
            &sha_a,
        )
        .unwrap();
        let same_basename = face_model_spec(
            ModelRole::Detector,
            "https://two.example/models/model.onnx",
            &sha_a,
        )
        .unwrap();
        let other_role = face_model_spec(
            ModelRole::Embedder,
            "https://one.example/models/model.onnx",
            &sha_a,
        )
        .unwrap();
        let other_checksum = face_model_spec(
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

    // Persisted cache names: changing any of these orphans every existing face model cache.
    #[cfg(feature = "execute")]
    #[test]
    fn default_model_cache_file_names_are_stable() {
        for (role, url, sha256, expected) in [
            (
                ModelRole::Detector,
                DEFAULT_DETECTOR_URL,
                DEFAULT_DETECTOR_SHA256,
                "face-id-detector-d5a05dd4dec91e85676fd1342db9b4e940439ffe9c18a1eadf48e9e1922d8ef3.onnx",
            ),
            (
                ModelRole::Embedder,
                DEFAULT_EMBEDDER_URL,
                DEFAULT_EMBEDDER_SHA256,
                "face-id-embedder-32e48dacf1403af09d06c2a9aa9a13b18f83cdae8b616995b10ae4acef1f26b5.onnx",
            ),
            (
                ModelRole::GenderAge,
                DEFAULT_GENDER_AGE_URL,
                DEFAULT_GENDER_AGE_SHA256,
                "face-id-gender-age-1cff61a44f71bbe5d6cb265c93e850f2facfe0abfc964d3b752ece49ba67f343.onnx",
            ),
        ] {
            let spec = face_model_spec(role, url, sha256).unwrap();
            assert_eq!(spec.cache_file_name(), expected);
            let unnormalized = format!("  {}\n", sha256.to_ascii_uppercase());
            let spec =
                face_model_spec(role, "https://mirror.example/m.onnx", &unnormalized).unwrap();
            assert_eq!(spec.cache_file_name(), expected);

            let cache_dir = FlowPath::new("models/face/".to_string(), "store".to_string(), None);
            assert_eq!(
                spec.cache_path(&cache_dir).object_path().as_ref(),
                format!("models/face/{expected}")
            );
        }
    }

    #[cfg(feature = "execute")]
    #[test]
    fn analyzer_cache_key_changes_with_inputs() {
        let specs = [
            face_model_spec(
                ModelRole::Detector,
                "https://example.com/d.onnx",
                &fake_sha('a'),
            )
            .unwrap(),
            face_model_spec(
                ModelRole::Embedder,
                "https://example.com/e.onnx",
                &fake_sha('b'),
            )
            .unwrap(),
            face_model_spec(
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
        same_content_at_new_url[0] = face_model_spec(
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
        changed_content[0] = face_model_spec(
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
