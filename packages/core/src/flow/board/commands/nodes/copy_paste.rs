use std::{collections::HashMap, sync::Arc};

use crate::{
    flow::{
        board::{Board, Comment, Layer, commands::Command},
        node::Node,
        pin::PinType,
        variable::Variable,
    },
    state::FlowLikeState,
};
use flow_like_types::async_trait;
use flow_like_types::{create_id, json::from_slice};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CopyPasteCommand {
    pub original_nodes: Vec<Node>,
    pub original_comments: Vec<Comment>,
    pub original_layers: Vec<Layer>,
    #[serde(default)]
    pub original_variables: Vec<Variable>,
    pub new_comments: Vec<Comment>,
    pub new_nodes: Vec<Node>,
    pub new_layers: Vec<Layer>,
    pub current_layer: Option<String>,
    pub old_mouse: Option<(f32, f32, f32)>,
    pub offset: (f32, f32, f32),
}

impl CopyPasteCommand {
    pub fn new(
        original_nodes: Vec<Node>,
        comments: Vec<Comment>,
        layers: Vec<Layer>,
        offset: (f32, f32, f32),
    ) -> Self {
        CopyPasteCommand {
            original_nodes,
            original_comments: comments,
            original_layers: layers,
            original_variables: vec![],
            old_mouse: None,
            current_layer: None,
            offset,
            new_nodes: vec![],
            new_comments: vec![],
            new_layers: vec![],
        }
    }
}

