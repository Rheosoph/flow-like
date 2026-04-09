//! Linker setup for host functions
//!
//! Registers all host functions with the wasmtime linker.

use crate::error::{WasmError, WasmResult};
use crate::host_functions::HostState;
use crate::limits::WasmCapabilities;
use crate::memory::WasmAllocator;
use flow_like_storage::object_store::path::Path;
use std::sync::Arc;
use wasmtime::{Caller, Linker, Memory, Ref, Val};

/// Store data passed to host functions
pub struct StoreData {
    pub host_state: HostState,
    pub memory: Option<Memory>,
    pub allocator: Option<WasmAllocator>,
    /// Set by `_emscripten_throw_longjmp`; consumed by `invoke_vii` to
    /// distinguish a longjmp-trap from a genuine WASM trap.
    pub longjmp_pending: bool,
}

impl StoreData {
    pub fn new(capabilities: WasmCapabilities) -> Self {
        Self {
            host_state: HostState::new(capabilities),
            memory: None,
            allocator: None,
            longjmp_pending: false,
        }
    }
}

/// Register all host functions with the linker
pub fn register_host_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    register_logging_functions(linker)?;
    register_pin_functions(linker)?;
    register_variable_functions(linker)?;
    register_cache_functions(linker)?;
    register_metadata_functions(linker)?;
    register_storage_functions(linker)?;
    register_http_functions(linker)?;
    register_websocket_functions(linker)?;
    register_streaming_functions(linker)?;
    register_auth_functions(linker)?;
    register_env_functions(linker)?;
    register_model_functions(linker)?;
    register_additional_model_functions(linker)?;
    register_schema_functions(linker)?;
    register_image_functions(linker)?;
    register_db_functions(linker)?;
    register_wasi_stubs(linker)?;
    register_emscripten_stubs(linker)?;

    Ok(())
}

/// Register env module functions for AssemblyScript compatibility
fn register_env_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // AssemblyScript abort function
    // Called when an assertion fails or error occurs
    linker
        .func_wrap(
            "env",
            "abort",
            |_caller: Caller<'_, StoreData>,
             _message: u32,
             _filename: u32,
             _line: u32,
             _column: u32| {
                // AssemblyScript passes string pointers and location info
                eprintln!("WASM abort called");
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register env::abort: {}", e)))?;

    // AssemblyScript host_log function used by our SDK
    linker
        .func_wrap(
            "env",
            "host_log",
            |caller: Caller<'_, StoreData>, level: u32, msg_ptr: u32, msg_len: u32| {
                if let Ok(message) = read_string_from_caller(&caller, msg_ptr, msg_len) {
                    caller.data().host_state.log(level as u8, message, None);
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register env::host_log: {}", e))
        })?;

    // AssemblyScript host_stream function for streaming events
    linker
        .func_wrap(
            "env",
            "host_stream",
            |caller: Caller<'_, StoreData>,
             event_type_ptr: u32,
             event_type_len: u32,
             data_ptr: u32,
             data_len: u32| {
                if let (Ok(event_type), Ok(data)) = (
                    read_string_from_caller(&caller, event_type_ptr, event_type_len),
                    read_string_from_caller(&caller, data_ptr, data_len),
                ) {
                    caller.data().host_state.stream_event(&event_type, &data);
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register env::host_stream: {}", e))
        })?;

    // AssemblyScript host_get_variable function
    linker
        .func_wrap(
            "env",
            "host_get_variable",
            |caller: Caller<'_, StoreData>, name_ptr: u32, name_len: u32| -> i64 {
                let name = match read_string_from_caller(&caller, name_ptr, name_len) {
                    Ok(n) => n,
                    Err(_) => return 0,
                };

                match caller.data().host_state.get_variable(&name) {
                    Some(v) => {
                        let json = serde_json::to_vec(&v).unwrap_or_default();
                        let (ptr, len) = caller.data().host_state.store_result(&json);
                        pack_ptr_len(ptr, len) as i64
                    }
                    None => 0,
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register env::host_get_variable: {}", e))
        })?;

    // AssemblyScript host_set_variable function
    linker
        .func_wrap(
            "env",
            "host_set_variable",
            |caller: Caller<'_, StoreData>,
             name_ptr: u32,
             name_len: u32,
             value_ptr: u32,
             value_len: u32|
             -> i32 {
                if let (Ok(name), Ok(value_str)) = (
                    read_string_from_caller(&caller, name_ptr, name_len),
                    read_string_from_caller(&caller, value_ptr, value_len),
                ) {
                    let value: serde_json::Value =
                        serde_json::from_str(&value_str).unwrap_or(serde_json::Value::Null);
                    caller.data().host_state.set_variable(&name, value);
                    return 0; // Success
                }
                -1 // Error
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register env::host_set_variable: {}", e))
        })?;

    // AssemblyScript host_time_now function
    linker
        .func_wrap(
            "env",
            "host_time_now",
            |_caller: Caller<'_, StoreData>| -> i64 {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0)
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register env::host_time_now: {}", e))
        })?;

    // AssemblyScript host_random function — backed by the OS CSPRNG via getrandom
    linker
        .func_wrap(
            "env",
            "host_random",
            |_caller: Caller<'_, StoreData>| -> i64 {
                let mut buf = [0u8; 8];
                getrandom::fill(&mut buf).expect("getrandom failed");
                i64::from_le_bytes(buf)
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register env::host_random: {}", e))
        })?;

    Ok(())
}

