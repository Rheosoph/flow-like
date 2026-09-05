//! Audio nodes for the media catalog.

extern crate flow_like_runtime as flow_like;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};
pub(crate) use flow_like_catalog_media_support::ensure_vertex_credentials_explicit;
use std::sync::Arc;

pub mod audio;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
