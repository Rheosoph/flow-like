use crate::abi::{WasmExecutionInput, WasmExecutionResult, WasmNodeDefinition};
use crate::component::linker::{
    configure_guest_network, register_component_host_functions, ComponentStoreData,
};
use crate::component::WasmComponent;
use crate::engine::WasmEngine;
use crate::error::{WasmError, WasmResult};
use crate::host_functions::HostState;
use crate::limits::{WasmCapabilities, WasmSecurityConfig};
use crate::wasi::isolated_wasi_ctx_builder;
use std::sync::Arc;
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};
use wasmtime::component::{Instance, Linker};
use wasmtime::{Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};

pub struct WasmComponentInstance {
    engine: Engine,
    store: Store<ComponentStoreData>,
    instance: Instance,
    component: Arc<WasmComponent>,
    fuel_limit: u64,
    security: WasmSecurityConfig,
}

fn is_executable_file(candidate: &Path) -> bool {
    let Ok(metadata) = candidate.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn resolve_wasmtime_executable() -> WasmResult<PathBuf> {
    let host_path = std::env::var_os("PATH").ok_or_else(|| {
        WasmError::execution(
            "wasi:cli/run",
            "Cannot locate the Wasmtime CLI because the host PATH is unset",
        )
    })?;
    let executable_name = if cfg!(windows) {
        "wasmtime.exe"
    } else {
        "wasmtime"
    };

    std::env::split_paths(&host_path)
        .find_map(|directory| {
            let candidate = directory.join(executable_name);
            is_executable_file(&candidate).then(|| candidate.canonicalize().ok())?
        })
        .ok_or_else(|| {
            WasmError::execution(
                "wasi:cli/run",
                "Cannot locate the Wasmtime CLI in the host PATH",
            )
        })
}

fn isolated_child_command(executable: &Path) -> Command {
    let mut command = Command::new(executable);
    command.env_clear().stdin(Stdio::null());
    command
}

fn external_cli_security_args(security: &WasmSecurityConfig) -> WasmResult<Vec<String>> {
    let caps = security.capabilities;
    let has_network = security.allow_wasi_network
        || caps.intersects(
            WasmCapabilities::HTTP_ALL
                | WasmCapabilities::TCP
                | WasmCapabilities::UDP
                | WasmCapabilities::DNS,
        );

    if has_network && security.allowed_hosts.is_some() {
        return Err(WasmError::execution(
            "wasi:cli/run",
            "External Wasmtime fallback cannot enforce a network host allowlist",
        ));
    }

    let mut args = vec!["run".to_string()];

    // The external Wasmtime HTTP implementation cannot enforce Flow-Like's
    // per-method permissions, so only expose it for a full HTTP grant.
    if caps.contains(WasmCapabilities::HTTP_ALL) {
        args.extend(["-S".to_string(), "http=y".to_string()]);
    }

    let has_socket_network = security.allow_wasi_network
        || caps.intersects(WasmCapabilities::TCP | WasmCapabilities::UDP | WasmCapabilities::DNS);
    if has_socket_network {
        args.extend(["-S".to_string(), "inherit-network=y".to_string()]);
        for (option, enabled) in [
            (
                "allow-ip-name-lookup",
                security.allow_wasi_network || caps.intersects(WasmCapabilities::DNS),
            ),
            (
                "tcp",
                security.allow_wasi_network || caps.intersects(WasmCapabilities::TCP),
            ),
            (
                "udp",
                security.allow_wasi_network || caps.intersects(WasmCapabilities::UDP),
            ),
        ] {
            args.extend([
                "-S".to_string(),
                format!("{option}={}", if enabled { "y" } else { "n" }),
            ]);
        }
    }

    Ok(args)
}

fn allows_external_cli_fallback(security: &WasmSecurityConfig) -> bool {
    // The external process is outside this store's fuel, epoch, and memory
    // limiter. Never use it for restrictive/untrusted metadata extraction.
    security.allow_wasi
}

fn cli_child_host_state(parent: &HostState) -> HostState {
    let mut child = HostState::new(parent.capabilities);
    child.model_context = parent.model_context.clone();
    child.model_usage_context = parent.model_usage_context.clone();
    // The child runs where the parent runs; the egress policy keys off this.
    child.metadata.execution_environment = parent.metadata.execution_environment;
    child
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
        register_component_host_functions(&mut linker, &security)?;

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
            security,
        })
    }

    async fn run_cli_component(
        &mut self,
        args: &[&str],
        stdin: Option<&str>,
    ) -> WasmResult<String> {
        let mut linker: Linker<ComponentStoreData> = Linker::new(&self.engine);
        register_component_host_functions(&mut linker, &self.security)?;

        const MAX_OUTPUT_SIZE: usize = 10 << 20;
        let stdout = MemoryOutputPipe::new(MAX_OUTPUT_SIZE);
        let stderr = MemoryOutputPipe::new(MAX_OUTPUT_SIZE);

        let mut builder = isolated_wasi_ctx_builder();
        builder.stdout(stdout.clone()).stderr(stderr.clone());
        configure_guest_network(&mut builder, &self.security);
        if let Some(stdin_text) = stdin {
            builder.stdin(MemoryInputPipe::new(stdin_text.as_bytes().to_vec()));
        }

        let mut argv = Vec::with_capacity(args.len() + 1);
        argv.push("flow-like-wasm-node");
        argv.extend_from_slice(args);
        builder.args(&argv);

        let child_host_state = cli_child_host_state(&self.store.data().host_state);
        let mut store = Store::new(
            &self.engine,
            ComponentStoreData::with_host_state(child_host_state, builder.build(), &self.security),
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
        let cli_args = external_cli_security_args(&self.security)?;
        let wasmtime_executable = resolve_wasmtime_executable()?;
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

        let mut cmd = isolated_child_command(&wasmtime_executable);
        cmd.args(cli_args);
        cmd.arg(&temp_path).arg("--");
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
                Err(in_process_err) if allows_external_cli_fallback(&self.security) => {
                    tracing::debug!(
                        "In-process CLI component failed: {in_process_err}, trying external wasmtime"
                    );
                    self.run_cli_component_external(&["get-node"]).await?
                }
                Err(in_process_err) => return Err(in_process_err),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_functions::ModelContext;
    use flow_like::models::llm::ModelUsageContext;
    use flow_like::state::{FlowLikeConfig, FlowLikeState};
    use flow_like::utils::http::HTTPClient;

    #[test]
    fn restrictive_external_cli_args_have_no_ambient_access() {
        let security = WasmSecurityConfig::restrictive();
        let args = external_cli_security_args(&security).unwrap();

        assert!(!allows_external_cli_fallback(&security));

        for denied in [
            "http=y",
            "inherit-network=y",
            "allow-ip-name-lookup=y",
            "tcp=y",
            "udp=y",
            "--dir",
        ] {
            assert!(!args.iter().any(|arg| arg == denied), "unexpected {denied}");
        }
    }

    #[test]
    fn permissive_external_cli_args_grant_network_without_environment() {
        let security = WasmSecurityConfig::permissive();
        let args = external_cli_security_args(&security).unwrap();

        assert!(allows_external_cli_fallback(&security));

        for granted in [
            "http=y",
            "inherit-network=y",
            "allow-ip-name-lookup=y",
            "tcp=y",
            "udp=y",
        ] {
            assert!(args.iter().any(|arg| arg == granted), "missing {granted}");
        }
        assert!(!args.iter().any(|arg| arg == "--env"));
        let forbidden_env_arg = ["inherit", "env=y"].join("-");
        assert!(!args.iter().any(|arg| arg == &forbidden_env_arg));
    }

    #[test]
    fn raw_socket_permission_does_not_grant_http() {
        let security = WasmSecurityConfig::default()
            .with_capabilities(WasmCapabilities::TCP | WasmCapabilities::DNS);
        let args = external_cli_security_args(&security).unwrap();

        assert!(args.iter().any(|arg| arg == "inherit-network=y"));
        assert!(args.iter().any(|arg| arg == "tcp=y"));
        assert!(args.iter().any(|arg| arg == "allow-ip-name-lookup=y"));
        assert!(!args.iter().any(|arg| arg == "http=y"));
        assert!(args.iter().any(|arg| arg == "udp=n"));
    }

    #[test]
    fn partial_http_permission_does_not_grant_unrestricted_wasi_http() {
        let security = WasmSecurityConfig::default().with_capabilities(WasmCapabilities::HTTP_GET);
        let args = external_cli_security_args(&security).unwrap();

        assert!(!args.iter().any(|arg| arg == "http=y"));
    }

    #[cfg(unix)]
    #[test]
    fn external_cli_child_process_receives_no_host_environment() {
        let output = isolated_child_command(Path::new("/usr/bin/env"))
            .output()
            .expect("the environment probe should run");

        assert!(output.status.success());
        assert!(
            output.stdout.is_empty(),
            "external Wasmtime child inherited host variables: {}",
            String::from_utf8_lossy(&output.stdout)
        );
    }

    #[cfg(unix)]
    #[test]
    fn external_cli_resolver_rejects_non_executable_files() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("test directory should be created");
        let candidate = directory.path().join("wasmtime");
        std::fs::write(&candidate, b"test").expect("test candidate should be written");

        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o644))
            .expect("test permissions should be set");
        assert!(!is_executable_file(&candidate));

        std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o755))
            .expect("test permissions should be set");
        assert!(is_executable_file(&candidate));
    }

    #[test]
    fn external_cli_fails_closed_when_network_allowlist_cannot_be_enforced() {
        let security = WasmSecurityConfig::restrictive()
            .with_capabilities(WasmCapabilities::HTTP_GET)
            .with_allowed_hosts(vec!["127.0.0.1".to_string()]);

        assert!(external_cli_security_args(&security).is_err());
    }

    #[test]
    fn cli_child_keeps_hosted_model_usage_attribution() {
        let mut parent = HostState::new(WasmCapabilities::MODELS);
        parent.model_usage_context = Some(ModelUsageContext {
            app_id: Some("app-1".to_string()),
            run_id: Some("run-1".to_string()),
            api_base_url: None,
        });
        parent.model_context = Some(ModelContext {
            app_state: Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            )),
            token: Some("token".to_string()),
            cache: None,
        });

        let child = cli_child_host_state(&parent);

        assert_eq!(child.model_usage_context, parent.model_usage_context);
        assert_eq!(
            child
                .model_context
                .as_ref()
                .and_then(|context| context.token.as_deref()),
            Some("token")
        );
        assert!(child.has_capability(WasmCapabilities::MODELS));
    }
}
