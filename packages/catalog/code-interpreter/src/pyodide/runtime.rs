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

use std::path::PathBuf;
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
    DirPerms, FilePerms, WasiCtxBuilder,
    pipe::MemoryOutputPipe,
    preview1::{self, WasiP1Ctx},
};

use flow_like_storage::{
    Path as ObjPath,
    object_store::{ObjectStore, PutPayload},
};
use flow_like_types::Bytes;
use flow_like_wasm::{AotCache, WasmConfig, WasmEngine};

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
const BOOTSTRAP_SCRIPT: &str = include_str!("bootstrap.py");

// ─── Public configuration ─────────────────────────────────────────────────────

/// Configuration for [`PyodideRuntime`].
#[derive(Debug, Clone, Default)]
pub struct RuntimeConfig {
    /// Explicit path to a WASI-compatible Python WASM binary.
    /// When `None` the runtime probes the standard locations listed in
    /// [`probe_wasm_path`].
    pub wasm_binary_path: Option<PathBuf>,
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
struct BootstrapResult {
    outputs: Value,
    stdout: String,
    stderr: String,
    error: Option<String>,
    success: bool,
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
    ) -> Result<bool> {
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

    fn table_growing(&mut self, _: u32, _: u32, _: Option<u32>) -> Result<bool> {
        Ok(true)
    }
}

// ─── Store data ───────────────────────────────────────────────────────────────

/// Per-execution state stored in the Wasmtime `Store`.
struct StoreData {
    wasi: WasiP1Ctx,
    limiter: MemoryLimiter,
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
async fn workspace_file_server(
    store: Arc<dyn ObjectStore>,
    prefix: String,
    ws_pending_dir: PathBuf,
    ws_data_dir: PathBuf,
    ws_notfound_dir: PathBuf,
) {
    let mut interval =
        tokio::time::interval(Duration::from_millis(WS_SERVER_POLL_MS));
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

            // Build the full object-store path.
            let store_path_str = if prefix.is_empty() {
                safe_relative.clone()
            } else {
                format!("{}/{}", prefix.trim_end_matches('/'), safe_relative)
            };
            let obj_path = ObjPath::from(store_path_str.as_str());

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
                            warn!("workspace: failed to read bytes for '{}': {}", safe_relative, e);
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
async fn upload_ws_puts(
    ws_puts_dir: &PathBuf,
    store: &Arc<dyn ObjectStore>,
    prefix: &str,
) {
    let mut stack = vec![ws_puts_dir.clone()];
    while let Some(dir) = stack.pop() {
        let mut entries = match tokio::fs::read_dir(&dir).await {
            Ok(e) => e,
            Err(_) => continue,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }

            let data = match tokio::fs::read(&path).await {
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

            let store_path_str = if prefix.is_empty() {
                relative.clone()
            } else {
                format!("{}/{}", prefix.trim_end_matches('/'), relative)
            };
            let obj_path = ObjPath::from(store_path_str.as_str());
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
}

impl PyodideRuntime {
    /// Create a new runtime instance.
    pub fn new(config: RuntimeConfig) -> Result<Self> {
        // Disable fuel metering — we use epoch-based timeouts exclusively.
        let wasm_config = WasmConfig {
            fuel_metering: false,
            ..WasmConfig::production()
        };
        let aot_cache = wasm_config.cache_dir.as_ref().map(AotCache::new);

        let engine = WasmEngine::new(wasm_config).context("create WasmEngine")?;
        // start_epoch_ticker uses tokio::spawn — valid because `new` is always
        // called from an async context (inside `do_run`).
        engine.start_epoch_ticker();

        Ok(Self {
            engine: Arc::new(engine),
            module: Arc::new(RwLock::new(None)),
            aot_cache,
            config,
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
    /// Compilation order: in-memory → AOT disk cache → full Cranelift.
    async fn get_module(&self) -> Result<Module> {
        {
            let guard = self.module.read().await;
            if let Some(m) = guard.as_ref() {
                return Ok(m.clone());
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

        let hash = blake3::hash(&bytes).to_hex().to_string();
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
                info!("Python WASM compiled and saved to AOT cache (hash: {}…)", &hash[..8]);
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
            let ws_pending  = exec_path.join("ws_pending");
            let ws_data     = exec_path.join("ws_data");
            let ws_notfound = exec_path.join("ws_notfound");
            let ws_puts     = exec_path.join("ws_puts");

            for dir in &[&ws_pending, &ws_data, &ws_notfound, &ws_puts] {
                tokio::fs::create_dir_all(dir).await.context("create ws dirs")?;
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

        // ── Build WASI context ────────────────────────────────────────────
        let stdout_pipe = MemoryOutputPipe::new(MAX_STDOUT_BYTES);
        let stderr_pipe = MemoryOutputPipe::new(MAX_STDERR_BYTES);

        let mut wb = WasiCtxBuilder::new();
        wb.stdout(stdout_pipe.clone())
            .stderr(stderr_pipe.clone())
            .args(&["python3", "/flow/bootstrap.py"]);

        // Only /flow is preopened — workspace access is via the API, not POSIX I/O.
        wb.preopened_dir(&exec_path, "/flow", DirPerms::all(), FilePerms::all())
            .context("preopened /flow")?;

        if req.network_enabled {
            wb.inherit_network();
        }

        let wasi: WasiP1Ctx = wb.build_p1();

        // ── Create Wasmtime store ─────────────────────────────────────────
        let store_data = StoreData {
            wasi,
            limiter: MemoryLimiter {
                max_bytes: req.memory_limit,
            },
        };
        let mut store = Store::new(self.engine.engine(), store_data);
        store.limiter(|data| &mut data.limiter as &mut dyn wasmtime::ResourceLimiter);

        let timeout_ticks = req.timeout.as_millis() as u64 / EPOCH_TICK_MS + 1;
        store.set_epoch_deadline(timeout_ticks);
        store.epoch_deadline_trap();

        // ── Linker ────────────────────────────────────────────────────────
        let mut linker: Linker<StoreData> = Linker::new(self.engine.engine());
        preview1::add_to_linker_async(&mut linker, |data: &mut StoreData| &mut data.wasi)
            .context("add WASI p1 to linker")?;

        // ── Instantiate + run ─────────────────────────────────────────────
        let instance = linker
            .instantiate_async(&mut store, &module)
            .await
            .context("instantiate Python WASM module")?;

        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .context("Python WASM module must export `_start` (WASI CLI)")?;

        debug!("Calling Python WASM _start (timeout: {:?})", req.timeout);
        let run_result = start.call_async(&mut store, ()).await;

        // ── Shut down workspace file server ───────────────────────────────
        if let Some(handle) = ws_server_handle {
            handle.abort();
        }

        // ── Upload staged puts ────────────────────────────────────────────
        if let Some(ws) = &req.workspace {
            let ws_puts = exec_path.join("ws_puts");
            upload_ws_puts(&ws_puts, &ws.store, &ws.prefix).await;
        }

        // ── Collect pipe output ───────────────────────────────────────────
        let raw_stdout = String::from_utf8_lossy(&stdout_pipe.contents()).into_owned();
        let raw_stderr = String::from_utf8_lossy(&stderr_pipe.contents()).into_owned();

        // ── Interpret the exit status ─────────────────────────────────────
        let runtime_error: Option<String> = match run_result {
            Ok(()) => None,
            Err(trap) => {
                let msg = trap.to_string();
                if msg.contains("exit status 0") || msg.contains("I32Exit(0)") {
                    None
                } else if msg.to_lowercase().contains("epoch") {
                    Some(format!(
                        "Execution timed out after {:.1}s",
                        req.timeout.as_secs_f64()
                    ))
                } else {
                    Some(msg)
                }
            }
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

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// List all object paths under `prefix` and return them as relative strings.
/// Returns an empty list on error (workspace simply appears empty to Python).
async fn list_workspace_files(store: &Arc<dyn ObjectStore>, prefix: &str) -> Vec<String> {
    let obj_prefix = ObjPath::from(prefix.trim_end_matches('/'));
    let mut stream = store.list(Some(&obj_prefix));
    let mut manifest = Vec::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(meta) => {
                let full = meta.location.as_ref();
                let relative = full
                    .strip_prefix(prefix.trim_end_matches('/'))
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
