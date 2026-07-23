use serde::{Deserialize, Serialize};

use crate::flow::board::Board;
use crate::flow::node::Node;
use crate::flow::pin::PinType;
use flow_like_types::Result;

/// Compact node representation for context
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct NodeContext {
    pub id: String,
    #[serde(rename = "t")] // "type" abbreviated
    pub node_type: String,
    #[serde(rename = "n")] // "name" abbreviated
    pub friendly_name: String,
    #[serde(rename = "i")] // "inputs" abbreviated
    pub inputs: Vec<PinContext>,
    #[serde(rename = "o")] // "outputs" abbreviated
    pub outputs: Vec<PinContext>,
    #[serde(rename = "p")] // "position" abbreviated
    pub position: (i32, i32),
    #[serde(rename = "s")] // "size" abbreviated
    pub estimated_size: (u16, u16),
}

/// Compact pin representation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinContext {
    #[serde(rename = "n")] // "name" abbreviated
    pub name: String,
    #[serde(rename = "t")] // "type" abbreviated
    pub type_name: String,
    /// Only included if pin has a non-empty default value
    #[serde(rename = "v", skip_serializing_if = "Option::is_none")] // "value" abbreviated
    pub default_value: Option<String>,
}

/// Compact edge representation
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EdgeContext {
    #[serde(rename = "f")] // "from" abbreviated
    pub from_node_id: String,
    #[serde(rename = "fp")] // "from_pin" abbreviated
    pub from_pin_name: String,
    #[serde(rename = "t")] // "to" abbreviated
    pub to_node_id: String,
    #[serde(rename = "tp")] // "to_pin" abbreviated
    pub to_pin_name: String,
}

/// Compact layer representation for context
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LayerContext {
    pub id: String,
    #[serde(rename = "n")] // "name" abbreviated
    pub name: String,
    /// Parent layer ID if nested, None if at root
    #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Node IDs contained in this layer
    #[serde(rename = "nodes")]
    pub node_ids: Vec<String>,
    #[serde(rename = "pos")] // "position" abbreviated
    pub position: (i32, i32),
    /// Input pins (for connecting TO this layer)
    #[serde(rename = "i", skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<PinContext>,
    /// Output pins (for connecting FROM this layer)
    #[serde(rename = "o", skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<PinContext>,
}

/// Compact variable representation for context
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VariableContext {
    pub id: String,
    #[serde(rename = "n")] // "name" abbreviated
    pub name: String,
    #[serde(rename = "t")] // "type" abbreviated
    pub data_type: String,
    #[serde(rename = "vt")] // "value_type" abbreviated
    pub value_type: String,
    #[serde(rename = "c", skip_serializing_if = "Option::is_none")] // "category" abbreviated
    pub category: Option<String>,
    #[serde(rename = "v", skip_serializing_if = "Option::is_none")] // "value" abbreviated
    pub default_value: Option<String>,
}

/// Complete graph context for the LLM
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphContext {
    pub nodes: Vec<NodeContext>,
    pub edges: Vec<EdgeContext>,
    /// All layers in the board with their hierarchy
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub layers: Vec<LayerContext>,
    /// All variables defined in the board
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub variables: Vec<VariableContext>,
    pub selected_nodes: Vec<String>,
}

