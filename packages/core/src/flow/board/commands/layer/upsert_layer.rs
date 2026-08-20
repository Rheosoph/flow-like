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

        // `current_layer` picks the parent for a layer that is being created. Updating an
        // existing layer must never move it: the boundary nodes of an open layer report that
        // layer as the current one, which would make it its own parent and cut it out of the
        // hierarchy every rename, pin edit or comment.
        self.layer.parent_id = board
            .layers
            .get(&self.layer.id)
            .map(|existing| existing.parent_id.clone())
            .unwrap_or_else(|| self.current_layer.clone())
            .filter(|parent| parent != &self.layer.id);

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
            if nodes_set.contains(&layer.id) && layer.id != self.layer.id {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::LayerType;
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use crate::utils::http::HTTPClient;
    use flow_like_storage::Path;

    fn state() -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ))
    }

    fn board_with_nested_layers() -> Board {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        let mut outer = Layer::new("outer".into(), "Outer".into(), LayerType::Collapsed);
        outer.parent_id = None;
        let mut inner = Layer::new("inner".into(), "Inner".into(), LayerType::Collapsed);
        inner.parent_id = Some(outer.id.clone());
        board.layers.insert(outer.id.clone(), outer);
        board.layers.insert(inner.id.clone(), inner);
        board
    }

    #[flow_like_types::tokio::test]
    async fn updating_an_open_layer_keeps_its_parent() {
        let mut board = board_with_nested_layers();

        // The boundary nodes of an open layer report that layer as `current_layer`.
        let mut renamed = board.layers["inner"].clone();
        renamed.name = "Renamed".into();
        let mut command = UpsertLayerCommand::new(renamed);
        command.current_layer = Some("inner".into());
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["inner"].name, "Renamed");
        assert_eq!(
            board.layers["inner"].parent_id.as_deref(),
            Some("outer"),
            "an update must not re-parent the layer"
        );
    }

    #[flow_like_types::tokio::test]
    async fn updating_a_layer_from_the_root_keeps_its_parent() {
        let mut board = board_with_nested_layers();

        let mut command = UpsertLayerCommand::new(board.layers["inner"].clone());
        command.current_layer = None;
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["inner"].parent_id.as_deref(), Some("outer"));
    }

    #[flow_like_types::tokio::test]
    async fn creating_a_layer_parents_it_to_the_current_layer() {
        let mut board = board_with_nested_layers();

        let created = Layer::new("created".into(), "Created".into(), LayerType::Collapsed);
        let mut command = UpsertLayerCommand::new(created);
        command.current_layer = Some("inner".into());
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["created"].parent_id.as_deref(), Some("inner"));
    }

    #[flow_like_types::tokio::test]
    async fn a_layer_can_never_become_its_own_parent() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());

        let created = Layer::new("self".into(), "Self".into(), LayerType::Collapsed);
        let mut command = UpsertLayerCommand::new(created);
        command.current_layer = Some("self".into());
        command.node_ids = vec!["self".into()];
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["self"].parent_id, None);
    }
}
