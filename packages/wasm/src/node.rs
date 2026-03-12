//! WASM Node Logic implementation
//!
//! Bridges WASM modules to the Flow-Like NodeLogic trait.

use crate::abi::{WasmExecutionInput, WasmNodeDefinition, WasmPinDefinition};
use crate::engine::WasmEngine;
use crate::error::WasmResult;
use crate::host_functions::{ExecutionMetadata, ModelContext, StorageContext};
use crate::limits::WasmSecurityConfig;
use crate::module::WasmModule;
use crate::unified::LoadedWasm;
use async_trait::async_trait;
use flow_like::flow::execution::context::ExecutionContext;
use flow_like::flow::execution::{LogLevel, Run};
use flow_like::flow::node::{Node, NodeLogic, NodeScores, NodeWasm};
use flow_like::flow::pin::{Pin, PinOptions, PinType, ValueType};
use flow_like::flow::variable::VariableType;
use flow_like_types::{sync::Mutex, tokio::sync::RwLock, Value};
use parking_lot::RwLock as ParkingRwLock;
use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::Arc;

pub struct WasmNodeLogic {
    loaded: LoadedWasm,
    engine: Arc<WasmEngine>,
    security: WasmSecurityConfig,
    /// For multi-node packages: target a specific node by name
    target_node_name: Option<String>,
    cached_definition: RwLock<Option<WasmNodeDefinition>>,
    /// Registry package ID for this external node
    package_id: Option<String>,
}

impl WasmNodeLogic {
    pub fn new(
        module: Arc<WasmModule>,
        engine: Arc<WasmEngine>,
        security: WasmSecurityConfig,
    ) -> Self {
        Self {
            loaded: LoadedWasm::Module(module),
            engine,
            security,
            target_node_name: None,
            cached_definition: RwLock::new(None),
            package_id: None,
        }
    }

    pub fn from_loaded(
        loaded: LoadedWasm,
        engine: Arc<WasmEngine>,
        security: WasmSecurityConfig,
    ) -> Self {
        Self {
            loaded,
            engine,
            security,
            target_node_name: None,
            cached_definition: RwLock::new(None),
            package_id: None,
        }
    }

    pub fn with_target_node(
        module: Arc<WasmModule>,
        engine: Arc<WasmEngine>,
        security: WasmSecurityConfig,
        definition: WasmNodeDefinition,
    ) -> Self {
        let target_name = definition.name.clone();
        Self {
            loaded: LoadedWasm::Module(module),
            engine,
            security,
            target_node_name: Some(target_name),
            cached_definition: RwLock::new(Some(definition)),
            package_id: None,
        }
    }

    pub fn from_loaded_with_target(
        loaded: LoadedWasm,
        engine: Arc<WasmEngine>,
        security: WasmSecurityConfig,
        definition: WasmNodeDefinition,
    ) -> Self {
        let target_name = definition.name.clone();
        Self {
            loaded,
            engine,
            security,
            target_node_name: Some(target_name),
            cached_definition: RwLock::new(Some(definition)),
            package_id: None,
        }
    }

    pub fn with_package_id(mut self, package_id: String) -> Self {
        self.package_id = Some(package_id);
        self
    }

    async fn get_definition(&self) -> WasmResult<WasmNodeDefinition> {
        {
            let cached = self.cached_definition.read().await;
            if let Some(def) = cached.as_ref() {
                return Ok(def.clone());
            }
        }

        let mut instance = self
            .loaded
            .instantiate(&self.engine, self.security.clone())
            .await?;
        let definitions = instance.call_get_nodes().await?;

        let def = if let Some(ref target) = self.target_node_name {
            definitions
                .into_iter()
                .find(|d| d.name == *target)
                .ok_or_else(|| {
                    crate::error::WasmError::invalid_node_definition(format!(
                        "Node '{}' not found in package",
                        target
                    ))
                })?
        } else {
            definitions.into_iter().next().ok_or_else(|| {
                crate::error::WasmError::invalid_node_definition(
                    "No node definitions found".to_string(),
                )
            })?
        };

        {
            let mut cache = self.cached_definition.write().await;
            *cache = Some(def.clone());
        }

        Ok(def)
    }

