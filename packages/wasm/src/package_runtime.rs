//! Package instances and host resources whose lifetime is bounded by one run.

use crate::abi::{WasmExecutionInput, WasmExecutionResult};
use crate::engine::WasmEngine;
use crate::error::{WasmError, WasmResult};
use crate::host_functions::{HostState, LogEntry, StreamEvent, WebSocketResources};
use crate::limits::{WasmCapabilities, WasmSecurityConfig};
use crate::unified::{LoadedWasm, UnifiedInstance};
use async_trait::async_trait;
use flow_like::flow::execution::resources::RunResource;
use parking_lot::RwLock;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{watch, Mutex};

/// The key is used only inside a run's registry. Host capabilities are checked
/// for each invocation. WASI grants remain part of the instance boundary because
/// its linker and socket resource table are configured at instantiation.
pub fn package_runtime_key(
    package_id: &str,
    artifact_hash: &str,
    security: &WasmSecurityConfig,
    principal: &str,
    shadow: bool,
) -> WasmResult<String> {
    let mut domain = security.clone();
    domain.capabilities &= WasmCapabilities::HTTP_ALL
        | WasmCapabilities::TCP
        | WasmCapabilities::UDP
        | WasmCapabilities::DNS;
    let identity = serde_json::to_vec(&(package_id, artifact_hash, domain, principal, shadow))
        .map_err(WasmError::Json)?;
    Ok(format!("wasm-package:{}", blake3::hash(&identity).to_hex()))
}

#[derive(Debug)]
pub struct PackageCallResult {
    pub result: WasmExecutionResult,
    pub logs: Vec<LogEntry>,
    pub events: Vec<StreamEvent>,
    pub error: Option<String>,
}

/// All callers enter the same instance sequentially. The registry belongs to
/// RunContext, so no process-wide cache can keep this state across runs.
pub struct PackageRuntime {
    instance: Mutex<Option<UnifiedInstance>>,
    cache: Arc<RwLock<HashMap<String, Value>>>,
    websocket: Arc<WebSocketResources>,
    closed: watch::Sender<bool>,
}

impl Default for PackageRuntime {
    fn default() -> Self {
        Self {
            instance: Mutex::new(None),
            cache: Arc::new(RwLock::new(HashMap::new())),
            websocket: Arc::new(WebSocketResources::default()),
            closed: watch::channel(false).0,
        }
    }
}

impl PackageRuntime {
    pub async fn call(
        &self,
        loaded: &LoadedWasm,
        engine: &WasmEngine,
        security: &WasmSecurityConfig,
        mut host_state: HostState,
        input: &WasmExecutionInput,
    ) -> WasmResult<PackageCallResult> {
        let mut cancelled = self.closed.subscribe();
        if *cancelled.borrow() {
            return Err(closed_error());
        }

        // Dropping an in-flight call can leave guest state partially updated.
        // Invalidate the entire package instead of reentering that instance.
        let mut guard = AbortCallOnDrop {
            runtime: self,
            armed: true,
        };
        host_state.cache = self.cache.clone();
        host_state.websocket = self.websocket.clone();
        host_state.run_scoped = true;
        let operation = async {
            let mut slot = self.instance.lock().await;
            if *self.closed.borrow() {
                return Err(closed_error());
            }
            if let Some(instance) = slot.as_mut() {
                instance.prepare_call(engine, security, host_state)?;
            } else {
                let mut instance = loaded
                    .instantiate_with_host_state(engine, security.clone(), host_state)
                    .await?;
                // Initialization has its own fuel allowance. Each actual node
                // call receives the full budget, including the first call.
                let state = std::mem::replace(
                    instance.host_state_mut(),
                    HostState::with_security(security),
                );
                instance.prepare_call(engine, security, state)?;
                *slot = Some(instance);
            }
            let instance = slot.as_mut().expect("package instance initialized");
            let result = instance.call_run(input).await?;
            let host = instance.host_state();
            Ok(PackageCallResult {
                result,
                logs: host.get_logs(),
                events: host.take_stream_events(),
                error: host.get_error(),
            })
        };

        let result = tokio::select! {
            biased;
            _ = cancelled.wait_for(|closed| *closed) => Err(closed_error()),
            result = tokio::time::timeout(security.limits.timeout, operation) => {
                result.unwrap_or_else(|_| Err(WasmError::Timeout {
                    duration_ms: security.limits.timeout.as_millis().min(u64::MAX as u128) as u64,
                }))
            }
        };
        if result.is_err() {
            self.abort();
        }
        guard.armed = false;
        result
    }
}

#[async_trait]
impl RunResource for PackageRuntime {
    fn abort(&self) {
        self.closed.send_replace(true);
        self.websocket.cancel();
        self.cache.write().clear();
        if let Ok(mut instance) = self.instance.try_lock() {
            instance.take();
        }
    }

    async fn shutdown(&self) {
        self.abort();
        self.websocket.shutdown().await;
        self.instance.lock().await.take();
    }
}

impl Drop for PackageRuntime {
    fn drop(&mut self) {
        self.abort();
    }
}

struct AbortCallOnDrop<'a> {
    runtime: &'a PackageRuntime,
    armed: bool,
}

impl Drop for AbortCallOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.runtime.abort();
        }
    }
}

fn closed_error() -> WasmError {
    WasmError::execution("run", "The package runtime is closed for this run")
}
