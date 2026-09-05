use flow_like_types::async_trait;

use schemars::JsonSchema;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::flow::board::LayerType;
use crate::{
    flow::board::{Board, commands::Command},
    state::FlowLikeState,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct MoveToLayerCommand {
    pub ids: Vec<String>,
    pub target: Option<String>,
    #[serde(default)]
    pub previous: HashMap<String, Option<String>>,
}

impl MoveToLayerCommand {
    pub fn new(ids: Vec<String>, target: Option<String>) -> Self {
        MoveToLayerCommand {
            ids,
            target,
            previous: HashMap::new(),
        }
    }
}

/// Walks `descendant_id`'s parent chain looking for `ancestor_id`, so a move that would nest a
/// layer under its own descendant can be refused instead of wedging the tree into a cycle.
fn is_ancestor(board: &Board, ancestor_id: &str, descendant_id: &str) -> bool {
    let mut current = board
        .layers
        .get(descendant_id)
        .and_then(|layer| layer.parent_id.clone());
    let mut seen = HashSet::new();

    while let Some(id) = current {
        if id == ancestor_id {
            return true;
        }
        if !seen.insert(id.clone()) {
            return false;
        }
        current = board
            .layers
            .get(&id)
            .and_then(|layer| layer.parent_id.clone());
    }

    false
}

#[async_trait]
impl Command for MoveToLayerCommand {
    async fn execute(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        // A stale `previous` from an earlier execute must never survive into this run's undo,
        // including when the early return below skips the move entirely.
        self.previous.clear();
        if let Some(target) = &self.target {
            let targets_a_module = matches!(
                board.layers.get(target).map(|layer| &layer.r#type),
                Some(LayerType::Module)
            );

            if !targets_a_module {
                tracing::warn!(
                    "MoveToLayer target {} is not an existing Module layer, skipping the move",
                    target
                );
                return Ok(());
            }
        }

        let ids = self.ids.clone();
        let target = self.target.clone();

        for id in &ids {
            // A duplicated id must not overwrite its recorded origin with the moved value.
            if self.previous.contains_key(id) {
                continue;
            }
            if let Some(node) = board.nodes.get_mut(id) {
                self.previous.insert(id.clone(), node.layer.clone());
                node.layer = target.clone();
                continue;
            }

            if let Some(comment) = board.comments.get_mut(id) {
                self.previous.insert(id.clone(), comment.layer.clone());
                comment.layer = target.clone();
                continue;
            }

            if board.layers.contains_key(id) {
                if let Some(target) = &target
                    && (target == id || is_ancestor(board, id, target))
                {
                    tracing::warn!(
                        "MoveToLayer skipping layer {} — moving it under {} would create a cycle",
                        id,
                        target
                    );
                    continue;
                }

                let layer = board
                    .layers
                    .get_mut(id)
                    .expect("layer looked up above still present");
                self.previous.insert(id.clone(), layer.parent_id.clone());
                layer.parent_id = target.clone();
                continue;
            }

            tracing::warn!("MoveToLayer skipping unknown id {}", id);
        }

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        for (id, previous) in self.previous.drain() {
            if let Some(node) = board.nodes.get_mut(&id) {
                node.layer = previous;
                continue;
            }

            if let Some(comment) = board.comments.get_mut(&id) {
                comment.layer = previous;
                continue;
            }

            if let Some(layer) = board.layers.get_mut(&id) {
                layer.parent_id = previous;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::{Comment, CommentType, Layer};
    use crate::flow::node::Node;
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use crate::utils::http::HTTPClient;
    use flow_like_storage::Path;
    use std::time::SystemTime;

    fn state() -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ))
    }

    fn module(board: &mut Board, id: &str, parent: Option<&str>) {
        let mut layer = Layer::new(id.into(), id.into(), LayerType::Module);
        layer.parent_id = parent.map(str::to_string);
        board.layers.insert(id.into(), layer);
    }

    fn collapsed(board: &mut Board, id: &str, parent: Option<&str>) {
        let mut layer = Layer::new(id.into(), id.into(), LayerType::Collapsed);
        layer.parent_id = parent.map(str::to_string);
        board.layers.insert(id.into(), layer);
    }

    fn function_layer(board: &mut Board, id: &str, parent: Option<&str>) {
        let mut layer = Layer::new(id.into(), id.into(), LayerType::Function);
        layer.parent_id = parent.map(str::to_string);
        board.layers.insert(id.into(), layer);
    }

    fn insert_node(board: &mut Board) -> String {
        let node = Node::new("n", "N", "", "test");
        let id = node.id.clone();
        board.nodes.insert(id.clone(), node);
        id
    }

    fn insert_comment(board: &mut Board) -> String {
        let comment = Comment {
            id: "comment-1".to_string(),
            author: None,
            content: "a comment".to_string(),
            comment_type: CommentType::Text,
            timestamp: SystemTime::now(),
            coordinates: (0.0, 0.0, 0.0),
            width: None,
            height: None,
            layer: None,
            color: None,
            z_index: None,
            hash: None,
            is_locked: None,
            node_id: None,
        };
        let id = comment.id.clone();
        board.comments.insert(id.clone(), comment);
        id
    }

    #[flow_like_types::tokio::test]
    async fn moving_a_node_to_a_module_and_back_via_undo() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod", None);
        let node_id = insert_node(&mut board);

        let mut command = MoveToLayerCommand::new(vec![node_id.clone()], Some("mod".into()));
        command.execute(&mut board, state()).await.expect("move");
        assert_eq!(board.nodes[&node_id].layer.as_deref(), Some("mod"));

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(board.nodes[&node_id].layer, None);
    }

    #[flow_like_types::tokio::test]
    async fn moving_a_comment_to_a_module() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod", None);
        let comment_id = insert_comment(&mut board);

        let mut command = MoveToLayerCommand::new(vec![comment_id.clone()], Some("mod".into()));
        command.execute(&mut board, state()).await.expect("move");
        assert_eq!(board.comments[&comment_id].layer.as_deref(), Some("mod"));

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(board.comments[&comment_id].layer, None);
    }

    #[flow_like_types::tokio::test]
    async fn moving_a_function_layer_between_modules_reparents_it() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod_a", None);
        module(&mut board, "mod_b", None);
        function_layer(&mut board, "fn1", Some("mod_a"));

        let mut command = MoveToLayerCommand::new(vec!["fn1".into()], Some("mod_b".into()));
        command.execute(&mut board, state()).await.expect("move");
        assert_eq!(board.layers["fn1"].parent_id.as_deref(), Some("mod_b"));

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(board.layers["fn1"].parent_id.as_deref(), Some("mod_a"));
    }

    #[flow_like_types::tokio::test]
    async fn a_module_among_the_ids_nests_under_the_target_module() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod_target", None);
        module(&mut board, "mod_other", None);
        let node_id = insert_node(&mut board);

        let mut command = MoveToLayerCommand::new(
            vec!["mod_other".into(), node_id.clone()],
            Some("mod_target".into()),
        );
        command.execute(&mut board, state()).await.expect("move");

        assert_eq!(
            board.layers["mod_other"].parent_id.as_deref(),
            Some("mod_target")
        );
        assert_eq!(board.nodes[&node_id].layer.as_deref(), Some("mod_target"));

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(board.layers["mod_other"].parent_id, None);
        assert_eq!(board.nodes[&node_id].layer, None);
    }

    #[flow_like_types::tokio::test]
    async fn a_module_moved_to_the_root_loses_its_parent() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod_parent", None);
        module(&mut board, "mod_child", Some("mod_parent"));

        let mut command = MoveToLayerCommand::new(vec!["mod_child".into()], None);
        command.execute(&mut board, state()).await.expect("move");
        assert_eq!(board.layers["mod_child"].parent_id, None);

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(
            board.layers["mod_child"].parent_id.as_deref(),
            Some("mod_parent")
        );
    }

    #[flow_like_types::tokio::test]
    async fn a_module_move_under_its_own_descendant_is_refused() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod_parent", None);
        module(&mut board, "mod_child", Some("mod_parent"));

        let mut command =
            MoveToLayerCommand::new(vec!["mod_parent".into()], Some("mod_child".into()));
        command.execute(&mut board, state()).await.expect("move");

        assert_eq!(board.layers["mod_parent"].parent_id, None);
        assert!(command.previous.is_empty());
    }

    #[flow_like_types::tokio::test]
    async fn a_collapsed_target_makes_the_whole_command_a_no_op() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        collapsed(&mut board, "col", None);
        let node_id = insert_node(&mut board);

        let mut command = MoveToLayerCommand::new(vec![node_id.clone()], Some("col".into()));
        command.execute(&mut board, state()).await.expect("no-op");

        assert_eq!(board.nodes[&node_id].layer, None);
        assert!(command.previous.is_empty());
    }

    #[flow_like_types::tokio::test]
    async fn a_target_descendant_of_the_moved_layer_is_skipped() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        collapsed(&mut board, "col", None);
        // Deliberately built by hand instead of via UpsertLayerCommand: a module nested under a
        // non-module layer should never happen through the normal command path, but the move
        // guard must still refuse to turn this into a cycle if the data ever ends up this way.
        module(&mut board, "mod_target", Some("col"));

        let mut command = MoveToLayerCommand::new(vec!["col".into()], Some("mod_target".into()));
        command.execute(&mut board, state()).await.expect("move");

        assert_eq!(board.layers["col"].parent_id, None);
        assert!(command.previous.is_empty());
    }

    #[flow_like_types::tokio::test]
    async fn an_unknown_id_is_skipped_silently() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod", None);

        let mut command =
            MoveToLayerCommand::new(vec!["does-not-exist".into()], Some("mod".into()));
        command.execute(&mut board, state()).await.expect("move");

        assert!(command.previous.is_empty());
    }

    #[flow_like_types::tokio::test]
    async fn undo_restores_a_mixed_batch_exactly() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        module(&mut board, "mod_a", None);
        module(&mut board, "mod_b", None);
        function_layer(&mut board, "fn1", Some("mod_a"));
        let node_id = insert_node(&mut board);
        let comment_id = insert_comment(&mut board);

        let mut command = MoveToLayerCommand::new(
            vec!["fn1".into(), node_id.clone(), comment_id.clone()],
            Some("mod_b".into()),
        );
        command.execute(&mut board, state()).await.expect("move");

        assert_eq!(board.layers["fn1"].parent_id.as_deref(), Some("mod_b"));
        assert_eq!(board.nodes[&node_id].layer.as_deref(), Some("mod_b"));
        assert_eq!(board.comments[&comment_id].layer.as_deref(), Some("mod_b"));

        command.undo(&mut board, state()).await.expect("undo");

        assert_eq!(board.layers["fn1"].parent_id.as_deref(), Some("mod_a"));
        assert_eq!(board.nodes[&node_id].layer, None);
        assert_eq!(board.comments[&comment_id].layer, None);
    }
}
