use crate::error::{WasmError, WasmResult};
use crate::host_functions::HostState;
use crate::limits::{WasmCapabilities, WasmSecurityConfig};
use futures::StreamExt;
use serde_json::Value;
use std::pin::Pin;
use wasmtime::component::Linker;
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpView};

pub struct ComponentStoreData {
    pub host_state: HostState,
    pub wasi_ctx: WasiCtx,
    pub http_ctx: WasiHttpCtx,
    pub resource_table: wasmtime::component::ResourceTable,
}

impl ComponentStoreData {
    pub fn new(security: &WasmSecurityConfig) -> Self {
        let mut builder = WasiCtxBuilder::new();

        let caps = security.capabilities;
        let has_network_caps = caps.intersects(WasmCapabilities::NETWORK_ALL);

        // Provide stdio/env/args so Component Model runtimes (C#, TypeScript)
        // that target wasi:cli/command can function correctly.
        builder.inherit_stdio();
        builder.inherit_env();
        builder.args(&["flow-like-wasm-node"]);

        if security.allow_wasi_network || has_network_caps {
            if let Some(ref hosts) = security.allowed_hosts {
                let allowed: std::collections::HashSet<String> = hosts.iter().cloned().collect();
                builder.socket_addr_check(move |addr, _use| {
                    let ip = addr.ip().to_string();
                    let allowed = allowed.clone();
                    Box::pin(async move { allowed.contains(&ip) })
                        as Pin<Box<dyn std::future::Future<Output = bool> + Send + Sync>>
                });
            } else {
                builder.inherit_network();
            }
        }

        // DNS lookups require explicit opt-in
        if security.allow_wasi_network
            || caps.intersects(WasmCapabilities::DNS)
            || caps.intersects(WasmCapabilities::NETWORK_ALL)
        {
            builder.allow_ip_name_lookup(true);
        }

        // TCP sockets
        if security.allow_wasi_network || caps.intersects(WasmCapabilities::TCP) {
            builder.allow_tcp(true);
        }

        // UDP sockets
        if security.allow_wasi_network || caps.intersects(WasmCapabilities::UDP) {
            builder.allow_udp(true);
        }

        Self {
            host_state: HostState::new(security.capabilities),
            wasi_ctx: builder.build(),
            http_ctx: WasiHttpCtx::new(),
            resource_table: wasmtime::component::ResourceTable::new(),
        }
    }
}

impl WasiView for ComponentStoreData {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.resource_table,
        }
    }
}

impl WasiHttpView for ComponentStoreData {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http_ctx
    }

    fn table(&mut self) -> &mut wasmtime::component::ResourceTable {
        &mut self.resource_table
    }
}

pub fn register_component_host_functions(
    linker: &mut Linker<ComponentStoreData>,
) -> WasmResult<()> {
    wasmtime_wasi::p2::add_to_linker_async(linker).map_err(|e| {
        WasmError::Initialization(format!("Failed to register WASI functions: {}", e))
    })?;
    wasmtime_wasi_http::add_only_http_to_linker_async(linker).map_err(|e| {
        WasmError::Initialization(format!("Failed to register WASI HTTP functions: {}", e))
    })?;
    register_logging(linker)?;
    register_pins(linker)?;
    register_variables(linker)?;
    register_cache(linker)?;
    register_streaming(linker)?;
    register_metadata(linker)?;
    register_storage(linker)?;
    register_models(linker)?;
    register_schema(linker)?;
    register_image(linker)?;
    register_db(linker)?;
    register_auth(linker)?;
    register_http(linker)?;
    register_websocket(linker)?;
    Ok(())
}

fn register_logging(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut logging = linker
        .instance("flow-like:node/logging@0.1.0")
        .map_err(map_err)?;

    logging
        .func_wrap(
            "log",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (level, message): (u8, String)| {
                store.data().host_state.log(level, message, None);
                Ok(())
            },
        )
        .map_err(map_err)?;

    Ok(())
}

