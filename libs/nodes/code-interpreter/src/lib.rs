//! Code-interpreter catalog for Flow-Like
//!
//! Provides a secure Python code execution node backed by a WASM/WASI Python
//! runtime (Pyodide or any WASI-compatible CPython build).
//!
//! # Feature flags
//! * `execute` — compile in the Wasmtime execution engine. Required for
//!   actually running nodes; without it nodes can still be introspected and
//!   displayed in the UI.

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod imported;
pub mod pyodide;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
