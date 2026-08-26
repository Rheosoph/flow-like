//! Standard catalog for Flow-Like
//!
//! This crate contains standard nodes:
//! - Math operations
//! - Control flow
//! - Variables
//! - Structs/data structures
//! - Logging
//! - Utilities
//! - A2UI (Agent-to-UI) operations
//! - Notifications
//! - Faker (mock data generation)

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod a2ui;
pub mod control;
pub mod faker;
pub mod logging;
pub mod math;
pub mod notifications;
pub mod structs;
pub mod testing;
pub mod utils;
pub mod variables;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