fn register_logging_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_log",
            "trace",
            |caller: Caller<'_, StoreData>, msg_ptr: u32, msg_len: u32| {
                if let Ok(message) = read_string_from_caller(&caller, msg_ptr, msg_len) {
                    caller.data().host_state.log(0, message, None);
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register log_trace: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_log",
            "debug",
            |caller: Caller<'_, StoreData>, msg_ptr: u32, msg_len: u32| {
                if let Ok(message) = read_string_from_caller(&caller, msg_ptr, msg_len) {
                    caller.data().host_state.log(1, message, None);
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register log_debug: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_log",
            "info",
            |caller: Caller<'_, StoreData>, msg_ptr: u32, msg_len: u32| {
                if let Ok(message) = read_string_from_caller(&caller, msg_ptr, msg_len) {
                    caller.data().host_state.log(2, message, None);
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register log_info: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_log",
            "warn",
            |caller: Caller<'_, StoreData>, msg_ptr: u32, msg_len: u32| {
                if let Ok(message) = read_string_from_caller(&caller, msg_ptr, msg_len) {
                    caller.data().host_state.log(3, message, None);
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register log_warn: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_log",
            "error",
            |caller: Caller<'_, StoreData>, msg_ptr: u32, msg_len: u32| {
                if let Ok(message) = read_string_from_caller(&caller, msg_ptr, msg_len) {
                    caller.data().host_state.log(4, message, None);
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register log_error: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_log",
            "log_json",
            |caller: Caller<'_, StoreData>,
             level: u32,
             msg_ptr: u32,
             msg_len: u32,
             data_ptr: u32,
             data_len: u32| {
                if let (Ok(message), Ok(data_str)) = (
                    read_string_from_caller(&caller, msg_ptr, msg_len),
                    read_string_from_caller(&caller, data_ptr, data_len),
                ) {
                    let data: Option<serde_json::Value> = serde_json::from_str(&data_str).ok();
                    caller.data().host_state.log(level as u8, message, data);
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register log_json: {}", e)))?;

    Ok(())
}

fn register_pin_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_pins",
            "get_input",
            |caller: Caller<'_, StoreData>, name_ptr: u32, name_len: u32| -> u64 {
                let name = match read_string_from_caller(&caller, name_ptr, name_len) {
                    Ok(n) => n,
                    Err(_) => return 0,
                };

                match caller.data().host_state.get_input(&name) {
                    Some(v) => {
                        let json = serde_json::to_vec(&v).unwrap_or_default();
                        let (ptr, len) = caller.data().host_state.store_result(&json);
                        pack_ptr_len(ptr, len)
                    }
                    None => 0,
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register get_input: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_pins",
            "set_output",
            |caller: Caller<'_, StoreData>,
             name_ptr: u32,
             name_len: u32,
             value_ptr: u32,
             value_len: u32| {
                if let (Ok(name), Ok(value_str)) = (
                    read_string_from_caller(&caller, name_ptr, name_len),
                    read_string_from_caller(&caller, value_ptr, value_len),
                ) {
                    if let Ok(value) = serde_json::from_str(&value_str) {
                        caller.data().host_state.set_output(&name, value);
                    }
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register set_output: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_pins",
            "activate_exec",
            |caller: Caller<'_, StoreData>, name_ptr: u32, name_len: u32| {
                if let Ok(name) = read_string_from_caller(&caller, name_ptr, name_len) {
                    caller.data().host_state.activate_exec(&name);
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register activate_exec: {}", e))
        })?;

    Ok(())
}

fn register_variable_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_vars",
            "get",
            |caller: Caller<'_, StoreData>, name_ptr: u32, name_len: u32| -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::VARIABLES_READ)
                {
                    return 0;
                }

                let name = match read_string_from_caller(&caller, name_ptr, name_len) {
                    Ok(n) => n,
                    Err(_) => return 0,
                };

                let vars = caller.data().host_state.variables.read();
                match vars.get(&name) {
                    Some(v) => {
                        let json = serde_json::to_vec(&v).unwrap_or_default();
                        drop(vars);
                        let (ptr, len) = caller.data().host_state.store_result(&json);
                        pack_ptr_len(ptr, len)
                    }
                    None => 0,
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register vars.get: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_vars",
            "set",
            |caller: Caller<'_, StoreData>,
             name_ptr: u32,
             name_len: u32,
             value_ptr: u32,
             value_len: u32| {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::VARIABLES_WRITE)
                {
                    return;
                }

                if let (Ok(name), Ok(value_str)) = (
                    read_string_from_caller(&caller, name_ptr, name_len),
                    read_string_from_caller(&caller, value_ptr, value_len),
                ) {
                    if let Ok(value) = serde_json::from_str(&value_str) {
                        caller
                            .data()
                            .host_state
                            .variables
                            .write()
                            .insert(name, value);
                    }
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register vars.set: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_vars",
            "delete",
            |caller: Caller<'_, StoreData>, name_ptr: u32, name_len: u32| {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::VARIABLES_WRITE)
                {
                    return;
                }

                if let Ok(name) = read_string_from_caller(&caller, name_ptr, name_len) {
                    caller.data().host_state.variables.write().remove(&name);
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register vars.delete: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_vars",
            "has",
            |caller: Caller<'_, StoreData>, name_ptr: u32, name_len: u32| -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::VARIABLES_READ)
                {
                    return 0;
                }

                let name = match read_string_from_caller(&caller, name_ptr, name_len) {
                    Ok(n) => n,
                    Err(_) => return 0,
                };

                if caller
                    .data()
                    .host_state
                    .variables
                    .read()
                    .contains_key(&name)
                {
                    1
                } else {
                    0
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register vars.has: {}", e)))?;

    Ok(())
}

fn register_cache_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_cache",
            "get",
            |caller: Caller<'_, StoreData>, key_ptr: u32, key_len: u32| -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_READ)
                {
                    return 0;
                }

                let key = match read_string_from_caller(&caller, key_ptr, key_len) {
                    Ok(k) => k,
                    Err(_) => return 0,
                };

                let cache = caller.data().host_state.cache.read();
                match cache.get(&key) {
                    Some(v) => {
                        let json = serde_json::to_vec(&v).unwrap_or_default();
                        drop(cache);
                        let (ptr, len) = caller.data().host_state.store_result(&json);
                        pack_ptr_len(ptr, len)
                    }
                    None => 0,
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register cache.get: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_cache",
            "set",
            |caller: Caller<'_, StoreData>,
             key_ptr: u32,
             key_len: u32,
             value_ptr: u32,
             value_len: u32| {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_WRITE)
                {
                    return;
                }

                if let (Ok(key), Ok(value_str)) = (
                    read_string_from_caller(&caller, key_ptr, key_len),
                    read_string_from_caller(&caller, value_ptr, value_len),
                ) {
                    if let Ok(value) = serde_json::from_str(&value_str) {
                        caller.data().host_state.cache.write().insert(key, value);
                    }
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register cache.set: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_cache",
            "delete",
            |caller: Caller<'_, StoreData>, key_ptr: u32, key_len: u32| {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_WRITE)
                {
                    return;
                }

                if let Ok(key) = read_string_from_caller(&caller, key_ptr, key_len) {
                    caller.data().host_state.cache.write().remove(&key);
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register cache.delete: {}", e))
        })?;

    linker
        .func_wrap(
            "flowlike_cache",
            "has",
            |caller: Caller<'_, StoreData>, key_ptr: u32, key_len: u32| -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_READ)
                {
                    return 0;
                }

                let key = match read_string_from_caller(&caller, key_ptr, key_len) {
                    Ok(k) => k,
                    Err(_) => return 0,
                };

                if caller.data().host_state.cache.read().contains_key(&key) {
                    1
                } else {
                    0
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register cache.has: {}", e)))?;

    Ok(())
}

fn register_metadata_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_meta",
            "get_node_id",
            |caller: Caller<'_, StoreData>| -> u64 {
                let id = &caller.data().host_state.metadata.node_id;
                let (ptr, len) = caller.data().host_state.store_result(id.as_bytes());
                pack_ptr_len(ptr, len)
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register get_node_id: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_meta",
            "get_run_id",
            |caller: Caller<'_, StoreData>| -> u64 {
                let id = &caller.data().host_state.metadata.run_id;
                let (ptr, len) = caller.data().host_state.store_result(id.as_bytes());
                pack_ptr_len(ptr, len)
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register get_run_id: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_meta",
            "get_app_id",
            |caller: Caller<'_, StoreData>| -> u64 {
                let id = &caller.data().host_state.metadata.app_id;
                let (ptr, len) = caller.data().host_state.store_result(id.as_bytes());
                pack_ptr_len(ptr, len)
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register get_app_id: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_meta",
            "get_board_id",
            |caller: Caller<'_, StoreData>| -> u64 {
                let id = &caller.data().host_state.metadata.board_id;
                let (ptr, len) = caller.data().host_state.store_result(id.as_bytes());
                pack_ptr_len(ptr, len)
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register get_board_id: {}", e))
        })?;

    linker
        .func_wrap(
            "flowlike_meta",
            "get_user_id",
            |caller: Caller<'_, StoreData>| -> u64 {
                let id = &caller.data().host_state.metadata.user_id;
                let (ptr, len) = caller.data().host_state.store_result(id.as_bytes());
                pack_ptr_len(ptr, len)
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register get_user_id: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_meta",
            "is_streaming",
            |caller: Caller<'_, StoreData>| -> i32 {
                if caller.data().host_state.metadata.stream_state {
                    1
                } else {
                    0
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register is_streaming: {}", e))
        })?;

    linker
        .func_wrap(
            "flowlike_meta",
            "get_log_level",
            |caller: Caller<'_, StoreData>| -> i32 {
                caller.data().host_state.metadata.log_level as i32
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register get_log_level: {}", e))
        })?;

    linker
        .func_wrap(
            "flowlike_meta",
            "time_now",
            |_caller: Caller<'_, StoreData>| -> i64 {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0)
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register time_now: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_meta",
            "random",
            |_caller: Caller<'_, StoreData>| -> u64 {
                use std::collections::hash_map::RandomState;
                use std::hash::{BuildHasher, Hasher};
                RandomState::new().build_hasher().finish()
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register random: {}", e)))?;

    Ok(())
}

fn register_storage_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // storage_dir — returns a FlowPath JSON for the board's storage directory
    linker
        .func_wrap(
            "flowlike_storage",
            "storage_dir",
            |caller: Caller<'_, StoreData>, node_scoped: i32| -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return 0;
                }
                storage_dir_impl(
                    &caller,
                    node_scoped != 0,
                    "storage",
                    |ctx| ctx.get_storage_dir(node_scoped != 0),
                    |ctx| ctx.stores.app_storage_store.clone(),
                )
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register storage.storage_dir: {}", e))
        })?;

    // upload_dir — returns a FlowPath JSON for the upload directory
    linker
        .func_wrap(
            "flowlike_storage",
            "upload_dir",
            |caller: Caller<'_, StoreData>| -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return 0;
                }
                storage_dir_impl(
                    &caller,
                    false,
                    "upload",
                    |ctx| ctx.get_upload_dir(),
                    |ctx| ctx.stores.app_storage_store.clone(),
                )
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register storage.upload_dir: {}", e))
        })?;

    // cache_dir — returns a FlowPath JSON for the cache directory
    linker
        .func_wrap(
            "flowlike_storage",
            "cache_dir",
            |caller: Caller<'_, StoreData>, node_scoped: i32, user_scoped: i32| -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return 0;
                }
                let node = node_scoped != 0;
                let user = user_scoped != 0;
                storage_dir_impl(
                    &caller,
                    node,
                    "cache",
                    |ctx| ctx.get_cache_dir(node, user),
                    |ctx| ctx.stores.temporary_store.clone(),
                )
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register storage.cache_dir: {}", e))
        })?;

    // user_dir — returns a FlowPath JSON for the user directory
    linker
        .func_wrap(
            "flowlike_storage",
            "user_dir",
            |caller: Caller<'_, StoreData>, node_scoped: i32| -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return 0;
                }
                storage_dir_impl(
                    &caller,
                    node_scoped != 0,
                    "user",
                    |ctx| ctx.get_user_dir(node_scoped != 0),
                    |ctx| ctx.stores.user_store.clone(),
                )
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register storage.user_dir: {}", e))
        })?;

    // read_request — reads bytes from a FlowPath (async)
    linker
        .func_wrap_async(
            "flowlike_storage",
            "read_request",
            |caller: Caller<'_, StoreData>, (path_ptr, path_len): (u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_READ)
                    {
                        return 0u64;
                    }

                    let flow_path_json = match read_string_from_caller(&caller, path_ptr, path_len)
                    {
                        Ok(s) => s,
                        Err(_) => return 0,
                    };

                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(_) => return 0,
                    };

                    let ctx = match &caller.data().host_state.storage_context {
                        Some(c) => c,
                        None => return 0,
                    };

                    let store = match ctx.resolve_store(&flow_path.store_ref) {
                        Some(s) => s,
                        None => return 0,
                    };

                    let path = Path::from(flow_path.path);
                    match store.as_generic().get(&path).await {
                        Ok(result) => match result.bytes().await {
                            Ok(bytes) => {
                                let (ptr, len) = caller.data().host_state.store_result(&bytes);
                                pack_ptr_len(ptr, len)
                            }
                            Err(_) => 0,
                        },
                        Err(_) => 0,
                    }
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register storage.read_request: {}", e))
        })?;

    // write_request — writes bytes to a FlowPath (async)
    linker
        .func_wrap_async(
            "flowlike_storage",
            "write_request",
            |caller: Caller<'_, StoreData>,
             (path_ptr, path_len, data_ptr, data_len): (u32, u32, u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_WRITE)
                    {
                        return -1i32;
                    }

                    let flow_path_json = match read_string_from_caller(&caller, path_ptr, path_len)
                    {
                        Ok(s) => s,
                        Err(_) => return -1,
                    };

                    let data = match read_bytes_from_caller(&caller, data_ptr, data_len) {
                        Ok(d) => d,
                        Err(_) => return -1,
                    };

                    if data.len() > crate::host_functions::storage::MAX_STORAGE_FILE_SIZE {
                        return -1;
                    }

                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(_) => return -1,
                    };

                    let ctx = match &caller.data().host_state.storage_context {
                        Some(c) => c,
                        None => return -1,
                    };

                    let store = match ctx.resolve_store(&flow_path.store_ref) {
                        Some(s) => s,
                        None => return -1,
                    };

                    let path = Path::from(flow_path.path);
                    let payload = flow_like_storage::object_store::PutPayload::from_bytes(
                        flow_like_types::Bytes::from(data),
                    );
                    match store.as_generic().put(&path, payload).await {
                        Ok(_) => 0,
                        Err(_) => -1,
                    }
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register storage.write_request: {}", e))
        })?;

    // list_request — lists paths under a FlowPath prefix (async)
    linker
        .func_wrap_async(
            "flowlike_storage",
            "list_request",
            |caller: Caller<'_, StoreData>, (path_ptr, path_len): (u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_READ)
                    {
                        return 0u64;
                    }

                    let flow_path_json = match read_string_from_caller(&caller, path_ptr, path_len)
                    {
                        Ok(s) => s,
                        Err(_) => return 0,
                    };

                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(_) => return 0,
                    };

                    let ctx = match &caller.data().host_state.storage_context {
                        Some(c) => c,
                        None => return 0,
                    };

                    let store = match ctx.resolve_store(&flow_path.store_ref) {
                        Some(s) => s,
                        None => return 0,
                    };

                    use futures::StreamExt;
                    let prefix = Path::from(flow_path.path.clone());
                    let entries: Vec<_> = store
                        .as_generic()
                        .list(Some(&prefix))
                        .filter_map(|r| async { r.ok() })
                        .map(|meta| StorageFlowPath {
                            path: meta.location.as_ref().to_string(),
                            store_ref: flow_path.store_ref.clone(),
                            cache_store_ref: flow_path.cache_store_ref.clone(),
                        })
                        .collect()
                        .await;

                    match serde_json::to_vec(&entries) {
                        Ok(json) => {
                            let (ptr, len) = caller.data().host_state.store_result(&json);
                            pack_ptr_len(ptr, len)
                        }
                        Err(_) => 0,
                    }
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register storage.list_request: {}", e))
        })?;

    Ok(())
}

