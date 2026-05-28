use crate::abi::{WasmExecutionInput, WasmExecutionResult, WasmNodeDefinition};
use crate::component::linker::{register_component_host_functions, ComponentStoreData};
use crate::component::WasmComponent;
use crate::engine::WasmEngine;
use crate::error::{WasmError, WasmResult};
use crate::host_functions::HostState;
use crate::limits::WasmSecurityConfig;
use std::sync::Arc;
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};
use wasmtime::component::{Instance, Linker};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{DirPerms, FilePerms};

pub struct WasmComponentInstance {
    engine: Engine,
    store: Store<ComponentStoreData>,
    instance: Instance,
    component: Arc<WasmComponent>,
    fuel_limit: u64,
}

impl WasmComponentInstance {
    pub async fn new(
        engine: &WasmEngine,
        component: Arc<WasmComponent>,
        security: WasmSecurityConfig,
    ) -> WasmResult<Self> {
        // Use the engine that compiled/deserialized this component to ensure
        // Store, Linker, and Component are all tied to the same Engine instance.
        let component_engine = component.component().engine();

        let mut linker: Linker<ComponentStoreData> = Linker::new(component_engine);
        register_component_host_functions(&mut linker)?;

        let mut store = Store::new(component_engine, ComponentStoreData::new(&security));

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

        let instance = linker
            .instantiate_async(&mut store, component.component())
            .await
            .map_err(|e| {
                WasmError::instantiation(format!("Failed to instantiate component: {}", e))
            })?;

        Ok(Self {
            engine: component_engine.clone(),
            store,
            instance,
            component,
            fuel_limit,
        })
    }

    async fn run_cli_component(
        &mut self,
        args: &[&str],
        stdin: Option<&str>,
    ) -> WasmResult<String> {
        let mut linker: Linker<ComponentStoreData> = Linker::new(&self.engine);
        register_component_host_functions(&mut linker)?;

        const MAX_OUTPUT_SIZE: usize = 10 << 20;
        let stdout = MemoryOutputPipe::new(MAX_OUTPUT_SIZE);
        let stderr = MemoryOutputPipe::new(MAX_OUTPUT_SIZE);

        let mut builder = wasmtime_wasi::WasiCtxBuilder::new();
        builder.stdout(stdout.clone()).stderr(stderr.clone());
        builder.inherit_network();
        builder.allow_ip_name_lookup(true);
        if let Some(stdin_text) = stdin {
            builder.stdin(MemoryInputPipe::new(stdin_text.as_bytes().to_vec()));
        }

        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push("flow-like-wasm-node");
        argv.extend_from_slice(args);
        builder.args(&argv);
        builder
            .preopened_dir(".", ".", DirPerms::all(), FilePerms::all())
            .map_err(|e| {
                WasmError::execution("wasi:cli/run", format!("Failed to preopen cwd: {}", e))
            })?;

        let mut store = Store::new(
            &self.engine,
            ComponentStoreData {
                host_state: HostState::new(self.store.data().host_state.capabilities),
                wasi_ctx: builder.build(),
                http_ctx: wasmtime_wasi_http::WasiHttpCtx::new(),
                resource_table: wasmtime::component::ResourceTable::new(),
            },
        );

        let command = wasmtime_wasi::p2::bindings::Command::instantiate_async(
            &mut store,
            self.component.component(),
            &linker,
        )
        .await
        .map_err(|e| WasmError::instantiation(format!("Failed to instantiate command: {}", e)))?;

        let run_result = command
            .wasi_cli_run()
            .call_run(&mut store)
            .await
            .map_err(|e| {
                let stderr_text = String::from_utf8_lossy(&stderr.contents()).to_string();
                let stdout_text = String::from_utf8_lossy(&stdout.contents()).to_string();
                WasmError::execution(
                    "wasi:cli/run",
                    format!(
                        "Call failed: {}. stdout='{}' stderr='{}'",
                        e, stdout_text, stderr_text
                    ),
                )
            })?;

        if run_result.is_err() {
            let stderr_bytes = stderr.contents();
            let stderr_text = String::from_utf8_lossy(&stderr_bytes).to_string();
            return Err(WasmError::execution(
                "wasi:cli/run",
                format!("CLI component returned error. stderr: {}", stderr_text),
            ));
        }

        let stdout_bytes = stdout.contents();
        Ok(String::from_utf8_lossy(&stdout_bytes).trim().to_string())
    }

