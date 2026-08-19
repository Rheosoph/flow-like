use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use flow_like::flow::{
    board::{Board, Layer, LayerCache, LayerCacheScope, LayerType},
    execution::{
        EventTrigger, LogLevel, context::ExecutionContext, internal_node::InternalNode,
        internal_pin::InternalPin,
    },
    node::{Node, NodeLogic},
    pin::{Pin, PinType},
    utils::evaluate_pin_value,
    variable::VariableType,
};
use flow_like_catalog_data::data::cache::{CacheScope, FlowCache, cache_get, cache_set};
use flow_like_types::{Value, async_trait, json::from_slice, sync::RwLock};

/// Cache persistence is an optimization and must never hold the execution chain open forever.
/// This also bounds local object-store writes, which do not have the HTTP client's request timeout.
const FUNCTION_CACHE_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Fold a value into the hasher in a shape that does not depend on how its maps happen to
/// be ordered in memory, so the same inputs always produce the same cache key.
fn hash_canonical_value(hasher: &mut blake3::Hasher, value: &Value) {
    match value {
        Value::Object(map) => {
            hasher.update(b"{");
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            for key in keys {
                hasher.update(&(key.len() as u64).to_le_bytes());
                hasher.update(key.as_bytes());
                hash_canonical_value(hasher, &map[key]);
            }
            hasher.update(b"}");
        }
        Value::Array(items) => {
            hasher.update(b"[");
            hasher.update(&(items.len() as u64).to_le_bytes());
            for item in items {
                hash_canonical_value(hasher, item);
            }
            hasher.update(b"]");
        }
        other => {
            let encoded = flow_like_types::json::to_string(other).unwrap_or_default();
            hasher.update(&(encoded.len() as u64).to_le_bytes());
            hasher.update(encoded.as_bytes());
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CallFunctionNode {}

impl CallFunctionNode {
    pub fn new() -> Self {
        CallFunctionNode {}
    }

    fn sync_mirrored_pin(mirrored_pin: &mut Pin, function_pin: &Pin, index: u16) {
        mirrored_pin.friendly_name = function_pin.friendly_name.clone();
        mirrored_pin.description = function_pin.description.clone();
        mirrored_pin.pin_type = function_pin.pin_type.clone();
        mirrored_pin.data_type = function_pin.data_type.clone();
        mirrored_pin.value_type = function_pin.value_type.clone();
        mirrored_pin.schema = function_pin.schema.clone();
        mirrored_pin.options = function_pin.options.clone();
        mirrored_pin.index = index;
    }

    async fn read_outputs(
        &self,
        context: &mut ExecutionContext,
        layer: &Layer,
        overrides: &BTreeMap<String, Value>,
    ) {
        let overrides_opt = if overrides.is_empty() {
            None
        } else {
            Some(overrides.clone())
        };

        for layer_pin in layer.pins.values() {
            if layer_pin.pin_type != PinType::Output
                || layer_pin.data_type == VariableType::Execution
            {
                continue;
            }

            for dep_pin_id in &layer_pin.depends_on {
                // Find the InternalPin for this dependency
                let mut found_pin = None;
                for node in context.nodes.values() {
                    if let Ok(pin) = node.get_pin_by_id(dep_pin_id) {
                        found_pin = Some(pin);
                        break;
                    }
                }
                let Some(pin) = found_pin else {
                    continue;
                };

                // Use evaluate_pin_value to follow the full dependency chain
                // while checking overrides at each step. This correctly handles
                // bridge pins, relay pins, and prevents stale shared pin reads.
                if let Ok(value) = evaluate_pin_value(pin, &overrides_opt).await {
                    let _ = context.set_pin_value(&layer_pin.name, value).await;
                    break;
                }
            }
        }
    }

    /// The cache handle a layer's settings describe. The prefix travels as the namespace,
    /// which is what the backends group entries by — so one function's results can be
    /// invalidated without touching the rest of the app's cache.
    fn cache_handle(settings: &LayerCache) -> FlowCache {
        FlowCache {
            scope: match settings.scope {
                LayerCacheScope::App => CacheScope::App,
                LayerCacheScope::User => CacheScope::User,
            },
            namespace: settings.prefix.trim().to_string(),
        }
    }

    /// The layer id is part of the key so two functions sharing a prefix cannot read each
    /// other's results.
    fn cache_key(layer_id: &str, inputs: &HashMap<String, Value>) -> String {
        let mut sorted: Vec<(&String, &Value)> = inputs.iter().collect();
        sorted.sort_by_key(|(left, _)| *left);

        let mut hasher = blake3::Hasher::new();
        hasher.update(layer_id.as_bytes());
        hasher.update(b"\n");
        for (name, value) in sorted {
            hasher.update(&(name.len() as u64).to_le_bytes());
            hasher.update(name.as_bytes());
            hash_canonical_value(&mut hasher, value);
        }

        format!("layer_{}", hasher.finalize().to_hex())
    }

    fn output_data_pins(context: &ExecutionContext) -> Vec<Arc<InternalPin>> {
        context
            .node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Output && pin.data_type != VariableType::Execution
            })
            .cloned()
            .collect()
    }

    fn output_exec_pin_names(context: &ExecutionContext) -> Vec<String> {
        context
            .node
            .pins
            .values()
            .filter(|pin| {
                pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution
            })
            .map(|pin| pin.name.clone())
            .collect()
    }

    /// Snapshot the values the call produced, so a later call with the same inputs can be
    /// answered without running the function.
    async fn collect_cacheable_outputs(&self, context: &mut ExecutionContext) -> Value {
        let mut outputs = flow_like_types::json::Map::new();
        for pin in Self::output_data_pins(context) {
            let value = context
                .evaluate_pin_ref::<Value>(pin.clone())
                .await
                .unwrap_or(Value::Null);
            outputs.insert(pin.name.clone(), value);
        }
        Value::Object(outputs)
    }

    /// Replay a cached call: fill the mirrored outputs and let execution continue as if the
    /// function had run.
    async fn apply_cached_outputs(
        &self,
        context: &mut ExecutionContext,
        cached: &Value,
    ) -> flow_like_types::Result<()> {
        let outputs = cached.as_object().ok_or_else(|| {
            flow_like_types::anyhow!("Cached function result is not an object, ignoring it")
        })?;

        for pin in Self::output_data_pins(context) {
            let value = outputs.get(&pin.name).cloned().unwrap_or(Value::Null);
            context.set_pin_ref_value(&pin, value).await?;
        }

        for name in Self::output_exec_pin_names(context) {
            context.activate_exec_pin(&name).await?;
        }

        Ok(())
    }

    /// Start persisting a miss without putting the write on the node's successor-critical path.
    /// The completion hook joins the task before the run is finalized, keeping writes reliable in
    /// short-lived runtimes while downstream nodes are free to execute concurrently.
    async fn persist_cache_result(
        context: &mut ExecutionContext,
        function_name: String,
        handle: FlowCache,
        key: String,
        outputs: Value,
        ttl: Option<u64>,
    ) {
        let node_id = context.id.to_string();
        let mut write_context = context.clone();
        // Completion callbacks retain their captures for the life of the run. Detach the cloned
        // context's registry so callback -> task -> context cannot point back to the callback.
        write_context.completion_callbacks = Arc::new(RwLock::new(Vec::new()));

        let write_function_name = function_name.clone();
        let cancellation = write_context.get_cancellation_token();
        let task = flow_like_types::tokio::spawn(async move {
            let write = async {
                match flow_like_types::tokio::time::timeout(
                    FUNCTION_CACHE_WRITE_TIMEOUT,
                    cache_set(&write_context, &handle, &key, outputs, ttl),
                )
                .await
                {
                    Ok(Ok(_)) => None,
                    Ok(Err(error)) => Some(format!(
                        "Could not cache the result of function '{}': {:?}",
                        write_function_name, error
                    )),
                    Err(_) => Some(format!(
                        "Caching the result of function '{}' timed out after {} seconds",
                        write_function_name,
                        FUNCTION_CACHE_WRITE_TIMEOUT.as_secs()
                    )),
                }
            };

            if let Some(cancellation) = cancellation {
                flow_like_types::tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => None,
                    warning = write => warning,
                }
            } else {
                write.await
            }
        });
        let task = Arc::new(flow_like_types::tokio::sync::Mutex::new(Some(task)));

        let completion_event: EventTrigger = Arc::new(move |run| {
            let task = task.clone();
            let node_id = node_id.clone();
            let function_name = function_name.clone();
            Box::pin(async move {
                let task = { task.lock().await.take() };
                let Some(task) = task else {
                    return Ok(());
                };

                let warning = match task.await {
                    Ok(warning) => warning,
                    Err(error) => Some(format!(
                        "Cache write task for function '{}' failed: {:?}",
                        function_name, error
                    )),
                };
                if let Some(warning) = warning {
                    run.log_node_warning(&node_id, None, &warning).await;
                }

                // Cache persistence is best-effort and must not change a successful flow result.
                Ok(())
            })
        });
        context.hook_completion_event(completion_event).await;
    }

    fn find_node_id_by_pin(
        board: &Board,
        layer: &Layer,
        layer_id: &str,
        pin_id: &str,
    ) -> Option<String> {
        layer
            .nodes
            .values()
            .find(|n| n.pins.contains_key(pin_id))
            .or_else(|| {
                board
                    .nodes
                    .values()
                    .find(|n| n.layer.as_deref() == Some(layer_id) && n.pins.contains_key(pin_id))
            })
            .map(|n| n.id.clone())
    }

    async fn run_pure_function(
        &self,
        context: &mut ExecutionContext,
        board: &Board,
        layer: &Layer,
        layer_id: &str,
        input_values: &std::collections::HashMap<String, Value>,
    ) -> flow_like_types::Result<()> {
        // For pure functions, find nodes feeding each output and trigger them.
        // Dependency resolution cascades to pull input values from overridden layer pins.
        let output_layer_pins: Vec<_> = layer
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output && p.data_type != VariableType::Execution)
            .collect();

        if output_layer_pins.is_empty() {
            return Ok(());
        }

        // Find all inner nodes that directly feed outputs
        let mut triggered: HashSet<String> = HashSet::new();
        let mut all_overrides: BTreeMap<String, Value> = BTreeMap::new();

        for layer_pin in &output_layer_pins {
            for dep_pin_id in &layer_pin.depends_on {
                // Find the node owning this pin
                let feeding_node_id = Self::find_node_id_by_pin(board, layer, layer_id, dep_pin_id);

                let Some(node_id) = feeding_node_id else {
                    continue;
                };
                if !triggered.insert(node_id.clone()) {
                    continue;
                }

                let Some(node_arc) = context.nodes.get(&node_id).cloned() else {
                    continue;
                };

                let mut fn_context = context
                    .create_function_context(&node_arc, &layer.variables)
                    .await;
                fn_context.delegated = true;

                for (pin_id, pin) in &layer.pins {
                    if pin.pin_type != PinType::Input || pin.data_type == VariableType::Execution {
                        continue;
                    }
                    if let Some(value) = input_values.get(&pin.name) {
                        fn_context.override_pin_value(pin_id, value.clone());
                    }
                }

                let result = InternalNode::trigger(&mut fn_context, &mut None, false).await;
                if let Some(overrides) = fn_context.context_pin_overrides.take() {
                    all_overrides.extend(overrides);
                }
                fn_context.end_trace();
                context.push_sub_context(&mut fn_context);

                if let Err(error) = result {
                    context.log_message(
                        &format!("Error in pure function: {:?}", error),
                        LogLevel::Error,
                    );
                    return Err(flow_like_types::anyhow!(
                        "Pure function execution failed: {:?}",
                        error
                    ));
                }
            }
        }

        // Read output values
        self.read_outputs(context, layer, &all_overrides).await;
        Ok(())
    }
}

