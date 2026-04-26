//! Media processing catalog for Flow-Like
//!
//! This crate contains media processing nodes:
//! - Image processing and transformation
//! - Bit manipulation

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod audio;
pub mod bit;
pub mod document;
pub mod image;
pub mod video;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
