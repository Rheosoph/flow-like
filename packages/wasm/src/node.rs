//! WASM Node Logic implementation
//!
//! Bridges WASM modules to the Flow-Like NodeLogic trait.

use crate::abi::{WasmExecutionInput, WasmNodeDefinition, WasmPinDefinition};
use crate::engine::WasmEngine;
use crate::error::WasmResult;
use crate::host_functions::{ExecutionMetadata, HostState, ModelContext, StorageContext};
use crate::limits::{WasmCapabilities, WasmSecurityConfig};
use crate::module::WasmModule;
use crate::package_runtime::{package_runtime_key, PackageRuntime};
use crate::unified::LoadedWasm;
use async_trait::async_trait;
use flow_like::flow::execution::context::{ExecutionContext, ExecutionContextCache};
use flow_like::flow::execution::LogLevel;
use flow_like::flow::node::{Node, NodeLogic, NodeScores, NodeWasm};
use flow_like::flow::pin::{Pin, PinOptions, PinType, ValueType};
use flow_like::flow::variable::VariableType;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::path::Path;
use flow_like_types::{tokio::sync::RwLock, Cacheable, Value};
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
            .instantiate(&self.engine, self.security.for_metadata())
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

async fn register_wasm_flowpath_stores(
    context: &ExecutionContext,
    exec_cache: &ExecutionContextCache,
    credentials_store: Option<FlowLikeStore>,
) -> flow_like_types::Result<()> {
    let dirs: Vec<(&str, Path, Option<FlowLikeStore>)> = vec![
        (
            "storage",
            exec_cache.get_storage(false)?,
            exec_cache.stores.app_storage_store.clone(),
        ),
        (
            "storage",
            exec_cache.get_storage(true)?,
            exec_cache.stores.app_storage_store.clone(),
        ),
        (
            "upload",
            exec_cache.get_upload_dir()?,
            exec_cache.stores.app_storage_store.clone(),
        ),
        (
            "cache",
            exec_cache.get_cache(false, false)?,
            exec_cache.stores.temporary_store.clone(),
        ),
        (
            "cache",
            exec_cache.get_cache(true, false)?,
            exec_cache.stores.temporary_store.clone(),
        ),
        (
            "cache",
            exec_cache.get_cache(false, true)?,
            exec_cache.stores.temporary_store.clone(),
        ),
        (
            "cache",
            exec_cache.get_cache(true, true)?,
            exec_cache.stores.temporary_store.clone(),
        ),
        (
            "user",
            exec_cache.get_user_dir(false)?,
            exec_cache.stores.user_store.clone(),
        ),
        (
            "user",
            exec_cache.get_user_dir(true)?,
            exec_cache.stores.user_store.clone(),
        ),
    ];

    for (dir_type, dir, backing_store) in dirs {
        let Some(backing_store) = backing_store else {
            continue;
        };

        let store_ref = format!("dirs__{dir_type}_{}", dir.as_ref());
        if credentials_store.is_some() || !context.has_cache(&store_ref).await {
            let primary_store = credentials_store
                .clone()
                .unwrap_or_else(|| backing_store.clone());
            let cacheable_store: Arc<dyn Cacheable> = Arc::new(primary_store);
            context.set_cache(&store_ref, cacheable_store).await;
        }

        if credentials_store.is_some() {
            let cache_store_ref = format!("cache_dirs__{dir_type}_{}", dir.as_ref());
            let cacheable_store: Arc<dyn Cacheable> = Arc::new(backing_store);
            context.set_cache(&cache_store_ref, cacheable_store).await;
        }
    }

    Ok(())
}

fn collect_flow_path_store_refs(value: &Value, refs: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            let is_flow_path = object.get("path").and_then(Value::as_str).is_some()
                && object.get("store_ref").and_then(Value::as_str).is_some();

            if is_flow_path {
                if let Some(store_ref) = object.get("store_ref").and_then(Value::as_str) {
                    refs.insert(store_ref.to_string());
                }

                if let Some(cache_store_ref) = object.get("cache_store_ref").and_then(Value::as_str)
                {
                    refs.insert(cache_store_ref.to_string());
                }
            }

            for child in object.values() {
                collect_flow_path_store_refs(child, refs);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_flow_path_store_refs(child, refs);
            }
        }
        _ => {}
    }
}

