//! ONNX/TFLite inference catalog for Flow-Like
//!
//! This crate contains ONNX and TFLite inference nodes for:
//! - Object detection
//! - Image classification
//! - Feature extraction
//! - Teachable Machine models
//! - Depth estimation
//! - Face detection and recognition
//! - OCR (text detection and recognition)
//! - Audio processing (VAD)
//! - Batch inference
//! - Named Entity Recognition (NER)
//! - Zero-shot NER (GLiNER)

extern crate flow_like_runtime as flow_like;

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

#[path = "onnx.rs"]
pub mod onnx;
pub mod teachable_machine;

pub use onnx::*;

// Re-export submodules for external access
pub use onnx::{
    audio, batch, classification, depth, detection, face, face_id, feature, gliner, load, ner, ocr,
    pose, segmentation,
};

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
