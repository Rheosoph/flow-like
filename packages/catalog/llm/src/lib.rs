//! LLM/Generative AI catalog for Flow-Like
//!
//! This crate contains LLM invocation, embedding, agents, and MCP tools.

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

#[path = "generative.rs"]
pub mod generative;

pub use generative::*;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