#[async_trait]
impl Command for CopyPasteCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let node_registry = state.node_registry.read().await.node_registry.clone();
        let mut translated_connection = HashMap::with_capacity(self.original_nodes.len());
        let mut intermediate_nodes = Vec::with_capacity(self.original_nodes.len());
        let mut intermediate_layers = Vec::with_capacity(self.original_layers.len());
        let offset = self.offset;
        let offset = self
            .original_comments
            .first()
            .map(|comment| {
                let old_coors = comment.coordinates;
                (
                    offset.0 - old_coors.0,
                    offset.1 - old_coors.1,
                    offset.2 - old_coors.2,
                )
            })
            .unwrap_or(offset);
        let mut offset = self
            .original_nodes
            .first()
            .map(|node| {
                let old_coors = node.coordinates.unwrap_or((0.0, 0.0, 0.0));
                (
                    offset.0 - old_coors.0,
                    offset.1 - old_coors.1,
                    offset.2 - old_coors.2,
                )
            })
            .unwrap_or(offset);

        if let Some(old_mouse) = self.old_mouse {
            offset = (
                self.offset.0 - old_mouse.0,
                self.offset.1 - old_mouse.1,
                self.offset.2 - old_mouse.2,
            );
        }

        let mut layer_translation = HashMap::with_capacity(self.original_layers.len());

        // First pass: create new IDs and build translation map
        for layer in self.original_layers.iter() {
            let layer_id = create_id();
            layer_translation.insert(layer.id.clone(), layer_id.clone());
            let mut new_layer = layer.clone();
            new_layer.id = layer_id.clone();
            new_layer.coordinates = (
                new_layer.coordinates.0 + offset.0,
                new_layer.coordinates.1 + offset.1,
                new_layer.coordinates.2 + offset.2,
            );

            // Handle parent_id translation for nested layers
            if new_layer.parent_id.is_none() || new_layer.parent_id == Some("".to_string()) {
                new_layer.parent_id = self.current_layer.clone();
            } else if let Some(parent_id) = new_layer.parent_id.clone() {
                // If parent is also being pasted, it will be translated in the second pass
                // For now, keep the original parent_id - it will be updated below
                new_layer.parent_id = Some(parent_id);
            }

            new_layer.pins = layer
                .pins
                .values()
                .map(|pin| {
                    let mut pin = pin.clone();
                    let old_pin_id = pin.id.clone();
                    let new_pin_id = create_id();
                    translated_connection.insert(old_pin_id, new_pin_id.clone());
                    pin.id = new_pin_id.clone();
                    (new_pin_id, pin)
                })
                .collect();

            intermediate_layers.push(new_layer.clone());
        }

        // Second pass: translate parent_ids now that all layer IDs are known
        for layer in intermediate_layers.iter_mut() {
            if let Some(parent_id) = &layer.parent_id
                && let Some(new_parent_id) = layer_translation.get(parent_id)
            {
                layer.parent_id = Some(new_parent_id.clone());
            }
            // Don't insert yet - pin connections need to be translated first in the final pass
        }

        for comment in self.original_comments.iter() {
            let mut new_comment = comment.clone();
            new_comment.id = create_id();
            new_comment.coordinates = (
                new_comment.coordinates.0 + offset.0,
                new_comment.coordinates.1 + offset.1,
                new_comment.coordinates.2 + offset.2,
            );

            if new_comment.layer.is_none() || new_comment.layer == Some("".to_string()) {
                new_comment.layer = self.current_layer.clone();
            } else if let Some(layer_id) = new_comment.layer.clone()
                && let Some(new_layer_id) = layer_translation.get(&layer_id)
            {
                new_comment.layer = Some(new_layer_id.clone());
            }

            board
                .comments
                .insert(new_comment.id.clone(), new_comment.clone());
            self.new_comments.push(new_comment);
        }

        for node in self.original_nodes.iter() {
            let mut new_node = node.clone();
            let blueprint_node = node_registry.get_node(&node.name).ok();

            // If the pasted node is external (has wasm metadata) but not in the registry, reject it
            if blueprint_node.is_none() && node.wasm.is_some() {
                return Err(flow_like_types::anyhow!(
                    "External node '{}' from package '{}' is not installed. Please install the package first.",
                    node.friendly_name,
                    node.wasm
                        .as_ref()
                        .map(|w| w.package_id.as_str())
                        .unwrap_or("unknown")
                ));
            }

            let blueprint_node = blueprint_node.unwrap_or(node.clone());
            let old_id = new_node.id.clone();
            let new_id = create_id();
            translated_connection.insert(old_id, new_id.clone());
            new_node.id = new_id.clone();
            new_node.category = blueprint_node.category.clone();
            new_node.docs = blueprint_node.docs.clone();
            new_node.icon = blueprint_node.icon.clone();
            new_node.scores = blueprint_node.scores.clone();
            new_node.start = blueprint_node.start;
            new_node.event_callback = blueprint_node.event_callback;
            new_node.wasm = blueprint_node.wasm.clone();

            // Preserve user-customized friendly_name and description for start nodes (events)
            let is_start_node = blueprint_node.start.unwrap_or(false);
            if !is_start_node {
                new_node.description = blueprint_node.description.clone();
            }
            new_node.coordinates = Some((
                new_node.coordinates.unwrap_or((0.0, 0.0, 0.0)).0 + offset.0,
                new_node.coordinates.unwrap_or((0.0, 0.0, 0.0)).1 + offset.1,
                new_node.coordinates.unwrap_or((0.0, 0.0, 0.0)).2 + offset.2,
            ));

            if new_node.layer.is_none() || new_node.layer == Some("".to_string()) {
                new_node.layer = self.current_layer.clone();
            } else if let Some(layer_id) = new_node.layer.clone()
                && let Some(new_layer_id) = layer_translation.get(&layer_id)
            {
                new_node.layer = Some(new_layer_id.clone());
            }

            new_node.pins = new_node
                .pins
                .values()
                .map(|pin| {
                    let mut pin = pin.clone();
                    let old_pin_id = pin.id.clone();
                    let (_, blueprint_pin) = blueprint_node
                        .pins
                        .iter()
                        .find(|(_, p)| p.name == pin.name && pin.pin_type == p.pin_type)
                        .unwrap_or((&String::new(), &pin));
                    let blueprint_pin = blueprint_pin.clone();
                    let new_pin_id = create_id();
                    translated_connection.insert(old_pin_id, new_pin_id.clone());
                    pin.id = new_pin_id.clone();
                    pin.description = blueprint_pin.description.clone();

                    if pin.name == "var_ref"
                        && let Some(var_ref) = pin.default_value.as_ref()
                    {
                        let var_ref = from_slice::<String>(var_ref);
                        if let Ok(var_ref) = var_ref {
                            let variable_ref = board.variables.get(&var_ref);
                            if variable_ref.is_none() {
                                // Try to find the original variable from the template/copy source
                                let original_var =
                                    self.original_variables.iter().find(|v| v.id == var_ref);

                                if let Some(orig) = original_var {
                                    // Clone the original variable to preserve all properties including category
                                    let mut new_var = orig.clone();
                                    new_var.id = var_ref.clone();
                                    board.variables.insert(var_ref.clone(), new_var);
                                } else {
                                    // Fallback: create a new variable with basic info
                                    let var_name = if new_node.friendly_name.starts_with("Get ") {
                                        new_node.friendly_name.replace("Get ", "")
                                    } else if new_node.friendly_name.starts_with("Set ") {
                                        new_node.friendly_name.replace("Set ", "")
                                    } else {
                                        new_node.friendly_name.clone()
                                    };
                                    println!(
                                        "Creating new variable: {}, friendly name: {}",
                                        var_name, new_node.friendly_name
                                    );
                                    let (_id, value_ref_pin) = new_node
                                        .pins
                                        .iter()
                                        .find(|(_, p)| p.name == "value_ref")
                                        .unwrap_or((&String::new(), &pin));
                                    let mut new_var = Variable::new(
                                        &var_name,
                                        value_ref_pin.data_type.clone(),
                                        value_ref_pin.value_type.clone(),
                                    );
                                    new_var.id = var_ref.clone();
                                    board.variables.insert(var_ref.clone(), new_var);
                                }
                            }
                        }
                    }

                    // Translate function_layer_id when pasting CallFunction nodes
                    if pin.name == "function_layer_id"
                        && let Some(ref_bytes) = pin.default_value.as_ref()
                    {
                        if let Ok(layer_ref) = from_slice::<String>(ref_bytes) {
                            if let Some(new_layer_id) = layer_translation.get(&layer_ref) {
                                if let Ok(bytes) = flow_like_types::json::to_vec(new_layer_id) {
                                    pin.default_value = Some(bytes);
                                }
                            }
                        }
                    }

                    // Only override schema/options from the blueprint if the blueprint
                    // actually defines them. Dynamic schemas (set by on_update, not
                    // in get_node()) must survive the paste cycle so that on_update
                    // can find the schema on the first pass without clearing pins.
                    if blueprint_pin.schema.is_some() {
                        pin.schema = blueprint_pin.schema.clone();
                    }
                    if blueprint_pin.options.is_some() {
                        pin.options = blueprint_pin.options.clone();
                    }

                    if new_node.start.unwrap_or(false)
                        && pin.pin_type == PinType::Input
                        && pin.name != "type"
                    {
                        pin.default_value = None;
                    }

                    (new_pin_id, pin)
                })
                .collect();

            // Preserve user-customized friendly_name for start nodes (events)
            if !is_start_node {
                new_node.friendly_name = blueprint_node.friendly_name.clone();
            }
            intermediate_nodes.push(new_node);
        }

        for node in intermediate_nodes.iter() {
            let mut new_node = node.clone();
            for pin in new_node.pins.values_mut() {
                pin.depends_on = pin
                    .depends_on
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();

                pin.connected_to = pin
                    .connected_to
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();
            }

            // Remap fn_refs to new node IDs, then validate and deduplicate
            if let Some(fn_refs) = &mut new_node.fn_refs {
                // First, remap to new IDs
                fn_refs.fn_refs = fn_refs
                    .fn_refs
                    .iter()
                    .filter_map(|ref_id| translated_connection.get(ref_id).cloned())
                    .collect();

                // Then validate and deduplicate - never trust the frontend!
                super::validate_and_deduplicate_fn_refs(fn_refs, board);
            }

            board.nodes.insert(new_node.id.clone(), new_node.clone());
            self.new_nodes.push(new_node);
        }

        for layer in intermediate_layers.iter() {
            let mut new_layer = layer.clone();
            for pin in new_layer.pins.values_mut() {
                pin.depends_on = pin
                    .depends_on
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();

                pin.connected_to = pin
                    .connected_to
                    .iter()
                    .filter(|dep_id| translated_connection.contains_key(*dep_id))
                    .map(|dep_id| translated_connection.get(dep_id).unwrap_or(dep_id).clone())
                    .collect();
            }
            board.layers.insert(new_layer.id.clone(), new_layer.clone());
            self.new_layers.push(new_layer);
        }

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        for node in self.new_nodes.iter() {
            board.nodes.remove(&node.id);
        }

        for comment in self.new_comments.iter() {
            board.comments.remove(&comment.id);
        }

        for layer in self.new_layers.iter() {
            board.layers.remove(&layer.id);
        }

        Ok(())
    }
}
