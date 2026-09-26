use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use flow_like_types::create_id;

use crate::flow::{
    board::{
        Board, Layer, LayerType,
        cleanup::{BoardCleanupLogic, NodeOrLayerRef, PinLookup},
    },
    pin::{Pin, PinType},
    variable::VariableType,
};

/// `(layer, direction, key pin)`, the identity of a bridge. See [`BridgeLayersCleanup`].
type BridgeKey = (String, Direction, String);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Direction {
    Input,
    Output,
}

impl Direction {
    fn of(pin_type: &PinType) -> Self {
        match pin_type {
            PinType::Input => Direction::Input,
            PinType::Output => Direction::Output,
        }
    }

    fn pin_type(self) -> PinType {
        match self {
            Direction::Input => PinType::Input,
            Direction::Output => PinType::Output,
        }
    }

    fn tag(self) -> &'static [u8] {
        match self {
            Direction::Input => b"input",
            Direction::Output => b"output",
        }
    }
}

const BRIDGE_ID_LENGTH: usize = 24;
const BASE36: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// A cuid-shaped id derived from the bridge key. Cleanup runs on every load without saving, so a
/// random id would differ between every replica that loads the same saved board, and a later
/// command referencing one replica's id would fail or prune wires on another. `attempt` rehashes
/// when the id is taken.
fn derived_bridge_id((layer, direction, key): &BridgeKey, attempt: u32) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.layer-bridge/v1\0");
    for part in [layer.as_bytes(), direction.tag(), key.as_bytes()] {
        hasher.update(part);
        hasher.update(b"\0");
    }
    hasher.update(&attempt.to_le_bytes());
    let digest = hasher.finalize();
    let bytes = digest.as_bytes();

    let mut id = String::with_capacity(BRIDGE_ID_LENGTH);
    id.push(char::from(BASE36[10 + usize::from(bytes[0] % 26)]));
    let mut rest = u128::from_le_bytes(std::array::from_fn(|position| bytes[position + 1]));
    for _ in 1..BRIDGE_ID_LENGTH {
        id.push(char::from(BASE36[(rest % 36) as usize]));
        rest /= 36;
    }
    id
}

enum PinOwner {
    Node { node: String, layer: Option<String> },
    Layer(String),
}

impl PinOwner {
    /// The innermost layer the pin itself lives in; a boundary pin belongs to its own layer.
    fn scope(&self) -> Option<&str> {
        match self {
            PinOwner::Node { layer, .. } => layer.as_deref(),
            PinOwner::Layer(layer) => Some(layer),
        }
    }
}

struct LayerInfo {
    parent: Option<String>,
    module: bool,
    reusable: bool,
}

/// The live layer hierarchy, read after `FixLayerParents` repaired it during the main pass.
struct Topology(HashMap<String, LayerInfo>);

impl Topology {
    fn of(board: &Board) -> Self {
        Topology(
            board
                .layers
                .values()
                .map(|layer| {
                    let info = LayerInfo {
                        parent: layer.parent_id.clone(),
                        module: matches!(layer.r#type, LayerType::Module),
                        reusable: matches!(layer.r#type, LayerType::Collapsed | LayerType::Macro),
                    };
                    (layer.id.clone(), info)
                })
                .collect(),
        )
    }

    fn parent(&self, layer: &str) -> Option<&str> {
        self.0.get(layer)?.parent.as_deref()
    }

    fn reusable(&self, layer: &str) -> bool {
        self.0.get(layer).is_some_and(|info| info.reusable)
    }

    /// Whether `scope` is `layer` or nested inside it. A damaged `parent_id` chain can be cyclic;
    /// walking it unguarded hangs the cleanup.
    fn within(&self, scope: Option<&str>, layer: &str) -> bool {
        let mut current = scope;
        let mut seen = HashSet::new();
        while let Some(id) = current {
            if id == layer {
                return true;
            }
            if !seen.insert(id) {
                return false;
            }
            current = self.parent(id);
        }
        false
    }

    /// The boundaries around `location`, innermost first. A module is a virtual file, not a
    /// runtime boundary, so wires cross it directly.
    fn chain(&self, location: Option<&str>) -> Vec<String> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut current = location;
        while let Some(id) = current {
            let Some(info) = self.0.get(id) else {
                break;
            };
            if !seen.insert(id) {
                break;
            }
            if !info.module {
                chain.push(id.to_string());
            }
            current = info.parent.as_deref();
        }
        chain
    }
}

/// Puts boundary ("bridge") pins on non-Module layers for every wire that crosses a layer
/// boundary.
///
/// A bridge is keyed by the pin on the single end of the relation and is a clone of that pin, so a
/// layer carries one boundary pin per used pin instead of one per wire:
///
/// | crossing       | key (per layer)               |
/// |----------------|-------------------------------|
/// | data, entering | outer producer (just outside) |
/// | data, leaving  | inner producer                |
/// | exec, entering | inner consumer (entry pin)    |
/// | exec, leaving  | inner producer                |
///
/// Exec entries stay consumer-keyed: an exec output has a single target, and function conversion
/// needs exactly one entry pin per inner entry.
///
/// A pin sits at its node's layer; a boundary pin of `X` sits at `X` when its other end is inside
/// `X`, otherwise at `X`'s parent. Every reciprocal edge `S → D` whose ends sit under different
/// boundaries becomes `S → out… → in… → D`: exits innermost first, entries outermost first for data
/// (each keyed by the previous hop) and innermost first for exec (each keyed by the next hop).
///
/// Existing bridges on Collapsed and Macro layers are reused by key, and legacy duplicates of one
/// key merge into the lowest id. Function-layer pins are the function's contract, so they are
/// never reused or merged.
///
/// A minted bridge takes an id derived from its key, so every replica that cleans the same saved
/// board ends with the same pin ids.
#[derive(Default)]
pub struct BridgeLayersCleanup {
    owners: HashMap<String, PinOwner>,
    /// Legacy boards keep some nodes inline in `layer.nodes` instead of `board.nodes`.
    inline_nodes: HashMap<String, String>,
    /// Reciprocal `(source, target)` edges whose ends are scoped to different layers.
    crossings: BTreeSet<(String, String)>,
}

impl BoardCleanupLogic for BridgeLayersCleanup {
    fn init(board: &mut Board) -> Self
    where
        Self: Sized,
    {
        Self {
            owners: HashMap::with_capacity((board.nodes.len() + board.layers.len()) * 4),
            inline_nodes: HashMap::new(),
            crossings: BTreeSet::new(),
        }
    }

    fn initial_layer_iteration(&mut self, layer: &Layer) {
        for node_id in layer.nodes.keys() {
            self.inline_nodes.insert(node_id.clone(), layer.id.clone());
        }
    }

    fn initial_pin_iteration(&mut self, pin: &Pin, parent: NodeOrLayerRef) {
        let owner = match parent {
            NodeOrLayerRef::Node(node) => PinOwner::Node {
                node: node.id.clone(),
                layer: node.layer.clone(),
            },
            NodeOrLayerRef::Layer(layer) => PinOwner::Layer(layer.id.clone()),
        };
        self.owners.insert(pin.id.clone(), owner);
    }

    fn main_pin_iteration(&mut self, pin: &mut Pin, pin_lookup: &PinLookup) {
        let Some(source) = self.owners.get(&pin.id) else {
            return;
        };
        for target in &pin.connected_to {
            let Some(target_owner) = self.owners.get(target) else {
                continue;
            };
            // Pins scoped to the same layer are never separated by a boundary.
            if source.scope() == target_owner.scope() {
                continue;
            }
            // `FixPinsCleanup` prunes the surviving half of a one-sided edge; bridging it would
            // resurrect the wire.
            let reciprocal = pin_lookup
                .get(target)
                .is_some_and(|(edges, _)| edges.depends_on.contains(&pin.id));
            if reciprocal {
                self.crossings.insert((pin.id.clone(), target.clone()));
            }
        }
    }

