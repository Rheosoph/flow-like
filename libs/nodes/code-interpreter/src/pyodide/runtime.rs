//! Wasmtime + WASI execution engine for the Python interpreter node.
//!
//! # How it works
//!
//! ```text
//!  PythonInterpreterNode::do_run()
//!    └─ builds ExecutionRequest (with WorkspaceInfo)
//!  PyodideRuntime::execute()
//!    ├─ writes /flow/{bootstrap.py, code.py, inputs.json, config.json}
//!    ├─ if workspace present:
//!    │    ├─ lists object store → writes /flow/ws_manifest.json
//!    │    ├─ creates /flow/{ws_pending, ws_data, ws_notfound, ws_puts}
//!    │    └─ spawns workspace_file_server tokio task
//!    ├─ opens Wasmtime Store<StoreData> with WASI p1 context (only /flow preopened)
//!    ├─ calls WASM `_start` → Python runs bootstrap.py
//!    │     bootstrap.py executes user code with workspace API object.
//!    │     workspace.get(path) → writes ws_pending/{uuid}={relative},
//!    │       polls for ws_data/{path} or ws_notfound/{uuid} to appear.
//!    │     workspace.put(path, data) → writes ws_puts/{path}.
//!    ├─ cancels workspace_file_server
//!    ├─ uploads ws_puts/ → object store
//!    └─ reads /flow/outputs.json → ExecutionResponse
//! ```
//!
//! # Required WASM binary
//!
//! See [`probe_wasm_path`] for the search order.
//!
//! # Security layers
//!
//! | Layer | Mechanism |
//! |-------|-----------|
//! | WASM sandbox | Wasmtime memory isolation |
//! | Filesystem | Only `/flow` is preopened (no direct host FS mount) |
//! | Workspace | Path traversal stripped in bootstrap; prefix enforced by host |
//! | Network | Disabled by default; enabled only when `network_enabled = true` |
//! | Timeout | Epoch interruption traps the WASM instance after the deadline |
//! | Memory | `ResourceLimiter` caps linear memory growth |
//! | Packages | Allowlist enforced in `bootstrap.py` |

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use wasmtime::{Linker, Module, Store};
use wasmtime_wasi::{
    FsPerms,
    p1::{self, WasiP1Ctx},
    p2::pipe::MemoryOutputPipe,
};

use flow_like_storage::{
    Path as ObjPath, join_object_path, normalize_object_path,
    object_store::{ObjectStore, ObjectStoreExt, PutPayload},
};
use flow_like_types::Bytes;
use flow_like_wasm::{AotCache, WasmConfig, WasmEngine, isolated_wasi_ctx_builder};

use super::network::GuestNetwork;

// ─── Constants ────────────────────────────────────────────────────────────────

/// Maximum bytes captured from the Python process's stdout.
const MAX_STDOUT_BYTES: usize = 10 * 1024 * 1024; // 10 MB
/// Maximum bytes captured from the Python process's stderr.
const MAX_STDERR_BYTES: usize = 2 * 1024 * 1024; // 2 MB
/// Epoch ticker interval — each tick is 10 ms.
const EPOCH_TICK_MS: u64 = 10;
/// How often the workspace file server polls for pending requests.
const WS_SERVER_POLL_MS: u64 = 20;

/// The Python bootstrap script compiled into the binary.
pub(crate) const BOOTSTRAP_SCRIPT: &str = include_str!("bootstrap.py");

// ─── Public configuration ─────────────────────────────────────────────────────

/// Configuration for [`PyodideRuntime`].
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    /// Explicit path to a WASI-compatible Python WASM binary.
    /// When `None` the runtime probes the standard locations listed in
    /// [`probe_wasm_path`].
    pub wasm_binary_path: Option<PathBuf>,
    /// Directory for the precompiled module cache. `None` uses the shared
    /// WASM cache. Engines with different settings must not share one.
    pub aot_cache_dir: Option<PathBuf>,
    /// Compile the engine with fuel metering so runs can carry a fuel budget.
    pub fuel_metering: bool,
}

// ─── Workspace info ───────────────────────────────────────────────────────────

/// Object-store workspace to expose inside the Python sandbox.
///
/// The Python `workspace` API object fetches files on demand via filesystem
/// IPC (see `bootstrap.py`).  The Rust side provides a concurrent file server
/// that services these requests without pre-downloading the entire workspace.
pub struct WorkspaceInfo {
    /// Generic object-store handle.
    pub store: Arc<dyn ObjectStore>,
    /// Path prefix within the store (e.g. `"projects/abc"`).
    /// Empty string = root of the store.
    pub prefix: String,
}

// ─── Execution request / response ─────────────────────────────────────────────

