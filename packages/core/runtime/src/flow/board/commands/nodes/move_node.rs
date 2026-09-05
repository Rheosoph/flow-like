use flow_like_types::async_trait;
use schemars::JsonSchema;
use std::sync::Arc;

use crate::{
    flow::board::{Board, commands::Command},
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct MoveNodeCommand {
    pub node_id: String,
    pub from_coordinates: Option<(f32, f32, f32)>,
    pub to_coordinates: (f32, f32, f32),
    pub current_layer: Option<String>,
}

impl MoveNodeCommand {
    pub fn new(
        node_id: String,
        to_coordinates: (f32, f32, f32),
        current_layer: Option<String>,
    ) -> Self {
        MoveNodeCommand {
            node_id,
            from_coordinates: None,
            to_coordinates,
            current_layer,
        }
    }
}

fn apply_offset_recursive(board: &mut Board, layer_id: &str, offset: (f32, f32, f32)) {
    // move all nodes in this layer
    for node in board
        .nodes
        .values_mut()
        .filter(|n| n.layer.as_deref() == Some(layer_id))
    {
        if let Some((x, y, z)) = node.coordinates {
            node.coordinates = Some((x + offset.0, y + offset.1, z + offset.2));
        }
    }

    // move all comments in this layer
    for comment in board
        .comments
        .values_mut()
        .filter(|c| c.layer.as_deref() == Some(layer_id))
    {
        comment.coordinates = (
            comment.coordinates.0 + offset.0,
            comment.coordinates.1 + offset.1,
            comment.coordinates.2 + offset.2,
        );
    }

    // find & move each direct child layer, then recurse
    let child_ids: Vec<String> = board
        .layers
        .values()
        .filter(|l| l.parent_id.as_deref() == Some(layer_id))
        .map(|l| l.id.clone())
        .collect();

    for child_id in child_ids {
        if let Some(child) = board.layers.get_mut(&child_id) {
            child.coordinates = (
                child.coordinates.0 + offset.0,
                child.coordinates.1 + offset.1,
                child.coordinates.2 + offset.2,
            );
        }
        apply_offset_recursive(board, &child_id, offset);
    }
}

#[async_trait]
impl Command for MoveNodeCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        if self.node_id.ends_with("-input") {
            let id = self.node_id.trim_end_matches("-input").to_string();
            let layer = board.layers.get_mut(&id).ok_or_else(|| {
                flow_like_types::anyhow!(format!("Layer with id {} not found", id))
            })?;

            self.from_coordinates = layer.in_coordinates;
            layer.in_coordinates = Some(self.to_coordinates);
            return Ok(());
        }

        if self.node_id.ends_with("-return") {
            let id = self.node_id.trim_end_matches("-return").to_string();
            let layer = board.layers.get_mut(&id).ok_or_else(|| {
                flow_like_types::anyhow!(format!("Layer with id {} not found", id))
            })?;

            self.from_coordinates = layer.out_coordinates;
            layer.out_coordinates = Some(self.to_coordinates);
            return Ok(());
        }

        if let Some(layer) = board.layers.get_mut(&self.node_id) {
            let offset = layer.coordinates;

            let offset = (
                self.to_coordinates.0 - offset.0,
                self.to_coordinates.1 - offset.1,
                self.to_coordinates.2 - offset.2,
            );

            self.from_coordinates = Some(layer.coordinates);
            layer.coordinates = self.to_coordinates;
            apply_offset_recursive(board, &self.node_id, offset);

            return Ok(());
        }

        if let Some(comment) = board.comments.get_mut(&self.node_id) {
            self.from_coordinates = Some(comment.coordinates);
            comment.coordinates = self.to_coordinates;
            return Ok(());
        }

        let current_layer = self.current_layer.clone().unwrap_or("".to_string());
        let mutable_layer = board.layers.get_mut(&current_layer);

        if let Some(node) = board.nodes.get_mut(&self.node_id) {
            self.from_coordinates = node.coordinates;
            let node_layer = node.layer.clone().unwrap_or("".to_string());

            if node_layer != current_layer {
                if let Some(layer) = mutable_layer {
                    match layer.nodes.get_mut(&self.node_id) {
                        Some(node) => {
                            node.coordinates = Some(self.to_coordinates);
                        }
                        None => {
                            let mut new_node = node.clone();
                            new_node.coordinates = Some(self.to_coordinates);
                            layer.nodes.insert(node.id.clone(), new_node);
                        }
                    }
                }
                return Ok(());
            }

            node.coordinates = Some(self.to_coordinates);
            return Ok(());
        }

        Err(flow_like_types::anyhow!(format!(
            "Node: {} not found",
            self.node_id
        )))
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        if self.node_id.ends_with("-input") {
            let id = self.node_id.trim_end_matches("-input").to_string();
            if let Some(layer) = board.layers.get_mut(&id) {
                layer.in_coordinates = self.from_coordinates;
            }
            return Ok(());
        }

        if self.node_id.ends_with("-return") {
            let id = self.node_id.trim_end_matches("-return").to_string();
            if let Some(layer) = board.layers.get_mut(&id) {
                layer.out_coordinates = self.from_coordinates;
            }
            return Ok(());
        }

        if let Some(layer) = board.layers.get_mut(&self.node_id) {
            if let Some(from_coordinates) = self.from_coordinates {
                layer.coordinates = from_coordinates;
                let offset = (
                    from_coordinates.0 - self.to_coordinates.0,
                    from_coordinates.1 - self.to_coordinates.1,
                    from_coordinates.2 - self.to_coordinates.2,
                );
                apply_offset_recursive(board, &self.node_id, offset);
            }
            return Ok(());
        }

        if let Some(comment) = board.comments.get_mut(&self.node_id) {
            if let Some(from_coordinates) = self.from_coordinates {
                comment.coordinates = from_coordinates;
            }
            return Ok(());
        }

        if let Some(node) = board.nodes.get_mut(&self.node_id) {
            if let Some(from_coordinates) = self.from_coordinates {
                let node_layer = node.layer.clone().unwrap_or("".to_string());
                let current_layer = self.current_layer.clone().unwrap_or("".to_string());
                let mutable_layer = board.layers.get_mut(&current_layer);

                if node_layer != current_layer {
                    if let Some(layer) = mutable_layer {
                        match layer.nodes.get_mut(&self.node_id) {
                            Some(node) => {
                                node.coordinates = Some(from_coordinates);
                            }
                            None => {
                                let mut new_node = node.clone();
                                new_node.coordinates = Some(from_coordinates);
                                layer.nodes.insert(node.id.clone(), new_node);
                            }
                        }
                    }
                    return Ok(());
                }

                node.coordinates = Some(from_coordinates);
            }
            return Ok(());
        }

        Ok(())
    }
}