    fn post_process(&mut self, board: &mut Board, _pin_lookup: &PinLookup) {
        let crossings = std::mem::take(&mut self.crossings);
        let may_hold_duplicates = board.layers.values().any(|layer| {
            matches!(layer.r#type, LayerType::Collapsed | LayerType::Macro) && layer.pins.len() > 1
        });
        if crossings.is_empty() && !may_hold_duplicates {
            return;
        }

        BridgePass {
            topology: Topology::of(board),
            owners: std::mem::take(&mut self.owners),
            inline_nodes: std::mem::take(&mut self.inline_nodes),
            index: HashMap::new(),
            redirects: HashMap::new(),
            next_index: HashMap::new(),
            touched: BTreeSet::new(),
        }
        .run(board, crossings);
    }
}

struct BridgePass {
    topology: Topology,
    owners: HashMap<String, PinOwner>,
    inline_nodes: HashMap<String, String>,
    /// Existing bridges of Collapsed and Macro layers, plus every bridge planned in this pass.
    index: HashMap<BridgeKey, String>,
    /// Merged duplicate to its survivor.
    redirects: HashMap<String, String>,
    next_index: HashMap<(String, Direction), u16>,
    touched: BTreeSet<String>,
}

impl BridgePass {
    fn run(&mut self, board: &mut Board, crossings: BTreeSet<(String, String)>) {
        self.merge_duplicate_bridges(board);

        let crossings: BTreeSet<(String, String)> = crossings
            .into_iter()
            .map(|(source, target)| (self.redirected(source), self.redirected(target)))
            .collect();

        let mut routed = false;
        for (source, target) in &crossings {
            routed |= self.route(board, source, target);
        }

        // Rerouting a legacy bridge changes its key, which can land on a key another bridge
        // already holds: a parallel path between siblings, or a bridge wired across its parent.
        if routed {
            self.merge_duplicate_bridges(board);
        }

        self.compact_indices(board);
    }

    fn redirected(&self, mut id: String) -> String {
        while let Some(survivor) = self.redirects.get(&id) {
            id = survivor.clone();
        }
        id
    }

    /// A merge can give two other bridges a common key (nested bridges fed by two merged
    /// duplicates), so this repeats until a round merges nothing. Every merging round removes a
    /// pin, so it terminates.
    fn merge_duplicate_bridges(&mut self, board: &mut Board) {
        loop {
            self.index.clear();
            let mut merged = false;
            for (key, ids) in self.existing_bridges(board) {
                if ids.len() > 1 && self.merge(board, &key, &ids) {
                    merged = true;
                } else if let Some(first) = ids.into_iter().next() {
                    self.index.insert(key, first);
                }
            }
            if !merged {
                return;
            }
        }
    }

    /// Bridges grouped by key, each group in ascending id order.
    fn existing_bridges(&self, board: &Board) -> BTreeMap<BridgeKey, Vec<String>> {
        let mut groups: BTreeMap<BridgeKey, Vec<String>> = BTreeMap::new();
        for layer in board.layers.values() {
            if !self.topology.reusable(&layer.id) {
                continue;
            }
            let mut pins: Vec<&Pin> = layer.pins.values().collect();
            pins.sort_by(|a, b| a.id.cmp(&b.id));
            for pin in pins {
                if let Some((direction, key)) = self.existing_key(board, &layer.id, pin) {
                    groups
                        .entry((layer.id.clone(), direction, key.to_string()))
                        .or_default()
                        .push(pin.id.clone());
                }
            }
        }
        groups
    }

    fn existing_key<'p>(
        &self,
        board: &Board,
        layer: &str,
        pin: &'p Pin,
    ) -> Option<(Direction, &'p str)> {
        let (key, key_is_inside) = match pin.pin_type {
            PinType::Output => (sole(&pin.depends_on)?, true),
            PinType::Input if pin.data_type == VariableType::Execution => {
                (sole(&pin.connected_to)?, true)
            }
            PinType::Input => (sole(&pin.depends_on)?, false),
        };
        self.pin(board, key)?;
        let inside = self.topology.within(self.owners.get(key)?.scope(), layer);
        (inside == key_is_inside).then_some((Direction::of(&pin.pin_type), key))
    }

    fn merge(
        &mut self,
        board: &mut Board,
        (layer_id, direction, key): &BridgeKey,
        ids: &[String],
    ) -> bool {
        let Some((survivor, duplicates)) = ids.split_first() else {
            return false;
        };
        let Some(template) = self.pin(board, key).cloned() else {
            return false;
        };
        let Some(layer) = board.layers.get_mut(layer_id) else {
            return false;
        };
        if !layer.pins.contains_key(survivor) {
            return false;
        }

        let removed: Vec<Pin> = duplicates
            .iter()
            .filter_map(|id| layer.pins.remove(id))
            .collect();
        if let Some(target) = layer.pins.get_mut(survivor) {
            for pin in &removed {
                target.connected_to.extend(pin.connected_to.iter().cloned());
                target.depends_on.extend(pin.depends_on.iter().cloned());
            }
            for id in ids {
                target.connected_to.remove(id);
                target.depends_on.remove(id);
            }
            adopt_metadata(target, &template, *direction);
        }

        let counterparts: BTreeSet<&String> = removed
            .iter()
            .flat_map(|pin| pin.connected_to.iter().chain(&pin.depends_on))
            .filter(|id| !ids.contains(*id))
            .collect();
        for counterpart in counterparts {
            let Some(pin) = self.pin_mut(board, counterpart) else {
                continue;
            };
            for duplicate in duplicates {
                if pin.connected_to.remove(duplicate) {
                    pin.connected_to.insert(survivor.clone());
                }
                if pin.depends_on.remove(duplicate) {
                    pin.depends_on.insert(survivor.clone());
                }
            }
        }

        for duplicate in duplicates {
            self.redirects.insert(duplicate.clone(), survivor.clone());
        }
        self.touched.insert(layer_id.clone());
        true
    }

    fn route(&mut self, board: &mut Board, source: &str, target: &str) -> bool {
        let Some(execution) = self.live_edge_is_execution(board, source, target) else {
            return false;
        };
        let (Some(from), Some(to)) = (self.location(source, target), self.location(target, source))
        else {
            return false;
        };

        let mut exits = self.topology.chain(from);
        let mut enters = self.topology.chain(to);
        if let Some(shared) = exits.iter().position(|layer| enters.contains(layer)) {
            let below = enters
                .iter()
                .position(|layer| *layer == exits[shared])
                .unwrap_or(enters.len());
            enters.truncate(below);
            exits.truncate(shared);
        }
        if exits.is_empty() && enters.is_empty() {
            return false;
        }

        let Some(path) = self.bridge_path(board, source, target, &exits, &enters, execution) else {
            return false;
        };
        self.rewire(board, &path);
        true
    }

    fn bridge_path(
        &mut self,
        board: &mut Board,
        source: &str,
        target: &str,
        exits: &[String],
        enters: &[String],
        execution: bool,
    ) -> Option<Vec<String>> {
        let mut path = vec![source.to_string()];
        for layer in exits {
            let bridge = self.bridge(board, layer, Direction::Output, path.last()?)?;
            path.push(bridge);
        }

        if execution {
            let mut inbound: Vec<String> = Vec::with_capacity(enters.len());
            for layer in enters {
                let consumer = inbound.last().map_or(target, String::as_str);
                let bridge = self.bridge(board, layer, Direction::Input, consumer)?;
                inbound.push(bridge);
            }
            path.extend(inbound.into_iter().rev());
        } else {
            for layer in enters.iter().rev() {
                let bridge = self.bridge(board, layer, Direction::Input, path.last()?)?;
                path.push(bridge);
            }
        }

        path.push(target.to_string());
        Some(path)
    }

    fn bridge(
        &mut self,
        board: &mut Board,
        layer: &str,
        direction: Direction,
        key: &str,
    ) -> Option<String> {
        let bridge_key = (layer.to_string(), direction, key.to_string());
        if let Some(existing) = self.index.get(&bridge_key) {
            return Some(existing.clone());
        }

        let mut bridge = self.pin(board, key)?.clone();
        bridge.id = self.free_bridge_id(board, &bridge_key);
        bridge.pin_type = direction.pin_type();
        bridge.connected_to.clear();
        bridge.depends_on.clear();
        bridge.value = None;
        bridge.index = self.next_index(board, layer, direction);

        let id = bridge.id.clone();
        board.layers.get_mut(layer)?.pins.insert(id.clone(), bridge);
        self.owners
            .insert(id.clone(), PinOwner::Layer(layer.to_string()));
        self.index.insert(bridge_key, id.clone());
        self.touched.insert(layer.to_string());
        Some(id)
    }