fn register_pins(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut pins = linker
        .instance("flow-like:node/pins@0.1.0")
        .map_err(map_err)?;

    pins.func_wrap(
        "get-input",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (name,): (String,)| {
            let val = store.data().host_state.get_input(&name);
            Ok((val.and_then(|v| serde_json::to_string(&v).ok()),))
        },
    )
    .map_err(map_err)?;

    pins.func_wrap(
        "set-output",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (name, value): (String, String)| {
            if let Ok(parsed) = serde_json::from_str::<Value>(&value) {
                store.data().host_state.set_output(&name, parsed);
            }
            Ok(())
        },
    )
    .map_err(map_err)?;

    pins.func_wrap(
        "activate-exec",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (name,): (String,)| {
            store.data().host_state.activate_exec(&name);
            Ok(())
        },
    )
    .map_err(map_err)?;

    Ok(())
}

fn register_variables(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut vars = linker
        .instance("flow-like:node/variables@0.1.0")
        .map_err(map_err)?;

    vars.func_wrap(
        "get-var",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (name,): (String,)| {
            if !store
                .data()
                .host_state
                .has_capability(WasmCapabilities::VARIABLES_READ)
            {
                return Ok((None::<String>,));
            }
            let val = store.data().host_state.get_variable(&name);
            Ok((val.and_then(|v| serde_json::to_string(&v).ok()),))
        },
    )
    .map_err(map_err)?;

    vars.func_wrap(
        "set-var",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (name, value): (String, String)| {
            if !store
                .data()
                .host_state
                .has_capability(WasmCapabilities::VARIABLES_WRITE)
            {
                return Ok(());
            }
            if let Ok(parsed) = serde_json::from_str::<Value>(&value) {
                store.data().host_state.set_variable(&name, parsed);
            }
            Ok(())
        },
    )
    .map_err(map_err)?;

    vars.func_wrap(
        "delete-var",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (name,): (String,)| {
            if store
                .data()
                .host_state
                .has_capability(WasmCapabilities::VARIABLES_WRITE)
            {
                store.data().host_state.variables.write().remove(&name);
            }
            Ok(())
        },
    )
    .map_err(map_err)?;

    vars.func_wrap(
        "has-var",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (name,): (String,)| {
            if !store
                .data()
                .host_state
                .has_capability(WasmCapabilities::VARIABLES_READ)
            {
                return Ok((false,));
            }
            Ok((store.data().host_state.variables.read().contains_key(&name),))
        },
    )
    .map_err(map_err)?;

    Ok(())
}

fn register_streaming(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut stream = linker
        .instance("flow-like:node/streaming@0.1.0")
        .map_err(map_err)?;

    stream
        .func_wrap(
            "emit",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (event_type, data): (String, String)| {
                store.data().host_state.stream_event(&event_type, &data);
                Ok(())
            },
        )
        .map_err(map_err)?;

    stream
        .func_wrap(
            "text",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (content,): (String,)| {
                store
                    .data()
                    .host_state
                    .add_stream_event("text".to_string(), serde_json::json!(content));
                Ok(())
            },
        )
        .map_err(map_err)?;

    Ok(())
}

fn register_metadata(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut meta = linker
        .instance("flow-like:node/metadata@0.1.0")
        .map_err(map_err)?;

    meta.func_wrap(
        "get-node-id",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            Ok((store.data().host_state.metadata.node_id.clone(),))
        },
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "get-run-id",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            Ok((store.data().host_state.metadata.run_id.clone(),))
        },
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "get-app-id",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            Ok((store.data().host_state.metadata.app_id.clone(),))
        },
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "get-board-id",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            Ok((store.data().host_state.metadata.board_id.clone(),))
        },
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "get-user-id",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            Ok((store.data().host_state.metadata.user_id.clone(),))
        },
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "time-now",
        |_store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            Ok((now,))
        },
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "random",
        |_store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| Ok((rand_float(),)),
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "is-streaming",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            Ok((store.data().host_state.metadata.stream_state,))
        },
    )
    .map_err(map_err)?;

    meta.func_wrap(
        "get-log-level",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
            Ok((store.data().host_state.metadata.log_level,))
        },
    )
    .map_err(map_err)?;

    Ok(())
}

