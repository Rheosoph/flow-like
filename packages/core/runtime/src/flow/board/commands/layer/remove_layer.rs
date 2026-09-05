use flow_like_types::async_trait;

use schemars::JsonSchema;
use std::collections::HashSet;
use std::sync::Arc;

use crate::flow::board::Layer;
use crate::{
    flow::{
        board::{Board, commands::Command},
        node::Node,
    },
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemoveLayerCommand {
    pub layer: Layer,
    pub layer_nodes: HashSet<String>,
    pub child_layers: HashSet<String>,
    pub layers: Vec<Layer>,
    pub nodes: Vec<Node>,
    pub preserve_nodes: bool,
}

impl RemoveLayerCommand {
    pub fn new(layer: Layer, nodes: Vec<Node>, preserve_nodes: bool) -> Self {
        RemoveLayerCommand {
            layer,
            nodes,
            preserve_nodes,
            layers: vec![],
            layer_nodes: HashSet::new(),
            child_layers: HashSet::new(),
        }
    }
}

#[async_trait]
impl Command for RemoveLayerCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        if !self.preserve_nodes {
            // 1) Collect & remove the target layer + all nested children
            let mut removed_layers = HashSet::new();
            let mut to_visit = vec![self.layer.id.clone()];

            while let Some(current_id) = to_visit.pop() {
                // enqueue child IDs before we remove
                let children = board
                    .layers
                    .values()
                    .filter(|l| l.parent_id.as_deref() == Some(&current_id))
                    .map(|l| l.id.clone())
                    .collect::<Vec<_>>();

                // remove this layer
                if let Some(layer) = board.layers.remove(&current_id) {
                    self.layers.push(layer.clone());
                    removed_layers.insert(current_id.clone());
                    // schedule its children for removal
                    to_visit.extend(children);
                }
            }

            // 2) Drop any nodes belonging to those removed layers
            board.nodes.retain(|_, node| {
                if let Some(layer_id) = &node.layer
                    && removed_layers.contains(layer_id)
                {
                    self.nodes.push(node.clone());
                    return false;
                }
                true
            });
        } else {
            // Preserve nodes: reparent them to the removed layer’s parent
            let parent = self.layer.parent_id.clone();
            let target = Some(self.layer.id.clone());

            // reparent nodes
            for node in board.nodes.values_mut() {
                if node.layer == target {
                    node.layer = parent.clone();
                    self.layer_nodes.insert(node.id.clone());
                }
            }

            // reparent comments
            for comment in board.comments.values_mut() {
                if comment.layer == target {
                    comment.layer = parent.clone();
                    self.layer_nodes.insert(comment.id.clone());
                }
            }

            // reparent child layers
            for layer in board.layers.values_mut() {
                if layer.parent_id == target {
                    layer.parent_id = parent.clone();
                    self.child_layers.insert(layer.id.clone());
                }
            }

            // finally drop the layer itself
            board.layers.remove(&self.layer.id);
        }

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        // 1) Restore fully-removed layers
        for layer in &self.layers {
            board.layers.insert(layer.id.clone(), layer.clone());
        }
        // 2) Restore the primary layer
        board
            .layers
            .insert(self.layer.id.clone(), self.layer.clone());
        // 3) Restore fully-removed nodes
        for node in &self.nodes {
            board.nodes.insert(node.id.clone(), node.clone());
        }
        // 4) Reparent any nodes that were preserved
        for id in &self.layer_nodes {
            if let Some(node) = board.nodes.get_mut(id) {
                node.layer = Some(self.layer.id.clone());
            }
        }
        // 5) Reparent any child layers that were preserved
        for id in &self.child_layers {
            if let Some(layer) = board.layers.get_mut(id) {
                layer.parent_id = Some(self.layer.id.clone());
            }
        }

        Ok(())
    }
}