/// Parameters for a single sandboxed Python execution.
pub struct ExecutionRequest {
    /// Python source code to execute.
    pub code: String,
    /// JSON data injected as the `inputs` dict inside the sandbox.
    pub inputs: Value,
    /// micropip packages to install before executing `code`.
    pub packages: Vec<String>,
    /// Package installation allowlist:
    /// `None` = any package; `Some([])` = none; `Some([...])` = listed only.
    pub package_allowlist: Option<Vec<String>>,
    /// Whether to give the sandbox WASI socket access.
    pub network_enabled: bool,
    /// Advisory hostname allowlist applied via a Python socket patch.
    pub allowed_hosts: Vec<String>,
    /// Optional object-store workspace — exposed as the `workspace` API object
    /// in Python.  Files are fetched lazily; writes are staged and uploaded
    /// after execution.
    pub workspace: Option<WorkspaceInfo>,
    /// Hard execution timeout enforced via epoch interruption.
    pub timeout: Duration,
    /// Maximum WASM linear memory in bytes.
    pub memory_limit: usize,
}

/// Result of a sandboxed Python execution.
pub struct ExecutionResponse {
    /// Contents of the Python `outputs` dict.
    pub outputs: Value,
    /// Captured stdout from user code.
    pub stdout: String,
    /// Captured stderr from the sandbox.
    pub stderr: String,
    /// Error traceback / message when `success` is false.
    pub error: Option<String>,
    /// `true` iff user code completed without an unhandled exception.
    pub success: bool,
}

/// Intermediate result written by `bootstrap.py` to `/flow/outputs.json`.
#[derive(Deserialize)]
pub(crate) struct BootstrapResult {
    pub outputs: Value,
    pub stdout: String,
    pub stderr: String,
    pub error: Option<String>,
    pub success: bool,
    #[serde(default)]
    pub stdout_dropped: u64,
    #[serde(default)]
    pub stderr_dropped: u64,
}

// ─── Resource limiter ─────────────────────────────────────────────────────────

/// Caps WASM linear memory growth.
struct MemoryLimiter {
    max_bytes: usize,
}

impl wasmtime::ResourceLimiter for MemoryLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> std::result::Result<bool, wasmtime::Error> {
        if desired > self.max_bytes {
            warn!(
                "Python sandbox rejected memory growth to {} bytes (limit: {})",
                desired, self.max_bytes
            );
            Ok(false)
        } else {
            Ok(true)
        }
    }

    fn table_growing(
        &mut self,
        _: usize,
        _: usize,
        _: Option<usize>,
    ) -> std::result::Result<bool, wasmtime::Error> {
        Ok(true)
    }
}

// ─── Store data ───────────────────────────────────────────────────────────────

/// Per-execution state stored in the Wasmtime `Store`.
struct StoreData {
    wasi: WasiP1Ctx,
    limiter: MemoryLimiter,
}

/// How a guest module is launched: its argv, extra guest environment, read-only mounts and
/// whether it is a reactor (`_initialize` then `run`) or a command (`_start`).
#[derive(Debug, Clone)]
pub(crate) struct GuestSpec {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub read_only_mounts: Vec<(PathBuf, String)>,
    pub reactor: bool,
}

impl GuestSpec {
    pub fn command(args: &[&str]) -> Self {
        Self {
            args: args.iter().map(|a| a.to_string()).collect(),
            env: Vec::new(),
            read_only_mounts: Vec::new(),
            reactor: false,
        }
    }

    pub fn python_cli() -> Self {
        Self::command(&["python3", "/flow/bootstrap.py"])
    }
}

/// Build the WASI Preview 1 context used by every Python sandbox execution.
///
/// Host environment variables are deliberately not inherited. Wasmtime starts
/// with an empty guest environment, and this function must only add individual,
/// non-secret guest values if the runtime ever gains an explicit guest-env API.
fn build_sandbox_wasi_context(
    exec_path: &Path,
    stdout_pipe: MemoryOutputPipe,
    stderr_pipe: MemoryOutputPipe,
    network: &GuestNetwork,
    spec: &GuestSpec,
) -> Result<WasiP1Ctx> {
    let mut builder = isolated_wasi_ctx_builder();
    builder
        .stdout(stdout_pipe)
        .stderr(stderr_pipe)
        .args(&spec.args);
    for (key, value) in &spec.env {
        builder.guest_env(key, value);
    }
    for (host, guest) in &spec.read_only_mounts {
        builder
            .preopened_dir(host, guest, FsPerms::ReadOnly)
            .map_err(|e| anyhow::anyhow!("{e:#}"))
            .with_context(|| format!("preopened {guest}"))?;
    }

    // Only /flow is preopened — workspace access is via the API, not POSIX I/O.
    builder
        .preopened_dir(exec_path, "/flow", FsPerms::ReadWrite)
        .map_err(|e| anyhow::anyhow!("{e:#}"))
        .context("preopened /flow")?;

    network.apply(&mut builder);

    Ok(builder.build_p1())
}

// ─── Workspace file server ────────────────────────────────────────────────────

