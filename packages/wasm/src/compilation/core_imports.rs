use super::MetadataState;
use crate::error::{WasmError, WasmResult};
use wasmtime::{Caller, Linker, Ref, Val, ValType};
use ValType::{I32, I64};

// The core SDK uses integer pointers and handles. Empty handles and failed writes
// preserve that ABI while granting no capability during metadata extraction.
pub(super) const IMPORTS: &[(&str, &str, &[ValType], Option<ValType>, i64)] = &[
    ("env", "abort", &[I32, I32, I32, I32], None, 0),
    ("env", "host_log", &[I32, I32, I32], None, 0),
    ("env", "host_stream", &[I32, I32, I32, I32], None, 0),
    ("env", "host_get_variable", &[I32, I32], Some(I64), 0),
    (
        "env",
        "host_set_variable",
        &[I32, I32, I32, I32],
        Some(I32),
        -1,
    ),
    ("env", "host_time_now", &[], Some(I64), 0),
    ("env", "host_random", &[], Some(I64), 0),
    ("flowlike_log", "trace", &[I32, I32], None, 0),
    ("flowlike_log", "debug", &[I32, I32], None, 0),
    ("flowlike_log", "info", &[I32, I32], None, 0),
    ("flowlike_log", "warn", &[I32, I32], None, 0),
    ("flowlike_log", "error", &[I32, I32], None, 0),
    (
        "flowlike_log",
        "log_json",
        &[I32, I32, I32, I32, I32],
        None,
        0,
    ),
    ("flowlike_pins", "get_input", &[I32, I32], Some(I64), 0),
    (
        "flowlike_pins",
        "set_output",
        &[I32, I32, I32, I32],
        None,
        0,
    ),
    ("flowlike_pins", "activate_exec", &[I32, I32], None, 0),
    ("flowlike_vars", "get", &[I32, I32], Some(I64), 0),
    ("flowlike_vars", "set", &[I32, I32, I32, I32], None, 0),
    ("flowlike_vars", "delete", &[I32, I32], None, 0),
    ("flowlike_vars", "has", &[I32, I32], Some(I32), 0),
    ("flowlike_cache", "get", &[I32, I32], Some(I64), 0),
    ("flowlike_cache", "set", &[I32, I32, I32, I32], None, 0),
    ("flowlike_cache", "delete", &[I32, I32], None, 0),
    ("flowlike_cache", "has", &[I32, I32], Some(I32), 0),
    ("flowlike_meta", "new_resource_handle", &[], Some(I64), 0),
    ("flowlike_meta", "get_node_id", &[], Some(I64), 0),
    ("flowlike_meta", "get_run_id", &[], Some(I64), 0),
    ("flowlike_meta", "get_app_id", &[], Some(I64), 0),
    ("flowlike_meta", "get_board_id", &[], Some(I64), 0),
    ("flowlike_meta", "get_user_id", &[], Some(I64), 0),
    ("flowlike_meta", "is_streaming", &[], Some(I32), 0),
    ("flowlike_meta", "get_log_level", &[], Some(I32), 0),
    ("flowlike_meta", "time_now", &[], Some(I64), 0),
    ("flowlike_meta", "random", &[], Some(I64), 0),
    ("flowlike_storage", "storage_dir", &[I32], Some(I64), 0),
    ("flowlike_storage", "upload_dir", &[], Some(I64), 0),
    ("flowlike_storage", "cache_dir", &[I32, I32], Some(I64), 0),
    ("flowlike_storage", "user_dir", &[I32], Some(I64), 0),
    (
        "flowlike_storage",
        "read_request",
        &[I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_storage",
        "copy_request",
        &[I32, I32, I32, I32],
        Some(I32),
        -1,
    ),
    (
        "flowlike_storage",
        "file_url_request",
        &[I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_storage",
        "write_request",
        &[I32, I32, I32, I32],
        Some(I32),
        -1,
    ),
    (
        "flowlike_storage",
        "write_start_request",
        &[I32, I32, I64],
        Some(I64),
        0,
    ),
    (
        "flowlike_storage",
        "write_chunk_request",
        &[I32, I32, I32, I32],
        Some(I32),
        -1,
    ),
    (
        "flowlike_storage",
        "write_finish_request",
        &[I32, I32],
        Some(I32),
        -1,
    ),
    (
        "flowlike_storage",
        "list_request",
        &[I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_http",
        "request",
        &[I32, I32, I32, I32, I32, I32, I32],
        Some(I32),
        -1,
    ),
    (
        "flowlike_ws",
        "connect_ref",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_ws",
        "send_ref",
        &[I32, I32, I32, I32, I32],
        Some(I32),
        0,
    ),
    ("flowlike_ws", "receive_ref", &[I32, I32, I32], Some(I64), 0),
    ("flowlike_ws", "close_ref", &[I32, I32], Some(I32), 0),
    (
        "flowlike_ws",
        "connect",
        &[I32, I32, I32, I32],
        Some(I32),
        -1,
    ),
    ("flowlike_ws", "send", &[I32, I32, I32, I32], Some(I32), -1),
    ("flowlike_ws", "receive", &[I32, I32], Some(I32), -1),
    ("flowlike_ws", "close", &[I32], Some(I32), -1),
    ("flowlike_stream", "emit", &[I32, I32, I32, I32], None, 0),
    ("flowlike_stream", "text", &[I32, I32], None, 0),
    (
        "flowlike_auth",
        "get_oauth_token",
        &[I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_auth",
        "has_oauth_token",
        &[I32, I32],
        Some(I32),
        0,
    ),
    (
        "flowlike_models",
        "embed_text",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_schema",
        "get_type_schema",
        &[I32, I32],
        Some(I64),
        0,
    ),
    ("flowlike_schema", "list_types", &[], Some(I64), 0),
    (
        "flowlike_image",
        "from_bytes",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_image",
        "to_bytes",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_db",
        "query",
        &[I32, I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_models",
        "embed_text_query",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_models",
        "embed_text_document",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_models",
        "embed_image",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_models",
        "llm_prompt",
        &[I32, I32, I32, I32, I32],
        Some(I64),
        0,
    ),
    (
        "flowlike_models",
        "llm_prompt_stream",
        &[I32, I32, I32, I32],
        Some(I64),
        0,
    ),
];

pub(super) fn register(linker: &mut Linker<MetadataState>) -> WasmResult<()> {
    for (module, name, params, result, denied) in IMPORTS {
        if (*module == "env"
            && matches!(
                *name,
                "host_get_variable" | "host_set_variable" | "host_time_now" | "host_random"
            ))
            || (*module == "flowlike_meta" && matches!(*name, "time_now" | "random"))
            || (*module == "flowlike_schema" && *name == "list_types")
        {
            continue;
        }
        let ty = wasmtime::FuncType::new(linker.engine(), params.iter().cloned(), result.clone());
        let denied = *denied;
        let result = result.clone();
        linker
            .func_new(module, name, ty, move |_, _, results| {
                if let Some(ty) = &result {
                    results[0] = match ty {
                        ValType::I32 => Val::I32(denied as i32),
                        ValType::I64 => Val::I64(denied),
                        _ => unreachable!(),
                    };
                }
                Ok(())
            })
            .map_err(|error| WasmError::Initialization(error.to_string()))?;
    }
    const DENIED: &[u8] = br#"{"ok":false,"error":{"code":"permission_denied","message":"Storage is unavailable during metadata extraction.","retryable":false}}"#;
    linker
        .func_wrap("flowlike_storage_ops", "request", |_: i32, _: i32| -> i64 {
            DENIED.len() as i64
        })
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    linker
        .func_wrap(
            "flowlike_storage_ops",
            "read_response",
            |mut caller: Caller<'_, MetadataState>, offset: u32, ptr: u32, length: u32| -> i32 {
                let Some(bytes) = (offset as usize)
                    .checked_add(length as usize)
                    .and_then(|end| DENIED.get(offset as usize..end))
                else {
                    return -1;
                };
                let Some(memory) = caller
                    .get_export("memory")
                    .and_then(|export| export.into_memory())
                else {
                    return -1;
                };
                if memory.write(&mut caller, ptr as usize, bytes).is_err() {
                    return -1;
                }
                i32::try_from(length).unwrap_or(-1)
            },
        )
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    register_legacy_variables(linker)?;
    register_observations(linker)?;
    linker
        .func_wrap(
            "flowlike_schema",
            "list_types",
            |mut caller: Caller<'_, MetadataState>| -> u64 {
                caller.data_mut().store_result(b"[]")
            },
        )
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    register_emscripten_stubs(linker)
}

fn register_observations(linker: &mut Linker<MetadataState>) -> WasmResult<()> {
    for (module, name) in [("env", "host_time_now"), ("flowlike_meta", "time_now")] {
        linker
            .func_wrap(module, name, || super::observations::time_millis() as i64)
            .map_err(|error| WasmError::Initialization(error.to_string()))?;
    }
    linker
        .func_wrap("env", "host_random", super::observations::legacy_random)
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    linker
        .func_wrap("flowlike_meta", "random", super::observations::core_random)
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    Ok(())
}

fn read_string(caller: &mut Caller<'_, MetadataState>, ptr: u32, length: u32) -> Option<String> {
    let memory = caller.get_export("memory")?.into_memory()?;
    let end = (ptr as usize).checked_add(length as usize)?;
    std::str::from_utf8(memory.data(caller).get(ptr as usize..end)?)
        .ok()
        .map(str::to_owned)
}

fn register_legacy_variables(linker: &mut Linker<MetadataState>) -> WasmResult<()> {
    // The AssemblyScript ABI historically exposes a store-local scratch map even
    // when no flow-variable capability is granted. It never receives app state.
    linker
        .func_wrap(
            "env",
            "host_set_variable",
            |mut caller: Caller<'_, MetadataState>,
             name: u32,
             name_len: u32,
             value: u32,
             value_len: u32|
             -> i32 {
                let Some(name) = read_string(&mut caller, name, name_len) else {
                    return -1;
                };
                let Some(value) = read_string(&mut caller, value, value_len) else {
                    return -1;
                };
                let value = serde_json::from_str(&value).unwrap_or(serde_json::Value::Null);
                caller.data_mut().variables.insert(name, value);
                0
            },
        )
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    linker
        .func_wrap(
            "env",
            "host_get_variable",
            |mut caller: Caller<'_, MetadataState>, name: u32, length: u32| -> u64 {
                let Some(name) = read_string(&mut caller, name, length) else {
                    return 0;
                };
                let Some(value) = caller.data().variables.get(&name) else {
                    return 0;
                };
                let Ok(bytes) = serde_json::to_vec(value) else {
                    return 0;
                };
                caller.data_mut().store_result(&bytes)
            },
        )
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    Ok(())
}

fn register_emscripten_stubs(linker: &mut Linker<MetadataState>) -> WasmResult<()> {
    linker
        .func_wrap(
            "env",
            "emscripten_notify_memory_growth",
            |_caller: Caller<'_, MetadataState>, _mem_index: i32| {},
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
            |_caller: Caller<'_, MetadataState>, _old_fd: i32, _new_fd: i32, _flags: i32| -> i32 {
                -38 // ENOSYS, not supported in WASM sandbox
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
            |mut caller: Caller<'_, MetadataState>, _args: ()| {
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
            |mut caller: Caller<'_, MetadataState>, (func_idx, arg0, arg1): (i32, i32, i32)| {
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
                        // Restore the shadow stack and set __THREW__ after longjmp.
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
