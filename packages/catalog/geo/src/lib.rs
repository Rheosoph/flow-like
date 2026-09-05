//! Geo catalog for Flow-Like
//!
//! This crate contains geolocation-related nodes:
//! - Map image fetching
//! - Geocoding (forward and reverse)
//! - Route planning
//! - H3 geospatial indexing

extern crate flow_like_runtime as flow_like;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};
use std::sync::Arc;

pub mod geo;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