fn register_cache(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut cache = linker
        .instance("flow-like:node/cache@0.1.0")
        .map_err(map_err)?;

    cache
        .func_wrap(
            "cache-get",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (key,): (String,)| {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_READ)
                {
                    return Ok((None::<String>,));
                }
                let val = store.data().host_state.cache.read().get(&key).cloned();
                Ok((val.and_then(|v| serde_json::to_string(&v).ok()),))
            },
        )
        .map_err(map_err)?;

    cache
        .func_wrap(
            "cache-set",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (key, value): (String, String)| {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_WRITE)
                {
                    return Ok(());
                }
                if let Ok(parsed) = serde_json::from_str::<Value>(&value) {
                    store.data().host_state.cache.write().insert(key, parsed);
                }
                Ok(())
            },
        )
        .map_err(map_err)?;

    cache
        .func_wrap(
            "cache-delete",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (key,): (String,)| {
                if store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_WRITE)
                {
                    store.data().host_state.cache.write().remove(&key);
                }
                Ok(())
            },
        )
        .map_err(map_err)?;

    cache
        .func_wrap(
            "cache-has",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (key,): (String,)| {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::CACHE_READ)
                {
                    return Ok((false,));
                }
                Ok((store.data().host_state.cache.read().contains_key(&key),))
            },
        )
        .map_err(map_err)?;

    Ok(())
}

fn register_storage(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut storage = linker
        .instance("flow-like:node/storage@0.1.0")
        .map_err(map_err)?;

    storage
        .func_wrap(
            "storage-dir",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (node_scoped,): (bool,)| {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return Ok((None::<String>,));
                }
                Ok((storage_dir_json(
                    &store.data().host_state,
                    node_scoped,
                    "storage",
                ),))
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap(
            "upload-dir",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return Ok((None::<String>,));
                }
                let ctx = match &store.data().host_state.storage_context {
                    Some(c) => c,
                    None => return Ok((None::<String>,)),
                };
                let dir = ctx.get_upload_dir();
                let store_hash = format!("wasm_dirs__upload_{}", dir.as_ref());
                if ctx.resolve_store(&store_hash).is_none() {
                    if let Some(s) = ctx.stores.app_storage_store.clone() {
                        ctx.register_store(&store_hash, s);
                    }
                }
                let json = serde_json::json!({
                    "path": dir.as_ref(),
                    "store_ref": store_hash,
                    "cache_store_ref": null
                });
                Ok((Some(json.to_string()),))
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap(
            "cache-dir",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (node_scoped, user_scoped): (bool, bool)| {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return Ok((None::<String>,));
                }
                let ctx = match &store.data().host_state.storage_context {
                    Some(c) => c,
                    None => return Ok((None::<String>,)),
                };
                let dir = ctx.get_cache_dir(node_scoped, user_scoped);
                let store_hash = format!("wasm_dirs__cache_{}", dir.as_ref());
                if ctx.resolve_store(&store_hash).is_none() {
                    if let Some(s) = ctx.stores.temporary_store.clone() {
                        ctx.register_store(&store_hash, s);
                    }
                }
                let json = serde_json::json!({
                    "path": dir.as_ref(),
                    "store_ref": store_hash,
                    "cache_store_ref": null
                });
                Ok((Some(json.to_string()),))
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap(
            "user-dir",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (node_scoped,): (bool,)| {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::STORAGE_READ)
                {
                    return Ok((None::<String>,));
                }
                let ctx = match &store.data().host_state.storage_context {
                    Some(c) => c,
                    None => return Ok((None::<String>,)),
                };
                let dir = ctx.get_user_dir(node_scoped);
                let store_hash = format!("wasm_dirs__user_{}", dir.as_ref());
                if ctx.resolve_store(&store_hash).is_none() {
                    if let Some(s) = ctx.stores.user_store.clone() {
                        ctx.register_store(&store_hash, s);
                    }
                }
                let json = serde_json::json!({
                    "path": dir.as_ref(),
                    "store_ref": store_hash,
                    "cache_store_ref": null
                });
                Ok((Some(json.to_string()),))
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap_async(
            "read-file",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (flow_path_json,): (String,)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_READ)
                    {
                        return Ok((None::<Vec<u8>>,));
                    }
                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(_) => return Ok((None,)),
                    };
                    let ctx = match &store.data().host_state.storage_context {
                        Some(c) => c,
                        None => return Ok((None,)),
                    };
                    let obj_store = match ctx.resolve_store(&flow_path.store_ref) {
                        Some(s) => s,
                        None => return Ok((None,)),
                    };
                    let path = flow_like_storage::object_store::path::Path::from(flow_path.path);
                    match obj_store.as_generic().get(&path).await {
                        Ok(result) => match result.bytes().await {
                            Ok(bytes) => Ok((Some(bytes.to_vec()),)),
                            Err(_) => Ok((None,)),
                        },
                        Err(_) => Ok((None,)),
                    }
                })
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap_async(
            "write-file",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (flow_path_json, data): (String, Vec<u8>)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_WRITE)
                    {
                        return Ok((false,));
                    }
                    if data.len() > crate::host_functions::storage::MAX_STORAGE_FILE_SIZE {
                        return Ok((false,));
                    }
                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(_) => return Ok((false,)),
                    };
                    let ctx = match &store.data().host_state.storage_context {
                        Some(c) => c,
                        None => return Ok((false,)),
                    };
                    let obj_store = match ctx.resolve_store(&flow_path.store_ref) {
                        Some(s) => s,
                        None => return Ok((false,)),
                    };
                    let path = flow_like_storage::object_store::path::Path::from(flow_path.path);
                    let payload = flow_like_storage::object_store::PutPayload::from_bytes(
                        flow_like_types::Bytes::from(data),
                    );
                    match obj_store.as_generic().put(&path, payload).await {
                        Ok(_) => Ok((true,)),
                        Err(_) => Ok((false,)),
                    }
                })
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap_async(
            "list-files",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (flow_path_json,): (String,)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_READ)
                    {
                        return Ok((None::<String>,));
                    }
                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(_) => return Ok((None,)),
                    };
                    let ctx = match &store.data().host_state.storage_context {
                        Some(c) => c,
                        None => return Ok((None,)),
                    };
                    let obj_store = match ctx.resolve_store(&flow_path.store_ref) {
                        Some(s) => s,
                        None => return Ok((None,)),
                    };
                    use futures::StreamExt;
                    let prefix =
                        flow_like_storage::object_store::path::Path::from(flow_path.path.clone());
                    let entries: Vec<_> = obj_store
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
                    match serde_json::to_string(&entries) {
                        Ok(json) => Ok((Some(json),)),
                        Err(_) => Ok((None,)),
                    }
                })
            },
        )
        .map_err(map_err)?;

    Ok(())
}

