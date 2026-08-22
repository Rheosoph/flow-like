use crate::error::{WasmError, WasmResult};
use crate::host_functions::HostState;
use crate::limits::{WasmCapabilities, WasmSecurityConfig};
use crate::llm_message::sdk_message_content;
use crate::wasi::{isolated_wasi_ctx_builder, IsolatedWasiCtxBuilder};
use futures::StreamExt;
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use wasmtime::component::Linker;
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::p2::{WasiHttpCtxView, WasiHttpView};
use wasmtime_wasi_http::WasiHttpCtx;

pub struct ComponentStoreData {
    pub host_state: HostState,
    pub wasi_ctx: WasiCtx,
    pub http_ctx: WasiHttpCtx,
    pub resource_table: wasmtime::component::ResourceTable,
    /// The node's own execution budget; outbound requests are bounded by it
    /// rather than by a fixed per-request timeout.
    pub node_timeout: std::time::Duration,
    /// Where the enclosing flow runs, stamped from the `WasmSecurityConfig` the
    /// node built for this execution.
    ///
    /// This is the *only* source the guest network paths key off. The same value
    /// also reaches the guest through `HostState::metadata`, but that struct
    /// derives `Default` — and the default is the permissive `Local` — so a path
    /// that read it before the node populated it would have disabled the egress
    /// policy by omission rather than failing closed.
    pub environment: flow_like::flow::execution::ExecutionEnvironment,
    http_hooks: EgressHttpHooks,
}

/// `wasi:http` hooks that apply the server-side egress policy to the
/// standard `outgoing-handler` path (exposed to packages with `HTTP_ALL`),
/// which otherwise bypasses the `flow-like:node/http` host function entirely.
///
/// The name and every resolved address are vetted before delegating to
/// wasmtime's default handler, which then re-resolves for the actual connect —
/// a narrower guarantee than the reqwest path (whose resolver result *is* the
/// connect target), but sufficient against the fixed metadata / loopback
/// destinations this policy exists for.
#[derive(Default)]
struct EgressHttpHooks {
    environment: flow_like::flow::execution::ExecutionEnvironment,
}

impl wasmtime_wasi_http::p2::WasiHttpHooks for EgressHttpHooks {
    fn send_request(
        &mut self,
        request: hyper::Request<wasmtime_wasi_http::p2::body::HyperOutgoingBody>,
        config: wasmtime_wasi_http::p2::types::OutgoingRequestConfig,
    ) -> wasmtime_wasi_http::p2::HttpResult<wasmtime_wasi_http::p2::types::HostFutureIncomingResponse>
    {
        use wasmtime_wasi_http::p2::bindings::http::types::ErrorCode;

        let environment = self.environment;
        let handle = wasmtime_wasi::runtime::spawn(async move {
            let Some(authority) = request.uri().authority().cloned() else {
                return Ok(Err(ErrorCode::HttpRequestUriInvalid));
            };
            let host = authority
                .host()
                .trim_matches(|c| c == '[' || c == ']')
                .to_string();
            let port = authority
                .port_u16()
                .unwrap_or(if config.use_tls { 443 } else { 80 });
            if let Err(e) =
                flow_like::flow::execution::egress::resolve_socket_addrs(environment, &host, port)
                    .await
            {
                tracing::warn!("WASI HTTP request to {} refused: {}", authority, e);
                return Ok(Err(ErrorCode::HttpRequestDenied));
            }
            Ok(wasmtime_wasi_http::p2::default_send_request_handler(request, config).await)
        });
        Ok(wasmtime_wasi_http::p2::types::HostFutureIncomingResponse::pending(handle))
    }
}