    /// Function-layer bridges are not indexed across passes, so a key can mint again while its
    /// earlier bridge still holds the derived id.
    fn free_bridge_id(&self, board: &Board, key: &BridgeKey) -> String {
        (0..=u32::MAX)
            .map(|attempt| derived_bridge_id(key, attempt))
            .find(|id| !self.owners.contains_key(id) && board.get_pin_by_id(id).is_none())
            .unwrap_or_else(create_id)
    }

    /// Replaces `path[0] → path[last]` with a wire through every hop in between.
    fn rewire(&self, board: &mut Board, path: &[String]) {
        let (Some(source), Some(target)) = (path.first(), path.last()) else {
            return;
        };
        if let Some(pin) = self.pin_mut(board, source) {
            pin.connected_to.remove(target);
        }
        if let Some(pin) = self.pin_mut(board, target) {
            pin.depends_on.remove(source);
        }
        for hop in path.windows(2) {
            if let Some(pin) = self.pin_mut(board, &hop[0]) {
                pin.connected_to.insert(hop[1].clone());
            }
            if let Some(pin) = self.pin_mut(board, &hop[1]) {
                pin.depends_on.insert(hop[0].clone());
            }
        }
    }

    /// `None` unless `source → target` is still reciprocal on the live board: `FixPinsCleanup`
    /// ran first and may have pruned one half.
    fn live_edge_is_execution(&self, board: &Board, source: &str, target: &str) -> Option<bool> {
        let source_pin = self.pin(board, source)?;
        let target_pin = self.pin(board, target)?;
        (source_pin.connected_to.contains(target) && target_pin.depends_on.contains(source))
            .then_some(source_pin.data_type == VariableType::Execution)
    }

    /// Where `pin` sits as seen from `other`; the outer `None` means an unknown pin.
    fn location(&self, pin: &str, other: &str) -> Option<Option<&str>> {
        Some(match self.owners.get(pin)? {
            PinOwner::Node { layer, .. } => layer.as_deref(),
            PinOwner::Layer(layer) => {
                if self.topology.within(self.owners.get(other)?.scope(), layer) {
                    Some(layer.as_str())
                } else {
                    self.topology.parent(layer)
                }
            }
        })
    }

    fn next_index(&mut self, board: &Board, layer: &str, direction: Direction) -> u16 {
        let next = self
            .next_index
            .entry((layer.to_string(), direction))
            .or_insert_with(|| {
                board
                    .layers
                    .get(layer)
                    .and_then(|layer| {
                        layer
                            .pins
                            .values()
                            .filter(|pin| Direction::of(&pin.pin_type) == direction)
                            .map(|pin| pin.index)
                            .max()
                    })
                    .unwrap_or(0)
            });
        *next = next.saturating_add(1);
        *next
    }

    /// `PinIndicesCleanup` numbered every layer before this runs, so appended bridges and removed
    /// duplicates would leave gaps it only closes on the next cleanup.
    fn compact_indices(&self, board: &mut Board) {
        for layer_id in &self.touched {
            let Some(layer) = board.layers.get_mut(layer_id) else {
                continue;
            };
            for direction in [Direction::Input, Direction::Output] {
                let mut pins: Vec<&mut Pin> = layer
                    .pins
                    .values_mut()
                    .filter(|pin| Direction::of(&pin.pin_type) == direction)
                    .collect();
                pins.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.id.cmp(&b.id)));
                for (position, pin) in pins.into_iter().enumerate() {
                    pin.index = position as u16 + 1;
                }
            }
        }
    }

    fn pin<'b>(&self, board: &'b Board, id: &str) -> Option<&'b Pin> {
        match self.owners.get(id)? {
            PinOwner::Layer(layer) => board.layers.get(layer)?.pins.get(id),
            PinOwner::Node { node, .. } => match board.nodes.get(node) {
                Some(owner) => owner.pins.get(id),
                None => board
                    .layers
                    .get(self.inline_nodes.get(node)?)?
                    .nodes
                    .get(node)?
                    .pins
                    .get(id),
            },
        }
    }

    fn pin_mut<'b>(&self, board: &'b mut Board, id: &str) -> Option<&'b mut Pin> {
        match self.owners.get(id)? {
            PinOwner::Layer(layer) => board.layers.get_mut(layer)?.pins.get_mut(id),
            PinOwner::Node { node, .. } if board.nodes.contains_key(node) => {
                board.nodes.get_mut(node)?.pins.get_mut(id)
            }
            PinOwner::Node { node, .. } => board
                .layers
                .get_mut(self.inline_nodes.get(node)?)?
                .nodes
                .get_mut(node)?
                .pins
                .get_mut(id),
        }
    }
}

fn sole(ids: &BTreeSet<String>) -> Option<&str> {
    let mut ids = ids.iter();
    let first = ids.next()?;
    ids.next().is_none().then_some(first.as_str())
}

