use flow_like_types::async_trait;

use schemars::JsonSchema;
use std::collections::HashSet;
use std::sync::Arc;

use super::remove_layer::splice_boundary;
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
        let owner = match self.old_layer.take() {
            Some(old_layer) => {
                let id = old_layer.id.clone();
                board.layers.insert(id.clone(), old_layer);
                Some(id)
            }
            // The editor's history holds this command without its cleanup receipts, so the live
            // layer still carries the bridges cleanup minted, and dropping them unspliced would cut
            // every wire into the group. Once the receipts ran, the live layer is pinless again.
            None => match board.layers.remove(&self.layer.id) {
                Some(live) => {
                    splice_boundary(board, &live);
                    live.parent_id
                }
                None => self.layer.parent_id.clone(),
            },
        };
        let target = Some(self.layer.id.clone());

        for node in board.nodes.values_mut() {
            if node.layer == target {
                node.layer = owner.clone();
            }
        }

        for comment in board.comments.values_mut() {
            if comment.layer == target {
                comment.layer = owner.clone();
            }
        }

        for layer in board.layers.values_mut() {
            if layer.parent_id == target {
                layer.parent_id = owner.clone();
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::commands::GenericCommand;
    use crate::flow::board::commands::pins::connect_pins::connect_pins;
    use crate::flow::board::{Comment, CommentType, LayerCache};
    use crate::flow::node::Node;
    use crate::flow::pin::PinType;
    use crate::flow::variable::VariableType;
    use crate::state::{FlowLikeConfig, FlowLikeState};
    use crate::utils::http::HTTPClient;
    use flow_like_storage::Path;
    use std::collections::{BTreeMap, BTreeSet};
    use std::time::SystemTime;

    type Wiring = BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>;

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

    fn pin_id(board: &Board, node: &str, name: &str) -> String {
        board.nodes[node]
            .pins
            .values()
            .find(|pin| pin.name == name)
            .map(|pin| pin.id.clone())
            .unwrap_or_else(|| panic!("pin {node}.{name} is missing"))
    }

    /// `src.value` fans into `a.in` and `b.in`, `far.value` feeds `b.extra` and `a.out` feeds
    /// `sink.value` and `far.in`; exec runs `src → a → sink`. Every node but `far`, which stays at
    /// the root, lives in `home`, a collapsed layer when given. Cleaned, so `home` has its bridges.
    fn fan_board(home: Option<&str>) -> Board {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        if let Some(home) = home {
            let layer = Layer::new(home.into(), home.into(), LayerType::Collapsed);
            board.layers.insert(home.into(), layer);
        }
        for id in ["src", "a", "b", "sink", "far"] {
            let mut node = Node::new(id, id, "", "test");
            node.id = id.into();
            node.layer = home.filter(|_| id != "far").map(str::to_string);
            board.nodes.insert(id.into(), node);
        }
        for (node, name, pin_type, data_type) in [
            ("src", "exec_out", PinType::Output, VariableType::Execution),
            ("src", "value", PinType::Output, VariableType::String),
            ("a", "exec_in", PinType::Input, VariableType::Execution),
            ("a", "exec_out", PinType::Output, VariableType::Execution),
            ("a", "in", PinType::Input, VariableType::String),
            ("a", "out", PinType::Output, VariableType::String),
            ("b", "in", PinType::Input, VariableType::String),
            ("b", "extra", PinType::Input, VariableType::String),
            ("sink", "exec_in", PinType::Input, VariableType::Execution),
            ("sink", "value", PinType::Input, VariableType::String),
            ("far", "value", PinType::Output, VariableType::String),
            ("far", "in", PinType::Input, VariableType::String),
        ] {
            let node = board.nodes.get_mut(node).expect("node exists");
            match pin_type {
                PinType::Input => node.add_input_pin(name, name, "", data_type),
                PinType::Output => node.add_output_pin(name, name, "", data_type),
            };
        }
        for (from_node, from, to_node, to) in [
            ("src", "exec_out", "a", "exec_in"),
            ("a", "exec_out", "sink", "exec_in"),
            ("src", "value", "a", "in"),
            ("src", "value", "b", "in"),
            ("far", "value", "b", "extra"),
            ("a", "out", "sink", "value"),
            ("a", "out", "far", "in"),
        ] {
            let from = pin_id(&board, from_node, from);
            let to = pin_id(&board, to_node, to);
            connect_pins(&mut board, from_node, &from, to_node, &to).expect("pins connect");
        }
        board.cleanup();
        board
    }

    fn collapse(home: Option<&str>, members: &[&str]) -> UpsertLayerCommand {
        let mut command = UpsertLayerCommand::new(Layer::new(
            "group".into(),
            "Group".into(),
            LayerType::Collapsed,
        ));
        command.node_ids = members.iter().map(|id| id.to_string()).collect();
        command.current_layer = home.map(str::to_string);
        command
    }

    fn add_comment(board: &mut Board, id: &str, layer: Option<&str>) {
        let comment = Comment {
            id: id.into(),
            author: None,
            content: id.into(),
            comment_type: CommentType::Text,
            timestamp: SystemTime::UNIX_EPOCH,
            coordinates: (0.0, 0.0, 0.0),
            width: None,
            height: None,
            layer: layer.map(str::to_string),
            color: None,
            z_index: None,
            hash: None,
            is_locked: None,
            node_id: None,
        };
        board.comments.insert(id.into(), comment);
    }

    fn node_wiring(board: &Board) -> Wiring {
        board
            .nodes
            .values()
            .flat_map(|node| node.pins.values())
            .map(|pin| {
                (
                    pin.id.clone(),
                    (pin.depends_on.clone(), pin.connected_to.clone()),
                )
            })
            .collect()
    }

    /// Every pin with its owner, plus where each node and layer sits. Hashes are left out because
    /// `node_updates` restates them.
    fn fingerprint(board: &Board) -> BTreeMap<String, String> {
        let node_pins = board.nodes.values().flat_map(|node| {
            node.pins
                .values()
                .map(move |pin| (format!("{}/{}", node.id, pin.id), pin))
        });
        let layer_pins = board.layers.values().flat_map(|layer| {
            layer
                .pins
                .values()
                .map(move |pin| (format!("{}/{}", layer.id, pin.id), pin))
        });
        let mut fingerprint: BTreeMap<String, String> = node_pins
            .chain(layer_pins)
            .map(|(key, pin)| (key, serde_json::to_string(pin).expect("pin serializes")))
            .collect();
        for node in board.nodes.values() {
            fingerprint.insert(format!("node:{}", node.id), format!("{:?}", node.layer));
        }
        for layer in board.layers.values() {
            fingerprint.insert(
                format!("layer:{}", layer.id),
                format!("{:?}", layer.parent_id),
            );
        }
        fingerprint
    }

    #[flow_like_types::tokio::test]
    async fn undoing_a_collapse_without_its_receipts_restores_the_direct_wires() {
        let mut board = fan_board(None);
        let direct = node_wiring(&board);

        let mut command = collapse(None, &["a", "b"]);
        command
            .execute(&mut board, state())
            .await
            .expect("collapse");
        board.cleanup();
        assert_eq!(
            board.layers["group"].pins.len(),
            5,
            "bridges: src.value, far.value and exec in; a.out and exec out"
        );
        assert_ne!(node_wiring(&board), direct);

        command.undo(&mut board, state()).await.expect("undo");
        board.cleanup();

        assert!(board.layers.is_empty());
        assert!(board.nodes.values().all(|node| node.layer.is_none()));
        assert_eq!(node_wiring(&board), direct);
    }

    #[flow_like_types::tokio::test]
    async fn undoing_a_collapse_inside_a_layer_returns_its_members_to_that_layer() {
        let mut board = fan_board(Some("L"));
        add_comment(&mut board, "note", Some("L"));
        let mut child = Layer::new("child".into(), "Child".into(), LayerType::Collapsed);
        child.parent_id = Some("L".into());
        board.layers.insert(child.id.clone(), child);
        assert_eq!(
            board.layers["L"].pins.len(),
            2,
            "far crosses L once each way"
        );
        let original = fingerprint(&board);

        let mut command = collapse(Some("L"), &["a", "b", "note", "child"]);
        command
            .execute(&mut board, state())
            .await
            .expect("collapse");
        board.cleanup();
        assert_eq!(board.layers["group"].parent_id.as_deref(), Some("L"));
        assert_eq!(board.nodes["a"].layer.as_deref(), Some("group"));
        assert_eq!(board.comments["note"].layer.as_deref(), Some("group"));
        assert_eq!(board.layers["child"].parent_id.as_deref(), Some("group"));

        command.undo(&mut board, state()).await.expect("undo");
        board.cleanup();

        assert!(!board.layers.contains_key("group"));
        for id in ["a", "b"] {
            assert_eq!(board.nodes[id].layer.as_deref(), Some("L"), "{id}");
        }
        assert_eq!(board.comments["note"].layer.as_deref(), Some("L"));
        assert_eq!(board.layers["child"].parent_id.as_deref(), Some("L"));
        assert_eq!(fingerprint(&board), original);
    }

    #[flow_like_types::tokio::test]
    async fn undoing_a_collapse_after_its_receipts_leaves_nothing_to_splice() {
        let state = state();
        let mut board = fan_board(None);
        let direct = node_wiring(&board);

        let commands = board
            .execute_commands(
                vec![GenericCommand::UpsertLayer(collapse(None, &["a", "b"]))],
                state.clone(),
            )
            .await
            .expect("collapse");
        assert!(
            commands.iter().any(|command| matches!(
                command,
                GenericCommand::UpsertLayer(receipt)
                    if receipt.old_layer.as_ref().is_some_and(|layer| layer.pins.is_empty())
            )),
            "the layer receipt restores the pinless layer before the collapse undoes"
        );

        board.undo(commands, state).await.expect("undo");

        assert!(board.layers.is_empty());
        assert_eq!(node_wiring(&board), direct);
    }

    #[flow_like_types::tokio::test]
    async fn redoing_an_undone_collapse_mints_the_same_bridges() {
        let state = state();
        let mut board = fan_board(None);
        let direct = node_wiring(&board);

        let command = board
            .execute_command(
                GenericCommand::UpsertLayer(collapse(None, &["a", "b"])),
                state.clone(),
            )
            .await
            .expect("collapse");
        let collapsed = fingerprint(&board);

        board
            .undo(vec![command.clone()], state.clone())
            .await
            .expect("undo");
        assert!(board.layers.is_empty());
        assert_eq!(node_wiring(&board), direct);

        board.redo(vec![command], state).await.expect("redo");
        assert_eq!(fingerprint(&board), collapsed);
    }

    #[flow_like_types::tokio::test]
    async fn collapse_at_the_member_mean_preserves_coordinates_through_undo_redo() {
        let state = state();
        let mut board = fan_board(None);
        board.nodes.get_mut("a").expect("member a").coordinates = Some((100.0, 40.0, 0.0));
        board.nodes.get_mut("b").expect("member b").coordinates = Some((500.0, 160.0, 0.0));
        board
            .nodes
            .get_mut("src")
            .expect("outside source")
            .coordinates = Some((-300.0, 90.0, 0.0));

        let mut reroute = Node::new("reroute", "Reroute", "", "test");
        reroute.id = "dot".into();
        reroute.coordinates = Some((900.0, 320.0, 0.0));
        board.nodes.insert(reroute.id.clone(), reroute);

        let mut child = Layer::new("child".into(), "Child".into(), LayerType::Collapsed);
        child.coordinates = (1300.0, 600.0, 0.0);
        board.layers.insert(child.id.clone(), child);
        let mut nested = Node::new("nested", "Nested", "", "test");
        nested.id = "nested".into();
        nested.layer = Some("child".into());
        nested.coordinates = Some((6000.0, 8000.0, 0.0));
        board.nodes.insert(nested.id.clone(), nested);
        board.cleanup();

        let original_positions: BTreeMap<_, _> = board
            .nodes
            .iter()
            .map(|(id, node)| (id.clone(), node.coordinates))
            .collect();
        let original_wiring = node_wiring(&board);
        let child_position = board.layers["child"].coordinates;
        // Every selected top-left contributes once. A child layer's contents do not.
        let anchor = (700.0, 280.0, 0.0);
        let mut collapse = collapse(None, &["a", "b", "dot", "child"]);
        collapse.layer.coordinates = anchor;
        let command = board
            .execute_command(GenericCommand::UpsertLayer(collapse), state.clone())
            .await
            .expect("collapse at the preview anchor");

        assert_eq!(board.layers["group"].coordinates, anchor);
        assert_eq!(board.layers["child"].parent_id.as_deref(), Some("group"));
        assert_eq!(board.nodes["nested"].layer.as_deref(), Some("child"));
        let collapsed = fingerprint(&board);

        for _ in 0..2 {
            board
                .undo(vec![command.clone()], state.clone())
                .await
                .expect("undo collapse");
            assert!(!board.layers.contains_key("group"));
            assert_eq!(board.layers["child"].parent_id, None);
            assert_eq!(node_wiring(&board), original_wiring);
            for (id, coordinates) in &original_positions {
                assert_eq!(&board.nodes[id].coordinates, coordinates, "undo moved {id}");
            }
            assert_eq!(board.layers["child"].coordinates, child_position);

            board
                .redo(vec![command.clone()], state.clone())
                .await
                .expect("redo collapse");
            assert_eq!(board.layers["group"].coordinates, anchor);
            assert_eq!(fingerprint(&board), collapsed);
            for (id, coordinates) in &original_positions {
                assert_eq!(&board.nodes[id].coordinates, coordinates, "redo moved {id}");
            }
            assert_eq!(board.layers["child"].coordinates, child_position);
        }
    }
}