fn register_models(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut models = linker
        .instance("flow-like:node/models@0.1.0")
        .map_err(map_err)?;

    models
        .func_wrap_async(
            "embed-text",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (bit_json, texts_json): (String, String)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return Ok((None::<String>,));
                    }
                    let bit: flow_like::bit::Bit = match serde_json::from_str(&bit_json) {
                        Ok(b) => b,
                        Err(_) => return Ok((None,)),
                    };
                    let texts: Vec<String> = match serde_json::from_str(&texts_json) {
                        Ok(t) => t,
                        Err(_) => return Ok((None,)),
                    };
                    let model_ctx = match &store.data().host_state.model_context {
                        Some(c) => c,
                        None => return Ok((None,)),
                    };
                    let app_state = model_ctx.app_state.clone();
                    #[cfg(feature = "model")]
                    {
                        let mut factory = app_state.embedding_factory.lock().await;
                        let model = match factory.build_text(&bit, app_state.clone()).await {
                            Ok(m) => m,
                            Err(_) => return Ok((None,)),
                        };
                        match model.text_embed_query(&texts).await {
                            Ok(embeddings) => match serde_json::to_string(&embeddings) {
                                Ok(json) => Ok((Some(json),)),
                                Err(_) => Ok((None,)),
                            },
                            Err(_) => Ok((None,)),
                        }
                    }
                    #[cfg(not(feature = "model"))]
                    {
                        let _ = (app_state, bit, texts);
                        Ok((None::<String>,))
                    }
                })
            },
        )
        .map_err(map_err)?;

    // embed-text-query — embed texts for retrieval queries
    models
        .func_wrap_async(
            "embed-text-query",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (model_json, texts_json): (String, String)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return Ok((None::<String>,));
                    }
                    let _ = (model_json, texts_json);
                    // Stub — embedding model resolution via CachedEmbeddingModel
                    Ok((None::<String>,))
                })
            },
        )
        .map_err(map_err)?;

    // embed-text-document — embed texts for document indexing
    models
        .func_wrap_async(
            "embed-text-document",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (model_json, texts_json): (String, String)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return Ok((None::<String>,));
                    }
                    let _ = (model_json, texts_json);
                    Ok((None::<String>,))
                })
            },
        )
        .map_err(map_err)?;

    // embed-image — embed an image via embedding model
    models
        .func_wrap_async(
            "embed-image",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (model_json, image_data): (String, Vec<u8>)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return Ok((None::<String>,));
                    }
                    let _ = (model_json, image_data);
                    Ok((None::<String>,))
                })
            },
        )
        .map_err(map_err)?;

    // llm-prompt — send prompt to LLM/VLM
    models
        .func_wrap_async(
            "llm-prompt",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (bit_json, messages_json, _do_stream): (String, String, bool)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        return Ok((None::<String>,));
                    }

                    let bit: flow_like::bit::Bit = match serde_json::from_str(&bit_json) {
                        Ok(b) => b,
                        Err(_) => return Ok((None,)),
                    };

                    let model_ctx = match &store.data().host_state.model_context {
                        Some(c) => c,
                        None => return Ok((None,)),
                    };
                    let app_state = model_ctx.app_state.clone();

                    // Parse messages_json: either a wrapper {messages, tools} or a plain array
                    #[derive(serde::Deserialize)]
                    struct LlmPromptRequest {
                        messages: Vec<Value>,
                        #[serde(default)]
                        tools: Option<Vec<Value>>,
                    }

                    let (raw_messages, raw_tools) =
                        match serde_json::from_str::<LlmPromptRequest>(&messages_json) {
                            Ok(req) => (req.messages, req.tools),
                            Err(_) => match serde_json::from_str::<Vec<Value>>(&messages_json) {
                                Ok(msgs) => (msgs, None),
                                Err(_) => return Ok((None,)),
                            },
                        };

                    // Convert WASM SDK messages → native HistoryMessage
                    let mut history_messages = Vec::with_capacity(raw_messages.len());
                    for msg in &raw_messages {
                        let role_str = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                        let role = match role_str {
                            "system" => flow_like_model_provider::history::Role::System,
                            "assistant" => flow_like_model_provider::history::Role::Assistant,
                            "tool" => flow_like_model_provider::history::Role::Tool,
                            _ => flow_like_model_provider::history::Role::User,
                        };

                        // Extract text content (handles both "content": "text" and "parts")
                        let content = if let Some(c) = msg.get("content").and_then(|v| v.as_str())
                        {
                            flow_like_model_provider::history::MessageContent::String(
                                c.to_string(),
                            )
                        } else if let Some(parts) = msg.get("parts").and_then(|v| v.as_array()) {
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
                                }
                            }
                            flow_like_model_provider::history::MessageContent::Contents(contents)
                        } else {
                            flow_like_model_provider::history::MessageContent::String(
                                String::new(),
                            )
                        };

                        // Extract tool calls (SDK format: {id, name, arguments})
                        let tool_calls = msg
                            .get("tool_calls")
                            .and_then(|v| v.as_array())
                            .map(|tcs| {
                                tcs.iter()
                                    .filter_map(|tc| {
                                        let id = tc.get("id")?.as_str()?.to_string();
                                        let name = tc.get("name")?.as_str()?.to_string();
                                        let args = tc.get("arguments").cloned().unwrap_or_default();
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

                    // Convert tool definitions if present
                    if let Some(tools) = raw_tools {
                        let native_tools: Vec<flow_like_model_provider::history::Tool> = tools
                            .iter()
                            .filter_map(|t| {
                                let name = t.get("name")?.as_str()?.to_string();
                                let desc =
                                    t.get("description").and_then(|d| d.as_str()).map(String::from);
                                let params = t.get("parameters").cloned().unwrap_or_default();
                                Some(flow_like_model_provider::history::Tool {
                                    tool_type: flow_like_model_provider::history::ToolType::Function,
                                    function: flow_like_model_provider::history::HistoryFunction {
                                        name,
                                        description: desc,
                                        parameters: serde_json::from_value(params).ok()?,
                                    },
                                })
                            })
                            .collect();
                        if !native_tools.is_empty() {
                            history.tools = Some(native_tools);
                        }
                    }

                    // Build model and invoke
                    let model = {
                        let mut factory = app_state.model_factory.lock().await;
                        match factory.build(&bit, app_state.clone(), None).await {
                            Ok(m) => m,
                            Err(e) => {
                                tracing::warn!("llm-prompt: failed to build model: {e}");
                                return Ok((None,));
                            }
                        }
                    };

                    let response = match model.invoke(&history, None).await {
                        Ok(r) => r,
                        Err(e) => {
                            tracing::warn!("llm-prompt: model invoke failed: {e}");
                            return Ok((None,));
                        }
                    };

                    // Convert response to SDK ChatMessage JSON
                    let resp_msg = match response.last_message() {
                        Some(m) => m,
                        None => return Ok((None,)),
                    };

                    let tool_calls_json: Option<Vec<Value>> =
                        if resp_msg.tool_calls.is_empty() {
                            None
                        } else {
                            Some(
                                resp_msg
                                    .tool_calls
                                    .iter()
                                    .map(|tc| {
                                        let args: Value =
                                            serde_json::from_str(&tc.function.arguments)
                                                .unwrap_or(Value::Object(Default::default()));
                                        serde_json::json!({
                                            "id": tc.id,
                                            "name": tc.function.name,
                                            "arguments": args,
                                        })
                                    })
                                    .collect(),
                            )
                        };

                    let result = serde_json::json!({
                        "role": "assistant",
                        "content": resp_msg.content.clone().unwrap_or_default(),
                        "tool_calls": tool_calls_json,
                    });

                    Ok((Some(result.to_string()),))
                })
            },
        )
        .map_err(map_err)?;

    Ok(())
}

