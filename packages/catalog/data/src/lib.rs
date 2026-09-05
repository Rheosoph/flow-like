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

extern crate flow_like_runtime as flow_like;

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod data;
pub mod events;
pub mod interaction;
pub mod remote_util;

pub use data::*;

// The shared generator also emits a convenience collector; the facade uses its entries.
#[allow(dead_code)]
mod own_registry {
    include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));
}

/// Collect data nodes in their original source order across the independent packages.
pub fn collect_nodes() -> Vec<Arc<dyn NodeLogic>> {
    let mut entries = own_registry::collect_node_entries();
    entries.extend(flow_like_catalog_data_github::collect_node_entries());
    entries.extend(flow_like_catalog_data_google::collect_node_entries());
    entries.extend(flow_like_catalog_data_microsoft::collect_node_entries());
    entries.extend(flow_like_catalog_data_atlassian::collect_node_entries());
    entries.extend(flow_like_catalog_data_notion::collect_node_entries());
    entries.extend(flow_like_catalog_data_databricks::collect_node_entries());
    entries.extend(flow_like_catalog_data_linkedin::collect_node_entries());
    entries.sort_by(|left, right| std::path::Path::new(left.0).cmp(std::path::Path::new(right.0)));
    entries.into_iter().map(|(_, node)| node).collect()
}

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
