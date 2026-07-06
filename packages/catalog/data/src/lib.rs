//! Data integration catalog for Flow-Like
//!
//! This crate contains data integration nodes:
//! - Google services
//! - Microsoft services
//! - GitHub integration
//! - Excel/CSV processing
//! - Database connections
//! - Path/file operations
//! - Events

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod data;
pub mod events;
pub mod interaction;
pub(crate) mod remote_util;

pub use data::*;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
