//! WASM Instance management
//!
//! Handles instantiated modules and their execution.

use crate::abi::{exports, WasmAbi, WasmExecutionInput, WasmExecutionResult, WasmNodeDefinition};
use crate::engine::WasmEngine;
use crate::error::{WasmError, WasmResult};
use crate::host_functions::linker::{register_host_functions, StoreData};
use crate::host_functions::HostState;
use crate::limits::WasmSecurityConfig;
use crate::memory::{WasmAllocator, WasmMemory};
use crate::module::WasmModule;
use std::sync::Arc;
use wasmtime::{Instance, Linker, Memory, Store, TypedFunc};

/// An instantiated WASM module ready for execution
pub struct WasmInstance {
    /// The store containing instance state
    store: Store<StoreData>,
    /// The wasmtime instance
    instance: Instance,
    /// Reference to the module
    module: Arc<WasmModule>,
    /// Memory export
    memory: Memory,
    /// Cached function references
    /// get_node for single-node modules
    get_node_func: Option<TypedFunc<(), i64>>,
    /// get_nodes for multi-node packages
    get_nodes_func: Option<TypedFunc<(), i64>>,
    run_func: TypedFunc<(i32, i32), i64>,
    alloc_func: Option<TypedFunc<i32, i32>>,
    dealloc_func: Option<TypedFunc<(i32, i32), ()>>,
    /// Fuel limit for tracking
    fuel_limit: u64,
}

impl WasmInstance {
    /// Create a new instance from a module
    pub async fn new(
        engine: &WasmEngine,
        module: Arc<WasmModule>,
        security: WasmSecurityConfig,
    ) -> WasmResult<Self> {
        // Use the engine that compiled/deserialized this module to ensure
        // Store, Linker, and Module are all tied to the same Engine instance.
        // This is critical for the Pulley fallback on iOS where the
        // module may come from a different engine than the primary one.
        let module_engine = module.module().engine();

        let mut linker = Linker::new(module_engine);
        register_host_functions(&mut linker)?;

        let mut store = Store::new(module_engine, StoreData::new(security.capabilities));

        // Configure store limits
        let fuel_limit = security.limits.fuel_limit;
        if engine.config().fuel_metering {
            store
                .set_fuel(fuel_limit)
                .map_err(|e| WasmError::Internal(format!("Failed to set fuel: {}", e)))?;
        }

        if engine.config().epoch_interruption {
            store.epoch_deadline_trap();
            let timeout_epochs = (security.limits.timeout.as_millis() / 10) as u64;
            store.set_epoch_deadline(timeout_epochs);
        }

        // Instantiate module
        let instance = linker
            .instantiate_async(&mut store, module.module())
            .await
            .map_err(|e| {
                WasmError::instantiation(format!("Failed to instantiate module: {}", e))
            })?;

        // Call _initialize if exported (needed for WASI reactor modules like Kotlin/Wasm)
        if let Ok(init) = instance.get_typed_func::<(), ()>(&mut store, "_initialize") {
            init.call_async(&mut store, ()).await.map_err(|e| {
                WasmError::instantiation(format!("Failed to call _initialize: {}", e))
            })?;
        }

        // Call _start if exported (needed for Grain, MoonBit and similar runtimes)
        if let Ok(start) = instance.get_typed_func::<(), ()>(&mut store, "_start") {
            start
                .call_async(&mut store, ())
                .await
                .map_err(|e| WasmError::instantiation(format!("Failed to call _start: {}", e)))?;
        }

        // Get memory export
        let memory =
            instance
                .get_memory(&mut store, "memory")
                .ok_or_else(|| WasmError::MissingExport {
                    export_name: "memory".to_string(),
                })?;

        // Store memory reference in StoreData so host functions can access it
        store.data_mut().memory = Some(memory);

        // Get function exports
        // Try get_node (single-node) first, then get_nodes (multi-node package)
        let get_node_func = instance
            .get_typed_func::<(), i64>(&mut store, exports::GET_NODE)
            .ok();

        let get_nodes_func = instance
            .get_typed_func::<(), i64>(&mut store, exports::GET_NODES)
            .ok();

        // Must have at least one of them
        if get_node_func.is_none() && get_nodes_func.is_none() {
            return Err(WasmError::MissingExport {
                export_name: format!("{} or {}", exports::GET_NODE, exports::GET_NODES),
            });
        }

        let run_func = instance
            .get_typed_func::<(i32, i32), i64>(&mut store, exports::RUN)
            .map_err(|_e| WasmError::MissingExport {
                export_name: exports::RUN.to_string(),
            })?;

        // Optional exports
        let alloc_func = instance
            .get_typed_func::<i32, i32>(&mut store, exports::ALLOC)
            .ok();

        let dealloc_func = instance
            .get_typed_func::<(i32, i32), ()>(&mut store, exports::DEALLOC)
            .ok();

        // Set up allocator
        let memory_size = memory.data_size(&store);
        let has_alloc = alloc_func.is_some();
        store.data_mut().allocator = Some(WasmAllocator::new(
            has_alloc,
            memory_size as u32, // Start bump allocator at end of initial memory
            security.limits.memory_limit as u32,
        ));

        Ok(Self {
            store,
            instance,
            module,
            memory,
            get_node_func,
            get_nodes_func,
            run_func,
            alloc_func,
            dealloc_func,
            fuel_limit,
        })
    }

