//! Document processing catalog for Flow-Like
//!
//! This crate contains document processing utilities:
//! - Markitdown conversion
//! - Keyword extraction (RAKE, YAKE, AI-based)

extern crate flow_like_runtime as flow_like;

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

#[path = "processing.rs"]
pub mod processing;

pub use processing::*;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