pub(super) fn configure_guest_network(
    builder: &mut IsolatedWasiCtxBuilder,
    security: &WasmSecurityConfig,
) {
    let caps = security.capabilities;
    let has_socket_caps =
        caps.intersects(WasmCapabilities::TCP | WasmCapabilities::UDP | WasmCapabilities::DNS);

    if security.allow_wasi_network || has_socket_caps {
        // Server-side, raw wasi:sockets must not reach the host plane either
        // (a guest can speak HTTP to 169.254.169.254 over a plain TCP socket);
        // the same address predicate as every other outbound path applies on
        // top of the package's own host allowlist.
        let guarded = security.execution_environment
            == flow_like::flow::execution::ExecutionEnvironment::Server;
        let allowed: Option<std::collections::HashSet<String>> = security
            .allowed_hosts
            .as_ref()
            .map(|hosts| hosts.iter().cloned().collect());
        builder.socket_addr_check(move |addr, _use| {
            let ip = addr.ip();
            let permitted = allowed
                .as_ref()
                .is_none_or(|allowed| allowed.contains(&ip.to_string()))
                && !(guarded && flow_like::flow::execution::egress::is_blocked_ip(ip));
            Box::pin(async move { permitted })
                as Pin<Box<dyn std::future::Future<Output = bool> + Send + Sync>>
        });
    }

    // Wasmtime enables TCP and UDP protocol use by default (while denying all
    // addresses). Once an address policy is opened above, absent protocols
    // must therefore be disabled explicitly to preserve capability precision.
    builder
        .allow_ip_name_lookup(security.allow_wasi_network || caps.intersects(WasmCapabilities::DNS))
        .allow_tcp(security.allow_wasi_network || caps.intersects(WasmCapabilities::TCP))
        .allow_udp(security.allow_wasi_network || caps.intersects(WasmCapabilities::UDP));
}

impl ComponentStoreData {
    pub fn new(security: &WasmSecurityConfig) -> Self {
        let mut builder = isolated_wasi_ctx_builder();

        // Provide output streams and args so Component Model runtimes (C#,
        // TypeScript) that target wasi:cli/command can function correctly.
        // Stdin stays closed and the guest environment remains empty.
        builder.inherit_output();
        configure_guest_network(&mut builder, security);
        builder.args(&["flow-like-wasm-node"]);
        if security.deterministic {
            builder.make_deterministic();
        }

        Self::with_host_state(
            HostState::new(security.capabilities),
            builder.build(),
            security,
        )
    }

