//! Flow-Like WASM SDK (Component Model)
//!
//! This SDK provides types, macros, and utilities for building WASM nodes
//! that can be executed by the Flow-Like runtime using the WASM Component Model.
//!
//! # Quick Start
//!
//! Mirrors the native catalog pattern: `#[register_node]` + `impl WasmNode`.
//!
//! ```rust,ignore
//! use flow_like_wasm_sdk::*;
//!
//! #[register_node]
//! #[derive(Default)]
//! pub struct MyNode;
//!
//! impl WasmNode for MyNode {
//!     fn get_node(&self) -> NodeDefinition {
//!         let mut node = NodeDefinition::new("my_node", "My Node", "Description", "Category");
//!         node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
//!         node.add_input_pin("text", "Text", "Input text", VariableType::String)
//!             .set_default_value(json!(""));
//!         node.add_output_pin("exec_out", "Done", "Done", VariableType::Execution);
//!         node.add_output_pin("result", "Result", "Output", VariableType::String);
//!         node
//!     }
//!
//!     fn run(&self, mut ctx: Context) -> ExecutionResult {
//!         let text = ctx.get_string("text").unwrap_or_default();
//!         ctx.set_output("result", text.to_uppercase());
//!         ctx.activate_exec("exec_out");
//!         ctx.success()
//!     }
//! }
//!
//! wasm_main!();
//! ```

// Generate Rust bindings from the WIT file inside a submodule so that
// `pub_export_macro: true` doesn't conflict at the crate root.
// Re-export the key items (Guest, export!, flow_like module) at crate root.
#[cfg(target_arch = "wasm32")]
pub mod _bindings {
    wit_bindgen::generate!({
        world: "flow-like-node",
        path: "wit",
        pub_export_macro: true,
        default_bindings_module: "flow_like_wasm_sdk::_bindings",
    });
}

#[cfg(target_arch = "wasm32")]
pub use _bindings::Guest;
#[cfg(target_arch = "wasm32")]
pub use _bindings::export;
#[cfg(target_arch = "wasm32")]
pub use _bindings::flow_like;

// On native targets (including when compiled as a dependency of test binaries),
// provide stubs in place of WIT bindings.
#[cfg(not(target_arch = "wasm32"))]
#[path = "wit_stub.rs"]
mod wit_stub;

#[cfg(not(target_arch = "wasm32"))]
pub use wit_stub::*;

mod context;
pub mod host;
pub mod interop;
pub mod mock;
#[cfg(feature = "rig")]
pub mod rig_provider;
#[cfg(feature = "rig")]
pub use rig_provider::{
    FlowLikeCompletionModel, FlowPathListTool, FlowPathReadTool, FlowPathToolError,
    FlowPathWriteTool,
};
mod types;

pub use context::*;
pub use interop::{
    AudioData, Bit, CachedEmbeddingModel, ChatContent, ChatMessage, ContentPart, DocumentData,
    FlowPath, FtsSearchQuery, HybridSearchQuery, ImageData, NodeDBConnection, NodeImage,
    ReasoningData, ToolCallData, ToolResultData, VectorSearchQuery, VideoData,
};
pub use mock::*;
pub use schemars;
pub use serde;
pub use serde_json;
pub use serde_json::json;
pub use types::*;

// Re-export proc macro so `use flow_like_wasm_sdk::*` brings in #[register_node]
pub use flow_like_wasm_sdk_macros::register_node;

// Re-export inventory so consumers don't need it as a direct dep
pub use inventory;

/// Trait for defining WASM nodes, mirroring the native catalog's `NodeLogic`.
///
/// Implement this on a `Default` struct and annotate the struct with
/// `#[register_node]`. Then call `wasm_main!()` once to generate the
/// WASM Component Model exports.
///
/// # Example
///
/// ```rust,ignore
/// use flow_like_wasm_sdk::*;
///
/// #[register_node]
/// #[derive(Default)]
/// pub struct MyNode;
///
/// impl WasmNode for MyNode {
///     fn get_node(&self) -> NodeDefinition {
///         let mut node = NodeDefinition::new("my_node", "My Node", "Does something", "Custom");
///         node.add_input_pin("exec", "Exec", "Trigger", VariableType::Execution);
///         node.add_output_pin("exec_out", "Done", "Done", VariableType::Execution);
///         node
///     }
///
///     fn run(&self, mut ctx: Context) -> ExecutionResult {
///         ctx.activate_exec("exec_out");
///         ctx.success()
///     }
/// }
///
/// wasm_main!();
/// ```
pub trait WasmNode: Default + 'static {
    fn get_node(&self) -> NodeDefinition;
    fn run(&self, ctx: Context) -> ExecutionResult;
}

