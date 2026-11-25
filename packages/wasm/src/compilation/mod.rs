//! Compilation and closed metadata extraction without the application runtime.

#[cfg(feature = "component-model")]
mod component_imports;
mod core_imports;
mod nodes;
mod observations;
mod schemas;
#[cfg(test)]
mod tests;

pub use crate::engine_config::EngineConfig as CompilationConfig;
pub use flow_like_wasm_schema::limits::WasmLimits;
pub use nodes::ExtractedNode;

use crate::error::{host_compatibility_error, WasmError, WasmResult};
use crate::wasi::isolated_wasi_ctx_builder;
use wasmtime::{Engine, Module, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub struct CompilationEngine {
    engine: Engine,
}

impl CompilationEngine {
    pub fn new(config: CompilationConfig) -> WasmResult<Self> {
        let engine = Engine::new(&config.to_wasmtime_config()?)
            .map_err(|error| WasmError::Initialization(error.to_string()))?;
        Ok(Self { engine })
    }

    pub fn precompile(&self, bytes: &[u8]) -> WasmResult<Vec<u8>> {
        #[cfg(feature = "component-model")]
        if is_component(bytes) {
            return self
                .engine
                .precompile_component(bytes)
                .map_err(|error| WasmError::compilation(error.to_string()));
        }
        self.engine
            .precompile_module(bytes)
            .map_err(|error| WasmError::compilation(error.to_string()))
    }
}

fn is_component(bytes: &[u8]) -> bool {
    bytes.get(..8) == Some(&[0, 97, 115, 109, 13, 0, 1, 0])
}

struct MetadataState {
    limits: wasmtime::StoreLimits,
    wasi: WasiCtx,
    table: wasmtime::component::ResourceTable,
    longjmp_pending: bool,
    variables: std::collections::HashMap<String, serde_json::Value>,
    result_buffer: Vec<u8>,
}

impl MetadataState {
    fn new(limits: &WasmLimits, stdout: Option<wasmtime_wasi::p2::pipe::MemoryOutputPipe>) -> Self {
        let mut builder = isolated_wasi_ctx_builder();
        builder
            .make_deterministic()
            .allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        if let Some(stdout) = stdout {
            builder
                .stdout(stdout)
                .args(&["flow-like-wasm-node", "get-node"]);
        } else {
            builder.args(&["flow-like-wasm-node"]);
        }
        Self {
            limits: wasmtime::StoreLimitsBuilder::new()
                .memory_size(limits.memory_limit)
                .table_elements(limits.max_table_elements as usize)
                .instances(limits.max_instances as usize)
                .tables(limits.max_tables as usize)
                .memories(limits.max_memories as usize)
                .build(),
            wasi: builder.build(),
            table: wasmtime::component::ResourceTable::new(),
            longjmp_pending: false,
            variables: Default::default(),
            result_buffer: Vec::new(),
        }
    }

    fn store_result(&mut self, bytes: &[u8]) -> u64 {
        let offset = self.result_buffer.len();
        self.result_buffer.extend_from_slice(bytes);
        ((offset as u64) << 32) | bytes.len() as u64
    }
}

impl WasiView for MetadataState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

fn store(
    engine: &Engine,
    limits: &WasmLimits,
    stdout: Option<wasmtime_wasi::p2::pipe::MemoryOutputPipe>,
) -> WasmResult<Store<MetadataState>> {
    let mut store = Store::new(engine, MetadataState::new(limits, stdout));
    store.limiter(|state| &mut state.limits);
    store
        .set_fuel(limits.fuel_limit)
        .map_err(|error| WasmError::Internal(error.to_string()))?;
    store
        .fuel_async_yield_interval(Some(100_000))
        .map_err(|error| WasmError::Internal(error.to_string()))?;
    store.set_epoch_deadline((limits.timeout.as_millis() / 10).max(1) as u64);
    store.epoch_deadline_trap();
    Ok(store)
}

// Aborting the ticker on every return path also covers a cancelled extraction.
struct EpochTicker(tokio::task::JoinHandle<()>);

impl Drop for EpochTicker {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Read metadata without credentials, files, networking, models, or database access.
/// WASI observations are deterministic; SDK clocks and random functions retain their
/// existing behavior. Limits apply to guest initialization too.
pub async fn extract_nodes(bytes: &[u8], limits: WasmLimits) -> WasmResult<Vec<ExtractedNode>> {
    let engine = CompilationEngine::new(CompilationConfig {
        limits: limits.clone(),
        ..Default::default()
    })?
    .engine;
    let ticker_engine = engine.clone();
    let _ticker = EpochTicker(tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(10));
        loop {
            interval.tick().await;
            ticker_engine.increment_epoch();
        }
    }));
    #[cfg(feature = "component-model")]
    let json = if is_component(bytes) {
        component_metadata(&engine, bytes, &limits).await?
    } else {
        core_metadata(&engine, bytes, &limits).await?
    };
    #[cfg(not(feature = "component-model"))]
    let json = core_metadata(&engine, bytes, &limits).await?;
    nodes::parse(&json)
}