/// A merged bridge becomes a clone of its key pin, keeping its own id, edges and position.
fn adopt_metadata(pin: &mut Pin, template: &Pin, direction: Direction) {
    pin.name = template.name.clone();
    pin.friendly_name = template.friendly_name.clone();
    pin.description = template.description.clone();
    pin.data_type = template.data_type.clone();
    pin.schema = template.schema.clone();
    pin.value_type = template.value_type.clone();
    pin.options = template.options.clone();
    pin.default_value = template.default_value.clone();
    pin.pin_type = direction.pin_type();
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet, HashMap},
        time::SystemTime,
    };

    use flow_like_storage::object_store::path::Path;

    use crate::flow::{
        board::{
            Board, ExecutionMode, ExecutionStage, Layer, LayerType, cleanup::BoardCleanupLogic,
            commands::pins::connect_pins::connect_pins,
        },
        execution::LogLevel,
        node::Node,
        pin::{Pin, PinType, ValueType},
        variable::VariableType,
    };

    use super::{BRIDGE_ID_LENGTH, BridgeLayersCleanup, Direction, derived_bridge_id};

    fn test_board() -> Board {
        Board {
            format_version: crate::flow::board::format::LEGACY_BOARD_FORMAT_VERSION,
            id: "board".to_string(),
            name: "Board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
            pin_index: None,
            page_metadata_source: Default::default(),
        }
    }

    fn add_layer(board: &mut Board, id: &str, parent: Option<&str>, kind: LayerType) {
        let mut layer = Layer::new(id.to_string(), id.to_string(), kind);
        layer.parent_id = parent.map(str::to_string);
        board.layers.insert(id.to_string(), layer);
    }

    fn add_node(board: &mut Board, id: &str, layer: Option<&str>) {
        let mut node = Node::new(id, id, "", "test");
        node.id = id.to_string();
        node.layer = layer.map(str::to_string);
        board.nodes.insert(id.to_string(), node);
    }

    fn add_pin(
        board: &mut Board,
        node: &str,
        name: &str,
        pin_type: PinType,
        data_type: VariableType,
    ) -> String {
        let node = board.nodes.get_mut(node).expect("node exists");
        let pin = match pin_type {
            PinType::Input => node.add_input_pin(name, name, "", data_type),
            PinType::Output => node.add_output_pin(name, name, "", data_type),
        };
        pin.id.clone()
    }

    /// A boundary pin as older builds minted it: one per wire, cloned from the inner pin.
    fn add_legacy_bridge(
        board: &mut Board,
        layer: &str,
        id: &str,
        name: &str,
        pin_type: PinType,
        data_type: VariableType,
        index: u16,
    ) {
        let pin = Pin {
            id: id.to_string(),
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            pin_type,
            data_type,
            schema: None,
            value_type: ValueType::Normal,
            depends_on: BTreeSet::new(),
            connected_to: BTreeSet::new(),
            default_value: None,
            index,
            options: None,
            value: None,
        };
        board
            .layers
            .get_mut(layer)
            .expect("layer exists")
            .pins
            .insert(id.to_string(), pin);
    }

    fn connect(board: &mut Board, from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) {
        connect_pins(board, from_node, from_pin, to_node, to_pin).expect("pins connect");
    }

    fn link(board: &mut Board, from: &str, to: &str) {
        pin_mut(board, from).connected_to.insert(to.to_string());
        pin_mut(board, to).depends_on.insert(from.to_string());
    }

    fn pin<'a>(board: &'a Board, id: &str) -> &'a Pin {
        board
            .get_pin_by_id(id)
            .unwrap_or_else(|| panic!("pin {id} is missing"))
    }

    fn pin_mut<'a>(board: &'a mut Board, id: &str) -> &'a mut Pin {
        if let Some(node) = board
            .nodes
            .values_mut()
            .find(|node| node.pins.contains_key(id))
        {
            return node.pins.get_mut(id).expect("pin exists");
        }
        board
            .layers
            .values_mut()
            .find_map(|layer| layer.pins.get_mut(id))
            .unwrap_or_else(|| panic!("pin {id} is missing"))
    }

    fn set<S: AsRef<str>>(ids: &[S]) -> BTreeSet<String> {
        ids.iter().map(|id| id.as_ref().to_string()).collect()
    }

    fn layer_pins<'a>(board: &'a Board, layer: &str) -> Vec<&'a Pin> {
        let mut pins: Vec<&Pin> = board.layers[layer].pins.values().collect();
        pins.sort_by(|a, b| a.id.cmp(&b.id));
        pins
    }

    fn only_bridge<'a>(board: &'a Board, layer: &str) -> &'a Pin {
        let pins = layer_pins(board, layer);
        assert_eq!(
            pins.len(),
            1,
            "layer {layer} must carry exactly one boundary pin, found {:?}",
            pins.iter().map(|pin| &pin.name).collect::<Vec<_>>()
        );
        pins[0]
    }

    fn fingerprint(board: &Board) -> BTreeMap<String, String> {
        let node_pins = board
            .nodes
            .values()
            .flat_map(|node| node.pins.values().map(move |pin| (&node.id, pin)));
        let layer_pins = board
            .layers
            .values()
            .flat_map(|layer| layer.pins.values().map(move |pin| (&layer.id, pin)));
        node_pins
            .chain(layer_pins)
            .map(|(owner, pin)| {
                (
                    format!("{owner}/{}", pin.id),
                    flow_like_types::json::to_string(pin).expect("pin serializes"),
                )
            })
            .collect()
    }

    fn assert_cleanup_is_idempotent(board: &mut Board) {
        let settled = fingerprint(board);
        board.cleanup();
        assert_eq!(
            settled,
            fingerprint(board),
            "a second cleanup must leave every pin unchanged"
        );
    }

    #[test]
    fn child_layer_bridges_do_not_create_parent_layer_pins() {
        let mut board = test_board();

        let layer_a = Layer::new("layer-a".to_string(), "A".to_string(), LayerType::Collapsed);
        let mut layer_b = Layer::new("layer-b".to_string(), "B".to_string(), LayerType::Collapsed);
        layer_b.parent_id = Some(layer_a.id.clone());

        board.layers.insert(layer_a.id.clone(), layer_a.clone());
        board.layers.insert(layer_b.id.clone(), layer_b.clone());

        let mut parent_source = Node::new("source", "Source", "", "test");
        parent_source.id = "parent-source".to_string();
        parent_source.layer = Some(layer_a.id.clone());
        let parent_source_pin = parent_source
            .add_output_pin("out", "Out", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        let mut child_node = Node::new("child", "Child", "", "test");
        child_node.id = "child-node".to_string();
        child_node.layer = Some(layer_b.id.clone());
        let child_in_pin = child_node
            .add_input_pin("in", "In", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();
        let child_out_pin = child_node
            .add_output_pin("out", "Out", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        let mut parent_sink = Node::new("sink", "Sink", "", "test");
        parent_sink.id = "parent-sink".to_string();
        parent_sink.layer = Some(layer_a.id.clone());
        let parent_sink_pin = parent_sink
            .add_input_pin("in", "In", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        board
            .nodes
            .insert(parent_source.id.clone(), parent_source.clone());
        board
            .nodes
            .insert(child_node.id.clone(), child_node.clone());
        board
            .nodes
            .insert(parent_sink.id.clone(), parent_sink.clone());

        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            &parent_source.id,
            &parent_source_pin,
            &child_node.id,
            &child_in_pin,
        )
        .unwrap();
        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            &child_node.id,
            &child_out_pin,
            &parent_sink.id,
            &parent_sink_pin,
        )
        .unwrap();

        let mut bridge_layers = BridgeLayersCleanup::init(&mut board);
        let mut pins = HashMap::new();

        for node in board.nodes.values() {
            for pin in node.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Node(node.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Node(node),
                );
            }
        }

        for layer in board.layers.values() {
            bridge_layers.initial_layer_iteration(layer);

            for pin in layer.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Layer(layer.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Layer(layer),
                );
            }
        }

        for node in board.nodes.values_mut() {
            for pin in node.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        for layer in board.layers.values_mut() {
            for pin in layer.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        bridge_layers.post_process(&mut board, &pins);

        let parent_layer = board.layers.get(&layer_a.id).unwrap();
        let child_layer = board.layers.get(&layer_b.id).unwrap();

        assert!(parent_layer.pins.is_empty());
        assert_eq!(child_layer.pins.len(), 2);

        let input_bridge = child_layer
            .pins
            .values()
            .find(|pin| pin.pin_type == PinType::Input)
            .unwrap();
        let output_bridge = child_layer
            .pins
            .values()
            .find(|pin| pin.pin_type == PinType::Output)
            .unwrap();

        let parent_source = board.nodes.get("parent-source").unwrap();
        let child_node = board.nodes.get("child-node").unwrap();
        let parent_sink = board.nodes.get("parent-sink").unwrap();

        assert_eq!(parent_source.pins[&parent_source_pin].connected_to.len(), 1);
        assert!(
            parent_source.pins[&parent_source_pin]
                .connected_to
                .contains(&input_bridge.id)
        );
        assert_eq!(child_node.pins[&child_in_pin].depends_on.len(), 1);
        assert!(
            child_node.pins[&child_in_pin]
                .depends_on
                .contains(&input_bridge.id)
        );
        assert!(input_bridge.depends_on.contains(&parent_source_pin));
        assert!(input_bridge.connected_to.contains(&child_in_pin));

        assert_eq!(child_node.pins[&child_out_pin].connected_to.len(), 1);
        assert!(
            child_node.pins[&child_out_pin]
                .connected_to
                .contains(&output_bridge.id)
        );
        assert_eq!(parent_sink.pins[&parent_sink_pin].depends_on.len(), 1);
        assert!(
            parent_sink.pins[&parent_sink_pin]
                .depends_on
                .contains(&output_bridge.id)
        );
        assert!(output_bridge.depends_on.contains(&child_out_pin));
        assert!(output_bridge.connected_to.contains(&parent_sink_pin));
    }

    #[test]
    fn a_module_never_gets_bridge_pins() {
        let mut board = test_board();
        let module = Layer::new("mod".to_string(), "Mod".to_string(), LayerType::Module);
        board.layers.insert(module.id.clone(), module);

        let mut inside = Node::new("source", "Source", "", "test");
        inside.id = "inside".to_string();
        inside.layer = Some("mod".to_string());
        let inside_pin = inside
            .add_output_pin("out", "Out", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        let mut outside = Node::new("sink", "Sink", "", "test");
        outside.id = "outside".to_string();
        let outside_pin = outside
            .add_input_pin("in", "In", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        board.nodes.insert(inside.id.clone(), inside);
        board.nodes.insert(outside.id.clone(), outside);

        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            "inside",
            &inside_pin,
            "outside",
            &outside_pin,
        )
        .unwrap();

        board.cleanup();

        assert!(board.layers["mod"].pins.is_empty());
        assert!(
            board.nodes["inside"].pins[&inside_pin]
                .connected_to
                .contains(&outside_pin),
            "a wire out of a module must stay direct"
        );
        assert!(
            board.nodes["outside"].pins[&outside_pin]
                .depends_on
                .contains(&inside_pin)
        );
    }

    #[test]
    fn collapsed_layer_under_module_still_bridges_to_the_module() {
        let mut board = test_board();

        let module = Layer::new("mod".to_string(), "Mod".to_string(), LayerType::Module);
        let mut collapsed = Layer::new(
            "collapsed".to_string(),
            "Collapsed".to_string(),
            LayerType::Collapsed,
        );
        collapsed.parent_id = Some(module.id.clone());

        board.layers.insert(module.id.clone(), module.clone());
        board.layers.insert(collapsed.id.clone(), collapsed.clone());

        let mut inside = Node::new("source", "Source", "", "test");
        inside.id = "inside".to_string();
        inside.layer = Some(collapsed.id.clone());
        let inside_pin = inside
            .add_output_pin("out", "Out", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        let mut on_module = Node::new("sink", "Sink", "", "test");
        on_module.id = "on-module".to_string();
        on_module.layer = Some(module.id.clone());
        let on_module_pin = on_module
            .add_input_pin("in", "In", "", VariableType::String)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        board.nodes.insert(inside.id.clone(), inside);
        board.nodes.insert(on_module.id.clone(), on_module);

        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            "inside",
            &inside_pin,
            "on-module",
            &on_module_pin,
        )
        .unwrap();

        board.cleanup();

        assert!(
            board.layers["mod"].pins.is_empty(),
            "a module must never carry bridge pins"
        );
        assert_eq!(
            board.layers["collapsed"].pins.len(),
            1,
            "the crossing relative to the collapsed layer must still be bridged"
        );

        let bridge = board.layers["collapsed"].pins.values().next().unwrap();
        assert_eq!(bridge.pin_type, PinType::Output);
        assert!(bridge.depends_on.contains(&inside_pin));
        assert!(bridge.connected_to.contains(&on_module_pin));

        assert!(
            board.nodes["inside"].pins[&inside_pin]
                .connected_to
                .contains(&bridge.id)
        );
        assert!(
            board.nodes["on-module"].pins[&on_module_pin]
                .depends_on
                .contains(&bridge.id)
        );
    }

    #[test]
    fn ancestor_layer_boundary_pin_creates_child_exec_input_bridge() {
        let mut board = test_board();

        let mut layer_a = Layer::new("layer-a".to_string(), "A".to_string(), LayerType::Collapsed);
        let mut layer_b = Layer::new("layer-b".to_string(), "B".to_string(), LayerType::Collapsed);
        layer_b.parent_id = Some(layer_a.id.clone());

        let parent_boundary_pin = Pin {
            id: "layer-a-exec-in".to_string(),
            name: "exec_in".to_string(),
            friendly_name: "Exec In".to_string(),
            description: String::new(),
            pin_type: PinType::Input,
            data_type: VariableType::Execution,
            schema: None,
            value_type: ValueType::Normal,
            depends_on: BTreeSet::new(),
            connected_to: BTreeSet::new(),
            default_value: None,
            index: 1,
            options: None,
            value: None,
        };
        layer_a
            .pins
            .insert(parent_boundary_pin.id.clone(), parent_boundary_pin.clone());

        board.layers.insert(layer_a.id.clone(), layer_a.clone());
        board.layers.insert(layer_b.id.clone(), layer_b.clone());

        let mut child_node = Node::new("child", "Child", "", "test");
        child_node.id = "child-node".to_string();
        child_node.layer = Some(layer_b.id.clone());
        let child_exec_in = child_node
            .add_input_pin("exec_in", "Exec In", "", VariableType::Execution)
            .set_value_type(ValueType::Normal)
            .id
            .clone();

        board
            .nodes
            .insert(child_node.id.clone(), child_node.clone());

        crate::flow::board::commands::pins::connect_pins::connect_pins(
            &mut board,
            &layer_a.id,
            &parent_boundary_pin.id,
            &child_node.id,
            &child_exec_in,
        )
        .unwrap();

        let mut bridge_layers = BridgeLayersCleanup::init(&mut board);
        let mut pins = HashMap::new();

        for node in board.nodes.values() {
            for pin in node.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Node(node.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Node(node),
                );
            }
        }

        for layer in board.layers.values() {
            bridge_layers.initial_layer_iteration(layer);

            for pin in layer.pins.values() {
                pins.insert(
                    pin.id.clone(),
                    (
                        crate::flow::board::cleanup::PinEdges::of(pin),
                        crate::flow::board::cleanup::NodeOrLayer::Layer(layer.id.clone()),
                    ),
                );
                bridge_layers.initial_pin_iteration(
                    pin,
                    crate::flow::board::cleanup::NodeOrLayerRef::Layer(layer),
                );
            }
        }

        for node in board.nodes.values_mut() {
            for pin in node.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        for layer in board.layers.values_mut() {
            for pin in layer.pins.values_mut() {
                bridge_layers.main_pin_iteration(pin, &pins);
            }
        }

        bridge_layers.post_process(&mut board, &pins);

        let parent_layer = board.layers.get(&layer_a.id).unwrap();
        let child_layer = board.layers.get(&layer_b.id).unwrap();
        let child_node = board.nodes.get("child-node").unwrap();

        assert_eq!(parent_layer.pins.len(), 1);
        assert_eq!(child_layer.pins.len(), 1);

        let bridge_pin = child_layer.pins.values().next().unwrap();
        assert_eq!(bridge_pin.pin_type, PinType::Input);
        assert_eq!(bridge_pin.data_type, VariableType::Execution);
        assert!(bridge_pin.depends_on.contains(&parent_boundary_pin.id));
        assert!(bridge_pin.connected_to.contains(&child_exec_in));

        let updated_parent_pin = &parent_layer.pins[&parent_boundary_pin.id];
        assert_eq!(updated_parent_pin.connected_to.len(), 1);
        assert!(updated_parent_pin.connected_to.contains(&bridge_pin.id));

        let updated_child_pin = &child_node.pins[&child_exec_in];
        assert_eq!(updated_child_pin.depends_on.len(), 1);
        assert!(updated_child_pin.depends_on.contains(&bridge_pin.id));
    }

    #[test]
    fn one_producer_fanning_into_a_collapsed_layer_gets_one_input_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "add-invoice", None);
        let payload = add_pin(
            &mut board,
            "add-invoice",
            "payload",
            PinType::Output,
            VariableType::Struct,
        );
        {
            let payload = pin_mut(&mut board, &payload);
            payload.friendly_name = "Payload".to_string();
            payload.description = "The created invoice".to_string();
            payload.schema = Some(r#"{"type":"object","title":"Invoice"}"#.to_string());
        }

        let consumers = [
            ("struct", VariableType::Struct),
            ("value", VariableType::Generic),
            ("text", VariableType::String),
            ("struct", VariableType::Struct),
            ("value", VariableType::Generic),
        ];
        let mut inputs = BTreeSet::new();
        for (position, (name, data_type)) in consumers.into_iter().enumerate() {
            let node = format!("consumer-{position}");
            add_node(&mut board, &node, Some("layer"));
            let input = add_pin(&mut board, &node, name, PinType::Input, data_type);
            connect(&mut board, "add-invoice", &payload, &node, &input);
            inputs.insert(input);
        }

        board.cleanup();

        let bridge = only_bridge(&board, "layer");
        let producer = pin(&board, &payload);
        assert_eq!(bridge.pin_type, PinType::Input);
        assert_eq!(bridge.name, "payload");
        assert_eq!(bridge.friendly_name, "Payload");
        assert_eq!(bridge.description, producer.description);
        assert_eq!(bridge.data_type, VariableType::Struct);
        assert_eq!(bridge.value_type, producer.value_type);
        assert_eq!(bridge.schema, producer.schema);
        assert_ne!(bridge.id, payload);
        assert_eq!(bridge.depends_on, set(&[&payload]));
        assert_eq!(bridge.connected_to, inputs);
        assert_eq!(producer.connected_to, set(&[&bridge.id]));
        for input in &inputs {
            assert_eq!(pin(&board, input).depends_on, set(&[&bridge.id]));
        }

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_later_connect_from_the_same_producer_reuses_the_input_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "payload",
            PinType::Output,
            VariableType::Struct,
        );
        add_node(&mut board, "first", Some("layer"));
        let first = add_pin(
            &mut board,
            "first",
            "value",
            PinType::Input,
            VariableType::Generic,
        );
        connect(&mut board, "producer", &output, "first", &first);
        board.cleanup();
        let bridge = only_bridge(&board, "layer").id.clone();

        add_node(&mut board, "second", Some("layer"));
        let second = add_pin(
            &mut board,
            "second",
            "text",
            PinType::Input,
            VariableType::String,
        );
        connect(&mut board, "producer", &output, "second", &second);
        board.cleanup();

        let reused = only_bridge(&board, "layer");
        assert_eq!(reused.id, bridge, "the existing bridge must be reused");
        assert_eq!(reused.connected_to, set(&[&first, &second]));
        assert_eq!(pin(&board, &output).connected_to, set(&[&bridge]));
        assert_eq!(pin(&board, &second).depends_on, set(&[&bridge]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_new_outer_consumer_reuses_the_output_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "inner", Some("layer"));
        let output = add_pin(
            &mut board,
            "inner",
            "result",
            PinType::Output,
            VariableType::Integer,
        );
        add_node(&mut board, "first", None);
        let first = add_pin(
            &mut board,
            "first",
            "in",
            PinType::Input,
            VariableType::Integer,
        );
        connect(&mut board, "inner", &output, "first", &first);
        board.cleanup();
        let bridge = only_bridge(&board, "layer").id.clone();

        add_node(&mut board, "second", None);
        let second = add_pin(
            &mut board,
            "second",
            "in",
            PinType::Input,
            VariableType::Integer,
        );
        connect(&mut board, "inner", &output, "second", &second);
        board.cleanup();

        let reused = only_bridge(&board, "layer");
        assert_eq!(reused.id, bridge, "the existing bridge must be reused");
        assert_eq!(reused.pin_type, PinType::Output);
        assert_eq!(reused.connected_to, set(&[&first, &second]));
        assert_eq!(pin(&board, &output).connected_to, set(&[&bridge]));
        assert_eq!(pin(&board, &second).depends_on, set(&[&bridge]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn legacy_input_bridges_of_one_producer_merge_into_the_lowest_id() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "payload",
            PinType::Output,
            VariableType::Struct,
        );
        pin_mut(&mut board, &output).schema =
            Some(r#"{"type":"object","title":"Invoice"}"#.to_string());

        let legacy = [
            ("bridge-c", "struct", VariableType::Struct),
            ("bridge-a", "value", VariableType::Generic),
            ("bridge-b", "text", VariableType::String),
        ];
        let mut inputs = BTreeSet::new();
        for (position, (bridge, name, data_type)) in legacy.into_iter().enumerate() {
            let node = format!("consumer-{position}");
            add_node(&mut board, &node, Some("layer"));
            let input = add_pin(&mut board, &node, name, PinType::Input, data_type.clone());
            add_legacy_bridge(
                &mut board,
                "layer",
                bridge,
                name,
                PinType::Input,
                data_type,
                position as u16 + 1,
            );
            link(&mut board, &output, bridge);
            link(&mut board, bridge, &input);
            inputs.insert(input);
        }

        board.cleanup();

        let survivor = only_bridge(&board, "layer");
        let producer = pin(&board, &output);
        assert_eq!(survivor.id, "bridge-a");
        assert_eq!(survivor.pin_type, PinType::Input);
        assert_eq!(survivor.name, "payload");
        assert_eq!(survivor.friendly_name, "payload");
        assert_eq!(survivor.data_type, VariableType::Struct);
        assert_eq!(survivor.schema, producer.schema);
        assert_eq!(survivor.index, 1);
        assert_eq!(survivor.depends_on, set(&[&output]));
        assert_eq!(survivor.connected_to, inputs);
        assert_eq!(producer.connected_to, set(&["bridge-a"]));
        for input in &inputs {
            assert_eq!(pin(&board, input).depends_on, set(&["bridge-a"]));
        }

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn legacy_output_bridges_of_one_producer_merge_into_the_lowest_id() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "inner", Some("layer"));
        let output = add_pin(
            &mut board,
            "inner",
            "result",
            PinType::Output,
            VariableType::Integer,
        );

        let mut consumers = BTreeSet::new();
        for (position, bridge) in ["out-b", "out-a"].into_iter().enumerate() {
            let node = format!("consumer-{position}");
            add_node(&mut board, &node, None);
            let input = add_pin(
                &mut board,
                &node,
                "in",
                PinType::Input,
                VariableType::Integer,
            );
            add_legacy_bridge(
                &mut board,
                "layer",
                bridge,
                &format!("legacy {position}"),
                PinType::Output,
                VariableType::Integer,
                position as u16 + 1,
            );
            link(&mut board, &output, bridge);
            link(&mut board, bridge, &input);
            consumers.insert(input);
        }

        board.cleanup();

        let survivor = only_bridge(&board, "layer");
        assert_eq!(survivor.id, "out-a");
        assert_eq!(survivor.pin_type, PinType::Output);
        assert_eq!(survivor.name, "result");
        assert_eq!(survivor.index, 1);
        assert_eq!(survivor.depends_on, set(&[&output]));
        assert_eq!(survivor.connected_to, consumers);
        assert_eq!(pin(&board, &output).connected_to, set(&["out-a"]));
        for consumer in &consumers {
            assert_eq!(pin(&board, consumer).depends_on, set(&["out-a"]));
        }

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn function_return_pins_fed_by_one_producer_stay_separate() {
        let mut board = test_board();
        add_layer(&mut board, "function", None, LayerType::Function);
        add_node(&mut board, "body", Some("function"));
        let output = add_pin(
            &mut board,
            "body",
            "result",
            PinType::Output,
            VariableType::String,
        );
        add_legacy_bridge(
            &mut board,
            "function",
            "return-a",
            "a",
            PinType::Output,
            VariableType::String,
            1,
        );
        add_legacy_bridge(
            &mut board,
            "function",
            "return-b",
            "b",
            PinType::Output,
            VariableType::String,
            2,
        );
        link(&mut board, &output, "return-a");
        link(&mut board, &output, "return-b");

        board.cleanup();

        let returns = layer_pins(&board, "function");
        assert_eq!(
            returns
                .iter()
                .map(|boundary| (boundary.id.as_str(), boundary.name.as_str()))
                .collect::<Vec<_>>(),
            vec![("return-a", "a"), ("return-b", "b")]
        );
        for boundary in returns {
            assert_eq!(boundary.depends_on, set(&[&output]));
        }
        assert_eq!(
            pin(&board, &output).connected_to,
            set(&["return-a", "return-b"])
        );

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_function_layer_never_reuses_its_contract_pins() {
        let mut board = test_board();
        add_layer(&mut board, "function", None, LayerType::Function);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "value",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "first", Some("function"));
        let first = add_pin(
            &mut board,
            "first",
            "in",
            PinType::Input,
            VariableType::String,
        );
        add_legacy_bridge(
            &mut board,
            "function",
            "param",
            "param",
            PinType::Input,
            VariableType::String,
            1,
        );
        link(&mut board, &output, "param");
        link(&mut board, "param", &first);

        add_node(&mut board, "second", Some("function"));
        let second = add_pin(
            &mut board,
            "second",
            "in",
            PinType::Input,
            VariableType::String,
        );
        connect(&mut board, "producer", &output, "second", &second);

        board.cleanup();

        let boundary = layer_pins(&board, "function");
        assert_eq!(
            boundary.len(),
            2,
            "the contract pin must not absorb the new wire"
        );
        let param = pin(&board, "param");
        assert_eq!(param.name, "param");
        assert_eq!(param.connected_to, set(&[&first]));
        let fresh = boundary
            .iter()
            .find(|boundary| boundary.id != "param")
            .expect("a fresh bridge");
        assert_eq!(fresh.name, "value");
        assert_eq!(fresh.depends_on, set(&[&output]));
        assert_eq!(fresh.connected_to, set(&[&second]));
        assert_eq!(
            pin(&board, &output).connected_to,
            set(&["param", fresh.id.as_str()])
        );

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn two_outer_exec_producers_share_one_exec_input_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "first", None);
        let first = add_pin(
            &mut board,
            "first",
            "exec_out",
            PinType::Output,
            VariableType::Execution,
        );
        add_node(&mut board, "second", None);
        let second = add_pin(
            &mut board,
            "second",
            "exec_out",
            PinType::Output,
            VariableType::Execution,
        );
        add_node(&mut board, "entry", Some("layer"));
        let entry = add_pin(
            &mut board,
            "entry",
            "exec_in",
            PinType::Input,
            VariableType::Execution,
        );
        connect(&mut board, "first", &first, "entry", &entry);
        connect(&mut board, "second", &second, "entry", &entry);

        board.cleanup();

        let bridge = only_bridge(&board, "layer");
        assert_eq!(bridge.pin_type, PinType::Input);
        assert_eq!(bridge.data_type, VariableType::Execution);
        assert_eq!(bridge.name, "exec_in");
        assert_eq!(bridge.depends_on, set(&[&first, &second]));
        assert_eq!(bridge.connected_to, set(&[&entry]));
        assert_eq!(pin(&board, &first).connected_to, set(&[&bridge.id]));
        assert_eq!(pin(&board, &second).connected_to, set(&[&bridge.id]));
        assert_eq!(pin(&board, &entry).depends_on, set(&[&bridge.id]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn nested_exec_entries_are_keyed_by_the_inner_consumer() {
        let mut board = test_board();
        add_layer(&mut board, "outer", None, LayerType::Collapsed);
        add_layer(&mut board, "inner", Some("outer"), LayerType::Collapsed);
        add_node(&mut board, "start", None);
        let start = add_pin(
            &mut board,
            "start",
            "exec_out",
            PinType::Output,
            VariableType::Execution,
        );
        add_node(&mut board, "outer-start", Some("outer"));
        let outer_start = add_pin(
            &mut board,
            "outer-start",
            "exec_out",
            PinType::Output,
            VariableType::Execution,
        );
        add_node(&mut board, "entry", Some("inner"));
        let entry = add_pin(
            &mut board,
            "entry",
            "exec_in",
            PinType::Input,
            VariableType::Execution,
        );
        connect(&mut board, "start", &start, "entry", &entry);
        connect(&mut board, "outer-start", &outer_start, "entry", &entry);

        board.cleanup();

        let outer = only_bridge(&board, "outer");
        let inner = only_bridge(&board, "inner");
        assert_eq!(outer.data_type, VariableType::Execution);
        assert_eq!(inner.data_type, VariableType::Execution);
        assert_eq!(pin(&board, &start).connected_to, set(&[&outer.id]));
        assert_eq!(outer.depends_on, set(&[&start]));
        assert_eq!(outer.connected_to, set(&[&inner.id]));
        assert_eq!(inner.depends_on, set(&[&outer.id, &outer_start]));
        assert_eq!(inner.connected_to, set(&[&entry]));
        assert_eq!(pin(&board, &outer_start).connected_to, set(&[&inner.id]));
        assert_eq!(pin(&board, &entry).depends_on, set(&[&inner.id]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn collapsing_a_selection_with_a_layer_node_bridges_its_outer_wires() {
        let mut board = test_board();
        add_layer(&mut board, "inner-layer", None, LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "payload",
            PinType::Output,
            VariableType::Struct,
        );
        add_node(&mut board, "nested", Some("inner-layer"));
        let nested = add_pin(
            &mut board,
            "nested",
            "value",
            PinType::Input,
            VariableType::Struct,
        );
        add_node(&mut board, "sibling", None);
        let sibling = add_pin(
            &mut board,
            "sibling",
            "value",
            PinType::Input,
            VariableType::Struct,
        );
        connect(&mut board, "producer", &output, "nested", &nested);
        connect(&mut board, "producer", &output, "sibling", &sibling);
        board.cleanup();
        let inner_bridge = only_bridge(&board, "inner-layer").id.clone();

        add_layer(&mut board, "collapsed", None, LayerType::Collapsed);
        board
            .layers
            .get_mut("inner-layer")
            .expect("layer exists")
            .parent_id = Some("collapsed".to_string());
        board.nodes.get_mut("sibling").expect("node exists").layer = Some("collapsed".to_string());
        board.cleanup();

        let bridge = only_bridge(&board, "collapsed");
        assert_eq!(bridge.pin_type, PinType::Input);
        assert_eq!(bridge.name, "payload");
        assert_eq!(bridge.depends_on, set(&[&output]));
        assert_eq!(bridge.connected_to, set(&[&inner_bridge, &sibling]));
        assert_eq!(pin(&board, &output).connected_to, set(&[&bridge.id]));
        assert_eq!(only_bridge(&board, "inner-layer").id, inner_bridge);
        assert_eq!(pin(&board, &inner_bridge).depends_on, set(&[&bridge.id]));
        assert_eq!(pin(&board, &inner_bridge).connected_to, set(&[&nested]));
        assert_eq!(pin(&board, &sibling).depends_on, set(&[&bridge.id]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn nested_layers_share_the_outer_input_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "outer", None, LayerType::Collapsed);
        add_layer(&mut board, "inner", Some("outer"), LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "payload",
            PinType::Output,
            VariableType::Struct,
        );
        add_node(&mut board, "deep", Some("inner"));
        let deep = add_pin(
            &mut board,
            "deep",
            "value",
            PinType::Input,
            VariableType::Struct,
        );
        add_node(&mut board, "shallow", Some("outer"));
        let shallow = add_pin(
            &mut board,
            "shallow",
            "value",
            PinType::Input,
            VariableType::Struct,
        );
        connect(&mut board, "producer", &output, "deep", &deep);
        connect(&mut board, "producer", &output, "shallow", &shallow);

        board.cleanup();

        let outer = only_bridge(&board, "outer");
        let inner = only_bridge(&board, "inner");
        assert_eq!(outer.pin_type, PinType::Input);
        assert_eq!(inner.pin_type, PinType::Input);
        assert_eq!(inner.name, "payload");
        assert_eq!(pin(&board, &output).connected_to, set(&[&outer.id]));
        assert_eq!(outer.depends_on, set(&[&output]));
        assert_eq!(outer.connected_to, set(&[&inner.id, &shallow]));
        assert_eq!(inner.depends_on, set(&[&outer.id]));
        assert_eq!(inner.connected_to, set(&[&deep]));
        assert_eq!(pin(&board, &deep).depends_on, set(&[&inner.id]));
        assert_eq!(pin(&board, &shallow).depends_on, set(&[&outer.id]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn nested_layers_share_the_inner_output_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "outer", None, LayerType::Collapsed);
        add_layer(&mut board, "inner", Some("outer"), LayerType::Collapsed);
        add_node(&mut board, "producer", Some("inner"));
        let output = add_pin(
            &mut board,
            "producer",
            "result",
            PinType::Output,
            VariableType::Integer,
        );
        add_node(&mut board, "root-sink", None);
        let root_sink = add_pin(
            &mut board,
            "root-sink",
            "in",
            PinType::Input,
            VariableType::Integer,
        );
        add_node(&mut board, "outer-sink", Some("outer"));
        let outer_sink = add_pin(
            &mut board,
            "outer-sink",
            "in",
            PinType::Input,
            VariableType::Integer,
        );
        connect(&mut board, "producer", &output, "root-sink", &root_sink);
        connect(&mut board, "producer", &output, "outer-sink", &outer_sink);

        board.cleanup();

        let inner = only_bridge(&board, "inner");
        let outer = only_bridge(&board, "outer");
        assert_eq!(inner.pin_type, PinType::Output);
        assert_eq!(outer.pin_type, PinType::Output);
        assert_eq!(outer.name, "result");
        assert_eq!(pin(&board, &output).connected_to, set(&[&inner.id]));
        assert_eq!(inner.connected_to, set(&[&outer.id, &outer_sink]));
        assert_eq!(outer.depends_on, set(&[&inner.id]));
        assert_eq!(outer.connected_to, set(&[&root_sink]));
        assert_eq!(pin(&board, &root_sink).depends_on, set(&[&outer.id]));
        assert_eq!(pin(&board, &outer_sink).depends_on, set(&[&inner.id]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_bridge_wired_straight_across_its_parent_gets_the_missing_outer_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "outer", None, LayerType::Collapsed);
        add_layer(&mut board, "inner", Some("outer"), LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "payload",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "consumer", Some("inner"));
        let input = add_pin(
            &mut board,
            "consumer",
            "in",
            PinType::Input,
            VariableType::String,
        );
        add_legacy_bridge(
            &mut board,
            "inner",
            "legacy",
            "in",
            PinType::Input,
            VariableType::String,
            1,
        );
        link(&mut board, &output, "legacy");
        link(&mut board, "legacy", &input);

        board.cleanup();

        let outer = only_bridge(&board, "outer");
        assert_eq!(only_bridge(&board, "inner").id, "legacy");
        assert_eq!(outer.name, "payload");
        assert_eq!(pin(&board, &output).connected_to, set(&[&outer.id]));
        assert_eq!(outer.connected_to, set(&["legacy"]));
        assert_eq!(pin(&board, "legacy").depends_on, set(&[&outer.id]));
        assert_eq!(pin(&board, "legacy").connected_to, set(&[&input]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn replicas_cleaning_the_same_legacy_board_mint_identical_bridges() {
        let mut board = test_board();
        add_layer(&mut board, "outer", None, LayerType::Collapsed);
        add_layer(&mut board, "inner", Some("outer"), LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "payload",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "consumer", Some("inner"));
        let input = add_pin(
            &mut board,
            "consumer",
            "in",
            PinType::Input,
            VariableType::String,
        );
        add_node(&mut board, "result", Some("inner"));
        let result = add_pin(
            &mut board,
            "result",
            "result",
            PinType::Output,
            VariableType::Integer,
        );
        add_node(&mut board, "sink", None);
        let sink = add_pin(
            &mut board,
            "sink",
            "in",
            PinType::Input,
            VariableType::Integer,
        );
        add_legacy_bridge(
            &mut board,
            "inner",
            "legacy-in",
            "in",
            PinType::Input,
            VariableType::String,
            1,
        );
        add_legacy_bridge(
            &mut board,
            "inner",
            "legacy-out",
            "result",
            PinType::Output,
            VariableType::Integer,
            1,
        );
        link(&mut board, &output, "legacy-in");
        link(&mut board, "legacy-in", &input);
        link(&mut board, &result, "legacy-out");
        link(&mut board, "legacy-out", &sink);

        let mut desktop = board.clone();
        let mut hub = board;
        desktop.cleanup();
        hub.cleanup();

        let minted = layer_pins(&desktop, "outer");
        assert_eq!(minted.len(), 2, "both crossings of the outer layer mint");
        for bridge in minted {
            assert_eq!(bridge.id.len(), BRIDGE_ID_LENGTH);
            assert!(bridge.id.starts_with(|c: char| c.is_ascii_lowercase()));
            assert!(
                bridge
                    .id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
            );
        }
        assert_eq!(
            fingerprint(&desktop),
            fingerprint(&hub),
            "every replica loading the same board must mint the same pin ids"
        );

        assert_cleanup_is_idempotent(&mut desktop);
    }

    #[test]
    fn a_function_layer_reminting_a_key_rehashes_the_taken_id() {
        let mut board = test_board();
        add_layer(&mut board, "function", None, LayerType::Function);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "value",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "first", Some("function"));
        let first = add_pin(
            &mut board,
            "first",
            "in",
            PinType::Input,
            VariableType::String,
        );
        connect(&mut board, "producer", &output, "first", &first);
        board.cleanup();

        let key = ("function".to_string(), Direction::Input, output.clone());
        let original = derived_bridge_id(&key, 0);
        assert_eq!(only_bridge(&board, "function").id, original);

        add_node(&mut board, "second", Some("function"));
        let second = add_pin(
            &mut board,
            "second",
            "in",
            PinType::Input,
            VariableType::String,
        );
        connect(&mut board, "producer", &output, "second", &second);
        board.cleanup();

        let rehashed = derived_bridge_id(&key, 1);
        assert_ne!(rehashed, original);
        assert_eq!(pin(&board, &original).connected_to, set(&[&first]));
        assert_eq!(pin(&board, &rehashed).connected_to, set(&[&second]));
        assert_eq!(
            pin(&board, &output).connected_to,
            set(&[&original, &rehashed])
        );

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_derived_id_held_by_a_node_pin_is_rehashed() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "value",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "consumer", Some("layer"));
        let input = add_pin(
            &mut board,
            "consumer",
            "in",
            PinType::Input,
            VariableType::String,
        );
        connect(&mut board, "producer", &output, "consumer", &input);

        let key = ("layer".to_string(), Direction::Input, output.clone());
        let taken = derived_bridge_id(&key, 0);
        add_node(&mut board, "squatter", None);
        let squatter = add_pin(
            &mut board,
            "squatter",
            "value",
            PinType::Output,
            VariableType::String,
        );
        let node = board.nodes.get_mut("squatter").expect("node exists");
        let mut held = node.pins.remove(&squatter).expect("pin exists");
        held.id = taken.clone();
        node.pins.insert(taken.clone(), held);

        board.cleanup();

        let bridge = only_bridge(&board, "layer");
        assert_eq!(bridge.id, derived_bridge_id(&key, 1));
        assert_eq!(pin(&board, &output).connected_to, set(&[&bridge.id]));
        assert!(board.nodes["squatter"].pins.contains_key(&taken));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_sibling_layer_crossing_routes_through_both_boundaries_once() {
        let mut board = test_board();
        add_layer(&mut board, "source-layer", None, LayerType::Collapsed);
        add_layer(&mut board, "target-layer", None, LayerType::Collapsed);
        add_node(&mut board, "producer", Some("source-layer"));
        let output = add_pin(
            &mut board,
            "producer",
            "result",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "consumer", Some("target-layer"));
        let input = add_pin(
            &mut board,
            "consumer",
            "in",
            PinType::Input,
            VariableType::String,
        );
        connect(&mut board, "producer", &output, "consumer", &input);

        board.cleanup();

        let exit = only_bridge(&board, "source-layer");
        let entry = only_bridge(&board, "target-layer");
        assert_eq!(exit.pin_type, PinType::Output);
        assert_eq!(entry.pin_type, PinType::Input);
        assert_eq!(entry.name, "result");
        assert_eq!(pin(&board, &output).connected_to, set(&[&exit.id]));
        assert_eq!(exit.connected_to, set(&[&entry.id]));
        assert_eq!(entry.depends_on, set(&[&exit.id]));
        assert_eq!(entry.connected_to, set(&[&input]));
        assert_eq!(pin(&board, &input).depends_on, set(&[&entry.id]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_legacy_parallel_path_between_siblings_collapses_into_one_route() {
        let mut board = test_board();
        add_layer(&mut board, "source-layer", None, LayerType::Collapsed);
        add_layer(&mut board, "target-layer", None, LayerType::Collapsed);
        add_node(&mut board, "producer", Some("source-layer"));
        let output = add_pin(
            &mut board,
            "producer",
            "result",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "consumer", Some("target-layer"));
        let input = add_pin(
            &mut board,
            "consumer",
            "in",
            PinType::Input,
            VariableType::String,
        );
        add_legacy_bridge(
            &mut board,
            "source-layer",
            "legacy-out",
            "result",
            PinType::Output,
            VariableType::String,
            1,
        );
        add_legacy_bridge(
            &mut board,
            "target-layer",
            "legacy-in",
            "in",
            PinType::Input,
            VariableType::String,
            1,
        );
        link(&mut board, &output, "legacy-out");
        link(&mut board, "legacy-out", &input);
        link(&mut board, &output, "legacy-in");
        link(&mut board, "legacy-in", &input);

        board.cleanup();

        let exit = only_bridge(&board, "source-layer");
        let entry = only_bridge(&board, "target-layer");
        assert_eq!(exit.id, "legacy-out");
        assert_eq!(pin(&board, &output).connected_to, set(&["legacy-out"]));
        assert_eq!(exit.connected_to, set(&[&entry.id]));
        assert_eq!(entry.depends_on, set(&["legacy-out"]));
        assert_eq!(entry.connected_to, set(&[&input]));
        assert_eq!(pin(&board, &input).depends_on, set(&[&entry.id]));

        assert_cleanup_is_idempotent(&mut board);
    }

    #[test]
    fn a_half_edge_is_never_resurrected_as_a_bridge() {
        let mut board = test_board();
        add_layer(&mut board, "layer", None, LayerType::Collapsed);
        add_node(&mut board, "producer", None);
        let output = add_pin(
            &mut board,
            "producer",
            "out",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "consumer", Some("layer"));
        let input = add_pin(
            &mut board,
            "consumer",
            "in",
            PinType::Input,
            VariableType::String,
        );
        add_node(&mut board, "other-producer", None);
        let other_output = add_pin(
            &mut board,
            "other-producer",
            "out",
            PinType::Output,
            VariableType::String,
        );
        add_node(&mut board, "other-consumer", Some("layer"));
        let other_input = add_pin(
            &mut board,
            "other-consumer",
            "in",
            PinType::Input,
            VariableType::String,
        );
        pin_mut(&mut board, &output)
            .connected_to
            .insert(input.clone());
        pin_mut(&mut board, &other_input)
            .depends_on
            .insert(other_output.clone());

        board.cleanup();

        assert!(board.layers["layer"].pins.is_empty());
        assert!(pin(&board, &output).connected_to.is_empty());
        assert!(pin(&board, &input).depends_on.is_empty());
        assert!(pin(&board, &other_output).connected_to.is_empty());
        assert!(pin(&board, &other_input).depends_on.is_empty());

        assert_cleanup_is_idempotent(&mut board);
    }
}
