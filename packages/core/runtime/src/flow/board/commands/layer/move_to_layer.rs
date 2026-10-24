use flow_like_types::async_trait;

use schemars::JsonSchema;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use crate::flow::board::{LayerType, PinOwner};
use crate::flow::pin::{Pin, PinType};
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
    /// Every pin the move rewired while it unthreaded group bridges, as it was before the move.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub previous_pins: Vec<PinBefore>,
}

/// A pin as it was before a move rewired or removed it.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PinBefore {
    pub holder: PinHolder,
    pub pin: Pin,
}

/// Where a pin lives.
#[derive(Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum PinHolder {
    Node(String),
    Layer(String),
    LayerNode { layer: String, node: String },
}

impl MoveToLayerCommand {
    pub fn new(ids: Vec<String>, target: Option<String>) -> Self {
        MoveToLayerCommand {
            ids,
            target,
            previous: HashMap::new(),
            previous_pins: Vec::new(),
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

/// Collapsed and Macro layers are groups: presentation over the graph, transparent to execution.
fn is_group(kind: &LayerType) -> bool {
    matches!(kind, LayerType::Collapsed | LayerType::Macro)
}

/// Whether a layer of `kind` may sit under `parent` (`None` = the board root). A group nests
/// anywhere; a module or Function layer only in a module or at the root.
fn may_nest(kind: &LayerType, parent: Option<&LayerType>) -> bool {
    is_group(kind) || matches!(parent, None | Some(LayerType::Module))
}

impl MoveToLayerCommand {
    /// The target's layer kind: `Some(None)` for the board root, `None` when the target layer does
    /// not exist and the whole move is skipped.
    fn target_kind(&self, board: &Board) -> Option<Option<LayerType>> {
        let Some(target) = &self.target else {
            return Some(None);
        };
        match board.layers.get(target) {
            Some(layer) => Some(Some(layer.r#type.clone())),
            None => {
                tracing::warn!(
                    "MoveToLayer target {} is not an existing layer, skipping the move",
                    target
                );
                None
            }
        }
    }

    /// The ids that may move: nodes and comments always, a layer only where it may nest and
    /// without forming a cycle.
    fn movable_ids(&self, board: &Board, target_kind: Option<&LayerType>) -> Vec<String> {
        let mut moving: Vec<String> = Vec::new();
        for id in &self.ids {
            let movable = board.nodes.contains_key(id)
                || board.comments.contains_key(id)
                || self.layer_may_move(board, id, target_kind);
            if movable && !moving.contains(id) {
                moving.push(id.clone());
            }
        }
        moving
    }

    fn layer_may_move(&self, board: &Board, id: &str, target_kind: Option<&LayerType>) -> bool {
        let Some(layer) = board.layers.get(id) else {
            tracing::warn!("MoveToLayer skipping unknown id {}", id);
            return false;
        };
        if !may_nest(&layer.r#type, target_kind) {
            tracing::warn!(
                "MoveToLayer skipping layer {} — a {:?} layer only nests in a module or at the board root",
                id,
                layer.r#type
            );
            return false;
        }
        if let Some(target) = &self.target
            && (target == id || is_ancestor(board, id, target))
        {
            tracing::warn!(
                "MoveToLayer skipping layer {} — moving it under {} would create a cycle",
                id,
                target
            );
            return false;
        }
        true
    }

    fn rehome(&mut self, board: &mut Board, moving: &[String]) {
        let target = self.target.clone();
        for id in moving {
            if let Some(node) = board.nodes.get_mut(id) {
                self.previous.insert(id.clone(), node.layer.clone());
                node.layer = target.clone();
            } else if let Some(comment) = board.comments.get_mut(id) {
                self.previous.insert(id.clone(), comment.layer.clone());
                comment.layer = target.clone();
            } else if let Some(layer) = board.layers.get_mut(id) {
                self.previous.insert(id.clone(), layer.parent_id.clone());
                layer.parent_id = target.clone();
            }
        }
    }

    fn restore_homes(&mut self, board: &mut Board) {
        for (id, previous) in self.previous.drain() {
            if let Some(node) = board.nodes.get_mut(&id) {
                node.layer = previous;
            } else if let Some(comment) = board.comments.get_mut(&id) {
                comment.layer = previous;
            } else if let Some(layer) = board.layers.get_mut(&id) {
                layer.parent_id = previous;
            }
        }
    }
}

/// The pins whose edges may run through group bridges a move invalidates: every pin of a moved
/// node and the outer side of every moved group. With `target`, items already there are skipped.
fn relay_starts<'i>(
    board: &Board,
    ids: impl IntoIterator<Item = &'i String>,
    target: Option<&Option<String>>,
) -> (Vec<String>, HashSet<String>) {
    let mut starts = Vec::new();
    let mut moved_groups = HashSet::new();
    for id in ids {
        if let Some(node) = board.nodes.get(id) {
            if target.is_none_or(|target| node.layer != *target) {
                starts.extend(node.pins.keys().cloned());
            }
        } else if let Some(layer) = board.layers.get(id).filter(|layer| is_group(&layer.r#type))
            && target.is_none_or(|target| layer.parent_id != *target)
        {
            starts.extend(layer.pins.keys().cloned());
            moved_groups.insert(id.clone());
        }
    }
    (starts, moved_groups)
}

/// Unthread the group relays of every start pin; returns the pins as they were before.
fn unthread(board: &mut Board, starts: &[String], moved_groups: HashSet<String>) -> Vec<PinBefore> {
    let mut unthread = Unthread::new(board, moved_groups);
    for start in starts {
        unthread.pin(start);
    }
    unthread.into_before()
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
        self.previous_pins.clear();
        let Some(target_kind) = self.target_kind(board) else {
            return Ok(());
        };
        let moving = self.movable_ids(board, target_kind.as_ref());
        let (starts, moved_groups) = relay_starts(board, &moving, Some(&self.target));
        self.previous_pins = unthread(board, &starts, moved_groups);
        self.rehome(board, &moving);
        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _state: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        // Cleanup bridged the moved items for where they went. Unthread those relays again, then
        // put every pin the move rewired back exactly as it was.
        let (starts, moved_groups) = relay_starts(board, self.previous.keys(), None);
        unthread(board, &starts, moved_groups);
        for before in self.previous_pins.drain(..) {
            restore_pin(board, before);
        }
        self.restore_homes(board);
        Ok(())
    }
}

fn restore_pin(board: &mut Board, before: PinBefore) {
    let pins = match &before.holder {
        PinHolder::Node(node) => board.nodes.get_mut(node).map(|node| &mut node.pins),
        PinHolder::Layer(layer) => board.layers.get_mut(layer).map(|layer| &mut layer.pins),
        PinHolder::LayerNode { layer, node } => board
            .layers
            .get_mut(layer)
            .and_then(|layer| layer.nodes.get_mut(node))
            .map(|node| &mut node.pins),
    };
    if let Some(pins) = pins {
        pins.insert(before.pin.id.clone(), before.pin);
    }
}

/// Rewires edges that run through group bridge pins straight to the pins at their far ends, and
/// drops bridges left without a source or a target. `BridgeLayersCleanup` then bridges the direct
/// wires for wherever the moved items now sit. Every pin it changes is recorded first.
struct Unthread<'b> {
    board: &'b mut Board,
    holders: HashMap<String, PinOwner>,
    /// Groups being moved: their own pins are starts, never relays.
    moved_groups: HashSet<String>,
    before: BTreeMap<String, PinBefore>,
}

impl<'b> Unthread<'b> {
    fn new(board: &'b mut Board, moved_groups: HashSet<String>) -> Self {
        let holders = board.build_pin_index();
        Self {
            board,
            holders,
            moved_groups,
            before: BTreeMap::new(),
        }
    }

    fn into_before(self) -> Vec<PinBefore> {
        self.before.into_values().collect()
    }

    fn holder(&self, id: &str) -> Option<PinHolder> {
        Some(match self.holders.get(id)? {
            PinOwner::Node(node) => PinHolder::Node(node.clone()),
            PinOwner::LayerPin(layer) => PinHolder::Layer(layer.clone()),
            PinOwner::LayerNode { layer, node } => PinHolder::LayerNode {
                layer: layer.clone(),
                node: node.clone(),
            },
        })
    }

    fn get(&self, id: &str) -> Option<&Pin> {
        match self.holder(id)? {
            PinHolder::Node(node) => self.board.nodes.get(&node)?.pins.get(id),
            PinHolder::Layer(layer) => self.board.layers.get(&layer)?.pins.get(id),
            PinHolder::LayerNode { layer, node } => self
                .board
                .layers
                .get(&layer)?
                .nodes
                .get(&node)?
                .pins
                .get(id),
        }
    }

    fn get_mut(&mut self, id: &str) -> Option<&mut Pin> {
        let holder = self.holder(id)?;
        if !self.before.contains_key(id) {
            let pin = self.get(id)?.clone();
            self.before.insert(
                id.to_string(),
                PinBefore {
                    holder: holder.clone(),
                    pin,
                },
            );
        }
        match holder {
            PinHolder::Node(node) => self.board.nodes.get_mut(&node)?.pins.get_mut(id),
            PinHolder::Layer(layer) => self.board.layers.get_mut(&layer)?.pins.get_mut(id),
            PinHolder::LayerNode { layer, node } => self
                .board
                .layers
                .get_mut(&layer)?
                .nodes
                .get_mut(&node)?
                .pins
                .get_mut(id),
        }
    }

    /// A bridge pin of a group that is not itself being moved.
    fn is_relay(&self, id: &str) -> bool {
        match self.holders.get(id) {
            Some(PinOwner::LayerPin(layer)) => {
                !self.moved_groups.contains(layer)
                    && self
                        .board
                        .layers
                        .get(layer)
                        .is_some_and(|layer| is_group(&layer.r#type))
            }
            _ => false,
        }
    }

    fn link(&mut self, from: &str, to: &str) {
        if let Some(pin) = self.get_mut(from) {
            pin.connected_to.insert(to.to_string());
        }
        if let Some(pin) = self.get_mut(to) {
            pin.depends_on.insert(from.to_string());
        }
    }

    fn unlink(&mut self, from: &str, to: &str) {
        if let Some(pin) = self.get_mut(from) {
            pin.connected_to.remove(to);
        }
        if let Some(pin) = self.get_mut(to) {
            pin.depends_on.remove(from);
        }
    }

    /// Unthread the relays on the side of `start` that leads away from it: both sides of a node
    /// pin, the outer side of a moved group's own bridge.
    fn pin(&mut self, start: &str) {
        let Some(pin) = self.get(start) else {
            return;
        };
        let own_group_pin = matches!(self.holders.get(start), Some(PinOwner::LayerPin(_)));
        let (forward, backward) = match (own_group_pin, &pin.pin_type) {
            (false, _) => (true, true),
            (true, PinType::Output) => (true, false),
            (true, PinType::Input) => (false, true),
        };
        if forward {
            let targets: Vec<String> = pin.connected_to.iter().cloned().collect();
            for target in targets {
                if !self.is_relay(&target) {
                    continue;
                }
                for end in self.ends(&target, start, true) {
                    self.link(start, &end);
                }
                self.unlink(start, &target);
                self.prune(&target);
            }
        }
        if backward && let Some(pin) = self.get(start) {
            let sources: Vec<String> = pin.depends_on.iter().cloned().collect();
            for source in sources {
                if !self.is_relay(&source) {
                    continue;
                }
                for end in self.ends(&source, start, false) {
                    self.link(&end, start);
                }
                self.unlink(&source, start);
                self.prune(&source);
            }
        }
    }

    /// The non-relay pins reached from relay `first` (entered from `from`) following edges away
    /// from `from`. A hop counts only when both halves of the edge agree, so a half-edge cleanup
    /// would prune never becomes a wire.
    fn ends(&self, first: &str, from: &str, forward: bool) -> Vec<String> {
        let mut ends = Vec::new();
        let mut seen: HashSet<String> = HashSet::from([first.to_string()]);
        let mut stack = vec![first.to_string()];
        while let Some(id) = stack.pop() {
            let Some(pin) = self.get(&id) else {
                continue;
            };
            let next = if forward {
                &pin.connected_to
            } else {
                &pin.depends_on
            };
            for hop in next {
                if hop == from {
                    continue;
                }
                let agrees = self.get(hop).is_some_and(|other| {
                    if forward {
                        other.depends_on.contains(&id)
                    } else {
                        other.connected_to.contains(&id)
                    }
                });
                if !agrees {
                    continue;
                }
                if self.is_relay(hop) {
                    if seen.insert(hop.clone()) {
                        stack.push(hop.clone());
                    }
                } else if !ends.contains(hop) {
                    ends.push(hop.clone());
                }
            }
        }
        ends
    }

    /// Drop a relay that lost every source or every target, and the relays that only it fed.
    fn prune(&mut self, relay: &str) {
        let Some(pin) = self.get(relay) else {
            return;
        };
        if !self.is_relay(relay) || (!pin.depends_on.is_empty() && !pin.connected_to.is_empty()) {
            return;
        }
        let targets: Vec<String> = pin.connected_to.iter().cloned().collect();
        let sources: Vec<String> = pin.depends_on.iter().cloned().collect();
        for target in &targets {
            self.unlink(relay, target);
        }
        for source in &sources {
            self.unlink(source, relay);
        }
        if let Some(PinHolder::Layer(layer)) = self.holder(relay) {
            let _ = self.get_mut(relay);
            if let Some(layer) = self.board.layers.get_mut(&layer) {
                layer.pins.remove(relay);
            }
        }
        self.holders.remove(relay);
        for next in targets.iter().chain(&sources) {
            self.prune(next);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::commands::pins::connect_pins::connect_pins;
    use crate::flow::board::{Comment, CommentType, Layer};
    use crate::flow::node::Node;
    use crate::flow::variable::VariableType;
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
    async fn a_node_moves_into_a_function_and_back_via_undo() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        function_layer(&mut board, "fn1", None);
        let node_id = insert_node(&mut board);

        let mut command = MoveToLayerCommand::new(vec![node_id.clone()], Some("fn1".into()));
        command.execute(&mut board, state()).await.expect("move");
        assert_eq!(board.nodes[&node_id].layer.as_deref(), Some("fn1"));

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(board.nodes[&node_id].layer, None);
    }

    #[flow_like_types::tokio::test]
    async fn a_layer_never_nests_inside_a_function() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        function_layer(&mut board, "fn1", None);
        module(&mut board, "mod", None);
        let node_id = insert_node(&mut board);

        let mut command =
            MoveToLayerCommand::new(vec!["mod".into(), node_id.clone()], Some("fn1".into()));
        command.execute(&mut board, state()).await.expect("move");

        assert_eq!(board.layers["mod"].parent_id, None);
        assert_eq!(board.nodes[&node_id].layer.as_deref(), Some("fn1"));
    }

    #[flow_like_types::tokio::test]
    async fn a_node_moves_into_a_group_and_back_via_undo() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        collapsed(&mut board, "col", None);
        let node_id = insert_node(&mut board);

        let mut command = MoveToLayerCommand::new(vec![node_id.clone()], Some("col".into()));
        command.execute(&mut board, state()).await.expect("move");
        assert_eq!(board.nodes[&node_id].layer.as_deref(), Some("col"));

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(board.nodes[&node_id].layer, None);
    }

    #[flow_like_types::tokio::test]
    async fn a_module_never_nests_inside_a_group_but_a_group_nests_in_a_function() {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        collapsed(&mut board, "col", None);
        collapsed(&mut board, "inner", None);
        module(&mut board, "mod", None);
        function_layer(&mut board, "fn1", None);

        let mut into_group =
            MoveToLayerCommand::new(vec!["mod".into(), "inner".into()], Some("col".into()));
        into_group.execute(&mut board, state()).await.expect("move");
        assert_eq!(board.layers["mod"].parent_id, None);
        assert_eq!(board.layers["inner"].parent_id.as_deref(), Some("col"));

        let mut into_function = MoveToLayerCommand::new(vec!["col".into()], Some("fn1".into()));
        into_function
            .execute(&mut board, state())
            .await
            .expect("move");
        assert_eq!(board.layers["col"].parent_id.as_deref(), Some("fn1"));
    }

    fn wired_node(board: &mut Board, id: &str, layer: Option<&str>) {
        let mut node = Node::new("n", "N", "", "test");
        node.id = id.into();
        node.layer = layer.map(str::to_string);
        node.add_input_pin("in", "in", "", VariableType::String);
        node.add_output_pin("out", "out", "", VariableType::String);
        board.nodes.insert(id.into(), node);
    }

    fn pin(board: &Board, node: &str, name: &str) -> String {
        board.nodes[node]
            .pins
            .values()
            .find(|pin| pin.name == name)
            .map(|pin| pin.id.clone())
            .expect("pin exists")
    }

    fn wire(board: &mut Board, from: &str, to: &str) {
        let from_pin = pin(board, from, "out");
        let to_pin = pin(board, to, "in");
        connect_pins(board, from, &from_pin, to, &to_pin).expect("pins connect");
    }

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
        let mut out: BTreeMap<String, String> = node_pins
            .chain(layer_pins)
            .map(|(key, pin)| (key, format!("{:?} {:?}", pin.depends_on, pin.connected_to)))
            .collect();
        for node in board.nodes.values() {
            out.insert(format!("{}@", node.id), format!("{:?}", node.layer));
        }
        out
    }

    /// `src` at the root feeds `n` in group `g`, which feeds `dst` at the root: both wires run
    /// through bridges of `g` once cleanup ran.
    fn bridged_board() -> Board {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        collapsed(&mut board, "g", None);
        collapsed(&mut board, "h", None);
        wired_node(&mut board, "src", None);
        wired_node(&mut board, "n", Some("g"));
        wired_node(&mut board, "dst", None);
        wire(&mut board, "src", "n");
        wire(&mut board, "n", "dst");
        board.cleanup();
        assert_eq!(
            board.layers["g"].pins.len(),
            2,
            "cleanup bridged both wires"
        );
        board
    }

    #[flow_like_types::tokio::test]
    async fn leaving_a_group_unthreads_its_bridges() {
        let mut board = bridged_board();
        let before = fingerprint(&board);

        let mut command = MoveToLayerCommand::new(vec!["n".into()], None);
        command.execute(&mut board, state()).await.expect("move");

        assert!(
            board.layers["g"].pins.is_empty(),
            "the group's relays are gone"
        );
        let src_out = pin(&board, "src", "out");
        let n_in = pin(&board, "n", "in");
        let n_out = pin(&board, "n", "out");
        let dst_in = pin(&board, "dst", "in");
        assert!(
            board.nodes["src"].pins[&src_out]
                .connected_to
                .contains(&n_in)
        );
        assert!(board.nodes["n"].pins[&n_in].depends_on.contains(&src_out));
        assert!(board.nodes["n"].pins[&n_out].connected_to.contains(&dst_in));
        assert!(board.nodes["dst"].pins[&dst_in].depends_on.contains(&n_out));

        board.cleanup();
        assert!(
            board.layers["g"].pins.is_empty(),
            "nothing crosses the group anymore"
        );

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(fingerprint(&board), before);
    }

    #[flow_like_types::tokio::test]
    async fn moving_between_groups_rebridges_through_the_new_group() {
        let mut board = bridged_board();
        let before = fingerprint(&board);

        let mut command = MoveToLayerCommand::new(vec!["n".into()], Some("h".into()));
        command.execute(&mut board, state()).await.expect("move");
        board.cleanup();

        assert!(board.layers["g"].pins.is_empty());
        assert_eq!(board.layers["h"].pins.len(), 2);

        command.undo(&mut board, state()).await.expect("undo");
        board.cleanup();
        assert_eq!(fingerprint(&board), before);
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