/// Runs concurrently with the WASM execution to service `workspace.get()`
/// requests from Python.
///
/// # Protocol
///
/// Python writes `/flow/ws_pending/{uuid}` with the relative path as content.
/// This task:
/// 1. Fetches the object at `{prefix}/{relative}` from the store.
/// 2. On success: writes the bytes to `/flow/ws_data/{relative}` (creating
///    subdirectories as needed), then removes the pending file.
/// 3. On 404: writes an empty file to `/flow/ws_notfound/{uuid}`, then
///    removes the pending file.
///
/// Python polls for `/flow/ws_data/{relative}` (hit) or
/// `/flow/ws_notfound/{uuid}` (miss) to appear.
///
/// The task runs until the `tokio::JoinHandle` it returns is aborted by the
/// caller (after WASM execution completes).
pub async fn workspace_file_server(
    store: Arc<dyn ObjectStore>,
    prefix: String,
    ws_pending_dir: PathBuf,
    ws_data_dir: PathBuf,
    ws_notfound_dir: PathBuf,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(WS_SERVER_POLL_MS));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        interval.tick().await;

        let dir = match tokio::fs::read_dir(&ws_pending_dir).await {
            Ok(d) => d,
            Err(_) => continue,
        };

        let mut entries = tokio::fs::ReadDir::from(dir);
        loop {
            let entry = match entries.next_entry().await {
                Ok(Some(e)) => e,
                _ => break,
            };

            let pending_path = entry.path();
            let req_id = match pending_path.file_name() {
                Some(n) => n.to_string_lossy().to_string(),
                None => continue,
            };

            let relative = match tokio::fs::read_to_string(&pending_path).await {
                Ok(s) => s.trim().to_string(),
                Err(_) => continue,
            };

            // Sanitise the relative path on the host side as well (defence in depth).
            let safe_relative: String = relative
                .split('/')
                .filter(|p| !p.is_empty() && *p != "." && *p != "..")
                .collect::<Vec<_>>()
                .join("/");

            if safe_relative.is_empty() {
                tokio::fs::remove_file(&pending_path).await.ok();
                continue;
            }

            let obj_path = workspace_object_path(&prefix, &safe_relative);

            match store.get(&obj_path).await {
                Ok(result) => {
                    match result.bytes().await {
                        Ok(bytes) => {
                            // Mirror directory structure under ws_data/.
                            let local_path = safe_relative
                                .split('/')
                                .fold(ws_data_dir.clone(), |p, part| p.join(part));
                            if let Some(parent) = local_path.parent() {
                                tokio::fs::create_dir_all(parent).await.ok();
                            }
                            if tokio::fs::write(&local_path, &bytes).await.is_ok() {
                                // Signal success: remove pending AFTER data is written.
                                tokio::fs::remove_file(&pending_path).await.ok();
                                debug!(
                                    "workspace: served '{}' ({} bytes)",
                                    safe_relative,
                                    bytes.len()
                                );
                            }
                        }
                        Err(e) => {
                            warn!(
                                "workspace: failed to read bytes for '{}': {}",
                                safe_relative, e
                            );
                        }
                    }
                }
                Err(_) => {
                    // Object not found — write notfound sentinel then remove pending.
                    let notfound_path = ws_notfound_dir.join(&req_id);
                    tokio::fs::write(&notfound_path, b"").await.ok();
                    tokio::fs::remove_file(&pending_path).await.ok();
                    debug!("workspace: '{}' not found in store", safe_relative);
                }
            }
        }
    }
}

/// Walk `ws_puts_dir` recursively and upload every file to the object store.
pub async fn upload_ws_puts(ws_puts_dir: &PathBuf, store: &Arc<dyn ObjectStore>, prefix: &str) {
    // The guest can create symlinks. Only files still inside its output directory
    // may be read by the host after execution finishes.
    let Ok(metadata) = tokio::fs::symlink_metadata(ws_puts_dir).await else {
        return;
    };
    if !metadata.is_dir() {
        return;
    }
    let Ok(root) = tokio::fs::canonicalize(ws_puts_dir).await else {
        return;
    };
    let mut stack = vec![(ws_puts_dir.clone(), vec![root.clone()])];
    while let Some((dir, ancestors)) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            let Ok(resolved) = tokio::fs::canonicalize(&path).await else {
                continue;
            };
            if !resolved.starts_with(&root) {
                continue;
            }
            let Ok(metadata) = tokio::fs::metadata(&resolved).await else {
                continue;
            };
            if metadata.is_dir() {
                if !ancestors.contains(&resolved) {
                    let mut descendants = ancestors.clone();
                    descendants.push(resolved);
                    stack.push((path, descendants));
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }

            let data = match tokio::fs::read(&resolved).await {
                Ok(d) => d,
                Err(e) => {
                    warn!("workspace upload: read {:?} failed: {}", path, e);
                    continue;
                }
            };

            let relative = path
                .strip_prefix(ws_puts_dir)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();

            let obj_path = workspace_object_path(prefix, &relative);
            let payload = PutPayload::from_bytes(Bytes::from(data));

            match store.put(&obj_path, payload).await {
                Ok(_) => debug!("workspace: uploaded '{}'", relative),
                Err(e) => warn!("workspace: upload '{}' failed: {}", relative, e),
            }
        }
    }
}

