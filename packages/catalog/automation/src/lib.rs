//! Automation catalog for Flow-Like
//!
//! This crate contains automation nodes for:
//! - Browser automation (CDP-based)
//! - Desktop/computer automation (mouse, keyboard, screenshots)
//! - Selectors and element fingerprinting
//! - Vision/template matching
//! - LLM-assisted self-healing
//! - RPA reliability primitives

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, inventory, register_node};

#[cfg(not(any(target_os = "ios", target_os = "android")))]
pub mod types;

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

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    {
        flow_like_catalog_core::get_catalog()
    }
    #[cfg(any(target_os = "ios", target_os = "android"))]
    {
        Vec::new()
    }
}