async fn resolve_input_flowpath_stores(
    context: &ExecutionContext,
    inputs: &serde_json::Map<String, Value>,
) -> HashMap<String, FlowLikeStore> {
    let mut refs = BTreeSet::new();
    for value in inputs.values() {
        collect_flow_path_store_refs(value, &mut refs);
    }

    let mut stores = HashMap::new();
    for store_ref in refs {
        let Some(cacheable) = context.get_cache(&store_ref).await else {
            tracing::debug!("[wasm] FlowPath input store_ref not found in cache: {store_ref}");
            continue;
        };

        let Some(store) = cacheable.downcast_ref::<FlowLikeStore>() else {
            tracing::debug!("[wasm] FlowPath input cache value is not a store: {store_ref}");
            continue;
        };

        stores.insert(store_ref, store.clone());
    }

    stores
}

/// Convert a `WasmNodeDefinition` into a `PackageNodeEntry` suitable for storage
/// in the `WasmPackageVersion.nodes` JSON column.
pub fn definition_to_package_entry(
    definition: &WasmNodeDefinition,
) -> crate::manifest::PackageNodeEntry {
    let mut pins = HashMap::new();
    for (i, wasm_pin) in definition.pins.iter().enumerate() {
        let pin = WasmNodeLogic::to_flow_pin(wasm_pin, i as u16);
        pins.insert(pin.name.clone(), pin);
    }

    let scores = definition.scores.as_ref().map(|s| NodeScores {
        privacy: s.privacy,
        security: s.security,
        performance: s.performance,
        governance: s.governance,
        reliability: s.reliability,
        cost: s.cost,
    });

    crate::manifest::PackageNodeEntry {
        id: definition.name.clone(),
        name: definition.name.clone(),
        friendly_name: Some(definition.friendly_name.clone()),
        description: definition.description.clone(),
        category: definition.category.clone(),
        icon: definition.icon.clone(),
        scores,
        pins,
        start: None,
        long_running: definition.long_running,
        docs: definition.docs.clone(),
        event_callback: None,
        fn_refs: None,
        oauth_providers: vec![],
        required_oauth_scopes: None,
        only_offline: false,
        // Deliberately not `definition.abi_version`. That is the host ABI the
        // module was built against, while this field lands in `Node::version`,
        // which `sync_node_schema` reads as the pin-schema generation. A guest
        // definition carries no schema generation at all, and conflating the
        // two would let an ABI bump present as a schema bump — which drops
        // every pin the new catalog entry does not declare from boards already
        // using the node.
        version: None,
        permissions: definition.permissions.clone(),
        metadata: HashMap::new(),
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

    node.ensure_flowscript_names();
    node
}

#[async_trait]
impl NodeLogic for WasmNodeLogic {
    fn get_node(&self) -> Node {
        let definition = if let Ok(cached) = self.cached_definition.try_read() {
            cached.as_ref().cloned()
        } else {
            None
        }
        .or_else(|| {
            let handle = flow_like_types::tokio::runtime::Handle::try_current().ok()?;
            if handle.runtime_flavor()
                != flow_like_types::tokio::runtime::RuntimeFlavor::MultiThread
            {
                return None;
            }

            flow_like_types::tokio::task::block_in_place(|| {
                handle.block_on(async { self.get_definition().await.ok() })
            })
        });

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

        let mut node = build_node_from_definition(&definition);

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

        let mut security = self.security.clone();
        security.execution_environment = context.execution_environment();
        if context
            .execution_cache
            .as_ref()
            .is_some_and(|cache| cache.shadow)
        {
            security.capabilities = strip_shadow_capabilities(security.capabilities);
        }
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
        let mut host_state = HostState::with_security(&security);
        let inputs_for_state: std::collections::HashMap<String, Value> =
            inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        host_state.set_inputs(inputs_for_state);

        // Build run_id
        let run_id = context.run_id().to_string();

        // Get node_id from context
        let node_id = context.id.to_string();

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
            execution_environment: context.execution_environment(),
        };

        // Populate storage context from ExecutionContext
        if let Some(exec_cache) = context.execution_cache.clone() {
            let has_storage_capability = self.security.capabilities.intersects(
                WasmCapabilities::STORAGE_READ
                    | WasmCapabilities::STORAGE_WRITE
                    | WasmCapabilities::STORAGE_DELETE,
            );
            let credentials_store = if has_storage_capability {
                match &context.credentials {
                    Some(credentials) => Some(credentials.to_store(false).await?),
                    None => None,
                }
            } else {
                None
            };
            if has_storage_capability {
                register_wasm_flowpath_stores(context, &exec_cache, credentials_store.clone())
                    .await?;
            }
            let input_store_cache = if has_storage_capability {
                resolve_input_flowpath_stores(context, &inputs).await
            } else {
                HashMap::new()
            };

            host_state.storage_context = Some(StorageContext {
                stores: exec_cache.stores.clone(),
                store_cache: ParkingRwLock::new(input_store_cache),
                credentials_store,
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
            token: context.token.clone(),
            cache: Some(context.cache.clone()),
        });
        host_state.model_usage_context = context.model_usage_context();

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

        let shadow = context
            .execution_cache
            .as_ref()
            .is_some_and(|cache| cache.shadow);
        let key = package_runtime_key(
            self.package_id.as_deref().expect("package validated above"),
            self.loaded.hash(),
            &security,
            &exec_input.user_id,
            shadow,
        )?;
        let runtime = context
            .resources
            .get_or_insert_with(key, || Arc::new(PackageRuntime::default()))?;
        let call = runtime
            .call(
                &self.loaded,
                &self.engine,
                &security,
                host_state,
                &exec_input,
            )
            .await
            .map_err(|e| flow_like_types::anyhow!("WASM execution failed: {}", e))?;
        let result = call.result;

        // Process outputs
        for (name, value) in result.outputs {
            context.set_pin_value(&name, value).await?;
        }

        // Activate exec pins
        for pin_name in &result.activate_exec {
            context.activate_exec_pin(pin_name).await?;
        }

        // Process logs
        for log in call.logs {
            let level = match log.level {
                0..=1 => LogLevel::Debug,
                2 => LogLevel::Info,
                3 => LogLevel::Warn,
                _ => LogLevel::Error,
            };
            context.log_message(&log.message, level);
        }

        // Process stream events
        for event in call.events {
            match event.event_type.as_str() {
                "text" => {
                    if let Some(text) = event.data.as_str() {
                        context
                            .stream_response("wasm_text", text.to_string())
                            .await?;
                    }
                }
                "llm_chunk" => {
                    context.stream_response("llm_chunk", event.data).await?;
                }
                _ => {}
            }
        }

        // Check for errors
        if let Some(error) = call.error {
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

/// The per-run capability mask for a shadow/replay run: every side-effecting
/// capability is cleared while reads stay available. Writes through a shadow
/// run fail loudly at the host boundary rather than silently no-oping.
fn strip_shadow_capabilities(capabilities: WasmCapabilities) -> WasmCapabilities {
    capabilities
        & !(WasmCapabilities::STORAGE_WRITE
            | WasmCapabilities::STORAGE_DELETE
            | WasmCapabilities::HTTP_WRITE
            | WasmCapabilities::VARIABLES_WRITE
            | WasmCapabilities::CACHE_WRITE
            | WasmCapabilities::OAUTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pin(
        name: &str,
        pin_type: &str,
        data_type: &str,
        schema: Option<&str>,
        enforce_schema: Option<bool>,
        default_value: Option<serde_json::Value>,
    ) -> WasmPinDefinition {
        WasmPinDefinition {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            pin_type: pin_type.to_string(),
            data_type: data_type.to_string(),
            default_value,
            value_type: None,
            schema: schema.map(|s| s.to_string()),
            valid_values: None,
            range: None,
            step: None,
            sensitive: None,
            enforce_schema,
            enforce_generic_value_type: None,
        }
    }

    #[test]
    fn shadow_mask_clears_every_side_effecting_capability_and_keeps_reads() {
        let stripped = strip_shadow_capabilities(WasmCapabilities::ALL);
        assert!(!stripped.intersects(
            WasmCapabilities::STORAGE_WRITE
                | WasmCapabilities::STORAGE_DELETE
                | WasmCapabilities::HTTP_WRITE
                | WasmCapabilities::VARIABLES_WRITE
                | WasmCapabilities::CACHE_WRITE
                | WasmCapabilities::OAUTH
        ));
        assert!(stripped.contains(WasmCapabilities::STORAGE_READ));
        assert!(stripped.contains(WasmCapabilities::VARIABLES_READ));
        assert!(stripped.contains(WasmCapabilities::CACHE_READ));
        assert!(stripped.contains(WasmCapabilities::HTTP_GET));

        assert_eq!(
            strip_shadow_capabilities(WasmCapabilities::NONE),
            WasmCapabilities::NONE
        );
    }

    #[test]
    fn test_collect_flow_path_store_refs_from_nested_inputs() {
        let value = serde_json::json!({
            "virtual": {
                "path": "",
                "store_ref": "virtual_dir_/virtual",
                "cache_store_ref": null
            },
            "items": [
                {
                    "path": "nested/file.txt",
                    "store_ref": "s3_store",
                    "cache_store_ref": "cache_dirs__storage_app"
                },
                {
                    "path": "not-a-flow-path"
                }
            ]
        });

        let mut refs = BTreeSet::new();
        collect_flow_path_store_refs(&value, &mut refs);

        assert_eq!(
            refs,
            BTreeSet::from([
                "cache_dirs__storage_app".to_string(),
                "s3_store".to_string(),
                "virtual_dir_/virtual".to_string(),
            ])
        );
    }

    #[test]
    fn test_map_wasm_data_type_all_variants() {
        assert_eq!(map_wasm_data_type("Execution"), VariableType::Execution);
        assert_eq!(map_wasm_data_type("execution"), VariableType::Execution);
        assert_eq!(map_wasm_data_type("String"), VariableType::String);
        assert_eq!(map_wasm_data_type("Integer"), VariableType::Integer);
        assert_eq!(map_wasm_data_type("Float"), VariableType::Float);
        assert_eq!(map_wasm_data_type("Boolean"), VariableType::Boolean);
        assert_eq!(map_wasm_data_type("Date"), VariableType::Date);
        assert_eq!(map_wasm_data_type("PathBuf"), VariableType::PathBuf);
        assert_eq!(map_wasm_data_type("Byte"), VariableType::Byte);
        assert_eq!(map_wasm_data_type("Struct"), VariableType::Struct);
        assert_eq!(map_wasm_data_type("Generic"), VariableType::Generic);
        assert_eq!(map_wasm_data_type("unknown_thing"), VariableType::Generic);
    }

    #[test]
    fn test_build_node_preserves_schema() {
        let schema_json =
            r#"{"type":"object","properties":{"name":{"type":"string"},"age":{"type":"integer"}}}"#;

        let def = WasmNodeDefinition {
            name: "test_struct".to_string(),
            friendly_name: "Test Struct".to_string(),
            description: "Tests struct schema".to_string(),
            category: "Test".to_string(),
            icon: None,
            pins: vec![
                make_pin("exec", "Input", "Execution", None, None, None),
                make_pin(
                    "config",
                    "Input",
                    "Struct",
                    Some(schema_json),
                    Some(true),
                    Some(serde_json::json!({"name": "default", "age": 0})),
                ),
                make_pin("exec_out", "Output", "Execution", None, None, None),
                make_pin(
                    "result",
                    "Output",
                    "Struct",
                    Some(schema_json),
                    Some(true),
                    None,
                ),
            ],
            scores: None,
            long_running: None,
            docs: None,
            abi_version: Some(1),
            permissions: vec![],
        };

        let node = build_node_from_definition(&def);

        assert_eq!(node.name, "test_struct");
        assert_eq!(node.pins.len(), 4);

        let mut pins: Vec<&Pin> = node.pins.values().collect();
        pins.sort_by_key(|p| p.index);

        // Exec input
        assert_eq!(pins[0].data_type, VariableType::Execution);
        assert_eq!(pins[0].pin_type, PinType::Input);

        // Struct input with schema
        assert_eq!(pins[1].data_type, VariableType::Struct);
        assert_eq!(pins[1].pin_type, PinType::Input);
        assert_eq!(pins[1].schema.as_deref(), Some(schema_json));
        assert!(pins[1].default_value.is_some());
        let opts = pins[1].options.as_ref().expect("options must be set");
        assert_eq!(opts.enforce_schema, Some(true));

        // Exec output
        assert_eq!(pins[2].data_type, VariableType::Execution);
        assert_eq!(pins[2].pin_type, PinType::Output);

        // Struct output with schema
        assert_eq!(pins[3].data_type, VariableType::Struct);
        assert_eq!(pins[3].pin_type, PinType::Output);
        assert_eq!(pins[3].schema.as_deref(), Some(schema_json));
        assert_eq!(pins[3].options.as_ref().unwrap().enforce_schema, Some(true));
    }

    #[test]
    fn test_build_node_from_sdk_json() {
        // Simulate what the SDK produces: enum values as strings
        let sdk_json = r#"{
            "name": "email_node",
            "friendly_name": "Send Email",
            "description": "Sends an email",
            "category": "IO/Email",
            "pins": [
                {
                    "name": "exec",
                    "friendly_name": "Exec",
                    "description": "Trigger",
                    "pin_type": "Input",
                    "data_type": "Execution"
                },
                {
                    "name": "payload",
                    "friendly_name": "Payload",
                    "description": "Email data",
                    "pin_type": "Input",
                    "data_type": "Struct",
                    "schema": "{\"type\":\"object\",\"properties\":{\"to\":{\"type\":\"string\"},\"subject\":{\"type\":\"string\"}}}",
                    "enforce_schema": true,
                    "default_value": {"to": "user@example.com", "subject": "Hello"}
                },
                {
                    "name": "exec_out",
                    "friendly_name": "Done",
                    "description": "Continue",
                    "pin_type": "Output",
                    "data_type": "Execution"
                }
            ]
        }"#;

        let def: WasmNodeDefinition =
            serde_json::from_str(sdk_json).expect("SDK JSON must parse into WasmNodeDefinition");

        let node = build_node_from_definition(&def);

        assert_eq!(node.name, "email_node");

        let mut pins: Vec<&Pin> = node.pins.values().collect();
        pins.sort_by_key(|p| p.index);

        assert_eq!(pins[0].data_type, VariableType::Execution);

        assert_eq!(pins[1].data_type, VariableType::Struct);
        assert!(pins[1].schema.is_some());
        let schema: serde_json::Value =
            serde_json::from_str(pins[1].schema.as_ref().unwrap()).unwrap();
        assert!(schema["properties"]["to"].is_object());
        assert!(schema["properties"]["subject"].is_object());
        assert_eq!(pins[1].options.as_ref().unwrap().enforce_schema, Some(true));
    }

    #[test]
    fn test_to_flow_pin_value_types() {
        for (vt_str, expected) in [
            ("Normal", ValueType::Normal),
            ("Array", ValueType::Array),
            ("HashMap", ValueType::HashMap),
            ("HashSet", ValueType::HashSet),
            ("ARRAY", ValueType::Array),
            ("hashmap", ValueType::HashMap),
        ] {
            let mut pin = make_pin("p", "Input", "String", None, None, None);
            pin.value_type = Some(vt_str.to_string());
            let flow_pin = WasmNodeLogic::to_flow_pin(&pin, 0);
            assert_eq!(flow_pin.value_type, expected, "failed for {vt_str}");
        }
    }

    #[test]
    fn test_to_flow_pin_options() {
        let mut pin = make_pin("slider", "Input", "Float", None, None, None);
        pin.range = Some((0.0, 100.0));
        pin.step = Some(0.5);
        pin.sensitive = Some(true);

        let flow_pin = WasmNodeLogic::to_flow_pin(&pin, 0);
        let opts = flow_pin.options.expect("options must be set");
        assert_eq!(opts.range, Some((0.0, 100.0)));
        assert_eq!(opts.step, Some(0.5));
        assert_eq!(opts.sensitive, Some(true));
    }
}
