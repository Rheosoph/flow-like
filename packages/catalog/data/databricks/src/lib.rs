//! Databricks integration nodes for Flow-Like.

extern crate flow_like_runtime as flow_like;

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};
pub mod data;
pub use data::databricks;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
