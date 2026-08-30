use flow_like_types::async_trait;

use schemars::JsonSchema;
use std::collections::HashSet;
use std::sync::Arc;

use crate::flow::board::{Layer, LayerType};
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

        let is_module = matches!(self.layer.r#type, LayerType::Module);

        // A module is organizational only: it has no boundary, so it can carry neither pins nor
        // the cache that a function-layer call would look up.
        if is_module {
            self.layer.pins.clear();
            self.layer.cache = None;
            self.layer.in_coordinates = None;
            self.layer.out_coordinates = None;
        }

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

        // A module only nests inside another module — anything else roots it.
        if is_module {
            self.layer.parent_id = self.layer.parent_id.take().filter(|parent| {
                matches!(
                    board.layers.get(parent).map(|layer| &layer.r#type),
                    Some(LayerType::Module)
                )
            });
        }

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
                // A module child would lose its only legal home under anything but a module.
                if !is_module && matches!(layer.r#type, LayerType::Module) {
                    continue;
                }

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
    use crate::flow::board::LayerCache;
    use crate::flow::node::Node;
    use crate::flow::variable::VariableType;
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

    fn module(id: &str) -> Layer {
        Layer::new(id.into(), id.into(), LayerType::Module)
    }

    #[flow_like_types::tokio::test]
    async fn a_module_never_carries_a_boundary() {
        let mut board = board_with_nested_layers();

        let mut created = module("mod");
        let mut node = Node::new("n", "N", "", "test");
        let pin = node
            .add_output_pin("out", "Out", "", VariableType::String)
            .clone();
        created.pins.insert(pin.id.clone(), pin);
        created.cache = Some(LayerCache::default());
        created.in_coordinates = Some((1.0, 2.0, 3.0));
        created.out_coordinates = Some((4.0, 5.0, 6.0));

        let mut command = UpsertLayerCommand::new(created);
        command.execute(&mut board, state()).await.expect("upsert");

        let stored = &board.layers["mod"];
        assert!(stored.pins.is_empty());
        assert_eq!(stored.cache, None);
        assert_eq!(stored.in_coordinates, None);
        assert_eq!(stored.out_coordinates, None);
    }

    #[flow_like_types::tokio::test]
    async fn a_module_created_inside_a_non_module_lands_at_the_root() {
        let mut board = board_with_nested_layers();

        let mut command = UpsertLayerCommand::new(module("mod"));
        command.current_layer = Some("inner".into());
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["mod"].parent_id, None);
    }

    #[flow_like_types::tokio::test]
    async fn a_module_created_inside_a_module_keeps_its_parent() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        let parent = module("parent");
        board.layers.insert(parent.id.clone(), parent);

        let mut command = UpsertLayerCommand::new(module("child"));
        command.current_layer = Some("parent".into());
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["child"].parent_id.as_deref(), Some("parent"));
    }

    #[flow_like_types::tokio::test]
    async fn adopting_never_moves_a_module_under_a_non_module() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        board.layers.insert("mod".into(), module("mod"));

        let created = Layer::new("group".into(), "Group".into(), LayerType::Collapsed);
        let mut command = UpsertLayerCommand::new(created);
        command.node_ids = vec!["mod".into()];
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["mod"].parent_id, None);
    }

    #[flow_like_types::tokio::test]
    async fn a_module_adopts_a_module_child() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        board.layers.insert("child".into(), module("child"));

        let mut command = UpsertLayerCommand::new(module("parent"));
        command.node_ids = vec!["child".into()];
        command.execute(&mut board, state()).await.expect("upsert");

        assert_eq!(board.layers["child"].parent_id.as_deref(), Some("parent"));
    }
}