#[async_trait]
impl NodeLogic for CallFunctionNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_call_function",
            "Call Function",
            "Calls a function defined on this board",
            "Control/Functions",
        );
        node.add_icon("/flow/icons/workflow.svg");

        node.add_input_pin(
            "function_layer_id",
            "Function",
            "The function to call",
            VariableType::String,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let function_layer_id: String = context.evaluate_pin("function_layer_id").await?;

        let board = context.get_board().await?;
        let layer = board
            .layers
            .get(&function_layer_id)
            .ok_or_else(|| flow_like_types::anyhow!("Function layer not found"))?;

        if !matches!(layer.r#type, LayerType::Function) {
            return Err(flow_like_types::anyhow!(
                "Layer '{}' is not a function",
                layer.name
            ));
        }

        // Collect input values (non-exec, non-function_layer_id)
        let input_pins: Vec<_> = {
            let pins: Vec<_> = context.node.pins.values().cloned().collect();
            pins.into_iter()
                .filter(|p| {
                    p.pin_type == PinType::Input
                        && p.data_type != VariableType::Execution
                        && p.name != "function_layer_id"
                })
                .collect()
        };

        let mut input_values = std::collections::HashMap::new();
        for pin in &input_pins {
            if let Ok(value) = context.evaluate_pin_ref::<Value>(pin.clone()).await {
                input_values.insert(pin.name.clone(), value);
            }
        }

        // Clear mirrored data outputs before each call to avoid leaking
        // previous invocation values when a function output is not produced.
        let output_data_pins: Vec<_> = Self::output_data_pins(context);
        for pin in &output_data_pins {
            let _ = context.set_pin_ref_value(pin, Value::Null).await;
        }

        // A hit replaces the entire call, side effects included — which is why caching is
        // opt-in per layer.
        let cache_settings = layer.cache.clone().filter(LayerCache::is_active);
        let cache_lookup = match &cache_settings {
            Some(settings) => {
                let handle = Self::cache_handle(settings);
                let key = Self::cache_key(&function_layer_id, &input_values);

                match cache_get(context, &handle, &key).await {
                    Ok(Some(hit)) => {
                        match self.apply_cached_outputs(context, &hit.value).await {
                            Ok(()) => {
                                context.log_message(
                                    &format!("Cache hit for function '{}'", layer.name),
                                    LogLevel::Debug,
                                );
                                return Ok(());
                            }
                            // A malformed entry must not take the call down with it.
                            Err(error) => {
                                context.log_message(
                                    &format!(
                                        "Ignoring unusable cache entry for function '{}': {:?}",
                                        layer.name, error
                                    ),
                                    LogLevel::Warn,
                                );
                                for pin in &output_data_pins {
                                    let _ = context.set_pin_ref_value(pin, Value::Null).await;
                                }
                            }
                        }
                        Some((handle, key))
                    }
                    Ok(None) => Some((handle, key)),
                    // The cache is an optimization; an unreachable backend degrades to a
                    // normal call rather than failing the flow.
                    Err(error) => {
                        context.log_message(
                            &format!(
                                "Cache read failed for function '{}', executing it instead: {:?}",
                                layer.name, error
                            ),
                            LogLevel::Warn,
                        );
                        None
                    }
                }
            }
            None => None,
        };

        // Check if the function is impure (has an input execution layer pin)
        let exec_in_layer_pin = layer
            .pins
            .values()
            .find(|p| p.pin_type == PinType::Input && p.data_type == VariableType::Execution);

        if let Some(exec_pin) = exec_in_layer_pin {
            // --- IMPURE function: exec chain inside the function ---
            // Deactivate all mirrored output exec pins
            let output_exec_names: Vec<String> = Self::output_exec_pin_names(context);
            for name in &output_exec_names {
                let _ = context.deactivate_exec_pin(name).await;
            }

            // Find entry node via the layer exec_in pin's connections
            let entry_node_id = exec_pin
                .connected_to
                .iter()
                .find_map(|pin_id| {
                    Self::find_node_id_by_pin(&board, layer, &function_layer_id, pin_id)
                })
                .ok_or_else(|| {
                    flow_like_types::anyhow!(
                        "Function '{}' has no node connected to its exec input",
                        layer.name
                    )
                })?;

            let entry_node = context
                .nodes
                .get(&entry_node_id)
                .ok_or_else(|| flow_like_types::anyhow!("Entry node not found in execution graph"))?
                .clone();

            let mut fn_context = context
                .create_function_context(&entry_node, &layer.variables)
                .await;
            fn_context.delegated = true;

            // Inject input values on layer input pins (non-exec)
            for (pin_id, pin) in &layer.pins {
                if pin.pin_type != PinType::Input || pin.data_type == VariableType::Execution {
                    continue;
                }
                if let Some(value) = input_values.get(&pin.name) {
                    fn_context.override_pin_value(pin_id, value.clone());
                }
            }

            let run_result = InternalNode::trigger(&mut fn_context, &mut None, true).await;

            if let Err(error) = run_result {
                fn_context.end_trace();
                context.push_sub_context(&mut fn_context);
                context.log_message(
                    &format!("Error calling function '{}': {:?}", layer.name, error),
                    LogLevel::Error,
                );
                return Err(flow_like_types::anyhow!(
                    "Failed to execute function '{}': {:?}",
                    layer.name,
                    error
                ));
            }

            // Trigger pure nodes feeding layer outputs — they may not have run
            // during the exec chain since the layer return boundary is virtual.
            let mut triggered: HashSet<String> = HashSet::new();
            for layer_pin in layer.pins.values() {
                if layer_pin.pin_type != PinType::Output
                    || layer_pin.data_type == VariableType::Execution
                {
                    continue;
                }
                for dep_pin_id in &layer_pin.depends_on {
                    let feeding_node_id =
                        Self::find_node_id_by_pin(&board, layer, &function_layer_id, dep_pin_id);
                    let Some(node_id) = feeding_node_id else {
                        continue;
                    };
                    if !triggered.insert(node_id.clone()) {
                        continue;
                    }
                    let Some(node_arc) = fn_context.nodes.get(&node_id).cloned() else {
                        continue;
                    };
                    if let Some(overrides) = &fn_context.context_pin_overrides
                        && overrides.contains_key(dep_pin_id.as_str())
                    {
                        continue;
                    }
                    let mut sub = fn_context.create_sub_context(&node_arc).await;
                    sub.delegated = true;
                    for (pin_id, pin) in &layer.pins {
                        if pin.pin_type != PinType::Input
                            || pin.data_type == VariableType::Execution
                        {
                            continue;
                        }
                        if let Some(value) = input_values.get(&pin.name) {
                            sub.override_pin_value(pin_id, value.clone());
                        }
                    }
                    let result = InternalNode::trigger(&mut sub, &mut None, false).await;
                    sub.end_trace();
                    fn_context.push_sub_context(&mut sub);
                    if let Err(error) = result {
                        context.log_message(
                            &format!("Error resolving output '{}': {:?}", layer_pin.name, error),
                            LogLevel::Error,
                        );
                    }
                }
            }

            let overrides = fn_context.context_pin_overrides.take().unwrap_or_default();
            fn_context.end_trace();
            context.push_sub_context(&mut fn_context);

            self.read_outputs(context, layer, &overrides).await;

            for name in &output_exec_names {
                let _ = context.activate_exec_pin(name).await;
            }
        } else {
            // --- PURE function: evaluate outputs by triggering feeding nodes ---
            self.run_pure_function(context, &board, layer, &function_layer_id, &input_values)
                .await?;
        }

        if let (Some(settings), Some((handle, key))) = (&cache_settings, &cache_lookup) {
            let outputs = self.collect_cacheable_outputs(context).await;
            Self::persist_cache_result(
                context,
                layer.name.clone(),
                handle.clone(),
                key.clone(),
                outputs,
                settings.ttl(),
            )
            .await;
        }

        Ok(())
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;

        let layer_pin = match node.get_pin_by_name("function_layer_id") {
            Some(pin) => pin.clone(),
            None => {
                node.error = Some("Function layer pin not found".to_string());
                return;
            }
        };

        let layer_id = match layer_pin.default_value {
            Some(ref value) => match from_slice::<String>(value) {
                Ok(id) => id,
                Err(_) => return,
            },
            None => return,
        };

        let layer = match board.layers.get(&layer_id) {
            Some(layer) => layer,
            None => {
                node.error = Some(format!("Function '{}' not found", layer_id));
                return;
            }
        };

        if !matches!(layer.r#type, LayerType::Function) {
            node.error = Some("Referenced layer is not a function".to_string());
            return;
        }

        node.friendly_name = format!("Call {}", layer.name);
        node.description = format!("Calls the function '{}'", layer.name);

        // Mirror ALL function layer pins (including Execution) on this node
        let mut input_pins: Vec<_> = layer
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .cloned()
            .collect();
        input_pins.sort_by_key(|a| a.index);

        let mut output_pins: Vec<_> = layer
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .cloned()
            .collect();
        output_pins.sort_by_key(|a| a.index);

        let mut relevant_input_pin_names = HashSet::new();
        relevant_input_pin_names.insert("function_layer_id".to_string());
        let mut relevant_output_pin_names = HashSet::new();

        for (index, pin) in input_pins.iter().enumerate() {
            if pin.name == "function_layer_id" {
                continue;
            }

            relevant_input_pin_names.insert(pin.name.clone());
            let mirrored_index = index as u16 + 2;
            if let Some(existing_pin) = node
                .pins
                .values_mut()
                .find(|p| p.name == pin.name && p.pin_type == PinType::Input)
            {
                Self::sync_mirrored_pin(existing_pin, pin, mirrored_index);
                continue;
            }
            let new_pin = node.add_input_pin(
                &pin.name,
                &pin.friendly_name,
                &pin.description,
                pin.data_type.clone(),
            );
            Self::sync_mirrored_pin(new_pin, pin, mirrored_index);
        }

        for (index, pin) in output_pins.iter().enumerate() {
            relevant_output_pin_names.insert(pin.name.clone());
            let mirrored_index = index as u16 + 1;
            if let Some(existing_pin) = node
                .pins
                .values_mut()
                .find(|p| p.name == pin.name && p.pin_type == PinType::Output)
            {
                Self::sync_mirrored_pin(existing_pin, pin, mirrored_index);
                continue;
            }
            let new_pin = node.add_output_pin(
                &pin.name,
                &pin.friendly_name,
                &pin.description,
                pin.data_type.clone(),
            );
            Self::sync_mirrored_pin(new_pin, pin, mirrored_index);
        }

        // Remove stale pins (only keep function_layer_id + mirrored pins)
        node.pins.retain(|_, p| {
            if p.pin_type == PinType::Input {
                relevant_input_pin_names.contains(&p.name)
            } else {
                relevant_output_pin_names.contains(&p.name)
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{
        board::{ExecutionMode, ExecutionStage},
        execution::LogLevel,
        pin::ValueType,
    };
    use flow_like_types::json::json;
    use std::{collections::BTreeSet, time::SystemTime};

    fn pin(name: &str, pin_type: PinType, data_type: VariableType, index: u16) -> Pin {
        Pin {
            id: format!("{name}_id"),
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            pin_type,
            data_type,
            schema: None,
            value_type: ValueType::Normal,
            depends_on: BTreeSet::new(),
            connected_to: BTreeSet::new(),
            default_value: None,
            index,
            options: None,
            value: None,
        }
    }

    fn board_with_layer(layer: Layer) -> Board {
        let mut board = Board::new_detached(Some("board".to_string()), Default::default());
        board.name = "Board".to_string();
        board.description.clear();
        board.layers.insert(layer.id.clone(), layer);
        board.hash = None;
        board
    }

    fn ordered_pin_names(node: &Node, pin_type: PinType) -> Vec<String> {
        let mut pins = node
            .pins
            .values()
            .filter(|pin| pin.pin_type == pin_type)
            .collect::<Vec<_>>();
        pins.sort_by_key(|pin| pin.index);
        pins.into_iter().map(|pin| pin.name.clone()).collect()
    }

    #[tokio::test]
    async fn on_update_reorders_existing_mirrored_function_pins() {
        let mut layer = Layer::new(
            "function-layer".to_string(),
            "Example Function".to_string(),
            LayerType::Function,
        );
        layer.pins.insert(
            "first_id".to_string(),
            pin("first", PinType::Input, VariableType::String, 1),
        );
        layer.pins.insert(
            "second_id".to_string(),
            pin("second", PinType::Input, VariableType::Integer, 2),
        );
        layer.pins.insert(
            "out_first_id".to_string(),
            pin("out_first", PinType::Output, VariableType::Boolean, 1),
        );
        layer.pins.insert(
            "out_second_id".to_string(),
            pin("out_second", PinType::Output, VariableType::Float, 2),
        );

        let mut board = board_with_layer(layer);
        let logic = CallFunctionNode::new();
        let mut node = logic.get_node();
        node.get_pin_mut_by_name("function_layer_id")
            .unwrap()
            .set_default_value(Some(json!("function-layer")));

        logic.on_update(&mut node, &board).await;

        assert_eq!(
            ordered_pin_names(&node, PinType::Input),
            vec!["function_layer_id", "first", "second"]
        );
        assert_eq!(
            ordered_pin_names(&node, PinType::Output),
            vec!["out_first", "out_second"]
        );

        let layer = board.layers.get_mut("function-layer").unwrap();
        layer.pins.get_mut("first_id").unwrap().index = 2;
        layer.pins.get_mut("second_id").unwrap().index = 1;
        layer.pins.get_mut("out_first_id").unwrap().index = 2;
        layer.pins.get_mut("out_second_id").unwrap().index = 1;

        logic.on_update(&mut node, &board).await;

        assert_eq!(
            ordered_pin_names(&node, PinType::Input),
            vec!["function_layer_id", "second", "first"]
        );
        assert_eq!(
            ordered_pin_names(&node, PinType::Output),
            vec!["out_second", "out_first"]
        );
    }

    fn inputs(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn cache_keys_ignore_the_order_inputs_happen_to_be_stored_in() {
        let forwards = inputs(&[("alpha", json!(1)), ("beta", json!("two"))]);
        let backwards = inputs(&[("beta", json!("two")), ("alpha", json!(1))]);

        assert_eq!(
            CallFunctionNode::cache_key("layer", &forwards),
            CallFunctionNode::cache_key("layer", &backwards)
        );
    }

    #[test]
    fn cache_keys_ignore_key_order_inside_nested_objects() {
        let forwards = inputs(&[("payload", json!({ "a": 1, "b": [ { "x": 1, "y": 2 } ] }))]);
        let backwards = inputs(&[("payload", json!({ "b": [ { "y": 2, "x": 1 } ], "a": 1 }))]);

        assert_eq!(
            CallFunctionNode::cache_key("layer", &forwards),
            CallFunctionNode::cache_key("layer", &backwards)
        );
    }

    #[test]
    fn cache_keys_separate_layers_inputs_and_input_names() {
        let base = inputs(&[("alpha", json!(1))]);

        assert_ne!(
            CallFunctionNode::cache_key("layer-a", &base),
            CallFunctionNode::cache_key("layer-b", &base)
        );
        assert_ne!(
            CallFunctionNode::cache_key("layer", &base),
            CallFunctionNode::cache_key("layer", &inputs(&[("alpha", json!(2))]))
        );
        // Length-prefixing the name keeps ("ab", 1) distinct from ("a", "b1")-style shifts.
        assert_ne!(
            CallFunctionNode::cache_key("layer", &inputs(&[("ab", json!(1))])),
            CallFunctionNode::cache_key("layer", &inputs(&[("a", json!(1))]))
        );
        // An array and the same items as separate arguments must not collide.
        assert_ne!(
            CallFunctionNode::cache_key("layer", &inputs(&[("a", json!([1, 2]))])),
            CallFunctionNode::cache_key("layer", &inputs(&[("a", json!([1])), ("b", json!([2]))]))
        );
    }

    #[test]
    fn cache_handle_carries_the_prefix_as_the_namespace() {
        let settings = LayerCache {
            enabled: true,
            prefix: "  pricing  ".to_string(),
            ttl_seconds: Some(60),
            scope: LayerCacheScope::User,
        };

        let handle = CallFunctionNode::cache_handle(&settings);
        assert_eq!(handle.namespace, "pricing");
        assert!(matches!(handle.scope, CacheScope::User));
        assert_eq!(settings.ttl(), Some(60));
    }

    #[test]
    fn a_zero_or_omitted_lifetime_explicitly_disables_expiry() {
        let mut settings = LayerCache {
            enabled: true,
            prefix: String::new(),
            ttl_seconds: Some(0),
            scope: LayerCacheScope::App,
        };
        assert_eq!(settings.ttl(), Some(0));

        settings.ttl_seconds = None;
        assert_eq!(settings.ttl(), Some(0));
    }

    #[test]
    fn caching_is_off_until_it_is_switched_on() {
        let settings = LayerCache::default();
        assert!(!settings.is_active());
        assert!(
            Layer::new("l".into(), "L".into(), LayerType::Function)
                .cache
                .is_none()
        );
    }
}