    /// Store data around an already-populated host state (child stores such
    /// as CLI subprocesses), sharing the node's budget and egress policy.
    pub fn with_host_state(
        host_state: HostState,
        wasi_ctx: WasiCtx,
        security: &WasmSecurityConfig,
    ) -> Self {
        Self {
            host_state,
            wasi_ctx,
            http_ctx: WasiHttpCtx::new(),
            resource_table: wasmtime::component::ResourceTable::new(),
            node_timeout: security.limits.timeout,
            environment: security.execution_environment,
            http_hooks: EgressHttpHooks {
                environment: security.execution_environment,
            },
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
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        // The hooks were stamped from the security config at construction and
        // are never re-derived here: reading `host_state.metadata` would take a
        // `Default`-derived field whose default is the permissive `Local`.
        WasiHttpCtxView {
            ctx: &mut self.http_ctx,
            table: &mut self.resource_table,
            hooks: &mut self.http_hooks,
        }
    }
}

fn allows_standard_wasi_http(security: &WasmSecurityConfig) -> bool {
    // Wasmtime's standard wasi:http implementation cannot enforce Flow-Like's
    // per-method capabilities or host allowlist. Only expose it when every HTTP
    // method is granted and no host restriction needs to be enforced.
    security.capabilities.contains(WasmCapabilities::HTTP_ALL) && security.allowed_hosts.is_none()
}

pub fn register_component_host_functions(
    linker: &mut Linker<ComponentStoreData>,
    security: &WasmSecurityConfig,
) -> WasmResult<()> {
    wasmtime_wasi::p2::add_to_linker_async(linker).map_err(|e| {
        WasmError::Initialization(format!("Failed to register WASI functions: {}", e))
    })?;
    if allows_standard_wasi_http(security) {
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(linker).map_err(|e| {
            WasmError::Initialization(format!("Failed to register WASI HTTP functions: {}", e))
        })?;
    }
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
                    "storage",
                    |ctx| ctx.get_storage_dir(node_scoped),
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
                Ok((storage_dir_json(
                    &store.data().host_state,
                    "upload",
                    |ctx| ctx.get_upload_dir(),
                ),))
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
                Ok((storage_dir_json(&store.data().host_state, "cache", |ctx| {
                    ctx.get_cache_dir(node_scoped, user_scoped)
                }),))
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
                Ok((storage_dir_json(&store.data().host_state, "user", |ctx| {
                    ctx.get_user_dir(node_scoped)
                }),))
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
                        tracing::warn!("[wasm write-file] rejected: no STORAGE_WRITE capability");
                        return Ok((false,));
                    }
                    if data.len() > crate::host_functions::storage::MAX_STORAGE_FILE_SIZE {
                        tracing::warn!(
                            "[wasm write-file] rejected: data too large ({})",
                            data.len()
                        );
                        return Ok((false,));
                    }
                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!(
                                "[wasm write-file] rejected: bad JSON {e}: {flow_path_json}"
                            );
                            return Ok((false,));
                        }
                    };
                    let ctx = match &store.data().host_state.storage_context {
                        Some(c) => c,
                        None => {
                            tracing::warn!("[wasm write-file] rejected: no storage context");
                            return Ok((false,));
                        }
                    };
                    Ok((crate::host_functions::storage::put_flow_path(
                        ctx,
                        &flow_path,
                        data,
                        "wasm write-file",
                    )
                    .await,))
                })
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap_async(
            "write-file-start",
            |mut store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (flow_path_json, total_size): (String, u64)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_WRITE)
                    {
                        tracing::warn!(
                            "[wasm write-file-start] rejected: no STORAGE_WRITE capability"
                        );
                        return Ok((None::<String>,));
                    }
                    let flow_path: StorageFlowPath = match serde_json::from_str(&flow_path_json) {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::warn!(
                                "[wasm write-file-start] rejected: bad JSON {e}: {flow_path_json}"
                            );
                            return Ok((None,));
                        }
                    };
                    let result = crate::host_functions::storage::start_write(
                        &mut store.data_mut().host_state.pending_writes.write(),
                        flow_path,
                        total_size,
                    );
                    Ok((result,))
                })
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap_async(
            "write-file-chunk",
            |mut store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (write_id, data): (String, Vec<u8>)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_WRITE)
                    {
                        return Ok((false,));
                    }
                    let ok = crate::host_functions::storage::append_chunk(
                        &mut store.data_mut().host_state.pending_writes.write(),
                        &write_id,
                        &data,
                    );
                    Ok((ok,))
                })
            },
        )
        .map_err(map_err)?;

    storage
        .func_wrap_async(
            "write-file-finish",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>, (write_id,): (String,)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::STORAGE_WRITE)
                    {
                        return Ok((false,));
                    }
                    let pw = store
                        .data()
                        .host_state
                        .pending_writes
                        .write()
                        .remove(&write_id);
                    let Some(pw) = pw else {
                        tracing::warn!(
                            "[wasm write-file-finish] rejected: unknown write_id {write_id}"
                        );
                        return Ok((false,));
                    };
                    let ctx = match &store.data().host_state.storage_context {
                        Some(c) => c,
                        None => {
                            tracing::warn!("[wasm write-file-finish] rejected: no storage context");
                            return Ok((false,));
                        }
                    };
                    Ok((crate::host_functions::storage::put_flow_path(
                        ctx,
                        &pw.flow_path,
                        pw.buffer,
                        "wasm write-file-finish",
                    )
                    .await,))
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
                    let access_token = model_ctx.token.clone();
                    let usage_context = store.data().host_state.model_usage_context.clone();
                    #[cfg(feature = "model")]
                    {
                        let mut factory = app_state.embedding_factory.lock().await;
                        let embedding_provider = bit.try_to_embedding();
                        let use_proxy = access_token.is_some()
                            && !flow_like::models::embedding_factory::prefers_local_execution(&bit)
                            && embedding_provider
                                .as_ref()
                                .is_some_and(|provider| provider.supports_remote());
                        let model_result = if use_proxy {
                            factory
                                .build_text_proxy(
                                    &bit,
                                    access_token.expect("proxy mode requires an access token"),
                                    usage_context,
                                )
                                .await
                        } else {
                            factory.build_text(&bit, app_state.clone()).await
                        };
                        let model = match model_result {
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
                        let _ = (app_state, access_token, usage_context, bit, texts);
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
                    #[cfg(feature = "model")]
                    {
                        let model_ctx = match store.data().host_state.model_context.clone() {
                            Some(context) => context,
                            None => return Ok((None,)),
                        };
                        let texts: Vec<String> = match serde_json::from_str(&texts_json) {
                            Ok(texts) => texts,
                            Err(_) => return Ok((None,)),
                        };
                        let model =
                            match crate::host_functions::resolve_cached_text_embedding_model(
                                &model_ctx,
                                &model_json,
                            )
                            .await
                            {
                                Some(model) => model,
                                None => return Ok((None,)),
                            };
                        match model.text_embed_query(&texts).await {
                            Ok(embeddings) => Ok((serde_json::to_string(&embeddings).ok(),)),
                            Err(_) => Ok((None,)),
                        }
                    }
                    #[cfg(not(feature = "model"))]
                    {
                        let _ = (model_json, texts_json);
                        Ok((None::<String>,))
                    }
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
                    #[cfg(feature = "model")]
                    {
                        let model_ctx = match store.data().host_state.model_context.clone() {
                            Some(context) => context,
                            None => return Ok((None,)),
                        };
                        let texts: Vec<String> = match serde_json::from_str(&texts_json) {
                            Ok(texts) => texts,
                            Err(_) => return Ok((None,)),
                        };
                        let model =
                            match crate::host_functions::resolve_cached_text_embedding_model(
                                &model_ctx,
                                &model_json,
                            )
                            .await
                            {
                                Some(model) => model,
                                None => return Ok((None,)),
                            };
                        match model.text_embed_document(&texts).await {
                            Ok(embeddings) => Ok((serde_json::to_string(&embeddings).ok(),)),
                            Err(_) => Ok((None,)),
                        }
                    }
                    #[cfg(not(feature = "model"))]
                    {
                        let _ = (model_json, texts_json);
                        Ok((None::<String>,))
                    }
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
             (bit_json, messages_json, do_stream): (String, String, bool)| {
                Box::new(async move {
                    if !store
                        .data()
                        .host_state
                        .has_capability(WasmCapabilities::MODELS)
                    {
                        println!("llm-prompt: MODELS capability not granted");
                        return Ok((None::<String>,));
                    }

                    let bit: flow_like::bit::Bit = match serde_json::from_str(&bit_json) {
                        Ok(b) => b,
                        Err(e) => {
                            println!("llm-prompt: failed to parse bit JSON: {e}");
                            let err = serde_json::json!({"error": format!("Failed to parse model descriptor: {e}")}).to_string();
                            return Ok((Some(err),));
                        }
                    };

                    let model_ctx = match &store.data().host_state.model_context {
                        Some(c) => c,
                        None => {
                            println!("llm-prompt: model_context is None");
                            let err = serde_json::json!({"error": "Model context not available — ensure the node has Models permission"}).to_string();
                            return Ok((Some(err),));
                        }
                    };
                    let app_state = model_ctx.app_state.clone();
                    let access_token = model_ctx.token.clone();
                    let usage_context = store.data().host_state.model_usage_context.clone();

                    // Parse messages_json: either a wrapper {messages, tools, ...params} or a plain array
                    #[derive(serde::Deserialize)]
                    struct LlmPromptRequest {
                        messages: Vec<Value>,
                        #[serde(default)]
                        tools: Option<Vec<Value>>,
                        #[serde(default)]
                        temperature: Option<f64>,
                        #[serde(default)]
                        max_tokens: Option<u64>,
                        #[serde(default)]
                        tool_choice: Option<Value>,
                        #[serde(default)]
                        output_schema: Option<Value>,
                        #[serde(default)]
                        additional_params: Option<Value>,
                    }

                    let (raw_messages, raw_tools, req_temperature, req_max_tokens, req_tool_choice, req_output_schema, req_additional_params) =
                        match serde_json::from_str::<LlmPromptRequest>(&messages_json) {
                            Ok(req) => (req.messages, req.tools, req.temperature, req.max_tokens, req.tool_choice, req.output_schema, req.additional_params),
                            Err(_) => match serde_json::from_str::<Vec<Value>>(&messages_json) {
                                Ok(msgs) => (msgs, None, None, None, None, None, None),
                                Err(e) => {
                                    println!("llm-prompt: failed to parse messages JSON: {e}");
                                    let err = serde_json::json!({"error": format!("Failed to parse messages: {e}")}).to_string();
                                    return Ok((Some(err),));
                                }
                            },
                        };

                    println!("llm-prompt: received {} messages, tools={}",
                        raw_messages.len(),
                        raw_tools.as_ref().map(|t| t.len()).unwrap_or(0)
                    );
                    if let Some(ref tools) = raw_tools {
                        for (i, t) in tools.iter().enumerate() {
                            println!("llm-prompt: raw tool[{i}]: {}", t);
                        }
                    }
                    for (i, m) in raw_messages.iter().enumerate() {
                        let role = m.get("role").and_then(|r| r.as_str()).unwrap_or("?");
                        let content_preview = m.get("content")
                            .and_then(|c| c.as_str())
                            .map(|s| if s.len() > 200 { format!("{}...", &s[..200]) } else { s.to_string() })
                            .unwrap_or_else(|| "<non-string>".to_string());
                        println!("llm-prompt: msg[{i}] role={role} content={content_preview}");
                    }

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

                        let content = sdk_message_content(msg);

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
                    history.stream = do_stream.then_some(true);

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
                    if let Some(ref params) = req_additional_params {
                        println!("llm-prompt: additional_params: {params}");
                    }

                    // Convert tool definitions if present
                    if let Some(tools) = raw_tools {
                        let mut native_tools: Vec<flow_like_model_provider::history::Tool> = Vec::new();
                        for (i, t) in tools.iter().enumerate() {
                            let name = match t.get("name").and_then(|n| n.as_str()) {
                                Some(n) => n.to_string(),
                                None => {
                                    println!("llm-prompt: tool[{i}] missing 'name' field");
                                    continue;
                                }
                            };
                            let desc = t.get("description").and_then(|d| d.as_str()).map(String::from);
                            let params = t.get("parameters").cloned().unwrap_or_default();
                            println!("llm-prompt: tool[{i}] '{name}' params: {params}");
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
                                    println!("llm-prompt: tool[{i}] '{name}' parameter deserialization FAILED: {e} — raw: {params}");
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
                        match factory
                            .build(
                                &bit,
                                app_state.clone(),
                                access_token.clone(),
                                usage_context,
                            )
                            .await
                        {
                            Ok(m) => m,
                            Err(e) => {
                                println!("llm-prompt: failed to build model: {e}");
                                let err = serde_json::json!({"error": format!("Failed to build model: {e}")}).to_string();
                                return Ok((Some(err),));
                            }
                        }
                    };

                    // Log the full History before invoking
                    if let Ok(history_json) = serde_json::to_string(&history) {
                        println!("llm-prompt: History to invoke (len={}): {}",
                            history_json.len(),
                            if history_json.len() > 2000 { format!("{}...", &history_json[..2000]) } else { history_json }
                        );
                    }

                    let stream_events = do_stream.then(|| {
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
                            println!("llm-prompt: model invoke failed: {e}");
                            let err = serde_json::json!({"error": format!("Model invocation failed: {e}")}).to_string();
                            return Ok((Some(err),));
                        }
                    };

                    if let Some(stream_events) = stream_events {
                        let collected = std::mem::take(&mut *stream_events.write());
                        let mut host_events = store.data().host_state.stream_events.write();
                        host_events.extend(collected);
                    }

                    // Convert response to SDK ChatMessage JSON
                    let resp_msg = match response.last_message() {
                        Some(m) => m,
                        None => {
                            println!("llm-prompt: model returned empty response (no messages)");
                            let err = serde_json::json!({"error": "Model returned empty response"}).to_string();
                            return Ok((Some(err),));
                        }
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

                    Ok((Some(result.to_string()),))
                })
            },
        )
        .map_err(map_err)?;

    // llm-prompt-stream — ABI v2 streaming LLM prompt
    models
        .func_wrap_async(
            "llm-prompt-stream",
            |store: wasmtime::StoreContextMut<'_, ComponentStoreData>,
             (bit_json, request_json): (String, String)| {
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
                        Err(e) => {
                            let err = serde_json::json!({"error": format!("Failed to parse model descriptor: {e}")}).to_string();
                            return Ok((Some(err),));
                        }
                    };

                    let model_ctx = match &store.data().host_state.model_context {
                        Some(c) => c,
                        None => {
                            let err = serde_json::json!({"error": "Model context not available"}).to_string();
                            return Ok((Some(err),));
                        }
                    };
                    let app_state = model_ctx.app_state.clone();
                    let access_token = model_ctx.token.clone();
                    let usage_context = store.data().host_state.model_usage_context.clone();

                    #[derive(serde::Deserialize)]
                    struct StreamRequest {
                        messages: Vec<Value>,
                        #[serde(default)]
                        tools: Option<Vec<Value>>,
                        #[serde(default)]
                        temperature: Option<f64>,
                        #[serde(default)]
                        max_tokens: Option<u64>,
                        #[serde(default)]
                        tool_choice: Option<Value>,
                        #[serde(default)]
                        output_schema: Option<Value>,
                        #[serde(default)]
                        #[allow(dead_code)] // wire contract: SDK sends additional_params (libs/wasm-sdk/wasm-sdk-rust/src/rig_provider.rs:413); parsed for parity with llm_prompt, not yet forwarded to History
                        additional_params: Option<Value>,
                    }

                    let req: StreamRequest = match serde_json::from_str(&request_json) {
                        Ok(r) => r,
                        Err(e) => {
                            let err = serde_json::json!({"error": format!("Failed to parse request: {e}")}).to_string();
                            return Ok((Some(err),));
                        }
                    };

                    let mut history_messages = Vec::with_capacity(req.messages.len());
                    for msg in &req.messages {
                        let role_str = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                        let role = match role_str {
                            "system" => flow_like_model_provider::history::Role::System,
                            "assistant" => flow_like_model_provider::history::Role::Assistant,
                            "tool" => flow_like_model_provider::history::Role::Tool,
                            _ => flow_like_model_provider::history::Role::User,
                        };
                        let content = sdk_message_content(msg);
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
                        match factory
                            .build(
                                &bit,
                                app_state.clone(),
                                access_token.clone(),
                                usage_context,
                            )
                            .await
                        {
                            Ok(m) => m,
                            Err(e) => {
                                let err = serde_json::json!({"error": format!("Failed to build model: {e}")}).to_string();
                                return Ok((Some(err),));
                            }
                        }
                    };

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
                            return Ok((Some(err),));
                        }
                    };

                    // Move collected stream events into host_state
                    {
                        let collected = std::mem::take(&mut *stream_events.write());
                        let mut host_events = store.data().host_state.stream_events.write();
                        host_events.extend(collected);
                    }

                    let resp_msg = match response.last_message() {
                        Some(m) => m,
                        None => {
                            let err = serde_json::json!({"error": "Model returned empty response"}).to_string();
                            return Ok((Some(err),));
                        }
                    };

                    let tool_calls_json: Option<Vec<Value>> = if resp_msg.tool_calls.is_empty() {
                        None
                    } else {
                        Some(resp_msg.tool_calls.iter().map(|tc| {
                            let args: Value = serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Object(Default::default()));
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

                // Same egress policy as native HTTP nodes: server-side, the
                // URL, its resolution and every redirect are checked against
                // the host-plane block list.
                // A request cannot usefully outlive its node, so the node's own
                // execution budget is the request timeout — nothing tighter.
                let environment = store.data().environment;
                let node_timeout = store.data().node_timeout;
                let client = flow_like::flow::execution::egress::GuardedHttpClient::configured(
                    environment,
                    |builder| builder.timeout(node_timeout),
                );
                let client = match client {
                    Ok(c) => c,
                    Err(_) => return Ok((None,)),
                };

                let mut req = match client.request(method_str, &url) {
                    Ok(req) => req,
                    Err(e) => {
                        tracing::warn!("WASM HTTP request to {} refused: {}", url, e);
                        return Ok((None,));
                    }
                };

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
                    Err(e) => {
                        tracing::warn!("WASM HTTP request to {} failed: {}", url, e);
                        return Ok((None,));
                    }
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
                let body_bytes = resp.bytes().await.unwrap_or_default();
                let body_b64 =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &body_bytes);

                let result = serde_json::json!({
                    "status": status,
                    "headers": resp_headers,
                    "body_base64": body_b64,
                });
                Ok((Some(result.to_string()),))
            })
        },
    )
    .map_err(map_err)?;

    Ok(())
}

