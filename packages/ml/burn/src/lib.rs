#[cfg(feature = "engine")]
mod backend;
mod contracts;
mod efficient_ad_types;
#[cfg(feature = "engine")]
mod engine;
#[cfg(feature = "engine")]
mod fusion;
#[cfg(feature = "engine")]
mod mobile;
#[cfg(feature = "engine")]
mod model;
#[cfg(ml_backend_rocm)]
mod rocm_preflight;
#[cfg(feature = "engine")]
mod temporal;
mod types;
pub use contracts::validate_inspection_config;
pub use efficient_ad_types::*;
mod mask_rcnn_types;
pub use mask_rcnn_types::*;
#[cfg(feature = "engine")]
mod efficient_ad;
#[cfg(feature = "engine")]
mod mask_rcnn;
#[cfg(feature = "engine")]
pub use efficient_ad::{EfficientAdPredictor, resume_efficient_ad, train_efficient_ad};
#[cfg(feature = "engine")]
mod vision;
#[cfg(feature = "engine")]
mod yolox;

#[cfg(feature = "engine")]
pub use backend::{
    compiled_backends, probe_backend, resolve_backend, resolve_backend_with_fallback,
};
#[cfg(feature = "engine")]
pub use engine::{MANIFEST_FILE, Predictor, resume, train};
pub use types::*;

#[cfg(all(test, feature = "cpu"))]
mod tests;

#[cfg(feature = "onnx-export")]
mod export;
#[cfg(feature = "onnx-export")]
pub use export::OnnxExportReport;
