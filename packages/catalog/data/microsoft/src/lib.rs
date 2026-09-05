//! Microsoft integration nodes for Flow-Like.

extern crate flow_like_runtime as flow_like;

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};
pub mod data;
pub use data::microsoft;

#[doc(hidden)]
pub mod events {
    pub mod chat_event {
        pub use flow_like_catalog_data_support::events::chat_event::*;
    }
}

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
