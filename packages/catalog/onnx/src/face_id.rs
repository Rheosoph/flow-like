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
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::{BoundingBox, FlowPath, NodeImage};
#[cfg(feature = "execute")]
use flow_like_types::create_id;
use flow_like_types::{Result, anyhow, async_trait, json::json};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::sync::Arc;

/// Default HuggingFace weights (model repos) used by `face_id` when none are supplied.
pub const DEFAULT_DETECTOR_URL: &str =
    "https://huggingface.co/RuteNL/SCRFD-face-detection-ONNX/resolve/main/34g_gnkps.onnx";
pub const DEFAULT_EMBEDDER_URL: &str =
    "https://huggingface.co/public-data/insightface/resolve/main/models/buffalo_l/w600k_r50.onnx";
pub const DEFAULT_GENDER_AGE_URL: &str =
    "https://huggingface.co/public-data/insightface/resolve/main/models/buffalo_l/genderage.onnx";

/// Handle to a cached `FaceAnalyzer` living in the execution context cache.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct NodeFaceAnalyzer {
    /// Cache ID for the analyzer
    pub analyzer_ref: String,
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
pub struct NodeFaceAnalyzerWrapper {
    pub analyzer: Arc<face_id::analyzer::FaceAnalyzer>,
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
    pub async fn new(
        ctx: &mut ExecutionContext,
        analyzer: face_id::analyzer::FaceAnalyzer,
    ) -> Self {
        let id = create_id();
        let wrapper = NodeFaceAnalyzerWrapper {
            analyzer: Arc::new(analyzer),
        };
        ctx.cache
            .write()
            .await
            .insert(id.clone(), Arc::new(wrapper));
        NodeFaceAnalyzer { analyzer_ref: id }
    }

    #[cfg(feature = "execute")]
    pub async fn get_analyzer(
        &self,
        ctx: &mut ExecutionContext,
    ) -> Result<Arc<face_id::analyzer::FaceAnalyzer>> {
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
        Ok(wrapper.analyzer.clone())
    }
}