    async fn run_cli_component_external(&mut self, args: &[&str]) -> WasmResult<String> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| WasmError::Internal(format!("System time error: {}", e)))?
            .as_nanos();
        let temp_path = std::env::temp_dir().join(format!(
            "flow-like-component-{}-{}.wasm",
            self.component.hash(),
            timestamp
        ));

        fs::write(&temp_path, self.component.bytes()).map_err(|e| {
            WasmError::execution(
                "wasi:cli/run",
                format!("Failed to write temp component file: {}", e),
            )
        })?;

        let mut cmd = Command::new("wasmtime");
        cmd.arg("run")
            .arg("-S")
            .arg("http")
            .arg(&temp_path)
            .arg("--");
        for arg in args {
            cmd.arg(arg);
        }

        let output = cmd.output().map_err(|e| {
            let _ = fs::remove_file(&temp_path);
            WasmError::execution(
                "wasi:cli/run",
                format!("Failed to execute wasmtime CLI fallback: {}", e),
            )
        })?;

        let _ = fs::remove_file(&temp_path);

        if !output.status.success() {
            return Err(WasmError::execution(
                "wasi:cli/run",
                format!(
                    "wasmtime CLI fallback failed with status {:?}: {}",
                    output.status.code(),
                    String::from_utf8_lossy(&output.stderr)
                ),
            ));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    pub async fn call_get_node(&mut self) -> WasmResult<WasmNodeDefinition> {
        let nodes = self.call_get_nodes().await?;
        nodes.into_iter().next().ok_or_else(|| {
            WasmError::invalid_node_definition("Empty node list from component".to_string())
        })
    }

    pub async fn call_get_nodes(&mut self) -> WasmResult<Vec<WasmNodeDefinition>> {
        let (func_name, func) = if let Ok(get_nodes) = self
            .instance
            .get_typed_func::<(), (String,)>(&mut self.store, "get-nodes")
        {
            ("get-nodes", get_nodes)
        } else if let Ok(get_node) = self
            .instance
            .get_typed_func::<(), (String,)>(&mut self.store, "get-node")
        {
            ("get-node", get_node)
        } else {
            let json_str = match self.run_cli_component(&["get-node"], None).await {
                Ok(value) => value,
                Err(in_process_err) => {
                    tracing::debug!("In-process CLI component failed: {in_process_err}, trying external wasmtime");
                    self.run_cli_component_external(&["get-node"]).await?
                }
            };
            if let Ok(defs) = serde_json::from_str::<Vec<WasmNodeDefinition>>(&json_str) {
                return Ok(defs);
            }
            let def: WasmNodeDefinition = serde_json::from_str(&json_str)
                .map_err(|e| WasmError::invalid_node_definition(format!("Invalid JSON: {}", e)))?;
            return Ok(vec![def]);
        };

        let (json_str,) = func
            .call_async(&mut self.store, ())
            .await
            .map_err(|e| WasmError::execution(func_name, format!("Call failed: {}", e)))?;

        // Try parsing as array first (multi-node), fall back to single object
        if let Ok(defs) = serde_json::from_str::<Vec<WasmNodeDefinition>>(&json_str) {
            return Ok(defs);
        }
        let def: WasmNodeDefinition = serde_json::from_str(&json_str)
            .map_err(|e| WasmError::invalid_node_definition(format!("Invalid JSON: {}", e)))?;
        Ok(vec![def])
    }

    pub async fn call_get_abi_version(&mut self) -> WasmResult<u32> {
        let func = self
            .instance
            .get_typed_func::<(), (u32,)>(&mut self.store, "get-abi-version")
            .map_err(|e| WasmError::execution("get-abi-version", format!("Not found: {}", e)))?;

        let (version,) = func
            .call_async(&mut self.store, ())
            .await
            .map_err(|e| WasmError::execution("get-abi-version", format!("Call failed: {}", e)))?;

        Ok(version)
    }

    pub async fn call_run(
        &mut self,
        input: &WasmExecutionInput,
    ) -> WasmResult<WasmExecutionResult> {
        let input_json = serde_json::to_string(input).map_err(WasmError::Json)?;
        let func = match self
            .instance
            .get_typed_func::<(String,), (String,)>(&mut self.store, "run")
        {
            Ok(func) => func,
            Err(_) => {
                let encoded_input =
                    base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &input_json);

                let in_process = self
                    .run_cli_component(&["run-b64", &encoded_input], None)
                    .await;

                let result_json = match in_process {
                    Ok(value) => value,
                    Err(in_process_err) => {
                        tracing::debug!(
                            "In-process CLI run failed: {in_process_err}, trying external wasmtime"
                        );
                        self.run_cli_component_external(&["run-b64", &encoded_input])
                            .await
                            .map_err(|e| {
                                WasmError::execution("run", format!("CLI fallback failed: {}", e))
                            })?
                    }
                };

                return serde_json::from_str(&result_json).map_err(|e| {
                    WasmError::execution("run", format!("Invalid JSON result: {}", e))
                });
            }
        };

        let (result_json,) = func
            .call_async(&mut self.store, (input_json,))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("all fuel consumed") {
                    return WasmError::OutOfFuel {
                        limit: self.fuel_limit,
                    };
                }
                if msg.contains("epoch deadline") || msg.contains("interrupt") {
                    return WasmError::Timeout { duration_ms: 0 };
                }
                WasmError::execution("run", format!("Call failed: {}", e))
            })?;

        serde_json::from_str(&result_json)
            .map_err(|e| WasmError::execution("run", format!("Invalid JSON result: {}", e)))
    }

    pub fn host_state(&self) -> &HostState {
        &self.store.data().host_state
    }

    pub fn host_state_mut(&mut self) -> &mut HostState {
        &mut self.store.data_mut().host_state
    }

    pub fn remaining_fuel(&self) -> Option<u64> {
        self.store.get_fuel().ok()
    }
}

impl std::fmt::Debug for WasmComponentInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmComponentInstance")
            .field("component_hash", &self.component.hash())
            .field("remaining_fuel", &self.remaining_fuel())
            .finish()
    }
}