/// Prepare graph context from a board
pub fn prepare_context(board: &Board, selected_node_ids: &[String]) -> Result<GraphContext> {
    let mut node_contexts = Vec::new();
    let mut pin_to_node_map = std::collections::HashMap::new();

    // Helper to process nodes
    let mut process_nodes = |nodes: &std::collections::HashMap<String, Node>| {
        for node in nodes.values() {
            for pin_id in node.pins.keys() {
                pin_to_node_map.insert(pin_id.clone(), node.id.clone());
            }
        }
    };

    // Build pin to node map for root nodes
    process_nodes(&board.nodes);
    // Build pin to node map for layer nodes
    for layer in board.layers.values() {
        process_nodes(&layer.nodes);
    }

    // Helper to create context
    let mut create_node_contexts = |nodes: &std::collections::HashMap<String, Node>| {
        for node in nodes.values() {
            // Only include non-execution pins with meaningful info
            let inputs: Vec<PinContext> = node
                .pins
                .iter()
                .filter(|(_, p)| p.pin_type == PinType::Input)
                .map(|(_, p)| {
                    let default_val = p
                        .default_value
                        .as_ref()
                        .map(|v| String::from_utf8_lossy(v).to_string())
                        .filter(|s| !s.is_empty() && s != "null");
                    PinContext {
                        name: p.name.clone(),
                        type_name: format!("{:?}", p.data_type),
                        default_value: default_val,
                    }
                })
                .collect();

            let outputs: Vec<PinContext> = node
                .pins
                .iter()
                .filter(|(_, p)| p.pin_type == PinType::Output)
                .map(|(_, p)| PinContext {
                    name: p.name.clone(),
                    type_name: format!("{:?}", p.data_type),
                    default_value: None, // Outputs don't have default values
                })
                .collect();

            // Estimate node size based on pin count
            let input_count = inputs.len();
            let output_count = outputs.len();
            let max_pins = input_count.max(output_count);
            let estimated_width = 200u16;
            let estimated_height = 32u16 + (max_pins as u16 * 20);

            let (x, y) = node
                .coordinates
                .map(|(x, y, _)| (x as i32, y as i32))
                .unwrap_or((0, 0));

            node_contexts.push(NodeContext {
                id: node.id.clone(),
                node_type: node.name.clone(),
                friendly_name: node.friendly_name.clone(),
                inputs,
                outputs,
                position: (x, y),
                estimated_size: (estimated_width, estimated_height),
            });
        }
    };

    create_node_contexts(&board.nodes);
    for layer in board.layers.values() {
        create_node_contexts(&layer.nodes);
    }

    let mut edge_contexts = Vec::new();

    let mut process_edges = |nodes: &std::collections::HashMap<String, Node>| {
        for node in nodes.values() {
            for pin in node.pins.values() {
                // We only care about outgoing connections to avoid duplicates
                if pin.pin_type == PinType::Output {
                    for connected_pin_id in &pin.connected_to {
                        if let Some(target_node_id) = pin_to_node_map.get(connected_pin_id) {
                            let target_pin = board.get_pin_by_id(connected_pin_id);
                            edge_contexts.push(EdgeContext {
                                from_node_id: node.id.clone(),
                                from_pin_name: pin.name.clone(),
                                to_node_id: target_node_id.clone(),
                                to_pin_name: target_pin.map(|p| p.name.clone()).unwrap_or_default(),
                            });
                        }
                    }
                }
            }
        }
    };

    process_edges(&board.nodes);
    for layer in board.layers.values() {
        process_edges(&layer.nodes);
    }

    // Build layer contexts
    let layer_contexts: Vec<LayerContext> = board
        .layers
        .values()
        .map(|layer| {
            let (x, y) = (layer.coordinates.0 as i32, layer.coordinates.1 as i32);

            // Build input and output pin lists for the layer
            let inputs: Vec<PinContext> = layer
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Input)
                .map(|p| PinContext {
                    name: p.name.clone(),
                    type_name: format!("{:?}", p.data_type),
                    default_value: None,
                })
                .collect();

            let outputs: Vec<PinContext> = layer
                .pins
                .values()
                .filter(|p| p.pin_type == PinType::Output)
                .map(|p| PinContext {
                    name: p.name.clone(),
                    type_name: format!("{:?}", p.data_type),
                    default_value: None,
                })
                .collect();

            LayerContext {
                id: layer.id.clone(),
                name: layer.name.clone(),
                parent_id: layer.parent_id.clone(),
                node_ids: layer.nodes.keys().cloned().collect(),
                position: (x, y),
                inputs,
                outputs,
            }
        })
        .collect();

    // Build variable contexts
    let variable_contexts: Vec<VariableContext> = board
        .variables
        .values()
        .map(|var| {
            let default_val = var
                .default_value
                .as_ref()
                .map(|v| String::from_utf8_lossy(v).to_string())
                .filter(|s| !s.is_empty() && s != "null");
            VariableContext {
                id: var.id.clone(),
                name: var.name.clone(),
                data_type: format!("{:?}", var.data_type),
                value_type: format!("{:?}", var.value_type),
                category: var.category.clone(),
                default_value: default_val,
            }
        })
        .collect();

    Ok(GraphContext {
        nodes: node_contexts,
        edges: edge_contexts,
        layers: layer_contexts,
        variables: variable_contexts,
        selected_nodes: selected_node_ids.to_vec(),
    })
}