// ─── Runtime ──────────────────────────────────────────────────────────────────

/// Manages a compiled Python WASM module and executes Python code in isolated
/// Wasmtime WASI instances.
///
/// Wraps the shared [`WasmEngine`] (which owns the compiled `wasmtime::Engine`,
/// epoch ticker, and AOT disk cache) and adds a per-binary in-memory module
/// cache.
pub struct PyodideRuntime {
    /// Shared engine with epoch ticker and AOT disk cache.
    engine: Arc<WasmEngine>,
    /// In-memory cached compiled module (shared across executions).
    module: Arc<RwLock<Option<Module>>>,
    /// AOT disk cache for fast deserialization on subsequent cold starts.
    aot_cache: Option<AotCache>,
    config: RuntimeConfig,
    ticker_stop: Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for PyodideRuntime {
    fn drop(&mut self) {
        self.ticker_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub(crate) struct GuestLimits {
    pub timeout: Duration,
    pub memory_limit: usize,
    pub fuel: Option<u64>,
}

#[derive(Debug)]
pub(crate) enum GuestOutcome {
    Exited,
    TimedOut,
    OutOfFuel,
    Failed(String),
}

pub(crate) struct GuestRun {
    pub outcome: GuestOutcome,
    pub stdout: String,
    pub stderr: String,
}

fn classify_guest_exit(result: std::result::Result<(), wasmtime::Error>) -> GuestOutcome {
    let Err(trap) = result else {
        return GuestOutcome::Exited;
    };
    match trap.downcast_ref::<wasmtime::Trap>() {
        Some(wasmtime::Trap::Interrupt) => return GuestOutcome::TimedOut,
        Some(wasmtime::Trap::OutOfFuel) => return GuestOutcome::OutOfFuel,
        _ => {}
    }
    let msg = trap.to_string();
    if msg.contains("exit status 0") || msg.contains("I32Exit(0)") {
        GuestOutcome::Exited
    } else if msg.to_lowercase().contains("epoch") {
        GuestOutcome::TimedOut
    } else {
        GuestOutcome::Failed(msg)
    }
}

fn spawn_epoch_ticker(engine: wasmtime::Engine) -> Arc<std::sync::atomic::AtomicBool> {
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = stop.clone();
    std::thread::Builder::new()
        .name("sandbox-epoch-ticker".into())
        .spawn(move || {
            while !flag.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(EPOCH_TICK_MS));
                engine.increment_epoch();
            }
        })
        .ok();
    stop
}

impl PyodideRuntime {
    /// Create a new runtime instance.
    pub fn new(config: RuntimeConfig) -> Result<Self> {
        let mut wasm_config = WasmConfig {
            fuel_metering: config.fuel_metering,
            ..WasmConfig::production()
        };
        if let Some(dir) = &config.aot_cache_dir {
            wasm_config.cache_dir = Some(dir.clone());
        }
        let aot_cache = wasm_config.cache_dir.as_ref().map(AotCache::new);

        let engine = WasmEngine::new(wasm_config).context("create WasmEngine")?;
        let ticker_stop = spawn_epoch_ticker(engine.engine().clone());

        Ok(Self {
            engine: Arc::new(engine),
            module: Arc::new(RwLock::new(None)),
            aot_cache,
            config,
            ticker_stop,
        })
    }

    // ── Module loading ────────────────────────────────────────────────────

    fn probe_wasm_path(&self) -> Option<PathBuf> {
        if let Some(p) = &self.config.wasm_binary_path {
            if p.exists() {
                return Some(p.clone());
            }
        }
        let candidates: &[Option<PathBuf>] = &[
            dirs_next::data_local_dir().map(|d| d.join("flow-like").join("python.wasm")),
            Some(PathBuf::from("/usr/local/share/flow-like/python.wasm")),
            // Lambda ephemeral storage (/tmp) and deployment package (/var/task).
            Some(PathBuf::from("/tmp/flow-like/python.wasm")),
            Some(PathBuf::from("/tmp/python.wasm")),
            Some(PathBuf::from("/var/task/python.wasm")),
            Some(PathBuf::from("./python.wasm")),
        ];
        candidates.iter().flatten().find(|p| p.exists()).cloned()
    }