fn register_schema(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut schema = linker
        .instance("flow-like:node/schema@0.1.0")
        .map_err(map_err)?;

    schema
        .func_wrap(
            "get-type-schema",
            |_store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (type_name,): (String,)| {
                use crate::host_functions::schema;
                Ok((schema::get_type_schema(&type_name).map(|s| s.to_string()),))
            },
        )
        .map_err(map_err)?;

    schema
        .func_wrap(
            "list-types",
            |_store: wasmtime::StoreContextMut<'_, ComponentStoreData>, ()| {
                use crate::host_functions::schema;
                let names = schema::list_type_names();
                match serde_json::to_string(&names) {
                    Ok(json) => Ok((Some(json),)),
                    Err(_) => Ok((None::<String>,)),
                }
            },
        )
        .map_err(map_err)?;

    Ok(())
}

fn register_image(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut img = linker
        .instance("flow-like:node/image@0.1.0")
        .map_err(map_err)?;

    img.func_wrap(
        "from-bytes",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (data, format): (Vec<u8>, String)| {
            if !store
                .data()
                .host_state
                .has_capability(WasmCapabilities::MODELS)
            {
                return Ok((None::<String>,));
            }
            let _ = (data, format);
            // Stub — image creation from bytes
            Ok((None::<String>,))
        },
    )
    .map_err(map_err)?;

    img.func_wrap(
        "to-bytes",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (image_ref, format): (String, String)| {
            if !store
                .data()
                .host_state
                .has_capability(WasmCapabilities::MODELS)
            {
                return Ok((None::<Vec<u8>>,));
            }
            let _ = (image_ref, format);
            // Stub — image to bytes conversion
            Ok((None::<Vec<u8>>,))
        },
    )
    .map_err(map_err)?;

    Ok(())
}

