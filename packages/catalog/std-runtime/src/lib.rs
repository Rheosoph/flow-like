//! Runtime nodes in the standard catalog.

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod control;
pub mod logging;
pub mod notifications;
pub mod testing;
pub mod utils;
pub mod variables;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