    /// Return the compiled module.
    ///
    /// Compilation order: in-memory → bundled .cwasm → AOT disk cache → full Cranelift.
    async fn get_module(&self) -> Result<Module> {
        {
            let guard = self.module.read().await;
            if let Some(m) = guard.as_ref() {
                return Ok(m.clone());
            }
        }

        // Try the build-time bundled .cwasm first (zero cold-start).
        #[cfg(has_bundled_python)]
        if self.config.wasm_binary_path.is_none() {
            let wasmtime_engine = self.engine.engine();
            match Self::load_bundled(wasmtime_engine) {
                Ok(module) => {
                    info!("Python WASM loaded from bundled .cwasm");
                    let mut guard = self.module.write().await;
                    *guard = Some(module.clone());
                    return Ok(module);
                }
                Err(e) => {
                    warn!("Bundled .cwasm failed to deserialize (engine mismatch?): {e:#}");
                }
            }
        }

        let path = self.probe_wasm_path().ok_or_else(|| {
            anyhow::anyhow!(
                "Python WASM binary not found.\n\
                 Place a WASI-compatible Python interpreter at one of:\n\
                 • ~/.local/share/flow-like/python.wasm\n\
                 • /usr/local/share/flow-like/python.wasm\n\
                 • /tmp/flow-like/python.wasm  (Lambda ephemeral)\n\
                 • /tmp/python.wasm            (Lambda ephemeral)\n\
                 • /var/task/python.wasm       (Lambda deployment package)\n\
                 • ./python.wasm\n\
                 See https://docs.flow-like.dev/nodes/python-interpreter"
            )
        })?;

        info!("Loading Python WASM binary from {:?}", path);
        let bytes = tokio::fs::read(&path)
            .await
            .with_context(|| format!("read WASM binary from {path:?}"))?;

        let hash = flow_like_storage::blake3::hash(&bytes).to_hex().to_string();
        let wasmtime_engine = self.engine.engine();

        let module = if let Some(aot) = &self.aot_cache {
            if let Some(m) = aot.load_module(wasmtime_engine, &hash) {
                info!("Python WASM loaded from AOT cache (hash: {}…)", &hash[..8]);
                m
            } else {
                let eng = wasmtime_engine.clone();
                let b = bytes;
                let m = tokio::task::spawn_blocking(move || Module::new(&eng, &b))
                    .await
                    .context("spawn_blocking compilation")??;
                aot.save_module(&m, &hash);
                info!(
                    "Python WASM compiled and saved to AOT cache (hash: {}…)",
                    &hash[..8]
                );
                m
            }
        } else {
            let eng = wasmtime_engine.clone();
            let b = bytes;
            tokio::task::spawn_blocking(move || Module::new(&eng, &b))
                .await
                .context("spawn_blocking compilation")??
        };

        {
            let mut guard = self.module.write().await;
            *guard = Some(module.clone());
        }
        Ok(module)
    }

    /// Deserialize the build-time bundled `.cwasm` artifact.
    ///
    /// # Safety
    /// The `.cwasm` is produced by our own `build.rs` from a verified `.wasm`
    /// binary — it is trusted native code compiled at build time.
    #[cfg(has_bundled_python)]
    fn load_bundled(engine: &wasmtime::Engine) -> Result<Module> {
        static BUNDLED_CWASM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/python.cwasm"));
        // SAFETY: produced by build.rs with a matching Engine configuration.
        let module = unsafe { Module::deserialize(engine, BUNDLED_CWASM) }?;
        Ok(module)
    }

    pub(crate) async fn compiled_module(&self) -> Result<Module> {
        self.get_module().await
    }