/// Registry entry for a WASM node (used by `inventory` for auto-collection).
pub struct WasmNodeEntry {
    pub get_node: fn() -> NodeDefinition,
    pub run: fn(Context) -> ExecutionResult,
}

impl WasmNodeEntry {
    pub const fn new(
        get_node: fn() -> NodeDefinition,
        run: fn(Context) -> ExecutionResult,
    ) -> Self {
        Self { get_node, run }
    }
}

inventory::collect!(WasmNodeEntry);

/// Generate the WASM Component Model exports (Guest impl).
///
/// Call this once at the top level. It auto-discovers all `#[register_node]`
/// structs via `inventory` and generates `get_node`, `get_nodes`, `run`
/// (with name-based dispatch), and `get_abi_version` exports.
///
/// # Example
///
/// ```rust,ignore
/// #[register_node]
/// #[derive(Default)]
/// pub struct NodeA;
/// // impl WasmNode for NodeA { ... }
///
/// #[register_node]
/// #[derive(Default)]
/// pub struct NodeB;
/// // impl WasmNode for NodeB { ... }
///
/// wasm_main!();
/// ```
#[macro_export]
macro_rules! wasm_main {
    () => {
        struct __WasmComponent;

        #[cfg(target_arch = "wasm32")]
        impl $crate::Guest for __WasmComponent {
            fn get_node() -> String {
                for entry in $crate::inventory::iter::<$crate::WasmNodeEntry> {
                    return $crate::serde_json::to_string(&(entry.get_node)()).unwrap_or_default();
                }
                "{}".to_string()
            }

            fn get_nodes() -> String {
                let nodes: Vec<$crate::NodeDefinition> = $crate::inventory::iter::<$crate::WasmNodeEntry>
                    .into_iter()
                    .map(|e| (e.get_node)())
                    .collect();
                $crate::serde_json::to_string(&nodes).unwrap_or_default()
            }

            fn run(input: String) -> String {
                let ctx = match $crate::Context::from_bytes(input.as_bytes()) {
                    Ok(ctx) => ctx,
                    Err(e) => return $crate::ExecutionResult::error(e).to_wasm(),
                };
                let node_name = ctx.node_name().to_string();

                for entry in $crate::inventory::iter::<$crate::WasmNodeEntry> {
                    let def = (entry.get_node)();
                    if def.name == node_name {
                        return (entry.run)(ctx).to_wasm();
                    }
                }

                $crate::ExecutionResult::error(format!("Unknown node: {}", node_name)).to_wasm()
            }

            fn get_abi_version() -> u32 {
                $crate::ABI_VERSION
            }
        }

        #[cfg(target_arch = "wasm32")]
        $crate::export!(__WasmComponent);
    };
}

// Re-export host functions under namespaces for convenience
pub mod log {
    pub use crate::host::{debug, error, fatal, info, log_json, warn};
}

pub mod stream {
    pub use crate::host::{stream, stream_json, stream_progress, stream_text, stream_text_raw};
}

pub mod var {
    pub use crate::host::{delete_variable, get_variable, has_variable, set_variable};
}

pub mod util {
    pub use crate::host::{now, random};
}

pub mod cache_ns {
    pub use crate::host::{cache_delete, cache_get, cache_has, cache_set};
}

pub mod meta {
    pub use crate::host::{
        get_app_id_from_host, get_board_id_from_host, get_log_level_from_host,
        get_node_id_from_host, get_run_id_from_host, get_user_id_from_host, is_streaming_from_host,
    };
}

pub mod storage_ns {
    pub use crate::host::{
        cache_dir, storage_dir, storage_list, storage_read, storage_write, upload_dir, user_dir,
    };
}

pub mod http_ns {
    pub use crate::host::http_request;
}

pub mod auth_ns {
    pub use crate::host::{get_oauth_token, has_oauth_token};
}

pub mod schema_ns {
    pub use crate::host::{get_type_schema, list_types};
}

pub mod image_ns {
    pub use crate::host::{image_from_bytes, image_to_bytes};
}

pub mod models_ns {
    pub use crate::host::{
        embed_image, embed_text, embed_text_document, embed_text_query, llm_prompt,
    };
}

pub mod db_ns {
    pub use crate::host::{
        db_count, db_delete, db_fts_search, db_hybrid_search, db_insert, db_list, db_upsert,
        db_vector_search,
    };
}

pub mod ws_ns {
    pub use crate::host::{ws_close, ws_connect, ws_receive, ws_send, ws_send_text};
}