    fn to_flow_pin(wasm_pin: &WasmPinDefinition, index: u16) -> Pin {
        let data_type = map_wasm_data_type(&wasm_pin.data_type);
        let pin_type = match wasm_pin.pin_type.to_lowercase().as_str() {
            "output" => PinType::Output,
            _ => PinType::Input,
        };

        let value_type = wasm_pin
            .value_type
            .as_deref()
            .map(|vt| match vt.to_lowercase().as_str() {
                "array" => ValueType::Array,
                "hashmap" => ValueType::HashMap,
                "hashset" => ValueType::HashSet,
                _ => ValueType::Normal,
            })
            .unwrap_or(ValueType::Normal);

        let default_value = wasm_pin
            .default_value
            .as_ref()
            .and_then(|v| flow_like_types::json::to_vec(v).ok());

        let options = {
            let has_any = wasm_pin.valid_values.is_some()
                || wasm_pin.range.is_some()
                || wasm_pin.step.is_some()
                || wasm_pin.sensitive.is_some()
                || wasm_pin.enforce_schema.is_some()
                || wasm_pin.enforce_generic_value_type.is_some();

            if has_any {
                Some(PinOptions {
                    valid_values: wasm_pin.valid_values.clone(),
                    range: wasm_pin.range,
                    step: wasm_pin.step,
                    sensitive: wasm_pin.sensitive,
                    enforce_schema: wasm_pin.enforce_schema,
                    enforce_generic_value_type: wasm_pin.enforce_generic_value_type,
                })
            } else {
                None
            }
        };

        Pin {
            id: flow_like_types::create_id(),
            name: wasm_pin.name.clone(),
            friendly_name: wasm_pin.friendly_name.clone(),
            description: wasm_pin.description.clone(),
            pin_type,
            data_type,
            schema: wasm_pin.schema.clone(),
            value_type,
            depends_on: BTreeSet::new(),
            connected_to: BTreeSet::new(),
            default_value,
            index,
            options,
            value: None,
        }
    }
}

fn map_wasm_data_type(wasm_type: &str) -> VariableType {
    match wasm_type.to_lowercase().as_str() {
        "string" => VariableType::String,
        "int" | "integer" | "i32" | "i64" | "u32" | "u64" => VariableType::Integer,
        "float" | "f32" | "f64" | "number" => VariableType::Float,
        "bool" | "boolean" => VariableType::Boolean,
        "date" | "datetime" => VariableType::Date,
        "path" | "pathbuf" => VariableType::PathBuf,
        "byte" | "bytes" | "binary" => VariableType::Byte,
        "exec" | "execution" => VariableType::Execution,
        "struct" | "object" | "json" => VariableType::Struct,
        _ => VariableType::Generic,
    }
}

/// Build a `Node` from a `WasmNodeDefinition` without requiring async or `block_on`.
pub fn build_node_from_definition(definition: &WasmNodeDefinition) -> Node {
    let mut node = Node::new(
        &definition.name,
        &definition.friendly_name,
        &definition.description,
        &definition.category,
    );

    for (i, wasm_pin) in definition.pins.iter().enumerate() {
        let pin = WasmNodeLogic::to_flow_pin(wasm_pin, i as u16);
        node.pins.insert(pin.id.clone(), pin);
    }

    if let Some(icon) = &definition.icon {
        node.icon = Some(icon.clone());
    }

    if let Some(scores) = &definition.scores {
        node.scores = Some(NodeScores {
            privacy: scores.privacy,
            security: scores.security,
            performance: scores.performance,
            governance: scores.governance,
            reliability: scores.reliability,
            cost: scores.cost,
        });
    }

    if definition.long_running.unwrap_or(false) {
        node.long_running = Some(true);
    }

    if !definition.permissions.is_empty() {
        let wasm = node.wasm.get_or_insert_with(|| NodeWasm {
            package_id: String::new(),
            permissions: Vec::new(),
        });
        wasm.permissions = definition.permissions.clone();
    }

    node
}