    /// Call the get_node export (works with both single-node and multi-node packages)
    /// For multi-node packages, returns the first node definition
    pub async fn call_get_node(&mut self) -> WasmResult<WasmNodeDefinition> {
        // Try get_node first (single-node)
        if let Some(ref get_node_func) = self.get_node_func {
            let result = get_node_func
                .call_async(&mut self.store, ())
                .await
                .map_err(|e| {
                    WasmError::execution(exports::GET_NODE, format!("Call failed: {}", e))
                })?;

            if WasmAbi::is_error(result) {
                return Err(WasmError::execution(
                    exports::GET_NODE,
                    format!("Returned error code: {}", WasmAbi::get_error_code(result)),
                ));
            }

            let (ptr, len) = WasmAbi::unpack_ptr_len(result);
            let json_bytes = WasmMemory::read_bytes(&self.memory, &self.store, ptr, len)?;
            let json_str = String::from_utf8(json_bytes)
                .map_err(|e| WasmError::invalid_node_definition(format!("Invalid UTF-8: {}", e)))?;

            let definition: WasmNodeDefinition = serde_json::from_str(&json_str)
                .map_err(|e| WasmError::invalid_node_definition(format!("Invalid JSON: {}", e)))?;

            return Ok(definition);
        }

        // Fall back to get_nodes (multi-node package) - return first node
        if let Some(ref get_nodes_func) = self.get_nodes_func {
            let result = get_nodes_func
                .call_async(&mut self.store, ())
                .await
                .map_err(|e| {
                    WasmError::execution(exports::GET_NODES, format!("Call failed: {}", e))
                })?;

            if WasmAbi::is_error(result) {
                return Err(WasmError::execution(
                    exports::GET_NODES,
                    format!("Returned error code: {}", WasmAbi::get_error_code(result)),
                ));
            }

            let (ptr, len) = WasmAbi::unpack_ptr_len(result);
            let json_bytes = WasmMemory::read_bytes(&self.memory, &self.store, ptr, len)?;
            let json_str = String::from_utf8(json_bytes)
                .map_err(|e| WasmError::invalid_node_definition(format!("Invalid UTF-8: {}", e)))?;

            let definitions: Vec<WasmNodeDefinition> = serde_json::from_str(&json_str)
                .map_err(|e| WasmError::invalid_node_definition(format!("Invalid JSON: {}", e)))?;

            return definitions.into_iter().next().ok_or_else(|| {
                WasmError::invalid_node_definition("Empty node list in package".to_string())
            });
        }

        Err(WasmError::MissingExport {
            export_name: format!("{} or {}", exports::GET_NODE, exports::GET_NODES),
        })
    }

    /// Call get_nodes to get all node definitions from a multi-node package
    pub async fn call_get_nodes(&mut self) -> WasmResult<Vec<WasmNodeDefinition>> {
        // If single-node module, return single definition in a vec
        if self.get_nodes_func.is_none() && self.get_node_func.is_some() {
            let def = self.call_get_node().await?;
            return Ok(vec![def]);
        }

        if let Some(ref get_nodes_func) = self.get_nodes_func {
            let result = get_nodes_func
                .call_async(&mut self.store, ())
                .await
                .map_err(|e| {
                    WasmError::execution(exports::GET_NODES, format!("Call failed: {}", e))
                })?;

            if WasmAbi::is_error(result) {
                return Err(WasmError::execution(
                    exports::GET_NODES,
                    format!("Returned error code: {}", WasmAbi::get_error_code(result)),
                ));
            }

            let (ptr, len) = WasmAbi::unpack_ptr_len(result);
            let json_bytes = WasmMemory::read_bytes(&self.memory, &self.store, ptr, len)?;
            let json_str = String::from_utf8(json_bytes)
                .map_err(|e| WasmError::invalid_node_definition(format!("Invalid UTF-8: {}", e)))?;

            let definitions: Vec<WasmNodeDefinition> = serde_json::from_str(&json_str)
                .map_err(|e| WasmError::invalid_node_definition(format!("Invalid JSON: {}", e)))?;

            return Ok(definitions);
        }

        Err(WasmError::MissingExport {
            export_name: exports::GET_NODES.to_string(),
        })
    }

    /// Check if this is a multi-node package
    pub fn is_package(&self) -> bool {
        self.get_nodes_func.is_some()
    }

