use std::collections::{BTreeMap, HashSet};

use flow_like::flow::{
    board::{Board, Layer, LayerType},
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    pin::{Pin, PinType},
    utils::evaluate_pin_value,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::from_slice};

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
        let output_data_pins: Vec<_> = context
            .node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output && p.data_type != VariableType::Execution)
            .cloned()
            .collect();
        for pin in &output_data_pins {
            let _ = context.set_pin_ref_value(pin, Value::Null).await;
        }

        // Check if the function is impure (has an input execution layer pin)
        let exec_in_layer_pin = layer
            .pins
            .values()
            .find(|p| p.pin_type == PinType::Input && p.data_type == VariableType::Execution);

        if let Some(exec_pin) = exec_in_layer_pin {
            // --- IMPURE function: exec chain inside the function ---
            // Deactivate all mirrored output exec pins
            let output_exec_names: Vec<String> = context
                .node
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Output && p.data_type == VariableType::Execution)
                .map(|p| p.name().to_string())
                .collect();
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
        input_pins.sort_by(|a, b| a.index.cmp(&b.index));

        let mut output_pins: Vec<_> = layer
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .cloned()
            .collect();
        output_pins.sort_by(|a, b| a.index.cmp(&b.index));

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
        let mut layers = std::collections::HashMap::new();
        layers.insert(layer.id.clone(), layer);

        Board {
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: std::collections::HashMap::new(),
            variables: std::collections::HashMap::new(),
            comments: std::collections::HashMap::new(),
            viewport: (0.0, 0.0, 0.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: std::collections::HashMap::new(),
            layers,
            page_ids: Vec::new(),
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Default::default(),
            logic_nodes: std::collections::HashMap::new(),
            app_state: None,
            hash: None,
        }
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
}