#[async_trait]
impl NodeLogic for WasmNodeLogic {
    fn get_node(&self) -> Node {
        let rt = flow_like_types::tokio::runtime::Handle::try_current();

        let definition = if let Ok(handle) = rt {
            flow_like_types::tokio::task::block_in_place(|| {
                handle.block_on(async { self.get_definition().await.ok() })
            })
        } else {
            None
        };

        let definition = definition.unwrap_or_else(|| WasmNodeDefinition {
            name: "wasm_node".to_string(),
            friendly_name: "WASM Node".to_string(),
            description: "A WebAssembly node".to_string(),
            category: "WASM".to_string(),
            pins: vec![],
            icon: None,
            scores: None,
            long_running: None,
            docs: None,
            abi_version: None,
            permissions: vec![],
        });

        let mut node = Node::new(
            &definition.name,
            &definition.friendly_name,
            &definition.description,
            &definition.category,
        );

        for (i, wasm_pin) in definition.pins.iter().enumerate() {
            let pin = Self::to_flow_pin(wasm_pin, i as u16);
            node.pins.insert(pin.id.clone(), pin);
        }

        if let Some(icon) = &definition.icon {
            node.icon = Some(icon.clone());
        }

        if let Some(scores) = &definition.scores {
            node.scores = Some(NodeScores {
                privacy: scores.privacy,
                security: scores.security,
                performance: scores.performance,
                governance: scores.governance,
                reliability: scores.reliability,
                cost: scores.cost,
            });
        }

        if definition.long_running.unwrap_or(false) {
            node.long_running = Some(true);
        }

        if let Some(package_id) = &self.package_id {
            node.wasm = Some(NodeWasm {
                package_id: package_id.clone(),
                permissions: definition.permissions.clone(),
            });
        } else if !definition.permissions.is_empty() {
            let wasm = node.wasm.get_or_insert_with(|| NodeWasm {
                package_id: String::new(),
                permissions: Vec::new(),
            });
            wasm.permissions = definition.permissions.clone();
        }

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Security guard for WASM node execution.
        //  1. No package_id → reject + fatal log (untrusted / manually placed)
        //  2. local:: prefix → locally injected dev node, allow
        //  3. Anything else  → catalog package, user consented via permissions
        match &self.package_id {
            None => {
                context.log_message(
                    "FATAL: WASM node executed without a package_id. \
                     This means the node was not loaded through a trusted catalog path. \
                     Execution has been blocked for safety.",
                    LogLevel::Fatal,
                );
                return Err(flow_like_types::anyhow!(
                    "Execution blocked: WASM node has no package_id. \
                     Only nodes loaded through the trusted catalog path may execute."
                ));
            }
            Some(id) if id.starts_with("local::") => {
                // Developer-sideloaded node – the local:: prefix is only assigned
                // by load_all_developer_nodes / developer_load_into_catalog which
                // directly insert into the registry, so presence here is proof of
                // legitimate local injection.
            }
            Some(_) => {
                // Named catalog package – the user consented to the permissions
                // the package requires when they installed it.
            }
        }

        let mut instance = self
            .loaded
            .instantiate(&self.engine, self.security.clone())
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to create WASM instance: {}", e))?;

        let definition = self
            .get_definition()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to get node definition: {}", e))?;

        // Collect input values
        let mut inputs = serde_json::Map::new();
        for pin in &definition.pins {
            if pin.pin_type.to_lowercase() == "input" && pin.data_type.to_lowercase() != "execution"
            {
                match context.evaluate_pin::<Value>(&pin.name).await {
                    Ok(val) => {
                        inputs.insert(pin.name.clone(), val);
                    }
                    Err(_) => {
                        // No value available (unconnected, no default) — skip
                    }
                }
            }
        }

        // Set up host state
        let host_state = instance.host_state_mut();
        let inputs_for_state: std::collections::HashMap<String, Value> =
            inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        host_state.set_inputs(inputs_for_state);

        // Build run_id
        let run_id: String = context
            .run
            .upgrade()
            .and_then(|r: Arc<Mutex<Run>>| r.try_lock().ok().map(|run| run.id.clone()))
            .unwrap_or_default();

        // Get node_id from context
        let node_id = context.id.clone();

        // Build app_id and board_id from execution cache
        let (app_id, board_id, sub, _board_dir) =
            if let Some(ref exec_cache) = context.execution_cache {
                (
                    exec_cache.app_id.clone(),
                    exec_cache.board_id.clone(),
                    exec_cache.sub.clone(),
                    exec_cache.board_dir.clone(),
                )
            } else {
                (
                    String::new(),
                    String::new(),
                    String::new(),
                    flow_like_storage::object_store::path::Path::from(""),
                )
            };

        host_state.metadata = ExecutionMetadata {
            node_id: node_id.clone(),
            run_id: run_id.clone(),
            app_id: app_id.clone(),
            board_id: board_id.clone(),
            user_id: sub.clone(),
            stream_state: context.stream_state,
            log_level: context.log_level as u8,
        };

        // Populate storage context from ExecutionContext
        if let Some(ref exec_cache) = context.execution_cache {
            host_state.storage_context = Some(StorageContext {
                stores: exec_cache.stores.clone(),
                store_cache: ParkingRwLock::new(HashMap::new()),
                app_id: exec_cache.app_id.clone(),
                board_dir: exec_cache.board_dir.clone(),
                board_id: exec_cache.board_id.clone(),
                node_id: node_id.clone(),
                sub: exec_cache.sub.clone(),
            });
        }

        // Populate model context from app state
        host_state.model_context = Some(ModelContext {
            app_state: context.app_state.clone(),
        });

        // Execute
        let exec_input = WasmExecutionInput {
            inputs,
            node_id,
            run_id,
            app_id,
            board_id,
            user_id: sub,
            stream_state: context.stream_state,
            log_level: context.log_level as u8,
            node_name: definition.name.clone(),
        };

        let result = instance
            .call_run(&exec_input)
            .await
            .map_err(|e| flow_like_types::anyhow!("WASM execution failed: {}", e))?;

        // Process outputs
        for (name, value) in result.outputs {
            context.set_pin_value(&name, value).await?;
        }

        // Activate exec pins
        for pin_name in &result.activate_exec {
            context.activate_exec_pin(pin_name).await?;
        }

        // Process logs
        for log in instance.host_state().get_logs() {
            let level = match log.level {
                0..=1 => LogLevel::Debug,
                2 => LogLevel::Info,
                3 => LogLevel::Warn,
                _ => LogLevel::Error,
            };
            context.log_message(&log.message, level);
        }

        // Process stream events
        for event in instance.host_state().take_stream_events() {
            if event.event_type == "text" {
                if let Some(text) = event.data.as_str() {
                    context
                        .stream_response("wasm_text", text.to_string())
                        .await?;
                }
            }
        }

        // Check for errors
        if let Some(error) = instance.host_state().get_error() {
            return Err(flow_like_types::anyhow!("WASM node error: {}", error));
        }

        if let Some(error) = result.error {
            return Err(flow_like_types::anyhow!("WASM execution error: {}", error));
        }

        Ok(())
    }

    async fn on_drop(&self) {}
}

impl std::fmt::Debug for WasmNodeLogic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmNodeLogic")
            .field("module_hash", &self.loaded.hash())
            .finish()
    }
}
