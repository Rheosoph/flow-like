#[cfg(feature = "engine")]
mod backend;
mod contracts;
#[cfg(feature = "engine")]
mod detr_layers;
#[cfg(feature = "engine")]
mod detr_loss;
#[cfg(feature = "engine")]
pub use detr_loss::{DetrClassificationLoss, DetrLossConfig, DetrMatch, detr_loss, detr_match};
mod rfdetr_types;
#[cfg(feature = "engine")]
mod rtdetr;
#[cfg(feature = "engine")]
mod rtdetr_backbone;
mod rtdetr_types;
pub use rfdetr_types::*;
#[cfg(feature = "engine")]
mod rfdetr;
#[cfg(feature = "engine")]
mod rfdetr_backbone;
#[cfg(feature = "engine")]
mod rfdetr_import;
#[cfg(feature = "engine")]
pub use rfdetr::{RfDetr, RfDetrOutput};
#[cfg(feature = "engine")]
pub use rfdetr_import::import_rf_detr_weights;
#[cfg(feature = "engine")]
mod dfine;
#[cfg(feature = "engine")]
mod dfine_backbone;
#[cfg(feature = "engine")]
pub use dfine::DfineNano;
mod dfine_types;
pub use dfine_types::*;
#[cfg(feature = "engine")]
pub use rtdetr::{DetrOutput, DetrPrediction, RtdetrV2};
pub use rtdetr_types::*;
#[cfg(feature = "engine")]
mod dinov2;
mod dinov2_types;
#[cfg(feature = "engine")]
pub use dinov2::{DinoV2, DinoV2Features};
pub use dinov2_types::*;
mod sam2_types;
pub use sam2_types::*;
#[cfg(feature = "engine")]
mod sam2;
#[cfg(feature = "engine")]
mod sam2_decoder;
#[cfg(feature = "engine")]
mod sam2_engine;
#[cfg(feature = "engine")]
mod sam2_hiera;
#[cfg(feature = "engine")]
pub use sam2::{Sam2, Sam2Output};
#[cfg(feature = "engine")]
pub use sam2_engine::{
    SAM2_MANIFEST_FILE, Sam2Predictor, import_sam2_weights, resume_sam2, train_sam2,
};
mod efficient_ad_types;
#[cfg(feature = "engine")]
mod engine;
#[cfg(feature = "engine")]
mod execution;
#[cfg(feature = "engine")]
mod fusion;
#[cfg(feature = "engine")]
mod mobile;
#[cfg(feature = "engine")]
mod model;
#[cfg(feature = "engine")]
mod pretrained;
mod pretrained_types;
#[cfg(feature = "engine")]
mod pytorch;
#[cfg(feature = "engine")]
pub use pretrained::{
    import_dfine_nano_weights, import_dinov2_safetensors, import_rtdetr_v2_weights,
    import_torchvision_resnet18_safetensors,
};
pub use pretrained_types::*;
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
pub use engine::{MANIFEST_FILE, Predictor, resume, train, train_from_pretrained};
pub use types::*;

#[cfg(all(test, feature = "cpu"))]
mod tests;

#[cfg(feature = "onnx-export")]
mod export;
#[cfg(feature = "onnx-export")]
pub use export::OnnxExportReport;