    pub(crate) async fn run_guest(
        &self,
        module: &Module,
        exec_path: &Path,
        limits: &GuestLimits,
        network: &GuestNetwork,
        spec: &GuestSpec,
    ) -> Result<GuestRun> {
        let stdout_pipe = MemoryOutputPipe::new(MAX_STDOUT_BYTES);
        let stderr_pipe = MemoryOutputPipe::new(MAX_STDERR_BYTES);
        let wasi = build_sandbox_wasi_context(
            exec_path,
            stdout_pipe.clone(),
            stderr_pipe.clone(),
            network,
            spec,
        )?;

        let store_data = StoreData {
            wasi,
            limiter: MemoryLimiter {
                max_bytes: limits.memory_limit,
            },
        };
        let mut store = Store::new(self.engine.engine(), store_data);
        store.limiter(|data| &mut data.limiter as &mut dyn wasmtime::ResourceLimiter);

        if self.config.fuel_metering {
            store
                .set_fuel(limits.fuel.unwrap_or(u64::MAX))
                .map_err(|e| anyhow::anyhow!("{e:#}"))
                .context("set fuel budget")?;
        }
        let timeout_ticks = limits.timeout.as_millis() as u64 / EPOCH_TICK_MS + 1;
        store.set_epoch_deadline(timeout_ticks);
        store.epoch_deadline_trap();

        let mut linker: Linker<StoreData> = Linker::new(self.engine.engine());
        p1::add_to_linker_async(&mut linker, |data: &mut StoreData| &mut data.wasi)
            .map_err(|e| anyhow::anyhow!("{e:#}"))
            .context("add WASI p1 to linker")?;
        if spec.reactor {
            linker
                .func_wrap("host", "call", |_: i32, _: i32, _: i32, _: i32| -> i32 { -1 })
                .map_err(|e| anyhow::anyhow!("{e:#}"))
                .context("define host door")?;
        }

        let instance = linker
            .instantiate_async(&mut store, module)
            .await
            .map_err(|e| anyhow::anyhow!("{e:#}"))
            .context("instantiate Python WASM module")?;
        debug!("Calling guest entry point (timeout: {:?})", limits.timeout);
        let result = if spec.reactor {
            let init = instance
                .get_typed_func::<(), ()>(&mut store, "_initialize")
                .map_err(|e| anyhow::anyhow!("{e:#}"))
                .context("reactor module must export `_initialize`")?;
            let run = instance
                .get_typed_func::<(), i32>(&mut store, "run")
                .map_err(|e| anyhow::anyhow!("{e:#}"))
                .context("reactor module must export `run`")?;
            match init.call_async(&mut store, ()).await {
                Ok(()) => run.call_async(&mut store, ()).await.map(|_| ()),
                Err(e) => Err(e),
            }
        } else {
            let start = instance
                .get_typed_func::<(), ()>(&mut store, "_start")
                .map_err(|e| anyhow::anyhow!("{e:#}"))
                .context("module must export `_start` (WASI CLI)")?;
            start.call_async(&mut store, ()).await
        };

        Ok(GuestRun {
            outcome: classify_guest_exit(result),
            stdout: String::from_utf8_lossy(&stdout_pipe.contents()).into_owned(),
            stderr: String::from_utf8_lossy(&stderr_pipe.contents()).into_owned(),
        })
    }

    // ── Public execution entry point ──────────────────────────────────────

    /// Execute the given request, always returning a response (even on error).
    pub async fn execute(&self, req: ExecutionRequest) -> ExecutionResponse {
        match self.execute_inner(req).await {
            Ok(resp) => resp,
            Err(e) => ExecutionResponse {
                outputs: Value::Object(Default::default()),
                stdout: String::new(),
                stderr: String::new(),
                error: Some(format!("Runtime error: {e:#}")),
                success: false,
            },
        }
    }

    async fn execute_inner(&self, req: ExecutionRequest) -> Result<ExecutionResponse> {
        let module = self.get_module().await?;

        // ── Prepare ephemeral /flow execution directory ───────────────────
        let exec_dir = tempfile::Builder::new()
            .prefix("flow-exec-")
            .tempdir()
            .context("create exec tempdir")?;
        let exec_path = exec_dir.path().to_path_buf();

        tokio::fs::write(exec_path.join("bootstrap.py"), BOOTSTRAP_SCRIPT)
            .await
            .context("write bootstrap.py")?;
        tokio::fs::write(exec_path.join("code.py"), &req.code)
            .await
            .context("write code.py")?;
        tokio::fs::write(
            exec_path.join("inputs.json"),
            serde_json::to_string_pretty(&req.inputs).context("serialise inputs")?,
        )
        .await
        .context("write inputs.json")?;
        tokio::fs::write(
            exec_path.join("config.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "packages": req.packages,
                "package_allowlist": req.package_allowlist,
                "network_enabled": req.network_enabled,
                "allowed_hosts": req.allowed_hosts,
            }))
            .context("serialise config")?,
        )
        .await
        .context("write config.json")?;

        // ── Workspace setup ───────────────────────────────────────────────
        let ws_server_handle = if let Some(ws) = &req.workspace {
            let ws_pending = exec_path.join("ws_pending");
            let ws_data = exec_path.join("ws_data");
            let ws_notfound = exec_path.join("ws_notfound");
            let ws_puts = exec_path.join("ws_puts");

            for dir in &[&ws_pending, &ws_data, &ws_notfound, &ws_puts] {
                tokio::fs::create_dir_all(dir)
                    .await
                    .context("create ws dirs")?;
            }

            // Write manifest (paths only — no content downloaded).
            let manifest = list_workspace_files(&ws.store, &ws.prefix).await;
            tokio::fs::write(
                exec_path.join("ws_manifest.json"),
                serde_json::to_string(&manifest).context("serialise ws manifest")?,
            )
            .await
            .context("write ws_manifest.json")?;

            debug!(
                "workspace: {} file(s) in manifest (prefix: {:?})",
                manifest.len(),
                ws.prefix
            );

            // Spawn the concurrent file server.
            let handle = tokio::spawn(workspace_file_server(
                ws.store.clone(),
                ws.prefix.clone(),
                ws_pending,
                ws_data,
                ws_notfound,
            ));
            Some(handle)
        } else {
            None
        };

