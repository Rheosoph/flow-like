//! Media processing catalog for Flow-Like
//!
//! This crate contains media processing nodes:
//! - Image processing and transformation
//! - Bit manipulation

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, inventory, register_node};

pub mod bit;
pub mod document;
pub mod image;

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    flow_like_catalog_core::get_catalog()
}