/// Helper: build a FlowPath for a directory, register the store, and return packed JSON.
fn storage_dir_impl(
    caller: &Caller<'_, StoreData>,
    _node_scoped: bool,
    dir_type: &str,
    dir_getter: impl FnOnce(&crate::host_functions::StorageContext) -> Path,
    store_getter: impl FnOnce(
        &crate::host_functions::StorageContext,
    ) -> Option<flow_like_storage::files::store::FlowLikeStore>,
) -> u64 {
    let ctx = match &caller.data().host_state.storage_context {
        Some(c) => c,
        None => return 0,
    };

    let dir = dir_getter(ctx);
    let store_hash = format!("wasm_dirs__{dir_type}_{}", dir.as_ref());

    if ctx.resolve_store(&store_hash).is_none() {
        let store = match store_getter(ctx) {
            Some(s) => s,
            None => return 0,
        };
        ctx.register_store(&store_hash, store);
    }

    let flow_path = StorageFlowPath {
        path: dir.as_ref().to_string(),
        store_ref: store_hash,
        cache_store_ref: None,
    };

    match serde_json::to_vec(&flow_path) {
        Ok(json) => {
            let (ptr, len) = caller.data().host_state.store_result(&json);
            pack_ptr_len(ptr, len)
        }
        Err(_) => 0,
    }
}

/// Minimal FlowPath for WASM — same shape as the real FlowPath for JSON compatibility.
#[derive(serde::Serialize, serde::Deserialize)]
struct StorageFlowPath {
    path: String,
    store_ref: String,
    cache_store_ref: Option<String>,
}

fn register_http_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_http",
            "request",
            |caller: Caller<'_, StoreData>,
             _method: i32,
             _url_ptr: u32,
             _url_len: u32,
             _headers_ptr: u32,
             _headers_len: u32,
             _body_ptr: u32,
             _body_len: u32|
             -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::HTTP_REQUEST)
                {
                    return -1;
                }
                // Async HTTP handled separately
                0
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register http.request: {}", e))
        })?;

    Ok(())
}

fn register_websocket_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // connect(url_ptr, url_len, headers_ptr, headers_len) -> i32 (session_id handle or -1)
    linker
        .func_wrap(
            "flowlike_ws",
            "connect",
            |caller: Caller<'_, StoreData>,
             _url_ptr: u32,
             _url_len: u32,
             _headers_ptr: u32,
             _headers_len: u32|
             -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return -1;
                }
                // Async WebSocket handled in component model
                0
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register ws.connect: {}", e)))?;

    // send(session_id, msg_ptr, msg_len, is_binary) -> i32
    linker
        .func_wrap(
            "flowlike_ws",
            "send",
            |caller: Caller<'_, StoreData>,
             _session_id: i32,
             _msg_ptr: u32,
             _msg_len: u32,
             _is_binary: i32|
             -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return -1;
                }
                0
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register ws.send: {}", e)))?;

    // receive(session_id, timeout_ms) -> i32 (result_ptr or -1)
    linker
        .func_wrap(
            "flowlike_ws",
            "receive",
            |caller: Caller<'_, StoreData>, _session_id: i32, _timeout_ms: u32| -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return -1;
                }
                0
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register ws.receive: {}", e)))?;

    // close(session_id) -> i32
    linker
        .func_wrap(
            "flowlike_ws",
            "close",
            |caller: Caller<'_, StoreData>, _session_id: i32| -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return -1;
                }
                0
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register ws.close: {}", e)))?;

    Ok(())
}

