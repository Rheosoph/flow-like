//! WASM Module management
//!
//! Handles compiled modules and their metadata.

use crate::abi::{exports, WasmNodeDefinition};
use crate::engine::WasmEngine;
use crate::error::{WasmError, WasmResult};
use crate::instance::WasmInstance;
use crate::limits::WasmSecurityConfig;
use std::sync::Arc;
use wasmtime::Module;

/// A compiled WASM module
pub struct WasmModule {
    /// Wasmtime compiled module
    module: Module,
    /// Content hash for caching
    hash: String,
    /// Parsed node definition (cached after first call)
    node_definition: parking_lot::RwLock<Option<WasmNodeDefinition>>,
    /// Whether module has alloc export
    has_alloc: bool,
    /// Whether module has dealloc export
    has_dealloc: bool,
    /// Whether module has on_drop export
    has_on_drop: bool,
    /// Whether module has get_abi_version export
    has_abi_version: bool,
    /// ABI version (if reported by module)
    abi_version: Option<u32>,
}

impl WasmModule {
    /// Compile a module from bytes
    pub async fn from_bytes(engine: &WasmEngine, bytes: &[u8], hash: String) -> WasmResult<Self> {
        let module = Module::new(engine.engine(), bytes)
            .map_err(|e| WasmError::compilation(format!("Failed to compile WASM module: {}", e)))?;

        Self::build(module, hash)
    }

    /// Wrap an already-deserialized (AOT-cached) module
    pub fn from_precompiled(module: Module, hash: String) -> WasmResult<Self> {
        Self::build(module, hash)
    }

    fn build(module: Module, hash: String) -> WasmResult<Self> {
        Self::validate_exports(&module)?;

        let has_alloc = module.get_export(exports::ALLOC).is_some();
        let has_dealloc = module.get_export(exports::DEALLOC).is_some();
        let has_on_drop = module.get_export(exports::ON_DROP).is_some();
        let has_abi_version = module.get_export(exports::GET_ABI_VERSION).is_some();

        Ok(Self {
            module,
            hash,
            node_definition: parking_lot::RwLock::new(None),
            has_alloc,
            has_dealloc,
            has_on_drop,
            has_abi_version,
            abi_version: None,
        })
    }

    /// Validate that module has required exports
    fn validate_exports(module: &Module) -> WasmResult<()> {
        // Must have either get_node (single node) or get_nodes (multi-node package)
        let has_get_node = module.get_export(exports::GET_NODE).is_some();
        let has_get_nodes = module.get_export(exports::GET_NODES).is_some();

        if !has_get_node && !has_get_nodes {
            return Err(WasmError::MissingExport {
                export_name: format!("{} or {}", exports::GET_NODE, exports::GET_NODES),
            });
        }

        // Must have run
        if module.get_export(exports::RUN).is_none() {
            return Err(WasmError::MissingExport {
                export_name: exports::RUN.to_string(),
            });
        }

        // Validate get_node signature if present
        if let Some(get_node_export) = module.get_export(exports::GET_NODE) {
            if let Some(func) = get_node_export.func() {
                let ty = func.params().collect::<Vec<_>>();
                let results = func.results().collect::<Vec<_>>();

                if !ty.is_empty() || results.len() != 1 {
                    return Err(WasmError::InvalidExportSignature {
                        export_name: exports::GET_NODE.to_string(),
                        expected: "() -> i64".to_string(),
                        actual: format!("({:?}) -> {:?}", ty, results),
                    });
                }
            }
        }

        // Validate get_nodes signature if present
        if let Some(get_nodes_export) = module.get_export(exports::GET_NODES) {
            if let Some(func) = get_nodes_export.func() {
                let ty = func.params().collect::<Vec<_>>();
                let results = func.results().collect::<Vec<_>>();

                if !ty.is_empty() || results.len() != 1 {
                    return Err(WasmError::InvalidExportSignature {
                        export_name: exports::GET_NODES.to_string(),
                        expected: "() -> i64".to_string(),
                        actual: format!("({:?}) -> {:?}", ty, results),
                    });
                }
            }
        }

        // Validate run signature
        let run_export = module.get_export(exports::RUN).unwrap();
        if let Some(func) = run_export.func() {
            let params = func.params().collect::<Vec<_>>();
            let results = func.results().collect::<Vec<_>>();

            if params.len() != 2 || results.len() != 1 {
                return Err(WasmError::InvalidExportSignature {
                    export_name: exports::RUN.to_string(),
                    expected: "(i32, i32) -> i64".to_string(),
                    actual: format!("({:?}) -> {:?}", params, results),
                });
            }
        }

        Ok(())
    }

    /// Get the wasmtime module
    pub fn module(&self) -> &Module {
        &self.module
    }

    /// Get content hash
    pub fn hash(&self) -> &str {
        &self.hash
    }

    /// Check if module has alloc export
    pub fn has_alloc(&self) -> bool {
        self.has_alloc
    }

    /// Check if module has dealloc export
    pub fn has_dealloc(&self) -> bool {
        self.has_dealloc
    }

    /// Get node definition (calls WASM get_node if not cached)
    pub async fn get_node_definition(
        self: &Arc<Self>,
        engine: &WasmEngine,
        security: &WasmSecurityConfig,
    ) -> WasmResult<WasmNodeDefinition> {
        // Check cache
        {
            let cached = self.node_definition.read();
            if let Some(def) = cached.as_ref() {
                return Ok(def.clone());
            }
        }

        // Create temporary instance to call get_node
        let mut instance = WasmInstance::new(engine, self.clone(), security.clone()).await?;
        let definition = instance.call_get_node().await?;

        // Cache it
        {
            let mut cache = self.node_definition.write();
            *cache = Some(definition.clone());
        }

        Ok(definition)
    }

    /// Create a new instance of this module
    pub async fn instantiate(
        self: &Arc<Self>,
        engine: &WasmEngine,
        security: &WasmSecurityConfig,
    ) -> WasmResult<WasmInstance> {
        WasmInstance::new(engine, self.clone(), security.clone()).await
    }
}

impl std::fmt::Debug for WasmModule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmModule")
            .field("hash", &self.hash)
            .field("has_alloc", &self.has_alloc)
            .field("has_dealloc", &self.has_dealloc)
            .field("has_on_drop", &self.has_on_drop)
            .field("abi_version", &self.abi_version)
            .finish()
    }
}

/// Module metadata extracted from node definition
#[derive(Debug, Clone)]
pub struct WasmModuleMeta {
    pub name: String,
    pub friendly_name: String,
    pub description: String,
    pub category: String,
    pub hash: String,
}

impl From<&WasmNodeDefinition> for WasmModuleMeta {
    fn from(def: &WasmNodeDefinition) -> Self {
        Self {
            name: def.name.clone(),
            friendly_name: def.friendly_name.clone(),
            description: def.description.clone(),
            category: def.category.clone(),
            hash: String::new(),
        }
    }
}