/// Helper for storage-dir: builds FlowPath JSON, registers the store.
fn storage_dir_json(
    host: &HostState,
    dir_type: &str,
    dir_getter: impl FnOnce(
        &crate::host_functions::StorageContext,
    ) -> flow_like_storage::object_store::path::Path,
) -> Option<String> {
    let ctx = host.storage_context.as_ref()?;
    let dir = dir_getter(ctx);
    let flow_path = ctx.dir_flow_path(dir_type, dir)?;
    serde_json::to_string(&flow_path).ok()
}

use crate::host_functions::storage::StorageFlowPath;

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

                // Resolve and connect through the egress policy so the socket
                // goes to a vetted address; the request keeps the hostname
                // for Host / SNI.
                let environment = store.data().environment;
                let request = match tokio_tungstenite::tungstenite::client::IntoClientRequest::into_client_request(url.as_str()) {
                    Ok(request) => request,
                    Err(_) => return Ok((None,)),
                };
                let uri = request.uri().clone();
                let Some(host) = uri.host().map(str::to_string) else {
                    return Ok((None,));
                };
                let port = uri.port_u16().unwrap_or(match uri.scheme_str() {
                    Some("wss") => 443,
                    _ => 80,
                });
                let addrs = match flow_like::flow::execution::egress::resolve_socket_addrs(
                    environment,
                    &host,
                    port,
                )
                .await
                {
                    Ok(addrs) => addrs,
                    Err(e) => {
                        tracing::warn!("WASM WebSocket connect to {} refused: {}", url, e);
                        return Ok((None,));
                    }
                };
                let tcp = match tokio::net::TcpStream::connect(addrs.as_slice()).await {
                    Ok(tcp) => tcp,
                    Err(_) => return Ok((None,)),
                };
                let connect_result =
                    tokio_tungstenite::client_async_tls_with_config(request, tcp, None, None).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use wasmtime_wasi::cli::WasiCliView;
    use wasmtime_wasi::p2::bindings::cli::environment::Host;

    fn guest_environment(security: &WasmSecurityConfig) -> Vec<(String, String)> {
        let mut data = ComponentStoreData::new(security);
        let mut cli = data.cli();
        Host::get_environment(&mut cli).expect("WASI environment should be readable")
    }

    #[test]
    fn host_environment_is_not_visible_to_component_guests() {
        let sentinel_value = std::env::var("PATH").expect("test host should define PATH");
        assert!(
            !sentinel_value.is_empty(),
            "test host PATH should not be empty"
        );

        for (name, security) in [
            ("restrictive", WasmSecurityConfig::restrictive()),
            ("permissive", WasmSecurityConfig::permissive()),
        ] {
            let environment = guest_environment(&security);
            assert!(
                environment.is_empty(),
                "{name} component guest inherited host environment, including the PATH sentinel"
            );
        }
    }

    #[test]
    fn standard_http_requires_an_http_capability_without_an_allowlist() {
        assert!(!allows_standard_wasi_http(
            &WasmSecurityConfig::restrictive()
        ));
        assert!(!allows_standard_wasi_http(
            &WasmSecurityConfig::default().with_capabilities(WasmCapabilities::TCP)
        ));
        assert!(allows_standard_wasi_http(
            &WasmSecurityConfig::default().with_capabilities(WasmCapabilities::HTTP_ALL)
        ));
        assert!(!allows_standard_wasi_http(
            &WasmSecurityConfig::default().with_capabilities(WasmCapabilities::HTTP_GET)
        ));
        assert!(!allows_standard_wasi_http(
            &WasmSecurityConfig::default()
                .with_capabilities(WasmCapabilities::HTTP_GET)
                .with_allowed_hosts(vec!["example.com".to_string()])
        ));
    }
}
