/// # ONNX Nodes
/// Loading and Inference for ONNX-based Models
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like_model_provider::ml::ort::session::Session;
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, Result, create_id, sync::Mutex};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::sync::Arc;

/// ONNX Audio Processing Nodes (VAD, Speech)
pub mod audio;
/// ONNX Batch Inference Nodes
pub mod batch;
/// ONNX Image Classification Nodes
pub mod classification;
/// ONNX Depth Estimation Nodes
pub mod depth;
/// ONNX Image Object Detection Nodes
pub mod detection;
/// Execution Provider configuration with graceful fallback
pub mod execution_providers;
/// ONNX Face Detection and Recognition Nodes
pub mod face;
/// face_id-backed face analysis (detection + recognition + gender/age)
pub mod face_id;
/// ONNX Image Feature Extractor Nodes
pub mod feature;
/// ONNX Model Loader Nodes
pub mod load;
/// ONNX Named Entity Recognition (NER) Nodes
pub mod ner;
/// ONNX OCR (Text Detection/Recognition) Nodes
pub mod ocr;
/// ONNX Pose Estimation Nodes
pub mod pose;
/// ONNX Semantic/Instance Segmentation Nodes
pub mod segmentation;
/// ONNX Model Utility Nodes
pub mod utils;

pub use execution_providers::{get_ep_info, initialize_ort, is_initialized};

/// Model provider type for automatic inference routing
pub enum Provider {
    DfineLike(detection::DfineLike),
    YoloLike(detection::YoloLike),
    BoxLabelsScoresLike(detection::BoxLabelsScoresLike),
    SsdMobileNetLike(detection::SsdMobileNetLike),
    YoloV2GridLike(detection::YoloV2GridLike),
    YoloV3Like(detection::YoloV3Like),
    YoloV4Like(detection::YoloV4Like),
    RetinaNetLike(detection::RetinaNetLike),
    TimmLike(classification::TimmLike),
    /// Generic provider for models that don't match known patterns
    Generic,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
/// ONNX Runtime Session Reference
pub struct NodeOnnxSession {
    /// Cache ID for Session
    pub session_ref: String,
}

/// ONNX Runtime Session Bundled with Provider Metadata
#[cfg(feature = "execute")]
pub struct SessionWithMeta {
    pub session: Session,
    pub provider: Provider,
    /// The execution providers that are actually active
    pub ep_active: Vec<String>,
    /// Whether GPU/NPU acceleration is active
    pub accelerated: bool,
}

/// ONNX Runtime Session Wrapper
#[cfg(feature = "execute")]
pub struct NodeOnnxSessionWrapper {
    /// Shared Mutable ONNX Runtime Session
    /// Todo: we might not need a Mutex?
    pub session: Arc<Mutex<SessionWithMeta>>,
}

#[cfg(feature = "execute")]
impl Cacheable for NodeOnnxSessionWrapper {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl NodeOnnxSession {
    /// Push new ONNX Runtime Session to Execution Context
    #[cfg(feature = "execute")]
    pub async fn new(ctx: &mut ExecutionContext, session: SessionWithMeta) -> Self {
        let id = create_id();
        let session_ref = Arc::new(Mutex::new(session));
        let wrapper = NodeOnnxSessionWrapper {
            session: session_ref.clone(),
        };
        ctx.cache
            .write()
            .await
            .insert(id.clone(), Arc::new(wrapper));
        NodeOnnxSession { session_ref: id }
    }

    /// Fetch ONNX Runtime Session from Cached Runtime Context
    #[cfg(feature = "execute")]
    pub async fn get_session(
        &self,
        ctx: &mut ExecutionContext,
    ) -> Result<Arc<Mutex<SessionWithMeta>>> {
        let session = ctx
            .cache
            .read()
            .await
            .get(&self.session_ref)
            .cloned()
            .ok_or_else(|| flow_like_types::anyhow!("ONNX session not found in cache!"))?;
        let session_wrapper = session
            .as_any()
            .downcast_ref::<NodeOnnxSessionWrapper>()
            .ok_or_else(|| {
                flow_like_types::anyhow!("Could not downcast to NodeOnnxSessionWrapper")
            })?;
        let session = session_wrapper.session.clone();
        Ok(session)
    }
}