/// Read a model from the cache dir, downloading and persisting it there on first use.
#[cfg(feature = "execute")]
async fn ensure_model_bytes(
    context: &mut ExecutionContext,
    cache_dir: &FlowPath,
    url: &str,
) -> Result<Vec<u8>> {
    let file_name = url
        .rsplit('/')
        .find(|segment| !segment.is_empty())
        .ok_or_else(|| anyhow!("Could not derive a file name from model URL: {url}"))?;

    let mut model_path = cache_dir.clone();
    model_path.path = format!("{}/{}", cache_dir.path.trim_end_matches('/'), file_name);

    if let Ok(bytes) = model_path.get(context, false).await
        && !bytes.is_empty()
    {
        return Ok(bytes);
    }

    context.log_message(
        &format!("Downloading face model weights from {url}"),
        flow_like::flow::execution::LogLevel::Info,
    );

    let response = reqwest::get(url)
        .await
        .map_err(|e| anyhow!("Failed to download {url}: {e}"))?
        .error_for_status()
        .map_err(|e| anyhow!("Failed to download {url}: {e}"))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| anyhow!("Failed to read model body from {url}: {e}"))?
        .to_vec();

    if let Err(e) = model_path.put(context, bytes.clone(), false).await {
        context.log_message(
            &format!("Failed to persist model {file_name} to cache dir: {e}"),
            flow_like::flow::execution::LogLevel::Warn,
        );
    }

    Ok(bytes)
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
            "Build a face_id analyzer (SCRFD detector + ArcFace embedder + gender/age). Weights are cached in the given directory: downloaded on first use, reused from storage afterwards.",
            "AI/ML/ONNX/Face",
        );
        node.set_version(1);
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
            "Directory (FlowPath) used to cache the downloaded ONNX weights",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "detector_url",
            "Detector URL",
            "SCRFD detector weights URL",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_DETECTOR_URL)));

        node.add_input_pin(
            "embedder_url",
            "Embedder URL",
            "ArcFace recognition weights URL",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_EMBEDDER_URL)));

        node.add_input_pin(
            "gender_age_url",
            "Gender/Age URL",
            "Gender & age estimation weights URL",
            VariableType::String,
        )
        .set_default_value(Some(json!(DEFAULT_GENDER_AGE_URL)));

        node.add_input_pin(
            "input_size",
            "Detector Input Size",
            "Square detector input size",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(640)));

        node.add_input_pin(
            "score_threshold",
            "Score Threshold",
            "Detector confidence threshold",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.5)));

        node.add_input_pin(
            "iou_threshold",
            "IoU Threshold",
            "Detector non-maximum-suppression IoU threshold",
            VariableType::Float,
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
            let detector_url: String = context
                .evaluate_pin("detector_url")
                .await
                .unwrap_or_else(|_| DEFAULT_DETECTOR_URL.to_string());
            let embedder_url: String = context
                .evaluate_pin("embedder_url")
                .await
                .unwrap_or_else(|_| DEFAULT_EMBEDDER_URL.to_string());
            let gender_age_url: String = context
                .evaluate_pin("gender_age_url")
                .await
                .unwrap_or_else(|_| DEFAULT_GENDER_AGE_URL.to_string());
            let input_size: i64 = context.evaluate_pin("input_size").await.unwrap_or(640);
            let score_threshold: f64 = context.evaluate_pin("score_threshold").await.unwrap_or(0.5);
            let iou_threshold: f64 = context.evaluate_pin("iou_threshold").await.unwrap_or(0.4);
            let input_size = input_size.max(1) as u32;

            let det_bytes = ensure_model_bytes(context, &cache_dir, &detector_url).await?;
            let rec_bytes = ensure_model_bytes(context, &cache_dir, &embedder_url).await?;
            let attr_bytes = ensure_model_bytes(context, &cache_dir, &gender_age_url).await?;

            let tmp_dir = std::env::temp_dir().join(format!("flowlike-faceid-{}", create_id()));
            flow_like_types::tokio::fs::create_dir_all(&tmp_dir)
                .await
                .map_err(|e| anyhow!("Failed to create temp dir for face models: {e}"))?;
            let det_path = tmp_dir.join("detector.onnx");
            let rec_path = tmp_dir.join("embedder.onnx");
            let attr_path = tmp_dir.join("genderage.onnx");
            flow_like_types::tokio::fs::write(&det_path, &det_bytes).await?;
            flow_like_types::tokio::fs::write(&rec_path, &rec_bytes).await?;
            flow_like_types::tokio::fs::write(&attr_path, &attr_bytes).await?;

            let build_score = score_threshold as f32;
            let build_iou = iou_threshold as f32;
            let build_paths = (det_path, rec_path, attr_path);
            let analyzer = flow_like_types::tokio::task::spawn_blocking(move || {
                let (det, rec, attr) = build_paths;
                // Share flow-like's globally configured accelerators so face_id's own sessions
                // run on GPU/NPU instead of defaulting to CPU.
                let (eps, _, _) = super::execution_providers::collect_execution_providers();
                face_id::analyzer::FaceAnalyzer::builder(det, rec, attr)
                    .detector_input_size((input_size, input_size))
                    .detector_score_threshold(build_score)
                    .detector_iou_threshold(build_iou)
                    .with_execution_providers(&eps)
                    .build()
            })
            .await
            .map_err(|e| anyhow!("Face analyzer build task panicked: {e}"))?
            .map_err(|e| anyhow!("Failed to build face analyzer: {e}"))?;

            let _ = flow_like_types::tokio::fs::remove_dir_all(&tmp_dir).await;

            let handle = NodeFaceAnalyzer::new(context, analyzer).await;
            context.set_pin_value("analyzer", json!(handle)).await?;
            context.activate_exec_pin("exec_out").await?;
            Ok(())
        }

        #[cfg(not(feature = "execute"))]
        Err(anyhow!(
            "Face analysis requires the 'execute' feature. Rebuild with --features execute"
        ))
    }
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
        node.set_version(1);
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

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin("faces", "Faces", "Analyzed faces", VariableType::Generic);

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
            use flow_like_types::image::GenericImageView;

            context.deactivate_exec_pin("exec_out").await?;

            let analyzer_ref: NodeFaceAnalyzer = context.evaluate_pin("analyzer").await?;
            let image: NodeImage = context.evaluate_pin("image").await?;

            let analyzer = analyzer_ref.get_analyzer(context).await?;

            let img_wrapper = image.get_image(context).await?;
            let img = img_wrapper.lock().await.clone();
            let (width, height) = img.dimensions();

            let analyses =
                flow_like_types::tokio::task::spawn_blocking(move || analyzer.analyze(&img))
                    .await
                    .map_err(|e| anyhow!("Face analysis task panicked: {e}"))?
                    .map_err(|e| anyhow!("Face analysis failed: {e}"))?;

            let w = width as f32;
            let h = height as f32;
            let faces: Vec<FaceIdResult> = analyses
                .iter()
                .map(|face| {
                    let detection = &face.detection;
                    // face_id reports relative (0..1) coords; scale to absolute pixels.
                    let mut bbox = BoundingBox {
                        x1: detection.bbox.x1,
                        y1: detection.bbox.y1,
                        x2: detection.bbox.x2,
                        y2: detection.bbox.y2,
                        score: detection.score,
                        class_name: Some("face".to_string()),
                        ..Default::default()
                    };
                    bbox.scale(w, h);
                    FaceIdResult {
                        bbox,
                        landmarks: detection
                            .landmarks
                            .as_ref()
                            .map(|points| points.iter().map(|(lx, ly)| [lx * w, ly * h]).collect()),
                        embedding: face.embedding.clone(),
                        gender: format!("{:?}", face.gender),
                        age: face.age,
                    }
                })
                .collect();

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
