//! Automation catalog for Flow-Like
//!
//! This crate contains automation nodes for:
//! - Browser automation (CDP-based)
//! - Desktop/computer automation (mouse, keyboard, screenshots)
//! - Selectors and element fingerprinting
//! - Vision/template matching
//! - LLM-assisted self-healing
//! - RPA reliability primitives

extern crate flow_like_runtime as flow_like;

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod capabilities;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod types;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod agent;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod browser;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod computer;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod fingerprint;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod llm;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod rpa;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod selector;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod session;
#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod vision;

#[cfg(not(any(target_os = "ios", target_os = "android")))]
include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        collect_nodes()
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        Vec::new()
    }
}

#[cfg(all(test, not(any(target_os = "ios", target_os = "android"))))]
mod catalog_tests {
    use super::*;

    #[test]
    fn every_node_requires_local_execution() {
        let runnable_on_servers: Vec<String> = get_catalog()
            .iter()
            .map(|logic| logic.get_node())
            .filter(|node| !node.only_offline)
            .map(|node| node.name)
            .collect();
        assert!(
            runnable_on_servers.is_empty(),
            "servers list this package as metadata only, so these nodes must set only_offline: {runnable_on_servers:?}"
        );
    }
}
