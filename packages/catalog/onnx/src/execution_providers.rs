//! Compatibility re-exports for FlowLike's shared ONNX Runtime configuration.
//!
//! The implementation lives in `flow-like-model-provider`, the lowest common owner of
//! both raw `ort` sessions and FastEmbed. This prevents either path from fixing ORT's
//! process-wide environment before the execution-provider policy is installed.

pub use flow_like_model_provider::ml::ort_runtime::*;