async fn core_metadata(engine: &Engine, bytes: &[u8], limits: &WasmLimits) -> WasmResult<String> {
    let module =
        Module::new(engine, bytes).map_err(|error| WasmError::compilation(error.to_string()))?;
    let mut linker = wasmtime::Linker::new(engine);
    core_imports::register(&mut linker)?;
    crate::core_wasi::register(&mut linker)?;
    let prepared = linker
        .instantiate_pre(&module)
        .map_err(host_compatibility_error)?;
    let mut store = store(engine, limits, None)?;
    let instance = prepared
        .instantiate_async(&mut store)
        .await
        .map_err(|error| {
            WasmError::instantiation(format!(
                "Failed to instantiate WASM for node extraction: {error:#}"
            ))
        })?;
    for name in ["_initialize", "_start"] {
        if let Ok(function) = instance.get_typed_func::<(), ()>(&mut store, name) {
            function
                .call_async(&mut store, ())
                .await
                .map_err(|error| WasmError::execution(name, format!("{error:#}")))?;
        }
    }
    // Preserve the runtime ABI checks even though the worker never invokes run.
    instance
        .get_typed_func::<(i32, i32), i64>(&mut store, "run")
        .map_err(|_| WasmError::MissingExport {
            export_name: "run".into(),
        })?;
    let function = instance
        .get_typed_func::<(), i64>(&mut store, "get_nodes")
        .or_else(|_| instance.get_typed_func::<(), i64>(&mut store, "get_node"))
        .map_err(|_| WasmError::MissingExport {
            export_name: "get_nodes or get_node".into(),
        })?;
    let packed = function
        .call_async(&mut store, ())
        .await
        .map_err(|error| WasmError::execution("get_nodes", format!("{error:#}")))?;
    if packed < 0 {
        return Err(WasmError::invalid_node_definition(
            "Guest returned an error",
        ));
    }
    let offset = (packed as u64 >> 32) as usize;
    let length = (packed as u64 & u32::MAX as u64) as usize;
    let memory =
        instance
            .get_memory(&mut store, "memory")
            .ok_or_else(|| WasmError::MissingExport {
                export_name: "memory".into(),
            })?;
    let data = offset
        .checked_add(length)
        .and_then(|end| memory.data(&store).get(offset..end))
        .ok_or_else(|| {
            WasmError::invalid_node_definition("Metadata points outside guest memory")
        })?;
    std::str::from_utf8(data)
        .map(str::to_owned)
        .map_err(|error| WasmError::invalid_node_definition(error.to_string()))
}

#[cfg(feature = "component-model")]
async fn component_metadata(
    engine: &Engine,
    bytes: &[u8],
    limits: &WasmLimits,
) -> WasmResult<String> {
    let component = wasmtime::component::Component::new(engine, bytes)
        .map_err(|error| WasmError::compilation(error.to_string()))?;
    let mut linker = wasmtime::component::Linker::new(engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)
        .map_err(|error| WasmError::Initialization(error.to_string()))?;
    component_imports::register(&mut linker)?;
    let prepared = linker
        .instantiate_pre(&component)
        .map_err(host_compatibility_error)?;
    let mut store = store(engine, limits, None)?;
    let instance = prepared
        .instantiate_async(&mut store)
        .await
        .map_err(|error| {
            WasmError::instantiation(format!(
                "Failed to instantiate WASM for node extraction: {error:#}"
            ))
        })?;
    if let Ok(function) = instance
        .get_typed_func::<(), (String,)>(&mut store, "get-nodes")
        .or_else(|_| instance.get_typed_func::<(), (String,)>(&mut store, "get-node"))
    {
        let (json,) = function
            .call_async(&mut store, ())
            .await
            .map_err(|error| WasmError::execution("get-nodes", format!("{error:#}")))?;
        return Ok(json);
    }
    // Command components expose their metadata through `get-node` on stdout.
    // Reinstantiate with the same closed sandbox and bounded output buffer.
    let stdout = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(10 << 20);
    let mut store = self::store(engine, limits, Some(stdout.clone()))?;
    let command =
        wasmtime_wasi::p2::bindings::Command::instantiate_async(&mut store, &component, &linker)
            .await
            .map_err(|error| WasmError::instantiation(format!("{error:#}")))?;
    command
        .wasi_cli_run()
        .call_run(&mut store)
        .await
        .map_err(|error| WasmError::execution("get-node", format!("{error:#}")))?
        .map_err(|_| WasmError::execution("get-node", "Metadata command failed"))?;
    String::from_utf8(stdout.contents().to_vec())
        .map_err(|error| WasmError::invalid_node_definition(error.to_string()))
}