fn register_streaming_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_stream",
            "emit",
            |caller: Caller<'_, StoreData>,
             event_ptr: u32,
             event_len: u32,
             data_ptr: u32,
             data_len: u32| {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STREAMING)
                {
                    return;
                }

                if let (Ok(event_type), Ok(data_str)) = (
                    read_string_from_caller(&caller, event_ptr, event_len),
                    read_string_from_caller(&caller, data_ptr, data_len),
                ) {
                    if let Ok(data) = serde_json::from_str(&data_str) {
                        caller.data().host_state.add_stream_event(event_type, data);
                    }
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register stream.emit: {}", e)))?;

    linker
        .func_wrap(
            "flowlike_stream",
            "text",
            |caller: Caller<'_, StoreData>, text_ptr: u32, text_len: u32| {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STREAMING)
                {
                    return;
                }

                if let Ok(text) = read_string_from_caller(&caller, text_ptr, text_len) {
                    caller
                        .data()
                        .host_state
                        .add_stream_event("text".to_string(), serde_json::json!(text));
                }
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register stream.text: {}", e)))?;

    Ok(())
}

fn register_auth_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "flowlike_auth",
            "get_oauth_token",
            |caller: Caller<'_, StoreData>, provider_ptr: u32, provider_len: u32| -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::OAUTH_ACCESS)
                {
                    return 0;
                }

                let provider = match read_string_from_caller(&caller, provider_ptr, provider_len) {
                    Ok(p) => p,
                    Err(_) => return 0,
                };

                let tokens = caller.data().host_state.oauth_tokens.read();
                match tokens.get(&provider) {
                    Some(token) => {
                        let json = serde_json::json!({
                            "access_token": token.access_token,
                            "token_type": token.token_type,
                            "expires_at": token.expires_at,
                        });
                        let bytes = serde_json::to_vec(&json).unwrap_or_default();
                        drop(tokens);
                        let (ptr, len) = caller.data().host_state.store_result(&bytes);
                        pack_ptr_len(ptr, len)
                    }
                    None => 0,
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register get_oauth_token: {}", e))
        })?;

    linker
        .func_wrap(
            "flowlike_auth",
            "has_oauth_token",
            |caller: Caller<'_, StoreData>, provider_ptr: u32, provider_len: u32| -> i32 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::OAUTH_ACCESS)
                {
                    return 0;
                }

                let provider = match read_string_from_caller(&caller, provider_ptr, provider_len) {
                    Ok(p) => p,
                    Err(_) => return 0,
                };

                if caller
                    .data()
                    .host_state
                    .oauth_tokens
                    .read()
                    .contains_key(&provider)
                {
                    1
                } else {
                    0
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register has_oauth_token: {}", e))
        })?;

    Ok(())
}

fn register_model_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // embed_text — embed texts using a model Bit (async, resolved server-side)
    // Input: bit_json (serialized Bit struct), texts_json (JSON array of strings)
    // Output: packed ptr/len to JSON array of float arrays
    linker
        .func_wrap_async(
            "flowlike_models",
            "embed_text",
            |caller: Caller<'_, StoreData>,
             (bit_ptr, bit_len, texts_ptr, texts_len): (u32, u32, u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return 0u64;
                    }

                    let bit_json = match read_string_from_caller(&caller, bit_ptr, bit_len) {
                        Ok(s) => s,
                        Err(_) => return 0,
                    };

                    let texts_json = match read_string_from_caller(&caller, texts_ptr, texts_len) {
                        Ok(s) => s,
                        Err(_) => return 0,
                    };

                    let bit: flow_like::bit::Bit = match serde_json::from_str(&bit_json) {
                        Ok(b) => b,
                        Err(_) => return 0,
                    };

                    let _texts: Vec<String> = match serde_json::from_str(&texts_json) {
                        Ok(t) => t,
                        Err(_) => return 0,
                    };

                    let model_ctx = match &caller.data().host_state.model_context {
                        Some(c) => c,
                        None => return 0,
                    };

                    let app_state = model_ctx.app_state.clone();
                    {
                        let _ = app_state;
                        let _ = bit;
                        return 0u64;
                    };

                    #[cfg(feature = "model")]
                    {
                        match model.text_embed_query(&texts).await {
                            Ok(embeddings) => match serde_json::to_vec(&embeddings) {
                                Ok(json) => {
                                    let (ptr, len) = caller.data().host_state.store_result(&json);
                                    pack_ptr_len(ptr, len)
                                }
                                Err(_) => 0,
                            },
                            Err(_) => 0,
                        }
                    }
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register models.embed_text: {}", e))
        })?;

    Ok(())
}

fn register_schema_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // get_type_schema — returns JSON schema string for a well-known type
    // Capability-gated: FlowPath requires STORAGE_READ, others require MODELS
    linker
        .func_wrap(
            "flowlike_schema",
            "get_type_schema",
            |caller: Caller<'_, StoreData>, name_ptr: u32, name_len: u32| -> u64 {
                let name = match read_string_from_caller(&caller, name_ptr, name_len) {
                    Ok(n) => n,
                    Err(_) => return 0,
                };

                let required = crate::host_functions::schema::required_capability(&name);
                if !caller.data().host_state.has_capability(required) {
                    return 0;
                }

                match crate::host_functions::schema::get_type_schema(&name) {
                    Some(schema) => {
                        let (ptr, len) = caller.data().host_state.store_result(schema.as_bytes());
                        pack_ptr_len(ptr, len)
                    }
                    None => 0,
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register schema.get_type_schema: {}", e))
        })?;

    // list_type_schemas — returns JSON array of type names the caller has access to
    linker
        .func_wrap(
            "flowlike_schema",
            "list_types",
            |caller: Caller<'_, StoreData>| -> u64 {
                let names: Vec<&str> = crate::host_functions::schema::list_type_names()
                    .into_iter()
                    .filter(|name| {
                        let required = crate::host_functions::schema::required_capability(name);
                        caller.data().host_state.has_capability(required)
                    })
                    .collect();
                match serde_json::to_vec(&names) {
                    Ok(json) => {
                        let (ptr, len) = caller.data().host_state.store_result(&json);
                        pack_ptr_len(ptr, len)
                    }
                    Err(_) => 0,
                }
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register schema.list_types: {}", e))
        })?;

    Ok(())
}

fn register_image_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // from_bytes — create image from raw bytes, return NodeImage JSON ref
    linker
        .func_wrap(
            "flowlike_image",
            "from_bytes",
            |caller: Caller<'_, StoreData>,
             data_ptr: u32,
             data_len: u32,
             fmt_ptr: u32,
             fmt_len: u32|
             -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::MODELS)
                {
                    return 0;
                }
                // Stub — image creation requires host-side DynamicImage.
                // Will return the JSON for a NodeImage handle once host builds one.
                let _ = (data_ptr, data_len, fmt_ptr, fmt_len);
                let _ = &caller;
                0
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register image.from_bytes: {}", e))
        })?;

    // to_bytes — get raw bytes from image handle
    linker
        .func_wrap(
            "flowlike_image",
            "to_bytes",
            |caller: Caller<'_, StoreData>,
             ref_ptr: u32,
             ref_len: u32,
             fmt_ptr: u32,
             fmt_len: u32|
             -> u64 {
                if !caller
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::MODELS)
                {
                    return 0;
                }
                let _ = (ref_ptr, ref_len, fmt_ptr, fmt_len);
                let _ = &caller;
                0
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register image.to_bytes: {}", e))
        })?;

    Ok(())
}

fn register_db_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // query — unified DB operation dispatch
    // op: 1=vector_search, 2=fts_search, 3=hybrid_search, 4=insert, 5=upsert, 6=delete, 7=list, 8=count
    linker
        .func_wrap_async(
            "flowlike_db",
            "query",
            |caller: Caller<'_, StoreData>,
             (op, conn_ptr, conn_len, payload_ptr, payload_len): (
                u32,
                u32,
                u32,
                u32,
                u32,
            )| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return 0u64;
                    }

                    let _conn_json =
                        match read_string_from_caller(&caller, conn_ptr, conn_len) {
                            Ok(s) => s,
                            Err(_) => return 0,
                        };

                    let _payload_json =
                        match read_string_from_caller(&caller, payload_ptr, payload_len) {
                            Ok(s) => s,
                            Err(_) => return 0,
                        };

                    // Stub — DB operations require host-side LanceDB connection.
                    // op determines which method to call on the resolved CachedDB.
                    let _ = op;
                    0u64
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register db.query: {}", e))
        })?;

    Ok(())
}

