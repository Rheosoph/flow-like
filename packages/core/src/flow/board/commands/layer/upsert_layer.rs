use flow_like_types::async_trait;

use schemars::JsonSchema;
use std::collections::HashSet;
use std::sync::Arc;

use crate::flow::board::Layer;
use crate::{
    flow::board::{Board, commands::Command},
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpsertLayerCommand {
    pub old_layer: Option<Layer>,
    pub layer: Layer,
    pub node_ids: Vec<String>,
    pub current_layer: Option<String>,
}

impl UpsertLayerCommand {
    pub fn new(layer: Layer) -> Self {
        UpsertLayerCommand {
            layer,
            old_layer: None,
            node_ids: vec![],
            current_layer: None,
        }
    }
}

#[async_trait]
impl Command for UpsertLayerCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let nodes_set: HashSet<String> = HashSet::from_iter(self.node_ids.iter().cloned());

        let mut added_coordinates = (0.0, 0.0, 0.0);
        let mut total_coordinates = 0;

        self.layer.parent_id = self.current_layer.clone();
        self.old_layer = board
            .layers
            .insert(self.layer.id.clone(), self.layer.clone());

        for node in board.nodes.values_mut() {
            if nodes_set.contains(&node.id) {
                node.layer = Some(self.layer.id.clone());
                total_coordinates += 1;
                let coordinates = node.coordinates.unwrap_or((0.0, 0.0, 0.0));
                added_coordinates = (
                    added_coordinates.0 + coordinates.0,
                    added_coordinates.1 + coordinates.1,
                    added_coordinates.2 + coordinates.2,
                );
            }
        }

        for comment in board.comments.values_mut() {
            if nodes_set.contains(&comment.id) {
                comment.layer = Some(self.layer.id.clone());
                total_coordinates += 1;
                added_coordinates = (
                    added_coordinates.0 + comment.coordinates.0,
                    added_coordinates.1 + comment.coordinates.1,
                    added_coordinates.2 + comment.coordinates.2,
                );
            }
        }

        for layer in board.layers.values_mut() {
            if nodes_set.contains(&layer.id) {
                layer.parent_id = Some(self.layer.id.clone());
                total_coordinates += 1;
                added_coordinates = (
                    added_coordinates.0 + layer.coordinates.0,
                    added_coordinates.1 + layer.coordinates.1,
                    added_coordinates.2 + layer.coordinates.2,
                );
            }
        }

        if self.old_layer.is_none() && total_coordinates > 0 {
            let center_position = (
                added_coordinates.0 / total_coordinates as f32,
                added_coordinates.1 / total_coordinates as f32,
                added_coordinates.2 / total_coordinates as f32,
            );

            self.layer.coordinates = center_position;
        }

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        let mut old_layer_id = None;
        if let Some(old_layer) = self.old_layer.take() {
            old_layer_id = Some(old_layer.id.clone());
            board.layers.insert(old_layer.id.clone(), old_layer.clone());
        } else {
            board.layers.remove(&self.layer.id);
        }

        for node in board.nodes.values_mut() {
            if node.layer == Some(self.layer.id.clone()) {
                node.layer = old_layer_id.clone();
            }
        }

        for comment in board.comments.values_mut() {
            if comment.layer == Some(self.layer.id.clone()) {
                comment.layer = old_layer_id.clone();
            }
        }

        for layer in board.layers.values_mut() {
            if layer.parent_id == Some(self.layer.id.clone()) {
                layer.parent_id = old_layer_id.clone();
            }
        }

        Ok(())
    }
}
