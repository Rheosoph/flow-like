//! Flow-Like WASM Runtime
//!
//! This crate provides WebAssembly runtime support for custom Flow-Like nodes.
//! It enables users to write nodes in any WASM-compatible language (Rust, Go,
//! TypeScript, Python, C++) while maintaining full access to the Flow-Like
//! execution context.
//! Compilation and metadata APIs are always available. Application host APIs
//! require the `execution` feature.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Flow-Like Runtime                         │
//! ├─────────────────────────────────────────────────────────────┤
//! │  ┌─────────────┐    ┌─────────────┐    ┌─────────────┐     │
//! │  │ Native Node │    │ Native Node │    │  WASM Node  │     │
//! │  │   (Rust)    │    │   (Rust)    │    │  (Any Lang) │     │
//! │  └─────────────┘    └─────────────┘    └──────┬──────┘     │
//! │                                               │             │
//! │                                        ┌──────▼──────┐     │
//! │                                        │ WasmEngine  │     │
//! │                                        │ (Wasmtime)  │     │
//! │                                        └─────────────┘     │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use flow_like_wasm::{WasmEngine, WasmModule, WasmConfig};
//!
//! // Create engine with default config
//! let engine = WasmEngine::new(WasmConfig::default())?;
//!
//! // Load a WASM module
//! let module = engine.load_module_from_file("my_node.wasm").await?;
//!
//! // Get node definition
//! let node_def = module.get_node_definition()?;
//!
//! // Execute the node (called by WasmNodeLogic)
//! let result = module.execute(&context).await?;
//! ```

pub mod compilation;
mod core_wasi;
mod engine_config;
pub mod error;
pub mod wasi;
pub use flow_like_wasm_schema::runtime as artifact;

#[cfg(feature = "execution")]
include!("execution_exports.rs");