fn register_additional_model_functions(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    // embed_text_query — embed texts for retrieval queries
    linker
        .func_wrap_async(
            "flowlike_models",
            "embed_text_query",
            |caller: Caller<'_, StoreData>,
             (model_ptr, model_len, texts_ptr, texts_len): (u32, u32, u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return 0u64;
                    }
                    let _ = (model_ptr, model_len, texts_ptr, texts_len);
                    0u64
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register models.embed_text_query: {}", e))
        })?;

    // embed_text_document — embed texts for document indexing
    linker
        .func_wrap_async(
            "flowlike_models",
            "embed_text_document",
            |caller: Caller<'_, StoreData>,
             (model_ptr, model_len, texts_ptr, texts_len): (u32, u32, u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return 0u64;
                    }
                    let _ = (model_ptr, model_len, texts_ptr, texts_len);
                    0u64
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register models.embed_text_document: {}",
                e
            ))
        })?;

    // embed_image — embed an image using an embedding model
    linker
        .func_wrap_async(
            "flowlike_models",
            "embed_image",
            |caller: Caller<'_, StoreData>,
             (model_ptr, model_len, image_ptr, image_len): (u32, u32, u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return 0u64;
                    }
                    let _ = (model_ptr, model_len, image_ptr, image_len);
                    0u64
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register models.embed_image: {}", e))
        })?;

    // llm_prompt — send a completion prompt to an LLM/VLM
    linker
        .func_wrap_async(
            "flowlike_models",
            "llm_prompt",
            |caller: Caller<'_, StoreData>,
                 (bit_ptr, bit_len, messages_ptr, messages_len, stream): (
                u32,
                u32,
                u32,
                u32,
                i32,
            )| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        println!("llm_prompt: MODELS capability not granted");
                        return 0u64;
                    }

                    let bit_json = match read_string_from_caller(&caller, bit_ptr, bit_len) {
                        Ok(s) => s,
                        Err(_) => {
                            println!("llm_prompt: failed to read bit from WASM memory");
                            return 0u64;
                        }
                    };

                    let messages_json =
                        match read_string_from_caller(&caller, messages_ptr, messages_len) {
                            Ok(s) => s,
                            Err(_) => {
                                println!("llm_prompt: failed to read messages from WASM memory");
                                return 0u64;
                            }
                        };

                    let bit: flow_like::bit::Bit = match serde_json::from_str(&bit_json) {
                        Ok(b) => b,
                        Err(e) => {
                            println!("llm_prompt: failed to parse bit JSON: {e}");
                            let err = serde_json::json!({"error": format!("Failed to parse model descriptor: {e}")}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };

                    let model_ctx = match &caller.data().host_state.model_context {
                        Some(c) => c,
                        None => {
                            println!("llm_prompt: model_context is None");
                            let err = serde_json::json!({"error": "Model context not available — ensure the node has Models permission"}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };
                    let app_state = model_ctx.app_state.clone();
                    let access_token = model_ctx.token.clone();

                    // Parse messages_json: either {messages, tools, ...params} or a plain array
                    #[derive(serde::Deserialize)]
                    struct LlmPromptRequest {
                        messages: Vec<serde_json::Value>,
                        #[serde(default)]
                        tools: Option<Vec<serde_json::Value>>,
                        #[serde(default)]
                        temperature: Option<f64>,
                        #[serde(default)]
                        max_tokens: Option<u64>,
                        #[serde(default)]
                        tool_choice: Option<serde_json::Value>,
                        #[serde(default)]
                        output_schema: Option<serde_json::Value>,
                        #[serde(default)]
                        additional_params: Option<serde_json::Value>,
                    }

                    let (raw_messages, raw_tools, req_temperature, req_max_tokens, req_tool_choice, req_output_schema, req_additional_params) =
                        match serde_json::from_str::<LlmPromptRequest>(&messages_json) {
                            Ok(req) => (req.messages, req.tools, req.temperature, req.max_tokens, req.tool_choice, req.output_schema, req.additional_params),
                            Err(_) => {
                                match serde_json::from_str::<Vec<serde_json::Value>>(
                                    &messages_json,
                                ) {
                                    Ok(msgs) => (msgs, None, None, None, None, None, None),
                                    Err(e) => {
                                        println!("llm_prompt: failed to parse messages JSON: {e}");
                                        let err = serde_json::json!({"error": format!("Failed to parse messages: {e}")}).to_string();
                                        let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                                        return pack_ptr_len(ptr, len);
                                    }
                                }
                            }
                        };

                    println!("llm_prompt: received {} messages, tools={}",
                        raw_messages.len(),
                        raw_tools.as_ref().map(|t| t.len()).unwrap_or(0)
                    );
                    if let Some(ref tools) = raw_tools {
                        for (i, t) in tools.iter().enumerate() {
                            println!("llm_prompt: raw tool[{i}]: {}", t);
                        }
                    }
                    for (i, m) in raw_messages.iter().enumerate() {
                        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                        let content_preview = m.get("content")
                            .and_then(|c| c.as_str())
                            .map(|s| if s.len() > 200 { format!("{}...", &s[..200]) } else { s.to_string() })
                            .unwrap_or_else(|| "<non-string>".to_string());
                        println!("llm_prompt: msg[{i}] role={role} content={content_preview}");
                    }

                    // Convert WASM SDK messages → native HistoryMessage
                    let mut history_messages = Vec::with_capacity(raw_messages.len());
                    for msg in &raw_messages {
                        let role_str =
                            msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                        let role = match role_str {
                            "system" => flow_like_model_provider::history::Role::System,
                            "assistant" => flow_like_model_provider::history::Role::Assistant,
                            "tool" => flow_like_model_provider::history::Role::Tool,
                            _ => flow_like_model_provider::history::Role::User,
                        };

                        let content =
                            if let Some(c) = msg.get("content").and_then(|v| v.as_str()) {
                                flow_like_model_provider::history::MessageContent::String(
                                    c.to_string(),
                                )
                            } else if let Some(parts) =
                                msg.get("parts").and_then(|v| v.as_array())
                            {
                                let mut contents = Vec::new();
                                for part in parts {
                                    if let Some(text) =
                                        part.get("text").and_then(|t| t.as_str())
                                    {
                                        contents.push(
                                            flow_like_model_provider::history::Content::Text {
                                                content_type:
                                                    flow_like_model_provider::history::ContentType::Text,
                                                text: text.to_string(),
                                            },
                                        );
                                    } else if let Some(reasoning) = part
                                        .get("reasoning")
                                        .and_then(|reasoning| reasoning.get("text"))
                                        .and_then(|text| text.as_array())
                                    {
                                        let text = reasoning
                                            .iter()
                                            .filter_map(|entry| entry.as_str())
                                            .collect::<Vec<_>>()
                                            .join("\n");
                                        if !text.is_empty() {
                                            contents.push(
                                                flow_like_model_provider::history::Content::Text {
                                                    content_type:
                                                        flow_like_model_provider::history::ContentType::Text,
                                                    text,
                                                },
                                            );
                                        }
                                    }
                                }
                                flow_like_model_provider::history::MessageContent::Contents(
                                    contents,
                                )
                            } else {
                                flow_like_model_provider::history::MessageContent::String(
                                    String::new(),
                                )
                            };

                        let tool_calls = msg
                            .get("tool_calls")
                            .and_then(|v| v.as_array())
                            .map(|tcs| {
                                tcs.iter()
                                    .filter_map(|tc| {
                                        let id = tc.get("id")?.as_str()?.to_string();
                                        let name = tc.get("name")?.as_str()?.to_string();
                                        let args =
                                            tc.get("arguments").cloned().unwrap_or_default();
                                        let args_str = if args.is_string() {
                                            args.as_str().unwrap_or("{}").to_string()
                                        } else {
                                            serde_json::to_string(&args).unwrap_or_default()
                                        };
                                        Some(flow_like_model_provider::history::ToolCall {
                                            id,
                                            r#type: "function".to_string(),
                                            function:
                                                flow_like_model_provider::history::ToolCallFunction {
                                                    name,
                                                    arguments: args_str,
                                                },
                                        })
                                    })
                                    .collect()
                            });

                        let tool_call_id = msg
                            .get("tool_call_id")
                            .and_then(|v| v.as_str())
                            .map(|s| s.to_string());

                        history_messages.push(
                            flow_like_model_provider::history::HistoryMessage {
                                role,
                                content,
                                name: None,
                                tool_calls,
                                tool_call_id,
                                annotations: None,
                            },
                        );
                    }

                    let mut history = flow_like_model_provider::history::History::new(
                        bit.id.clone(),
                        history_messages,
                    );
                    let should_stream = stream != 0;
                    history.stream = should_stream.then_some(true);

                    // Apply optional request parameters
                    if let Some(temp) = req_temperature {
                        history.temperature = Some(temp as f32);
                    }
                    if let Some(max) = req_max_tokens {
                        history.max_completion_tokens = Some(max as u32);
                    }
                    if let Some(tc_val) = req_tool_choice {
                        if let Ok(tc) = serde_json::from_value::<flow_like_model_provider::history::ToolChoice>(tc_val) {
                            history.tool_choice = Some(tc);
                        }
                    }
                    if let Some(schema) = req_output_schema {
                        history.response_format = Some(flow_like_model_provider::history::ResponseFormat::Object(
                            serde_json::json!({
                                "type": "json_schema",
                                "json_schema": {
                                    "name": schema.get("title").and_then(|t| t.as_str()).unwrap_or("response_schema"),
                                    "schema": schema,
                                    "strict": true
                                }
                            }),
                        ));
                    }
                    // additional_params are provider-specific; log but don't lose them
                    if let Some(ref params) = req_additional_params {
                        println!("llm_prompt: additional_params: {params}");
                    }

                    // Convert tool definitions if present
                    if let Some(tools) = raw_tools {
                        let mut native_tools: Vec<flow_like_model_provider::history::Tool> = Vec::new();
                        for (i, t) in tools.iter().enumerate() {
                            let name = match t.get("name").and_then(|n| n.as_str()) {
                                Some(n) => n.to_string(),
                                None => {
                                    println!("llm_prompt: tool[{i}] missing 'name' field");
                                    continue;
                                }
                            };
                            let desc = t.get("description").and_then(|d| d.as_str()).map(String::from);
                            let params = t.get("parameters").cloned().unwrap_or_default();
                            println!("llm_prompt: tool[{i}] '{name}' params: {params}");
                            match serde_json::from_value::<flow_like_model_provider::history::HistoryFunctionParameters>(params.clone()) {
                                Ok(parsed) => {
                                    native_tools.push(flow_like_model_provider::history::Tool {
                                        tool_type: flow_like_model_provider::history::ToolType::Function,
                                        function: flow_like_model_provider::history::HistoryFunction {
                                            name,
                                            description: desc,
                                            parameters: parsed,
                                        },
                                    });
                                }
                                Err(e) => {
                                    println!("llm_prompt: tool[{i}] '{name}' parameter deserialization FAILED: {e} — raw: {params}");
                                }
                            }
                        }
                        if !native_tools.is_empty() {
                            history.tools = Some(native_tools);
                        }
                    }

                    // Build model and invoke
                    let model = {
                        let mut factory = app_state.model_factory.lock().await;
                        match factory.build(&bit, app_state.clone(), access_token.clone()).await {
                            Ok(m) => m,
                            Err(e) => {
                                println!("llm_prompt: failed to build model: {e}");
                                let err = serde_json::json!({"error": format!("Failed to build model: {e}")}).to_string();
                                let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                                return pack_ptr_len(ptr, len);
                            }
                        }
                    };

                    // Log the full History before invoking
                    if let Ok(history_json) = serde_json::to_string(&history) {
                        println!("llm_prompt: History to invoke (len={}): {}",
                            history_json.len(),
                            if history_json.len() > 2000 { format!("{}...", &history_json[..2000]) } else { history_json }
                        );
                    }

                    let stream_events = should_stream.then(|| {
                        Arc::new(parking_lot::RwLock::new(Vec::<crate::host_functions::StreamEvent>::new()))
                    });

                    let callback = stream_events.as_ref().map(|stream_events_cb| {
                        let stream_events_cb = stream_events_cb.clone();
                        Arc::new(
                            move |chunk: flow_like_model_provider::response_chunk::ResponseChunk| {
                                let events = stream_events_cb.clone();
                                let future: std::pin::Pin<Box<dyn std::future::Future<Output = flow_like_types::Result<()>> + Send>> = Box::pin(async move {
                                    if let Ok(chunk_json) = serde_json::to_value(&chunk) {
                                        events.write().push(crate::host_functions::StreamEvent {
                                            event_type: "llm_chunk".to_string(),
                                            data: chunk_json,
                                        });
                                    }
                                    Ok(())
                                });
                                future
                            },
                        ) as flow_like_model_provider::llm::LLMCallback
                    });

                    let response = match model.invoke(&history, callback).await {
                        Ok(r) => r,
                        Err(e) => {
                            println!("llm_prompt: model invoke failed: {e}");
                            let err = serde_json::json!({"error": format!("Model invocation failed: {e}")}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };

                    if let Some(stream_events) = stream_events {
                        let collected = std::mem::take(&mut *stream_events.write());
                        let mut host_events = caller.data().host_state.stream_events.write();
                        host_events.extend(collected);
                    }

                    // Convert response to SDK ChatMessage JSON
                    let resp_msg = match response.last_message() {
                        Some(m) => m,
                        None => {
                            println!("llm_prompt: model returned empty response (no messages)");
                            let err = serde_json::json!({"error": "Model returned empty response"}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };

                    let tool_calls_json: Option<Vec<serde_json::Value>> =
                        if resp_msg.tool_calls.is_empty() {
                            None
                        } else {
                            Some(
                                resp_msg
                                    .tool_calls
                                    .iter()
                                    .map(|tc| {
                                        let args: serde_json::Value =
                                            serde_json::from_str(&tc.function.arguments)
                                                .unwrap_or(serde_json::Value::Object(
                                                    Default::default(),
                                                ));
                                        serde_json::json!({
                                            "id": tc.id,
                                            "name": tc.function.name,
                                            "arguments": args,
                                        })
                                    })
                                    .collect(),
                            )
                        };

                    let usage = serde_json::json!({
                        "prompt_tokens": response.usage.prompt_tokens,
                        "completion_tokens": response.usage.completion_tokens,
                        "total_tokens": response.usage.total_tokens,
                    });

                    let result = serde_json::json!({
                        "role": "assistant",
                        "content": resp_msg.content.clone().unwrap_or_default(),
                        "reasoning": resp_msg.reasoning.clone(),
                        "tool_calls": tool_calls_json,
                        "message_id": response.id,
                        "usage": usage,
                    });

                    let result_str = result.to_string();
                    let (ptr, len) =
                        caller.data().host_state.store_result(result_str.as_bytes());
                    pack_ptr_len(ptr, len)
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register models.llm_prompt: {}", e))
        })?;

    // llm_prompt_stream — ABI v2 streaming LLM prompt
    // Streams ResponseChunk events via add_stream_event("llm_chunk", ...) during invoke.
    // Returns final response JSON with usage and message_id.
    linker
        .func_wrap_async(
            "flowlike_models",
            "llm_prompt_stream",
            |caller: Caller<'_, StoreData>,
             (bit_ptr, bit_len, req_ptr, req_len): (u32, u32, u32, u32)| {
                Box::new(async move {
                    if !caller
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return 0u64;
                    }

                    let bit_json = match read_string_from_caller(&caller, bit_ptr, bit_len) {
                        Ok(s) => s,
                        Err(_) => return 0u64,
                    };
                    let request_json = match read_string_from_caller(&caller, req_ptr, req_len) {
                        Ok(s) => s,
                        Err(_) => return 0u64,
                    };

                    let bit: flow_like::bit::Bit = match serde_json::from_str(&bit_json) {
                        Ok(b) => b,
                        Err(e) => {
                            let err = serde_json::json!({"error": format!("Failed to parse model descriptor: {e}")}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };

                    let model_ctx = match &caller.data().host_state.model_context {
                        Some(c) => c,
                        None => {
                            let err = serde_json::json!({"error": "Model context not available"}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };
                    let app_state = model_ctx.app_state.clone();
                    let access_token = model_ctx.token.clone();

                    #[derive(serde::Deserialize)]
                    struct StreamRequest {
                        messages: Vec<serde_json::Value>,
                        #[serde(default)]
                        tools: Option<Vec<serde_json::Value>>,
                        #[serde(default)]
                        temperature: Option<f64>,
                        #[serde(default)]
                        max_tokens: Option<u64>,
                        #[serde(default)]
                        tool_choice: Option<serde_json::Value>,
                        #[serde(default)]
                        output_schema: Option<serde_json::Value>,
                        #[serde(default)]
                        additional_params: Option<serde_json::Value>,
                    }

                    let req: StreamRequest = match serde_json::from_str(&request_json) {
                        Ok(r) => r,
                        Err(e) => {
                            let err = serde_json::json!({"error": format!("Failed to parse request: {e}")}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };

                    // Convert messages → HistoryMessage (same logic as llm_prompt)
                    let mut history_messages = Vec::with_capacity(req.messages.len());
                    for msg in &req.messages {
                        let role_str = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                        let role = match role_str {
                            "system" => flow_like_model_provider::history::Role::System,
                            "assistant" => flow_like_model_provider::history::Role::Assistant,
                            "tool" => flow_like_model_provider::history::Role::Tool,
                            _ => flow_like_model_provider::history::Role::User,
                        };
                        let content = if let Some(c) = msg.get("content").and_then(|v| v.as_str()) {
                            flow_like_model_provider::history::MessageContent::String(c.to_string())
                        } else if let Some(parts) = msg.get("parts").and_then(|v| v.as_array()) {
                            let contents = parts.iter().filter_map(|part| {
                                part
                                    .get("text")
                                    .and_then(|t| t.as_str())
                                    .map(|text| text.to_string())
                                    .or_else(|| {
                                        part
                                            .get("reasoning")
                                            .and_then(|reasoning| reasoning.get("text"))
                                            .and_then(|text| text.as_array())
                                            .map(|entries| {
                                                entries
                                                    .iter()
                                                    .filter_map(|entry| entry.as_str())
                                                    .collect::<Vec<_>>()
                                                    .join("\n")
                                            })
                                    })
                                    .filter(|text| !text.is_empty())
                                    .map(|text| flow_like_model_provider::history::Content::Text {
                                        content_type: flow_like_model_provider::history::ContentType::Text,
                                        text,
                                    })
                            }).collect();
                            flow_like_model_provider::history::MessageContent::Contents(contents)
                        } else {
                            flow_like_model_provider::history::MessageContent::String(String::new())
                        };
                        let tool_calls = msg.get("tool_calls").and_then(|v| v.as_array()).map(|tcs| {
                            tcs.iter().filter_map(|tc| {
                                let id = tc.get("id")?.as_str()?.to_string();
                                let name = tc.get("name")?.as_str()?.to_string();
                                let args = tc.get("arguments").cloned().unwrap_or_default();
                                let args_str = if args.is_string() { args.as_str().unwrap_or("{}").to_string() } else { serde_json::to_string(&args).unwrap_or_default() };
                                Some(flow_like_model_provider::history::ToolCall { id, r#type: "function".to_string(), function: flow_like_model_provider::history::ToolCallFunction { name, arguments: args_str } })
                            }).collect()
                        });
                        let tool_call_id = msg.get("tool_call_id").and_then(|v| v.as_str()).map(|s| s.to_string());
                        history_messages.push(flow_like_model_provider::history::HistoryMessage { role, content, name: None, tool_calls, tool_call_id, annotations: None });
                    }

                    let mut history = flow_like_model_provider::history::History::new(bit.id.clone(), history_messages);
                    history.stream = Some(true);

                    if let Some(temp) = req.temperature { history.temperature = Some(temp as f32); }
                    if let Some(max) = req.max_tokens { history.max_completion_tokens = Some(max as u32); }
                    if let Some(tc_val) = req.tool_choice {
                        if let Ok(tc) = serde_json::from_value::<flow_like_model_provider::history::ToolChoice>(tc_val) {
                            history.tool_choice = Some(tc);
                        }
                    }
                    if let Some(schema) = req.output_schema {
                        history.response_format = Some(flow_like_model_provider::history::ResponseFormat::Object(
                            serde_json::json!({
                                "type": "json_schema",
                                "json_schema": {
                                    "name": schema.get("title").and_then(|t| t.as_str()).unwrap_or("response_schema"),
                                    "schema": schema,
                                    "strict": true
                                }
                            }),
                        ));
                    }

                    if let Some(tools) = req.tools {
                        let mut native_tools = Vec::new();
                        for t in &tools {
                            let name = match t.get("name").and_then(|n| n.as_str()) { Some(n) => n.to_string(), None => continue };
                            let desc = t.get("description").and_then(|d| d.as_str()).map(String::from);
                            let params = t.get("parameters").cloned().unwrap_or_default();
                            if let Ok(parsed) = serde_json::from_value::<flow_like_model_provider::history::HistoryFunctionParameters>(params) {
                                native_tools.push(flow_like_model_provider::history::Tool {
                                    tool_type: flow_like_model_provider::history::ToolType::Function,
                                    function: flow_like_model_provider::history::HistoryFunction { name, description: desc, parameters: parsed },
                                });
                            }
                        }
                        if !native_tools.is_empty() { history.tools = Some(native_tools); }
                    }

                    let model = {
                        let mut factory = app_state.model_factory.lock().await;
                        match factory.build(&bit, app_state.clone(), access_token.clone()).await {
                            Ok(m) => m,
                            Err(e) => {
                                let err = serde_json::json!({"error": format!("Failed to build model: {e}")}).to_string();
                                let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                                return pack_ptr_len(ptr, len);
                            }
                        }
                    };

                    // Create streaming callback that emits chunks as stream events
                    let stream_events: Arc<parking_lot::RwLock<Vec<crate::host_functions::StreamEvent>>> = Arc::new(parking_lot::RwLock::new(Vec::new()));
                    let stream_events_cb = stream_events.clone();
                    let callback: flow_like_model_provider::llm::LLMCallback = Arc::new(move |chunk: flow_like_model_provider::response_chunk::ResponseChunk| {
                        let events = stream_events_cb.clone();
                        Box::pin(async move {
                            if let Ok(chunk_json) = serde_json::to_value(&chunk) {
                                events.write().push(crate::host_functions::StreamEvent {
                                    event_type: "llm_chunk".to_string(),
                                    data: chunk_json,
                                });
                            }
                            Ok(())
                        })
                    });

                    let response = match model.invoke(&history, Some(callback)).await {
                        Ok(r) => r,
                        Err(e) => {
                            let err = serde_json::json!({"error": format!("Model invocation failed: {e}")}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };

                    // Move collected stream events into host_state so the runtime can poll them
                    {
                        let collected = std::mem::take(&mut *stream_events.write());
                        let mut host_events = caller.data().host_state.stream_events.write();
                        host_events.extend(collected);
                    }

                    let resp_msg = match response.last_message() {
                        Some(m) => m,
                        None => {
                            let err = serde_json::json!({"error": "Model returned empty response"}).to_string();
                            let (ptr, len) = caller.data().host_state.store_result(err.as_bytes());
                            return pack_ptr_len(ptr, len);
                        }
                    };

                    let tool_calls_json: Option<Vec<serde_json::Value>> = if resp_msg.tool_calls.is_empty() {
                        None
                    } else {
                        Some(resp_msg.tool_calls.iter().map(|tc| {
                            let args: serde_json::Value = serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::Value::Object(Default::default()));
                            serde_json::json!({"id": tc.id, "name": tc.function.name, "arguments": args})
                        }).collect())
                    };

                    let usage = serde_json::json!({
                        "prompt_tokens": response.usage.prompt_tokens,
                        "completion_tokens": response.usage.completion_tokens,
                        "total_tokens": response.usage.total_tokens,
                    });

                    let result = serde_json::json!({
                        "role": "assistant",
                        "content": resp_msg.content.clone().unwrap_or_default(),
                        "reasoning": resp_msg.reasoning.clone(),
                        "tool_calls": tool_calls_json,
                        "message_id": response.id,
                        "usage": usage,
                    });

                    let result_str = result.to_string();
                    let (ptr, len) = caller.data().host_state.store_result(result_str.as_bytes());
                    pack_ptr_len(ptr, len)
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register models.llm_prompt_stream: {}", e))
        })?;

    Ok(())
}

/// Read a string from WASM memory using caller context
fn read_string_from_caller(
    caller: &Caller<'_, StoreData>,
    ptr: u32,
    len: u32,
) -> Result<String, ()> {
    let memory = caller.data().memory.ok_or(())?;
    let data = memory.data(caller);

    let start = ptr as usize;
    let end = start.checked_add(len as usize).ok_or(())?;

    if end > data.len() {
        return Err(());
    }

    String::from_utf8(data[start..end].to_vec()).map_err(|_| ())
}

/// Read raw bytes from WASM memory using caller context
fn read_bytes_from_caller(
    caller: &Caller<'_, StoreData>,
    ptr: u32,
    len: u32,
) -> Result<Vec<u8>, ()> {
    let memory = caller.data().memory.ok_or(())?;
    let data = memory.data(caller);

    let start = ptr as usize;
    let end = start.checked_add(len as usize).ok_or(())?;

    if end > data.len() {
        return Err(());
    }

    Ok(data[start..end].to_vec())
}

/// Pack pointer and length into single u64 (ptr in high 32 bits, len in low 32 bits)
fn pack_ptr_len(ptr: u32, len: u32) -> u64 {
    ((ptr as u64) << 32) | (len as u64)
}

/// Register WASI snapshot_preview1 stubs for TinyGo/Go WASM modules
fn register_wasi_stubs(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "proc_exit",
            |_caller: Caller<'_, StoreData>, _code: i32| {},
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi proc_exit stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_write",
            |_caller: Caller<'_, StoreData>,
             _fd: i32,
             _iovs: i32,
             _iovs_len: i32,
             _nwritten: i32|
             -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_write stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_close",
            |_caller: Caller<'_, StoreData>, _fd: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_close stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_seek",
            |_caller: Caller<'_, StoreData>,
             _fd: i32,
             _offset: i64,
             _whence: i32,
             _newoffset: i32|
             -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_seek stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_fdstat_get",
            |_caller: Caller<'_, StoreData>, _fd: i32, _buf: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_fdstat_get stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_sizes_get",
            |_caller: Caller<'_, StoreData>, _count: i32, _buf_size: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi environ_sizes_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "environ_get",
            |_caller: Caller<'_, StoreData>, _environ: i32, _environ_buf: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi environ_get stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "args_sizes_get",
            |_caller: Caller<'_, StoreData>, _argc: i32, _argv_buf_size: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi args_sizes_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "args_get",
            |_caller: Caller<'_, StoreData>, _argv: i32, _argv_buf: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi args_get stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "clock_time_get",
            |_caller: Caller<'_, StoreData>, _clock_id: i32, _precision: i64, _time: i32| -> i32 {
                0
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi clock_time_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_read",
            |_caller: Caller<'_, StoreData>,
             _fd: i32,
             _iovs: i32,
             _iovs_len: i32,
             _nread: i32|
             -> i32 {
                0 // no data read
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi fd_read stub: {}", e))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "random_get",
            |_caller: Caller<'_, StoreData>, _buf: i32, _buf_len: i32| -> i32 { 0 },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi random_get stub: {}", e))
        })?;

    // fd_prestat_get / fd_prestat_dir_name — used by Swift/WASM to discover preopened dirs.
    // We have none, so return EBADF (8) immediately.
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_prestat_get",
            |_caller: Caller<'_, StoreData>, _fd: i32, _buf: i32| -> i32 {
                8 // WASI_EBADF — no preopened directories
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi fd_prestat_get stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "fd_prestat_dir_name",
            |_caller: Caller<'_, StoreData>, _fd: i32, _path: i32, _path_len: i32| -> i32 {
                8 // WASI_EBADF
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register wasi fd_prestat_dir_name stub: {}",
                e
            ))
        })?;

    // path_open — opens a file relative to a preopened directory; no filesystem in WASM sandbox.
    linker
        .func_wrap(
            "wasi_snapshot_preview1",
            "path_open",
            |_caller: Caller<'_, StoreData>,
             _dirfd: i32,
             _dirflags: i32,
             _path: i32,
             _path_len: i32,
             _oflags: i32,
             _fs_rights_base: i64,
             _fs_rights_inheriting: i64,
             _fdflags: i32,
             _opened_fd: i32|
             -> i32 { 28 }, // WASI_ENOSYS
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register wasi path_open stub: {}", e))
        })?;

    Ok(())
}

/// Register Emscripten stubs for C/C++ WASM modules
fn register_emscripten_stubs(linker: &mut Linker<StoreData>) -> WasmResult<()> {
    linker
        .func_wrap(
            "env",
            "emscripten_notify_memory_growth",
            |_caller: Caller<'_, StoreData>, _mem_index: i32| {},
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register emscripten_notify_memory_growth stub: {}",
                e
            ))
        })?;

    linker
        .func_wrap(
            "env",
            "__syscall_dup3",
            |_caller: Caller<'_, StoreData>, _old_fd: i32, _new_fd: i32, _flags: i32| -> i32 {
                -38 // ENOSYS — not supported in WASM sandbox
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!("Failed to register __syscall_dup3 stub: {}", e))
        })?;

    // Emscripten longjmp emulation for STANDALONE_WASM + SUPPORT_LONGJMP=emscripten.
    //
    // `_emscripten_throw_longjmp` signals a longjmp and traps to unwind back to
    // the nearest `invoke_vii` frame. `invoke_vii` catches the trap, restores the
    // Emscripten shadow stack, and calls `setThrew(1,0)` so Lua's setjmp handler
    // can detect the longjmp.
    linker
        .func_wrap_async(
            "env",
            "_emscripten_throw_longjmp",
            |mut caller: Caller<'_, StoreData>, _args: ()| {
                Box::new(async move {
                    caller.data_mut().longjmp_pending = true;
                    Result::<(), wasmtime::Error>::Err(wasmtime::Error::msg("__longjmp__"))
                })
            },
        )
        .map_err(|e| {
            WasmError::Initialization(format!(
                "Failed to register _emscripten_throw_longjmp: {}",
                e
            ))
        })?;

    linker
        .func_wrap_async(
            "env",
            "invoke_vii",
            |mut caller: Caller<'_, StoreData>, (func_idx, arg0, arg1): (i32, i32, i32)| {
                Box::new(async move {
                    // Save the Emscripten shadow stack pointer before the call.
                    let saved_sp: i32 = {
                        let get_sp = caller
                            .get_export("emscripten_stack_get_current")
                            .and_then(|e| e.into_func());
                        match get_sp {
                            Some(f) => {
                                let mut out = [Val::I32(0)];
                                let _ = f.call_async(&mut caller, &[], &mut out).await;
                                out[0].i32().unwrap_or(0)
                            }
                            None => 0,
                        }
                    };

                    // Look up the function in the indirect call table.
                    let func: Option<wasmtime::Func> = {
                        let table = caller
                            .get_export("__indirect_function_table")
                            .and_then(|e| e.into_table());
                        match table {
                            Some(t) => match t.get(&mut caller, func_idx as u64) {
                                Some(Ref::Func(Some(f))) => Some(f),
                                _ => None,
                            },
                            None => None,
                        }
                    };

                    let Some(func) = func else {
                        return Ok(());
                    };

                    let result = func
                        .call_async(&mut caller, &[Val::I32(arg0), Val::I32(arg1)], &mut [])
                        .await;

                    if result.is_err() && caller.data().longjmp_pending {
                        // Longjmp — restore shadow stack and set __THREW__.
                        caller.data_mut().longjmp_pending = false;

                        if let Some(restore) = caller
                            .get_export("_emscripten_stack_restore")
                            .and_then(|e| e.into_func())
                        {
                            let _ = restore
                                .call_async(&mut caller, &[Val::I32(saved_sp)], &mut [])
                                .await;
                        }

                        if let Some(set_threw) =
                            caller.get_export("setThrew").and_then(|e| e.into_func())
                        {
                            let _ = set_threw
                                .call_async(&mut caller, &[Val::I32(1), Val::I32(0)], &mut [])
                                .await;
                        }

                        return Ok(());
                    }

                    result
                })
            },
        )
        .map_err(|e| WasmError::Initialization(format!("Failed to register invoke_vii: {}", e)))?;

    Ok(())
}