    /// Call the run export with execution input
    pub async fn call_run(
        &mut self,
        input: &WasmExecutionInput,
    ) -> WasmResult<WasmExecutionResult> {
        // Serialize input to JSON
        let input_json = serde_json::to_vec(input).map_err(WasmError::Json)?;
        let input_len = input_json.len() as u32;

        // Allocate memory for input
        let input_ptr = self.allocate(input_len).await?;

        // Write input to WASM memory
        WasmMemory::write_bytes(&self.memory, &mut self.store, input_ptr, &input_json)?;

        // Call run function
        let result = self
            .run_func
            .call_async(&mut self.store, (input_ptr as i32, input_len as i32))
            .await
            .map_err(|e| {
                // Check for specific error types
                let msg = e.to_string();
                if msg.contains("all fuel consumed") {
                    return WasmError::OutOfFuel {
                        limit: self.fuel_limit,
                    };
                }
                if msg.contains("epoch deadline") || msg.contains("interrupt") {
                    return WasmError::Timeout {
                        duration_ms: 0, // Could track this better
                    };
                }
                WasmError::execution(exports::RUN, format!("Call failed: {}", e))
            })?;

        // Deallocate input
        self.deallocate(input_ptr, input_len).await?;

        if WasmAbi::is_error(result) {
            return Err(WasmError::execution(
                exports::RUN,
                format!("Returned error code: {}", WasmAbi::get_error_code(result)),
            ));
        }

        let (ptr, len) = WasmAbi::unpack_ptr_len(result);
        let json_bytes = WasmMemory::read_bytes(&self.memory, &self.store, ptr, len)?;
        let json_str = String::from_utf8(json_bytes).map_err(|e| {
            WasmError::execution(exports::RUN, format!("Invalid UTF-8 result: {}", e))
        })?;

        let exec_result: WasmExecutionResult = serde_json::from_str(&json_str).map_err(|e| {
            WasmError::execution(exports::RUN, format!("Invalid JSON result: {}", e))
        })?;

        Ok(exec_result)
    }

    /// Allocate memory in WASM
    async fn allocate(&mut self, size: u32) -> WasmResult<u32> {
        if let Some(alloc_func) = &self.alloc_func {
            let ptr = alloc_func
                .call_async(&mut self.store, size as i32)
                .await
                .map_err(|e| WasmError::memory_access(format!("Allocation failed: {}", e)))?;
            Ok(ptr as u32)
        } else {
            // Use bump allocator
            let ptr = {
                let allocator =
                    self.store.data_mut().allocator.as_mut().ok_or_else(|| {
                        WasmError::Internal("Allocator not initialized".to_string())
                    })?;
                allocator.bump_alloc(size, 8)?
            };

            // Grow WASM memory if the allocation extends beyond current size
            let end = ptr as u64 + size as u64;
            let current_size = self.memory.data_size(&self.store) as u64;
            if end > current_size {
                let page_size: u64 = 65536;
                let needed_pages = end.saturating_sub(current_size).div_ceil(page_size);
                self.memory
                    .grow(&mut self.store, needed_pages)
                    .map_err(|e| {
                        WasmError::memory_access(format!(
                            "Failed to grow memory by {} pages: {}",
                            needed_pages, e
                        ))
                    })?;
            }

            Ok(ptr)
        }
    }

    /// Deallocate memory in WASM
    async fn deallocate(&mut self, ptr: u32, size: u32) -> WasmResult<()> {
        if let Some(dealloc_func) = &self.dealloc_func {
            dealloc_func
                .call_async(&mut self.store, (ptr as i32, size as i32))
                .await
                .map_err(|e| WasmError::memory_access(format!("Deallocation failed: {}", e)))?;
        }
        // If no dealloc, memory will be reclaimed when instance is dropped
        Ok(())
    }

    /// Get remaining fuel
    pub fn remaining_fuel(&self) -> Option<u64> {
        self.store.get_fuel().ok()
    }

    /// Add more fuel to the store
    pub fn add_fuel(&mut self, fuel: u64) -> WasmResult<()> {
        self.store
            .set_fuel(self.store.get_fuel().unwrap_or(0) + fuel)
            .map_err(|e| WasmError::Internal(format!("Failed to add fuel: {}", e)))
    }

    /// Get reference to host state
    pub fn host_state(&self) -> &HostState {
        &self.store.data().host_state
    }

    /// Get mutable reference to host state
    pub fn host_state_mut(&mut self) -> &mut HostState {
        &mut self.store.data_mut().host_state
    }

    /// Get memory size in bytes
    pub fn memory_size(&self) -> usize {
        self.memory.data_size(&self.store)
    }
}

impl std::fmt::Debug for WasmInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmInstance")
            .field("module_hash", &self.module.hash())
            .field("memory_size", &self.memory_size())
            .field("remaining_fuel", &self.remaining_fuel())
            .finish()
    }
}
