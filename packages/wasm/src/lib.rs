//! Flow-Like WASM Runtime
//!
//! This crate provides WebAssembly runtime support for custom Flow-Like nodes.
//! It enables users to write nodes in any WASM-compatible language (Rust, Go,
//! TypeScript, Python, C++) while maintaining full access to the Flow-Like
//! execution context.
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

extern crate flow_like_runtime as flow_like;

pub mod abi;
pub mod aot_cache;
pub mod client;
#[cfg(feature = "component-model")]
pub mod component;
pub mod engine;
pub mod error;
pub mod host_functions;
pub mod instance;
pub mod limits;
mod llm_message;
pub mod manifest;
pub mod memory;
pub mod module;
pub mod node;
pub mod package_runtime;
pub mod registry;
pub mod unified;
pub mod wasi;
pub mod widget;
pub mod widget_bundle;
pub mod widget_source;

pub use abi::{WasmAbi, WASM_ABI_VERSION};
pub use aot_cache::AotCache;
pub use client::RegistryClient;
pub use engine::{WasmConfig, WasmEngine};
pub use error::{WasmError, WasmResult};
pub use instance::WasmInstance;
pub use limits::{WasmCapabilities, WasmLimits, WasmSecurityConfig};
pub use manifest::{
    MemoryTier, OAuthScopeRequirement, PackageManifest, PackagePermissions, PackageWidgetEntry,
    TimeoutTier,
};
pub use memory::WasmMemory;
pub use module::WasmModule;
pub use node::{build_node_from_definition, definition_to_package_entry, WasmNodeLogic};
pub use registry::{
    CachedPackage, DownloadRequest, DownloadResponse, PackageSource, PackageStatus, PackageSummary,
    PackageVersion, PublishRequest, PublishResponse, RegistryConfig, RegistryEntry, SearchFilters,
    SearchResults, SortField,
};
pub use unified::{LoadedWasm, UnifiedInstance};
pub use wasi::{isolated_wasi_ctx_builder, IsolatedWasiCtxBuilder};
pub use widget::{
    ContractEvent, ContractInput, ContractInputType, ContractQuery, WidgetContract, WidgetSizing,
    CONTRACT_VERSION, WIDGET_PROTOCOL,
};
pub use widget_bundle::{
    entry_hash, sha256_hex, widget_store_dir, BuilderWidget, BundleSharedEntry, BundleSizeHint,
    BundleWidgetEntry, WidgetBundleBuilder, WidgetBundleManifest, WidgetBundleReader,
    BUNDLE_FORMAT_VERSION, BUNDLE_MANIFEST_PATH, WIDGET_BUNDLE_EXTENSION, WIDGET_BUNDLE_MEDIA_TYPE,
};
