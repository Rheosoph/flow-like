use std::{collections::HashSet, sync::Arc};

use flow_like::flow::{
    board::{Board, Layer, LayerType},
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    pin::PinType,
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

    async fn read_outputs(&self, context: &mut ExecutionContext, layer: &Layer) {
        for layer_pin in layer.pins.values() {
            if layer_pin.pin_type != PinType::Output
                || layer_pin.data_type == VariableType::Execution
            {
                continue;
            }

            // Find the inner node output pin that feeds this layer output (via depends_on)
            for dep_pin_id in &layer_pin.depends_on {
                let mut found = false;
                for node in context.nodes.values() {
                    if let Ok(inner_pin) = node.get_pin_by_id(dep_pin_id) {
                        if let Some(value) = inner_pin.get_value().await {
                            let _ = context.set_pin_value(&layer_pin.name, value).await;
                        }
                        found = true;
                        break;
                    }
                }
                if found {
                    break;
                }
            }
        }
    }

    fn find_node_id_by_pin(board: &Board, layer: &Layer, layer_id: &str, pin_id: &str) -> Option<String> {
        layer.nodes.values()
            .find(|n| n.pins.contains_key(pin_id))
            .or_else(|| board.nodes.values().find(|n| n.layer.as_deref() == Some(layer_id) && n.pins.contains_key(pin_id)))
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
            .filter(|p| {
                p.pin_type == PinType::Output && p.data_type != VariableType::Execution
            })
            .collect();

        if output_layer_pins.is_empty() {
            return Ok(());
        }

        // Find all inner nodes that directly feed outputs
        let mut triggered: HashSet<String> = HashSet::new();

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
                    if pin.pin_type != PinType::Input
                        || pin.data_type == VariableType::Execution
                    {
                        continue;
                    }
                    if let Some(value) = input_values.get(&pin.name) {
                        fn_context.override_pin_value(pin_id, value.clone());
                    }
                }

                let result =
                    InternalNode::trigger(&mut fn_context, &mut None, false).await;
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
        self.read_outputs(context, layer).await;
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
                .filter(|p| {
                    p.pin_type == PinType::Output && p.data_type == VariableType::Execution
                })
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
            fn_context.end_trace();
            context.push_sub_context(&mut fn_context);

            if let Err(error) = run_result {
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

            self.read_outputs(context, layer).await;

            for name in &output_exec_names {
                let _ = context.activate_exec_pin(name).await;
            }
        } else {
            // --- PURE function: evaluate outputs by triggering feeding nodes ---
            self.run_pure_function(context, &board, layer, &function_layer_id, &input_values).await?;
        }

        Ok(())
    }


    async fn on_update(&self, node: &mut Node, board: Arc<Board>) {
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

        let mut relevant_pin_names = HashSet::new();
        relevant_pin_names.insert("function_layer_id".to_string());

        for pin in &input_pins {
            relevant_pin_names.insert(pin.name.clone());
            if node.pins.iter().any(|(_, p)| {
                p.name == pin.name && p.pin_type == PinType::Input
            }) {
                continue;
            }
            let new_pin = node.add_input_pin(
                &pin.name,
                &pin.friendly_name,
                &pin.description,
                pin.data_type.clone(),
            );
            new_pin.value_type = pin.value_type.clone();
            new_pin.schema = pin.schema.clone();
            new_pin.options = pin.options.clone();
        }

        for pin in &output_pins {
            relevant_pin_names.insert(pin.name.clone());
            if node.pins.iter().any(|(_, p)| {
                p.name == pin.name && p.pin_type == PinType::Output
            }) {
                continue;
            }
            let new_pin = node.add_output_pin(
                &pin.name,
                &pin.friendly_name,
                &pin.description,
                pin.data_type.clone(),
            );
            new_pin.value_type = pin.value_type.clone();
            new_pin.schema = pin.schema.clone();
            new_pin.options = pin.options.clone();
        }

        // Remove stale pins (only keep function_layer_id + mirrored pins)
        node.pins.retain(|_, p| relevant_pin_names.contains(&p.name));
    }
}
