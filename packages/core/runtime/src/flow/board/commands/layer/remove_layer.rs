use flow_like_types::async_trait;

use schemars::JsonSchema;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use crate::flow::board::{Layer, PinOwner};
use crate::{
    flow::{
        board::{Board, commands::Command},
        node::Node,
        pin::Pin,
        variable::VariableType,
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

    /// Reverses [`splice_boundary`] from the captured layer alone. A relay whose direct wire exists
    /// was spliced: on a cleaned board the two ends of one boundary pin are never also wired
    /// directly, because a data input has one source and an exec output one target.
    fn unsplice(&self, board: &mut Board) {
        let pins = PinIndex::of(board);
        let spliced: Vec<Relay> = relays(&self.layer)
            .into_iter()
            .filter(|relay| pins.wired(board, &relay.producer, &relay.consumer))
            .collect();

        for relay in &spliced {
            if let Some(producer) = pins.get_mut(board, &relay.producer) {
                producer.connected_to.remove(&relay.consumer);
            }
            if let Some(consumer) = pins.get_mut(board, &relay.consumer) {
                consumer.depends_on.remove(&relay.producer);
            }
        }
        for relay in &spliced {
            if let Some(producer) = pins.get_mut(board, &relay.producer) {
                producer.connected_to.insert(relay.entry.clone());
            }
            if let Some(consumer) = pins.get_mut(board, &relay.consumer) {
                consumer.depends_on.insert(relay.exit.clone());
            }
        }
        self.restore_counterpart_halves(board, &pins);
    }

    /// Re-adds the counterpart half of every edge the captured pins recorded, including the ones
    /// no relay covers (an inner wire of a pin without producer, a rival the single-source claim
    /// skipped) that `FixPinsCleanup` pruned after the splice.
    fn restore_counterpart_halves(&self, board: &mut Board, pins: &PinIndex) {
        let foreign = |id: &&String| !self.layer.pins.contains_key(id.as_str());
        for boundary in self.layer.pins.values() {
            for consumer in boundary.connected_to.iter().filter(foreign) {
                if let Some(consumer) = pins.get_mut(board, consumer) {
                    consumer.depends_on.insert(boundary.id.clone());
                }
            }
            for producer in boundary.depends_on.iter().filter(foreign) {
                if let Some(producer) = pins.get_mut(board, producer) {
                    producer.connected_to.insert(boundary.id.clone());
                }
            }
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
            // The caller's copy can predate earlier commands of the same batch, and undo must
            // restore the boundary pins and edges the layer really had.
            if let Some(live) = board.layers.remove(&self.layer.id) {
                self.layer = live;
                splice_boundary(board, &self.layer);
            } else {
                // A repeated Extend: this command spliced nothing, so its undo must not reroute
                // the direct wires through the stale copy's pins.
                self.layer.pins.clear();
            }

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
        }

        Ok(())
    }

    async fn undo(
        &mut self,
        board: &mut Board,
        _: Arc<FlowLikeState>,
    ) -> flow_like_types::Result<()> {
        if self.preserve_nodes {
            self.unsplice(board);
        }
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
        // 4) Reparent any nodes and comments that were preserved
        for id in &self.layer_nodes {
            if let Some(node) = board.nodes.get_mut(id) {
                node.layer = Some(self.layer.id.clone());
            }
            if let Some(comment) = board.comments.get_mut(id) {
                comment.layer = Some(self.layer.id.clone());
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

/// Wires every producer of a boundary pin of `layer`, already taken out of `board`, straight to
/// its consumers, so the wires that crossed the layer outlive its pins.
///
/// Only a spliced relay's own boundary references are dropped. Any other reference to a removed
/// pin is left for `FixPinsCleanup` to prune; [`RemoveLayerCommand::unsplice`] restores it from
/// the captured layer, because the editor's history keeps no derived cleanup receipts.
pub(super) fn splice_boundary(board: &mut Board, layer: &Layer) {
    let pins = PinIndex::of(board);
    let relays = relays(layer);
    let mut sources: HashMap<&str, &str> = HashMap::new();
    let mut spliced: Vec<&Relay> = Vec::new();
    for relay in &relays {
        let (Some(producer), Some(consumer)) = (
            pins.get(board, &relay.producer),
            pins.get(board, &relay.consumer),
        ) else {
            continue;
        };
        // An earlier command of the same batch may have cut one half; splicing would revive it.
        if !producer.connected_to.contains(&relay.entry)
            || !consumer.depends_on.contains(&relay.exit)
        {
            continue;
        }
        // A data input reads a single source, so the first relay in order claims it. An exec
        // producer keeps every consumer: the executor relayed the boundary pin to all of them.
        if consumer.data_type != VariableType::Execution {
            let rival = consumer
                .depends_on
                .iter()
                .any(|source| *source != relay.producer && !layer.pins.contains_key(source));
            let claimed = *sources
                .entry(relay.consumer.as_str())
                .or_insert(relay.producer.as_str());
            if rival || claimed != relay.producer {
                continue;
            }
        }
        spliced.push(relay);
    }

    for relay in spliced {
        if let Some(producer) = pins.get_mut(board, &relay.producer) {
            producer.connected_to.remove(&relay.entry);
            producer.connected_to.insert(relay.consumer.clone());
        }
        if let Some(consumer) = pins.get_mut(board, &relay.consumer) {
            consumer.depends_on.remove(&relay.exit);
            consumer.depends_on.insert(relay.producer.clone());
        }
    }
}

/// One wire through the removed layer, `producer → entry … exit → consumer`. `entry` and `exit`
/// are the layer's own pins, the same pin unless the wire passes straight through the layer.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
struct Relay {
    producer: String,
    consumer: String,
    entry: String,
    exit: String,
}

#[derive(Clone, Copy)]
enum Side {
    Producers,
    Consumers,
}

impl Side {
    fn edges(self, pin: &Pin) -> &BTreeSet<String> {
        match self {
            Side::Producers => &pin.depends_on,
            Side::Consumers => &pin.connected_to,
        }
    }

    fn back_edges(self, pin: &Pin) -> &BTreeSet<String> {
        match self {
            Side::Producers => &pin.connected_to,
            Side::Consumers => &pin.depends_on,
        }
    }
}

/// Every producer × consumer pair a boundary pin of `layer` relays between. Ordered, so the splice
/// is deterministic whatever order the layer's pin map yields.
fn relays(layer: &Layer) -> BTreeSet<Relay> {
    let mut relays = BTreeSet::new();
    for boundary in layer.pins.keys() {
        let producers = outside_ends(layer, boundary, Side::Producers);
        let consumers = outside_ends(layer, boundary, Side::Consumers);
        for (producer, entry) in &producers {
            for (consumer, exit) in &consumers {
                if producer != consumer {
                    relays.insert(Relay {
                        producer: producer.to_string(),
                        consumer: consumer.to_string(),
                        entry: entry.to_string(),
                        exit: exit.to_string(),
                    });
                }
            }
        }
    }
    relays
}

/// The pins outside `layer` on `side` of `start`, each with the layer pin it is wired to. A hop
/// between two of the layer's own pins counts only when both halves agree, so a half-edge the
/// cleanup would prune never becomes a real wire.
fn outside_ends<'l>(layer: &'l Layer, start: &'l str, side: Side) -> BTreeSet<(&'l str, &'l str)> {
    let mut ends = BTreeSet::new();
    let mut visited = HashSet::from([start]);
    let mut stack = vec![start];
    while let Some(id) = stack.pop() {
        let Some(pin) = layer.pins.get(id) else {
            continue;
        };
        for next in side.edges(pin) {
            match layer.pins.get(next) {
                Some(hop) => {
                    if side.back_edges(hop).contains(id) && visited.insert(next.as_str()) {
                        stack.push(next.as_str());
                    }
                }
                None => {
                    ends.insert((next.as_str(), id));
                }
            }
        }
    }
    ends
}

/// Pin owners for one splice. The splice only rewires, so the index stays exact throughout.
struct PinIndex(HashMap<String, PinOwner>);

impl PinIndex {
    fn of(board: &Board) -> Self {
        PinIndex(board.build_pin_index())
    }

    fn get<'b>(&self, board: &'b Board, id: &str) -> Option<&'b Pin> {
        match self.0.get(id)? {
            PinOwner::Node(node) => board.nodes.get(node)?.pins.get(id),
            PinOwner::LayerPin(layer) => board.layers.get(layer)?.pins.get(id),
            PinOwner::LayerNode { layer, node } => {
                board.layers.get(layer)?.nodes.get(node)?.pins.get(id)
            }
        }
    }

    fn get_mut<'b>(&self, board: &'b mut Board, id: &str) -> Option<&'b mut Pin> {
        match self.0.get(id)? {
            PinOwner::Node(node) => board.nodes.get_mut(node)?.pins.get_mut(id),
            PinOwner::LayerPin(layer) => board.layers.get_mut(layer)?.pins.get_mut(id),
            PinOwner::LayerNode { layer, node } => board
                .layers
                .get_mut(layer)?
                .nodes
                .get_mut(node)?
                .pins
                .get_mut(id),
        }
    }

    fn wired(&self, board: &Board, producer: &str, consumer: &str) -> bool {
        self.get(board, producer)
            .is_some_and(|pin| pin.connected_to.contains(consumer))
            && self
                .get(board, consumer)
                .is_some_and(|pin| pin.depends_on.contains(producer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::commands::GenericCommand;
    use crate::flow::board::commands::layer::upsert_layer::UpsertLayerCommand;
    use crate::flow::board::commands::pins::connect_pins::connect_pins;
    use crate::flow::board::{Comment, CommentType, LayerType};
    use crate::flow::pin::{PinType, ValueType};
    use crate::state::FlowLikeConfig;
    use crate::utils::http::HTTPClient;
    use flow_like_storage::Path;
    use std::collections::BTreeMap;
    use std::time::SystemTime;

    type Wiring = BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)>;

    fn state() -> Arc<FlowLikeState> {
        Arc::new(FlowLikeState::new(
            FlowLikeConfig::new(),
            HTTPClient::new_without_refetch(),
        ))
    }

    fn new_board() -> Board {
        Board::new_detached(Some("b".into()), Path::default())
    }

    fn add_layer(board: &mut Board, id: &str, parent: Option<&str>) {
        let mut layer = Layer::new(id.into(), id.into(), LayerType::Collapsed);
        layer.parent_id = parent.map(str::to_string);
        board.layers.insert(id.into(), layer);
    }

    fn add_node(board: &mut Board, id: &str, layer: Option<&str>) {
        let mut node = Node::new(id, id, "", "test");
        node.id = id.into();
        node.layer = layer.map(str::to_string);
        board.nodes.insert(id.into(), node);
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

    /// Adds a pin with a readable id to the node or layer called `owner`.
    fn add_pin(
        board: &mut Board,
        owner: &str,
        id: &str,
        pin_type: PinType,
        data_type: VariableType,
    ) {
        let pin = Pin {
            id: id.into(),
            name: id.into(),
            friendly_name: id.into(),
            description: String::new(),
            pin_type,
            data_type,
            schema: None,
            value_type: ValueType::Normal,
            depends_on: BTreeSet::new(),
            connected_to: BTreeSet::new(),
            default_value: None,
            index: 1,
            options: None,
            value: None,
        };
        match board.nodes.get_mut(owner) {
            Some(node) => node.pins.insert(id.into(), pin),
            None => board
                .layers
                .get_mut(owner)
                .unwrap_or_else(|| panic!("owner {owner} is missing"))
                .pins
                .insert(id.into(), pin),
        };
    }

    fn data_in(board: &mut Board, owner: &str, id: &str) {
        add_pin(board, owner, id, PinType::Input, VariableType::String);
    }

    fn data_out(board: &mut Board, owner: &str, id: &str) {
        add_pin(board, owner, id, PinType::Output, VariableType::String);
    }

    fn exec_in(board: &mut Board, owner: &str, id: &str) {
        add_pin(board, owner, id, PinType::Input, VariableType::Execution);
    }

    fn exec_out(board: &mut Board, owner: &str, id: &str) {
        add_pin(board, owner, id, PinType::Output, VariableType::Execution);
    }

    fn pin_mut<'b>(board: &'b mut Board, id: &str) -> &'b mut Pin {
        PinIndex::of(board)
            .get_mut(board, id)
            .unwrap_or_else(|| panic!("pin {id} is missing"))
    }

    fn link(board: &mut Board, from: &str, to: &str) {
        pin_mut(board, from).connected_to.insert(to.into());
        pin_mut(board, to).depends_on.insert(from.into());
    }

    fn pin<'b>(board: &'b Board, id: &str) -> &'b Pin {
        board
            .get_pin_by_id(id)
            .unwrap_or_else(|| panic!("pin {id} is missing"))
    }

    fn sources(board: &Board, id: &str) -> BTreeSet<String> {
        pin(board, id).depends_on.clone()
    }

    fn targets(board: &Board, id: &str) -> BTreeSet<String> {
        pin(board, id).connected_to.clone()
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    fn uncollapse(board: &Board, layer: &str) -> RemoveLayerCommand {
        RemoveLayerCommand::new(board.layers[layer].clone(), Vec::new(), true)
    }

    fn snapshot(board: &Board) -> serde_json::Value {
        serde_json::json!({
            "nodes": board.nodes,
            "layers": board.layers,
            "comments": board.comments,
        })
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

    /// `src` feeds `mid1` and `mid2`, which feed `sink`; exec runs `src → mid1 → sink`. The
    /// returned wiring is the direct one, before `mid1` and `mid2` collapse into `group`.
    async fn collapsed_group() -> (Board, Wiring) {
        let mut board = new_board();
        for id in ["src", "mid1", "mid2", "sink"] {
            add_node(&mut board, id, None);
        }
        let mut real_pin = |node: &str, name: &str, pin_type: PinType, data_type| {
            let node = board.nodes.get_mut(node).expect("node exists");
            match pin_type {
                PinType::Input => node.add_input_pin(name, name, "", data_type).id.clone(),
                PinType::Output => node.add_output_pin(name, name, "", data_type).id.clone(),
            }
        };
        let src_exec = real_pin("src", "exec_out", PinType::Output, VariableType::Execution);
        let src_value = real_pin("src", "value", PinType::Output, VariableType::String);
        let mid1_exec_in = real_pin("mid1", "exec_in", PinType::Input, VariableType::Execution);
        let mid1_exec_out = real_pin("mid1", "exec_out", PinType::Output, VariableType::Execution);
        let mid1_in = real_pin("mid1", "in", PinType::Input, VariableType::String);
        let mid1_out = real_pin("mid1", "out", PinType::Output, VariableType::String);
        let mid2_in = real_pin("mid2", "in", PinType::Input, VariableType::String);
        let mid2_out = real_pin("mid2", "out", PinType::Output, VariableType::String);
        let sink_exec = real_pin("sink", "exec_in", PinType::Input, VariableType::Execution);
        let sink_a = real_pin("sink", "a", PinType::Input, VariableType::String);
        let sink_b = real_pin("sink", "b", PinType::Input, VariableType::String);

        for (from_node, from, to_node, to) in [
            ("src", &src_exec, "mid1", &mid1_exec_in),
            ("mid1", &mid1_exec_out, "sink", &sink_exec),
            ("src", &src_value, "mid1", &mid1_in),
            ("src", &src_value, "mid2", &mid2_in),
            ("mid1", &mid1_out, "sink", &sink_a),
            ("mid2", &mid2_out, "sink", &sink_b),
        ] {
            connect_pins(&mut board, from_node, from, to_node, to).expect("pins connect");
        }
        board.cleanup();
        let direct = node_wiring(&board);

        let mut collapse = UpsertLayerCommand::new(Layer::new(
            "group".into(),
            "Group".into(),
            LayerType::Collapsed,
        ));
        collapse.node_ids = vec!["mid1".into(), "mid2".into()];
        collapse
            .execute(&mut board, state())
            .await
            .expect("collapse");
        board.cleanup();

        assert_eq!(
            board.layers["group"].pins.len(),
            5,
            "one bridge per used pin: value in, exec in, exec out and two value outs"
        );
        let bridge = targets(&board, &src_value);
        assert_eq!(bridge.len(), 1, "src.value must feed one shared bridge");
        assert!(
            board.layers["group"]
                .pins
                .contains_key(bridge.first().unwrap())
        );

        (board, direct)
    }

    #[flow_like_types::tokio::test]
    async fn a_data_fan_out_bridge_splices_its_producer_to_every_consumer() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "outer", None);
        add_node(&mut board, "a", Some("L"));
        add_node(&mut board, "b", Some("L"));
        data_out(&mut board, "outer", "p");
        data_in(&mut board, "a", "c1");
        data_in(&mut board, "b", "c2");
        data_in(&mut board, "L", "B");
        link(&mut board, "p", "B");
        link(&mut board, "B", "c1");
        link(&mut board, "B", "c2");

        uncollapse(&board, "L")
            .execute(&mut board, state())
            .await
            .expect("uncollapse");

        assert!(!board.layers.contains_key("L"));
        assert_eq!(targets(&board, "p"), set(&["c1", "c2"]));
        assert_eq!(sources(&board, "c1"), set(&["p"]));
        assert_eq!(sources(&board, "c2"), set(&["p"]));
        assert_eq!(board.nodes["a"].layer, None);
        assert_eq!(board.nodes["b"].layer, None);
    }

    #[flow_like_types::tokio::test]
    async fn an_exec_entry_with_two_outer_producers_wires_both_to_the_inner_entry() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "first", None);
        add_node(&mut board, "second", None);
        add_node(&mut board, "inner", Some("L"));
        exec_out(&mut board, "first", "e1");
        exec_out(&mut board, "second", "e2");
        exec_in(&mut board, "inner", "exec_in");
        exec_in(&mut board, "L", "B");
        link(&mut board, "e1", "B");
        link(&mut board, "e2", "B");
        link(&mut board, "B", "exec_in");

        uncollapse(&board, "L")
            .execute(&mut board, state())
            .await
            .expect("uncollapse");

        assert_eq!(targets(&board, "e1"), set(&["exec_in"]));
        assert_eq!(targets(&board, "e2"), set(&["exec_in"]));
        assert_eq!(sources(&board, "exec_in"), set(&["e1", "e2"]));
    }

    #[flow_like_types::tokio::test]
    async fn an_output_bridge_splices_the_inner_producer_to_every_outer_consumer() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "inner", Some("L"));
        add_node(&mut board, "x", None);
        add_node(&mut board, "y", None);
        data_out(&mut board, "inner", "p");
        data_in(&mut board, "x", "c1");
        data_in(&mut board, "y", "c2");
        data_out(&mut board, "L", "B");
        link(&mut board, "p", "B");
        link(&mut board, "B", "c1");
        link(&mut board, "B", "c2");

        uncollapse(&board, "L")
            .execute(&mut board, state())
            .await
            .expect("uncollapse");

        assert_eq!(targets(&board, "p"), set(&["c1", "c2"]));
        assert_eq!(sources(&board, "c1"), set(&["p"]));
        assert_eq!(sources(&board, "c2"), set(&["p"]));
    }

    #[flow_like_types::tokio::test]
    async fn a_pass_through_splices_the_outer_producer_to_the_outer_consumer() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "outer", None);
        add_node(&mut board, "inner", Some("L"));
        add_node(&mut board, "sink", None);
        data_out(&mut board, "outer", "p");
        data_in(&mut board, "inner", "c_in");
        data_in(&mut board, "sink", "c_out");
        data_in(&mut board, "L", "B_in");
        data_out(&mut board, "L", "B_out");
        link(&mut board, "p", "B_in");
        link(&mut board, "B_in", "c_in");
        link(&mut board, "B_in", "B_out");
        link(&mut board, "B_out", "c_out");

        uncollapse(&board, "L")
            .execute(&mut board, state())
            .await
            .expect("uncollapse");

        assert_eq!(targets(&board, "p"), set(&["c_in", "c_out"]));
        assert_eq!(sources(&board, "c_in"), set(&["p"]));
        assert_eq!(sources(&board, "c_out"), set(&["p"]));
    }

    #[flow_like_types::tokio::test]
    async fn removing_a_nested_layer_splices_the_parent_bridge_to_its_consumers() {
        let mut board = new_board();
        add_layer(&mut board, "X", None);
        add_layer(&mut board, "Y", Some("X"));
        add_node(&mut board, "outer", None);
        add_node(&mut board, "a", Some("Y"));
        add_node(&mut board, "b", Some("Y"));
        data_out(&mut board, "outer", "p");
        data_in(&mut board, "a", "c1");
        data_in(&mut board, "b", "c2");
        data_in(&mut board, "X", "BX");
        data_in(&mut board, "Y", "BY");
        link(&mut board, "p", "BX");
        link(&mut board, "BX", "BY");
        link(&mut board, "BY", "c1");
        link(&mut board, "BY", "c2");

        uncollapse(&board, "Y")
            .execute(&mut board, state())
            .await
            .expect("uncollapse");

        assert!(!board.layers.contains_key("Y"));
        assert_eq!(targets(&board, "BX"), set(&["c1", "c2"]));
        assert_eq!(sources(&board, "BX"), set(&["p"]));
        assert_eq!(sources(&board, "c1"), set(&["BX"]));
        assert_eq!(sources(&board, "c2"), set(&["BX"]));
        assert_eq!(board.nodes["a"].layer.as_deref(), Some("X"));
    }

    #[flow_like_types::tokio::test]
    async fn a_consumer_rewired_earlier_in_the_batch_keeps_its_new_source() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "outer", None);
        add_node(&mut board, "other", None);
        add_node(&mut board, "a", Some("L"));
        add_node(&mut board, "b", Some("L"));
        data_out(&mut board, "outer", "p");
        data_out(&mut board, "other", "q");
        data_in(&mut board, "a", "c1");
        data_in(&mut board, "b", "c2");
        data_in(&mut board, "L", "B");
        link(&mut board, "p", "B");
        link(&mut board, "B", "c1");
        link(&mut board, "B", "c2");
        let stale = uncollapse(&board, "L");
        connect_pins(&mut board, "other", "q", "a", "c1").expect("rewire");

        let mut command = stale;
        command
            .execute(&mut board, state())
            .await
            .expect("uncollapse");
        board.cleanup();

        assert_eq!(sources(&board, "c1"), set(&["q"]));
        assert_eq!(sources(&board, "c2"), set(&["p"]));
        assert_eq!(targets(&board, "p"), set(&["c2"]));
        assert_eq!(targets(&board, "q"), set(&["c1"]));
    }

    #[flow_like_types::tokio::test]
    async fn a_data_consumer_keeps_a_single_source() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "outer", None);
        add_node(&mut board, "inner", Some("L"));
        data_out(&mut board, "outer", "p1");
        data_out(&mut board, "outer", "p2");
        data_in(&mut board, "inner", "c");
        data_in(&mut board, "L", "B");
        link(&mut board, "p1", "B");
        link(&mut board, "p2", "B");
        link(&mut board, "B", "c");
        let original = snapshot(&board);

        let mut command = uncollapse(&board, "L");
        command
            .execute(&mut board, state())
            .await
            .expect("uncollapse");

        assert_eq!(sources(&board, "c"), set(&["p1"]));
        assert_eq!(targets(&board, "p1"), set(&["c"]));

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(snapshot(&board), original);
    }

    #[flow_like_types::tokio::test]
    async fn undo_restores_every_pin_and_the_live_layer() {
        let mut board = new_board();
        add_layer(&mut board, "P", None);
        add_layer(&mut board, "L", Some("P"));
        add_layer(&mut board, "child", Some("L"));
        add_node(&mut board, "outer", Some("P"));
        add_node(&mut board, "e", Some("P"));
        add_node(&mut board, "inner", Some("L"));
        add_node(&mut board, "sink", Some("P"));
        data_out(&mut board, "outer", "p");
        exec_out(&mut board, "e", "e1");
        exec_out(&mut board, "e", "e2");
        data_in(&mut board, "inner", "c");
        exec_in(&mut board, "inner", "exec_in");
        data_out(&mut board, "inner", "out");
        data_in(&mut board, "sink", "s1");
        data_in(&mut board, "sink", "s2");
        data_in(&mut board, "sink", "s3");
        data_in(&mut board, "child", "child_in");
        data_in(&mut board, "L", "B_in");
        exec_in(&mut board, "L", "B_exec");
        data_out(&mut board, "L", "B_out");
        data_out(&mut board, "L", "B_through");
        link(&mut board, "p", "B_in");
        link(&mut board, "B_in", "c");
        link(&mut board, "B_in", "child_in");
        link(&mut board, "B_in", "B_through");
        link(&mut board, "B_through", "s3");
        link(&mut board, "e1", "B_exec");
        link(&mut board, "e2", "B_exec");
        link(&mut board, "B_exec", "exec_in");
        link(&mut board, "out", "B_out");
        link(&mut board, "B_out", "s1");
        link(&mut board, "B_out", "s2");
        add_comment(&mut board, "note", Some("L"));
        let original = snapshot(&board);
        let live = board.layers["L"].clone();

        let mut command = RemoveLayerCommand::new(
            Layer::new("L".into(), "stale copy".into(), LayerType::Collapsed),
            Vec::new(),
            true,
        );
        command
            .execute(&mut board, state())
            .await
            .expect("uncollapse");

        assert_eq!(
            command.layer.name, live.name,
            "undo must hold the live layer"
        );
        assert_eq!(targets(&board, "p"), set(&["c", "child_in", "s3"]));
        assert_eq!(sources(&board, "exec_in"), set(&["e1", "e2"]));
        assert_eq!(targets(&board, "out"), set(&["s1", "s2"]));
        assert_eq!(board.layers["child"].parent_id.as_deref(), Some("P"));
        assert_eq!(board.nodes["inner"].layer.as_deref(), Some("P"));
        assert_eq!(board.comments["note"].layer.as_deref(), Some("P"));
        let spliced = snapshot(&board);

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(snapshot(&board), original);

        command.execute(&mut board, state()).await.expect("redo");
        assert_eq!(snapshot(&board), spliced);
    }

    #[flow_like_types::tokio::test]
    async fn undo_without_cleanup_receipts_restores_every_recorded_edge() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "outer", None);
        add_node(&mut board, "a", Some("L"));
        add_node(&mut board, "inner", Some("L"));
        data_out(&mut board, "outer", "p1");
        data_out(&mut board, "outer", "p2");
        data_in(&mut board, "a", "a_in");
        data_in(&mut board, "inner", "c");
        // `B` lost its outer producer, so it only feeds the inside.
        data_in(&mut board, "L", "B");
        data_in(&mut board, "L", "S");
        link(&mut board, "B", "a_in");
        link(&mut board, "p1", "S");
        link(&mut board, "p2", "S");
        link(&mut board, "S", "c");
        board.cleanup();
        let original = snapshot(&board);

        let mut command = uncollapse(&board, "L");
        command
            .execute(&mut board, state())
            .await
            .expect("uncollapse");
        board.cleanup();
        assert_eq!(sources(&board, "c"), set(&["p1"]));
        assert!(sources(&board, "a_in").is_empty());
        assert!(targets(&board, "p2").is_empty());

        command.undo(&mut board, state()).await.expect("undo");
        board.cleanup();

        assert_eq!(sources(&board, "a_in"), set(&["B"]));
        assert_eq!(targets(&board, "B"), set(&["a_in"]));
        assert_eq!(targets(&board, "p2"), set(&["S"]));
        assert_eq!(sources(&board, "S"), set(&["p1", "p2"]));
        assert_eq!(sources(&board, "c"), set(&["S"]));
        assert_eq!(snapshot(&board), original);
    }

    #[flow_like_types::tokio::test]
    async fn a_repeated_extend_keeps_the_direct_wires_on_undo() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_node(&mut board, "outer", None);
        add_node(&mut board, "inner", Some("L"));
        data_out(&mut board, "outer", "p");
        data_in(&mut board, "inner", "c");
        data_in(&mut board, "L", "B");
        link(&mut board, "p", "B");
        link(&mut board, "B", "c");
        let original = snapshot(&board);

        let mut first = uncollapse(&board, "L");
        let mut second = uncollapse(&board, "L");
        first
            .execute(&mut board, state())
            .await
            .expect("first extend");
        second
            .execute(&mut board, state())
            .await
            .expect("second extend");
        assert!(
            second.layer.pins.is_empty(),
            "a command that removed no layer must hold no relays"
        );

        second.undo(&mut board, state()).await.expect("undo second");
        assert_eq!(targets(&board, "p"), set(&["c"]));
        assert_eq!(sources(&board, "c"), set(&["p"]));

        first.undo(&mut board, state()).await.expect("undo first");
        assert_eq!(snapshot(&board), original);
    }

    #[flow_like_types::tokio::test]
    async fn uncollapse_through_the_command_pipeline_keeps_the_wires_and_undoes() {
        let state = state();
        let (mut board, direct) = collapsed_group().await;
        let collapsed = fingerprint(&board);

        let commands = board
            .execute_commands(
                vec![GenericCommand::RemoveLayer(uncollapse(&board, "group"))],
                state.clone(),
            )
            .await
            .expect("uncollapse");

        assert!(!board.layers.contains_key("group"));
        assert_eq!(node_wiring(&board), direct);

        board.undo(commands, state).await.expect("undo");
        assert_eq!(fingerprint(&board), collapsed);
    }

    #[flow_like_types::tokio::test]
    async fn collapse_then_uncollapse_restores_the_direct_wires() {
        let (mut board, direct) = collapsed_group().await;

        let mut command = RemoveLayerCommand::new(
            Layer::new("group".into(), "Group".into(), LayerType::Collapsed),
            Vec::new(),
            true,
        );
        command
            .execute(&mut board, state())
            .await
            .expect("uncollapse");
        board.cleanup();

        assert!(!board.layers.contains_key("group"));
        assert!(board.nodes.values().all(|node| node.layer.is_none()));
        assert_eq!(node_wiring(&board), direct);
    }

    #[flow_like_types::tokio::test]
    async fn deleting_with_contents_removes_the_subtree_without_splicing() {
        let mut board = new_board();
        add_layer(&mut board, "L", None);
        add_layer(&mut board, "child", Some("L"));
        add_node(&mut board, "outer", None);
        add_node(&mut board, "inner", Some("L"));
        add_node(&mut board, "nested", Some("child"));
        data_out(&mut board, "outer", "p");
        data_in(&mut board, "inner", "c");
        data_in(&mut board, "L", "B");
        link(&mut board, "p", "B");
        link(&mut board, "B", "c");
        let original = snapshot(&board);

        let mut command = RemoveLayerCommand::new(board.layers["L"].clone(), Vec::new(), false);
        command.execute(&mut board, state()).await.expect("delete");

        assert!(board.layers.is_empty());
        assert_eq!(board.nodes.keys().collect::<Vec<_>>(), vec!["outer"]);
        assert_eq!(
            targets(&board, "p"),
            set(&["B"]),
            "the dangling edge is left for the cleanup"
        );

        command.undo(&mut board, state()).await.expect("undo");
        assert_eq!(snapshot(&board), original);
    }
}