        let network = if req.network_enabled {
            if req.allowed_hosts.is_empty() {
                GuestNetwork::PublicInternet
            } else {
                GuestNetwork::resolve_hosts(&req.allowed_hosts).await
            }
        } else {
            GuestNetwork::Off
        };
        let limits = GuestLimits {
            timeout: req.timeout,
            memory_limit: req.memory_limit,
            fuel: None,
        };
        let run = self
            .run_guest(&module, &exec_path, &limits, &network, &GuestSpec::python_cli())
            .await;

        // ── Shut down workspace file server ───────────────────────────────
        if let Some(handle) = ws_server_handle {
            handle.abort();
        }

        // ── Upload staged puts ────────────────────────────────────────────
        if let Some(ws) = &req.workspace {
            let ws_puts = exec_path.join("ws_puts");
            upload_ws_puts(&ws_puts, &ws.store, &ws.prefix).await;
        }

        let run = run?;
        let raw_stdout = run.stdout;
        let raw_stderr = run.stderr;

        let runtime_error: Option<String> = match run.outcome {
            GuestOutcome::Exited => None,
            GuestOutcome::TimedOut => Some(format!(
                "Execution timed out after {:.1}s",
                req.timeout.as_secs_f64()
            )),
            GuestOutcome::OutOfFuel => Some("Execution exhausted its fuel budget".to_string()),
            GuestOutcome::Failed(msg) => Some(msg),
        };

        // ── Read outputs.json ─────────────────────────────────────────────
        let outputs_path = exec_path.join("outputs.json");
        let bootstrap: Option<BootstrapResult> = if outputs_path.exists() {
            tokio::fs::read(&outputs_path)
                .await
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
        } else {
            None
        };

        drop(exec_dir);