fn register_db(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut db = linker
        .instance("flow-like:node/db@0.1.0")
        .map_err(map_err)?;

    db.func_wrap_async(
        "query",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (op, connection_json, payload_json): (u32, String, String)| {
            Box::new(async move {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::MODELS)
                {
                    return Ok((None::<String>,));
                }
                let _ = (op, connection_json, payload_json);
                // Stub — DB operations via connection cache key
                Ok((None::<String>,))
            })
        },
    )
    .map_err(map_err)?;

    Ok(())
}

fn register_auth(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut auth = linker
        .instance("flow-like:node/auth@0.1.0")
        .map_err(map_err)?;

    auth.func_wrap(
        "get-oauth-token",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (provider,): (String,)| {
            if !store
                .data()
                .host_state
                .has_capability(WasmCapabilities::OAUTH_ACCESS)
            {
                return Ok((None::<String>,));
            }
            let tokens = store.data().host_state.oauth_tokens.read();
            match tokens.get(&provider) {
                Some(token) => {
                    let json = serde_json::json!({
                        "access_token": token.access_token,
                        "token_type": token.token_type,
                        "expires_at": token.expires_at,
                    });
                    Ok((Some(json.to_string()),))
                }
                None => Ok((None,)),
            }
        },
    )
    .map_err(map_err)?;

    auth.func_wrap(
        "has-oauth-token",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (provider,): (String,)| {
            if !store
                .data()
                .host_state
                .has_capability(WasmCapabilities::OAUTH_ACCESS)
            {
                return Ok((false,));
            }
            Ok((store
                .data()
                .host_state
                .oauth_tokens
                .read()
                .contains_key(&provider),))
        },
    )
    .map_err(map_err)?;

    Ok(())
}