        // ── Assemble response ─────────────────────────────────────────────
        Ok(match bootstrap {
            Some(br) => {
                let error = br.error.or(runtime_error);
                let success = br.success && error.is_none();
                ExecutionResponse {
                    outputs: br.outputs,
                    stdout: br.stdout,
                    stderr: br.stderr,
                    error,
                    success,
                }
            }
            None => ExecutionResponse {
                outputs: Value::Object(Default::default()),
                stdout: raw_stdout,
                stderr: raw_stderr,
                error: runtime_error.or_else(|| {
                    Some(
                        "Python bootstrap did not produce /flow/outputs.json — \
                         check that the WASM binary is a valid WASI Python interpreter."
                            .to_string(),
                    )
                }),
                success: false,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_uploads_stay_within_the_guest_output_directory() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let output = temp.path().join("ws_puts");
        let nested = output.join("nested");
        tokio::fs::create_dir_all(&nested).await.unwrap();
        tokio::fs::write(nested.join("good.txt"), b"workspace")
            .await
            .unwrap();
        let outside = temp.path().join("host-secret");
        tokio::fs::write(&outside, b"secret").await.unwrap();
        symlink(&outside, output.join("leak.txt")).unwrap();
        symlink(temp.path(), output.join("host-directory")).unwrap();
        symlink(&output, nested.join("cycle")).unwrap();
        symlink(nested.join("good.txt"), output.join("alias.txt")).unwrap();
        symlink(&nested, output.join("alias-directory")).unwrap();

        let store: Arc<dyn ObjectStore> =
            Arc::new(flow_like_storage::object_store::memory::InMemory::new());
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            upload_ws_puts(&output, &store, "workspace"),
        )
        .await
        .expect("symlink cycles must terminate");
        for path in ["nested/good.txt", "alias.txt", "alias-directory/good.txt"] {
            assert_eq!(
                store
                    .get(&workspace_object_path("workspace", path))
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap(),
                "workspace"
            );
        }
        for path in [
            "leak.txt",
            "host-directory/host-secret",
            "nested/cycle/alias.txt",
        ] {
            assert!(
                store
                    .head(&workspace_object_path("workspace", path))
                    .await
                    .is_err()
            );
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn workspace_uploads_reject_a_replaced_output_directory() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("host");
        tokio::fs::create_dir(&outside).await.unwrap();
        tokio::fs::write(outside.join("secret"), b"secret")
            .await
            .unwrap();
        let output = temp.path().join("ws_puts");
        symlink(&outside, &output).unwrap();
        let store: Arc<dyn ObjectStore> =
            Arc::new(flow_like_storage::object_store::memory::InMemory::new());
        upload_ws_puts(&output, &store, "workspace").await;
        assert!(
            store
                .head(&workspace_object_path("workspace", "secret"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn python_network_option_preserves_protocol_grants_without_enabling_dns() {
        use wasmtime_wasi::p2::bindings::sockets::{
            instance_network, ip_name_lookup,
            network::{ErrorCode, IpAddressFamily},
            tcp_create_socket, udp_create_socket,
        };
        use wasmtime_wasi::sockets::WasiSocketsView;

        for network_enabled in [false, true] {
            let network = if network_enabled {
                GuestNetwork::PublicInternet
            } else {
                GuestNetwork::Off
            };
            let exec_dir = tempfile::tempdir().unwrap();
            let mut wasi = build_sandbox_wasi_context(
                exec_dir.path(),
                MemoryOutputPipe::new(1024),
                MemoryOutputPipe::new(1024),
                &network,
                &GuestSpec::python_cli(),
            )
            .unwrap();
            let mut sockets = wasi.sockets();
            let tcp =
                tcp_create_socket::Host::create_tcp_socket(&mut sockets, IpAddressFamily::Ipv4);
            let udp =
                udp_create_socket::Host::create_udp_socket(&mut sockets, IpAddressFamily::Ipv4)
                    .await;
            if network_enabled {
                tcp.expect("Python's enabled network option should allow TCP");
                udp.expect("Python's enabled network option should allow UDP");
            } else {
                assert_eq!(
                    tcp.unwrap_err().downcast().unwrap(),
                    ErrorCode::AccessDenied
                );
                assert_eq!(
                    udp.unwrap_err().downcast().unwrap(),
                    ErrorCode::AccessDenied
                );
            }
            let network = instance_network::Host::instance_network(&mut sockets).unwrap();
            let dns =
                ip_name_lookup::Host::resolve_addresses(&mut sockets, network, "localhost".into());
            assert_eq!(
                dns.unwrap_err().downcast().unwrap(),
                ErrorCode::PermanentResolverFailure
            );
        }
    }

    #[test]
    fn preview1_guest_cannot_see_the_host_environment() {
        assert!(
            std::env::var_os("PATH").is_some(),
            "the test host needs a non-empty environment sentinel"
        );

        let wasm = wat::parse_str(
            r#"
                (module
                    (import "wasi_snapshot_preview1" "environ_sizes_get"
                        (func $environ_sizes_get (param i32 i32) (result i32)))
                    (memory (export "memory") 1)
                    (data (i32.const 0) "\ff\ff\ff\ff\ff\ff\ff\ff")
                    (func (export "environment_sizes") (result i32 i32 i32)
                        (local $errno i32)
                        i32.const 0
                        i32.const 4
                        call $environ_sizes_get
                        local.set $errno
                        local.get $errno
                        i32.const 0
                        i32.load
                        i32.const 4
                        i32.load))
            "#,
        )
        .expect("environment probe should compile from WAT");

        let engine = wasmtime::Engine::default();
        let module = Module::new(&engine, wasm).expect("environment probe should compile");
        let exec_dir = tempfile::tempdir().expect("sandbox directory should be created");
        let wasi = build_sandbox_wasi_context(
            exec_dir.path(),
            MemoryOutputPipe::new(1024),
            MemoryOutputPipe::new(1024),
            &GuestNetwork::Off,
            &GuestSpec::python_cli(),
        )
        .expect("sandbox WASI context should build");

        let mut linker: Linker<StoreData> = Linker::new(&engine);
        p1::add_to_linker_sync(&mut linker, |data: &mut StoreData| &mut data.wasi)
            .expect("WASI Preview 1 should link");
        let mut store = Store::new(
            &engine,
            StoreData {
                wasi,
                limiter: MemoryLimiter {
                    max_bytes: usize::MAX,
                },
            },
        );
        let instance = linker
            .instantiate(&mut store, &module)
            .expect("environment probe should instantiate");
        let environment_sizes = instance
            .get_typed_func::<(), (i32, i32, i32)>(&mut store, "environment_sizes")
            .expect("environment probe export should exist")
            .call(&mut store, ())
            .expect("environment probe should run");

        assert_eq!(
            environment_sizes,
            (0, 0, 0),
            "Python WASM inherited variables from the credential-bearing host process (errno, count, bytes)"
        );
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Canonical object key for `relative` under the workspace `prefix`. Both may
/// be raw names or keys that came out of a list response.
fn workspace_object_path(prefix: &str, relative: &str) -> ObjPath {
    join_object_path(&normalize_object_path(prefix), relative)
}

/// List all object paths under `prefix` and return them as relative strings.
/// The entries are canonical (percent-encoded) keys, which `Workspace.get()`
/// sends back verbatim. Returns an empty list on error (workspace simply
/// appears empty to Python).
pub async fn list_workspace_files(store: &Arc<dyn ObjectStore>, prefix: &str) -> Vec<String> {
    let obj_prefix = normalize_object_path(prefix);
    let mut stream = store.list(Some(&obj_prefix));
    let mut manifest = Vec::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(meta) => {
                let full = meta.location.as_ref();
                let relative = full
                    .strip_prefix(obj_prefix.as_ref())
                    .unwrap_or(full)
                    .trim_start_matches('/');
                if !relative.is_empty() {
                    manifest.push(relative.to_string());
                }
            }
            Err(e) => warn!("workspace list error: {}", e),
        }
    }

    manifest
}