fn register_http(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut http = linker
        .instance("flow-like:node/http@0.1.0")
        .map_err(map_err)?;

    http.func_wrap_async(
        "request",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (method, url, headers_json, body): (u8, String, String, Option<Vec<u8>>)| {
            Box::new(async move {
                let is_read = matches!(method, 0 | 5 | 6);
                let required = if is_read {
                    WasmCapabilities::HTTP_GET
                } else {
                    WasmCapabilities::HTTP_WRITE
                };
                if !store.data().host_state.has_capability(required) {
                    return Ok((None::<String>,));
                }

                let method_str = match method {
                    0 => reqwest::Method::GET,
                    1 => reqwest::Method::POST,
                    2 => reqwest::Method::PUT,
                    3 => reqwest::Method::DELETE,
                    4 => reqwest::Method::PATCH,
                    5 => reqwest::Method::HEAD,
                    6 => reqwest::Method::OPTIONS,
                    _ => return Ok((None,)),
                };

                let client = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(30))
                    .build();
                let client = match client {
                    Ok(c) => c,
                    Err(_) => return Ok((None,)),
                };

                let mut req = client.request(method_str, &url);

                if let Ok(hdrs) =
                    serde_json::from_str::<std::collections::HashMap<String, String>>(&headers_json)
                {
                    for (k, v) in hdrs {
                        req = req.header(&k, &v);
                    }
                }

                if let Some(b) = body {
                    req = req.body(b);
                }

                let resp = match req.send().await {
                    Ok(r) => r,
                    Err(_) => return Ok((None,)),
                };

                let status = resp.status().as_u16();
                let resp_headers: std::collections::HashMap<String, String> = resp
                    .headers()
                    .iter()
                    .filter_map(|(k, v)| {
                        v.to_str()
                            .ok()
                            .map(|s| (k.as_str().to_string(), s.to_string()))
                    })
                    .collect();
                let body_text = resp.text().await.unwrap_or_default();

                let result = serde_json::json!({
                    "status": status,
                    "headers": resp_headers,
                    "body": body_text,
                });
                Ok((Some(result.to_string()),))
            })
        },
    )
    .map_err(map_err)?;

    Ok(())
}

/// Helper for storage-dir: builds FlowPath JSON, registers the store.
fn storage_dir_json(host: &HostState, node_scoped: bool, dir_type: &str) -> Option<String> {
    let ctx = host.storage_context.as_ref()?;
    let dir = ctx.get_storage_dir(node_scoped);
    let store_hash = format!("wasm_dirs__{dir_type}_{}", dir.as_ref());
    if ctx.resolve_store(&store_hash).is_none() {
        let store = ctx.stores.app_storage_store.clone()?;
        ctx.register_store(&store_hash, store);
    }
    let json = serde_json::json!({
        "path": dir.as_ref(),
        "store_ref": store_hash,
        "cache_store_ref": null
    });
    Some(json.to_string())
}

/// Minimal FlowPath for component model — same shape as core module for JSON compatibility.
#[derive(serde::Serialize, serde::Deserialize)]
struct StorageFlowPath {
    path: String,
    store_ref: String,
    cache_store_ref: Option<String>,
}

fn register_websocket(linker: &mut Linker<ComponentStoreData>) -> WasmResult<()> {
    let mut ws = linker
        .instance("flow-like:node/websocket@0.1.0")
        .map_err(map_err)?;

    // connect(url, headers_json) -> Option<session_id>
    ws.func_wrap_async(
        "connect",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (url, headers_json): (String, String)| {
            Box::new(async move {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return Ok((None::<String>,));
                }

                let connect_result = tokio_tungstenite::connect_async(&url).await;
                let (ws_stream, _response) = match connect_result {
                    Ok(r) => r,
                    Err(_) => return Ok((None,)),
                };

                let (sink, stream) = futures::StreamExt::split(ws_stream);
                let session_id = format!("ws_{}", rand_float().to_bits());

                let conn = crate::host_functions::WsConnection { sink, stream };
                store
                    .data()
                    .host_state
                    .ws_connections
                    .lock()
                    .await
                    .insert(session_id.clone(), conn);

                let _ = headers_json; // reserved for future header injection
                Ok((Some(session_id),))
            })
        },
    )
    .map_err(map_err)?;

    // send(session_id, message, is_binary) -> bool
    ws.func_wrap_async(
        "send",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (session_id, message, is_binary): (String, Vec<u8>, bool)| {
            Box::new(async move {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return Ok((false,));
                }

                let connections = store.data().host_state.ws_connections.clone();
                let mut guard = connections.lock().await;
                let conn = match guard.get_mut(&session_id) {
                    Some(c) => c,
                    None => return Ok((false,)),
                };

                let msg = if is_binary {
                    tokio_tungstenite::tungstenite::Message::Binary(message.into())
                } else {
                    let text = String::from_utf8(message).unwrap_or_default();
                    tokio_tungstenite::tungstenite::Message::Text(text.into())
                };

                let sent = futures::SinkExt::send(&mut conn.sink, msg).await.is_ok();
                Ok((sent,))
            })
        },
    )
    .map_err(map_err)?;

    // receive(session_id, timeout_ms) -> Option<json_string>
    // Returns JSON: { "type": "text"|"binary"|"close", "data": "..." }
    ws.func_wrap_async(
        "receive",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
         (session_id, timeout_ms): (String, u32)| {
            Box::new(async move {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return Ok((None::<String>,));
                }

                let connections = store.data().host_state.ws_connections.clone();
                let mut guard = connections.lock().await;
                let conn = match guard.get_mut(&session_id) {
                    Some(c) => c,
                    None => return Ok((None,)),
                };

                let timeout = std::time::Duration::from_millis(timeout_ms as u64);
                let msg = tokio::time::timeout(timeout, conn.stream.next()).await;

                let msg = match msg {
                    Ok(Some(Ok(m))) => m,
                    _ => return Ok((None,)),
                };

                let result = match msg {
                    tokio_tungstenite::tungstenite::Message::Text(t) => {
                        serde_json::json!({ "type": "text", "data": t.to_string() })
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(b) => {
                        let encoded = base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &b,
                        );
                        serde_json::json!({ "type": "binary", "data": encoded })
                    }
                    tokio_tungstenite::tungstenite::Message::Close(frame) => {
                        let reason = frame
                            .map(|f| f.reason.to_string())
                            .unwrap_or_default();
                        serde_json::json!({ "type": "close", "data": reason })
                    }
                    tokio_tungstenite::tungstenite::Message::Ping(d) => {
                        serde_json::json!({ "type": "ping", "data": String::from_utf8_lossy(&d).to_string() })
                    }
                    tokio_tungstenite::tungstenite::Message::Pong(d) => {
                        serde_json::json!({ "type": "pong", "data": String::from_utf8_lossy(&d).to_string() })
                    }
                    _ => return Ok((None,)),
                };
                Ok((Some(result.to_string()),))
            })
        },
    )
    .map_err(map_err)?;

    // close(session_id) -> bool
    ws.func_wrap_async(
        "close",
        |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (session_id,): (String,)| {
            Box::new(async move {
                if !store
                    .data()
                    .host_state
                    .has_capability(WasmCapabilities::WEBSOCKET)
                {
                    return Ok((false,));
                }

                let connections = store.data().host_state.ws_connections.clone();
                let mut guard = connections.lock().await;
                let conn = match guard.remove(&session_id) {
                    Some(c) => c,
                    None => return Ok((false,)),
                };

                let mut sink = conn.sink;
                let _ = futures::SinkExt::close(&mut sink).await;
                Ok((true,))
            })
        },
    )
    .map_err(map_err)?;

    Ok(())
}

fn map_err(e: impl std::fmt::Display) -> WasmError {
    WasmError::Initialization(format!("Failed to register component host function: {}", e))
}

fn rand_float() -> f64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let s = RandomState::new();
    let mut hasher = s.build_hasher();
    hasher.write_u64(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64,
    );
    (hasher.finish() as f64) / (u64::MAX as f64)
}
