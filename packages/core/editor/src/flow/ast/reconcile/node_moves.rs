//! Anchored statements written into a `function` whose layer does not own them yet.
//!
//! Cutting anchored lines out of an event and pasting them into a `function` is an
//! extract-function refactor, not a request for new nodes: the text is authoritative about where
//! a statement lives. Each such node keeps its identity (configuration, id, position) and moves
//! into the Function layer. Every edge that would cross the function boundary afterwards is cut on
//! a planning copy of the board BEFORE the structural planner runs, so the planner rewires both
//! sides from the text alone — exactly as it re-joins execution around a removed statement. Pure
//! helpers that feed nothing but moved nodes travel with them.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use flow_like_ast::model::{Block, BoardAst, Stmt};

use super::{
    all_board_nodes, board_node_semantic_scope, find_board_node, is_exec_pin, is_trigger_entry,
    node_pin_literal_string,
};
use crate::flow::board::{Board, LayerType};
use crate::flow::copilot::BoardCommand;
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum MoveDestination {
    /// A live Function layer.
    Layer(String),
    /// The `function` at this index of the flattened document, whose layer this plan creates.
    NewFunction(usize),
}

/// One direct board edge between a moved node and something outside its destination.
#[derive(Debug, Clone)]
pub(super) struct CutEdge {
    pub(super) from_owner: String,
    pub(super) from_pin: String,
    pub(super) to_owner: String,
    pub(super) to_pin: String,
}

/// An execution endpoint that stays where it was and lost its edge to a moved node.
#[derive(Debug, Clone)]
struct StrandedExec {
    owner: String,
    pin_id: String,
    pin_name: String,
    /// `owner`'s output lost its target (otherwise its input lost a source).
    lost_successor: bool,
    moved: String,
}

impl StrandedExec {
    fn rewired_by(&self, commands: &[BoardCommand]) -> bool {
        let matches = |node: &str, pin: &str| {
            node == self.owner && (pin == self.pin_name || pin == self.pin_id)
        };
        commands.iter().any(|command| match command {
            BoardCommand::ConnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                if self.lost_successor {
                    matches(from_node, from_pin)
                } else {
                    matches(to_node, to_pin)
                }
            }
            _ => false,
        })
    }
}

#[derive(Debug, Default)]
pub(super) struct NodeMovePlan {
    /// Every node this edit re-homes: the written anchored statements and the pure sources that
    /// travel with them.
    pub(super) moves: BTreeMap<String, MoveDestination>,
    pub(super) cut_edges: Vec<CutEdge>,
    stranded: Vec<StrandedExec>,
    function_names: BTreeMap<MoveDestination, String>,
    /// Every statement anchor the document writes. The text is authoritative about what follows a
    /// written statement, so only an unwritten one can be stranded.
    written: HashSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ScopeToken {
    Live(Option<String>),
    New(usize),
}

enum PinOwner {
    Node {
        node: String,
        name: String,
    },
    /// A Function layer's boundary pin: the function's own contract, seen from inside it.
    FunctionBoundary {
        layer: String,
        name: String,
    },
    /// A Collapsed/Macro bridge: a bend in a wire that continues on its other side.
    Bridge {
        layer: String,
        depends_on: Vec<String>,
        connected_to: Vec<String>,
    },
}

/// Who owns every pin of the board.
struct PinIndex {
    owners: HashMap<String, PinOwner>,
}

impl PinIndex {
    fn new(board: &Board) -> Self {
        let mut owners = HashMap::new();
        for node in all_board_nodes(board) {
            for pin in node.pins.values() {
                owners.insert(
                    pin.id.clone(),
                    PinOwner::Node {
                        node: node.id.clone(),
                        name: pin.name.clone(),
                    },
                );
            }
        }
        for layer in board.layers.values() {
            let function = matches!(layer.r#type, LayerType::Function);
            for pin in layer.pins.values() {
                let owner = if function {
                    PinOwner::FunctionBoundary {
                        layer: layer.id.clone(),
                        name: pin.name.clone(),
                    }
                } else {
                    PinOwner::Bridge {
                        layer: layer.id.clone(),
                        depends_on: pin.depends_on.iter().cloned().collect(),
                        connected_to: pin.connected_to.iter().cloned().collect(),
                    }
                };
                owners.insert(pin.id.clone(), owner);
            }
        }
        Self { owners }
    }

    fn node_of(&self, pin_id: &str) -> Option<&str> {
        match self.owners.get(pin_id) {
            Some(PinOwner::Node { node, .. }) => Some(node),
            _ => None,
        }
    }

    /// The real pins at the far end of `pin_id`, following presentational bridges. `upstream`
    /// walks towards producers.
    fn real_endpoints(&self, pin_id: &str, upstream: bool) -> Vec<String> {
        let mut out = Vec::new();
        let mut stack = vec![pin_id.to_string()];
        let mut seen = HashSet::new();
        while let Some(id) = stack.pop() {
            if !seen.insert(id.clone()) {
                continue;
            }
            match self.owners.get(&id) {
                Some(PinOwner::Bridge {
                    depends_on,
                    connected_to,
                    ..
                }) => {
                    let next = if upstream { depends_on } else { connected_to };
                    stack.extend(next.iter().cloned());
                }
                Some(_) => out.push(id),
                None => {}
            }
        }
        out.sort();
        out
    }
}

impl NodeMovePlan {
    pub(super) fn is_empty(&self) -> bool {
        self.moves.is_empty()
    }

    pub(super) fn function_name(&self, destination: &MoveDestination) -> &str {
        self.function_names
            .get(destination)
            .map(String::as_str)
            .unwrap_or_default()
    }

    /// The board the structural planner reasons over: the live board with every cut edge removed.
    pub(super) fn planning_board(&self, existing: &Board) -> Option<Board> {
        if self.cut_edges.is_empty() {
            return None;
        }
        let mut board = existing.clone();
        for edge in &self.cut_edges {
            edit_pin(&mut board, &edge.from_pin, |pin| {
                pin.connected_to.remove(&edge.to_pin);
            });
            edit_pin(&mut board, &edge.to_pin, |pin| {
                pin.depends_on.remove(&edge.from_pin);
            });
        }
        Some(board)
    }

    pub(super) fn disconnect_commands(&self) -> Vec<BoardCommand> {
        self.cut_edges
            .iter()
            .map(|edge| BoardCommand::DisconnectPins {
                from_node: edge.from_owner.clone(),
                from_pin: edge.from_pin.clone(),
                to_node: edge.to_owner.clone(),
                to_pin: edge.to_pin.clone(),
                summary: Some("Detach a node moved into a function".to_string()),
            })
            .collect()
    }

    /// Whether `owner`'s execution output `pin_id` lost its only target to a moved node.
    pub(super) fn output_lost_to_move(&self, owner: &str, pin_id: &str) -> bool {
        self.stranded.iter().any(|stranded| {
            stranded.lost_successor && stranded.owner == owner && stranded.pin_id == pin_id
        })
    }

    /// A moved statement leaves a gap where it was. Writing the section it came from fills it (a
    /// call to the function, or the next statement); a document that leaves that section out would
    /// silently stop what used to run there.
    pub(super) fn stranded_diagnostics(
        &self,
        planning: &Board,
        commands: &[BoardCommand],
        removed: &HashSet<String>,
    ) -> Vec<String> {
        self.stranded
            .iter()
            .filter(|stranded| !self.gap_is_filled(stranded, planning, commands, removed))
            .map(|stranded| self.stranded_message(stranded, planning))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    /// A Function whose body keeps nodes must still enter and leave through them, written or not.
    fn gap_is_filled(
        &self,
        stranded: &StrandedExec,
        planning: &Board,
        commands: &[BoardCommand],
        removed: &HashSet<String>,
    ) -> bool {
        let boundary = planning.layers.contains_key(&stranded.owner);
        let still_fed = !stranded.lost_successor && input_is_fed(planning, &stranded.pin_id);
        removed.contains(&stranded.owner)
            || (!boundary && self.written.contains(&stranded.owner))
            || self.function_left_empty(planning, &stranded.owner, removed)
            || stranded.rewired_by(commands)
            || still_fed
    }

    fn stranded_message(&self, stranded: &StrandedExec, planning: &Board) -> String {
        let destination = self
            .moves
            .get(&stranded.moved)
            .map(|destination| self.function_name(destination))
            .unwrap_or_default();
        let effect = if stranded.lost_successor {
            "with nothing to run next"
        } else {
            "with no execution predecessor"
        };
        format!(
            "moving {} into function `{destination}` leaves {} {effect}; also write the statements around the moved lines, with a `{destination}(…)` call where they were",
            entity_label(planning, &stranded.moved),
            entity_label(planning, &stranded.owner),
        )
    }

    /// A Function layer all of whose nodes move out or are removed needs no entry or exit wiring.
    fn function_left_empty(&self, board: &Board, owner: &str, removed: &HashSet<String>) -> bool {
        board
            .layers
            .get(owner)
            .is_some_and(|layer| matches!(layer.r#type, LayerType::Function))
            && all_board_nodes(board).into_iter().all(|node| {
                board_node_semantic_scope(board, node).as_deref() != Some(owner)
                    || self.moves.contains_key(&node.id)
                    || removed.contains(&node.id)
            })
    }
}

pub(super) fn plan_node_moves(board: &Board, ast: &BoardAst) -> NodeMovePlan {
    let (mut moves, function_names) = written_moves(board, ast);
    if moves.is_empty() {
        return NodeMovePlan::default();
    }
    let written = document_statement_anchors(ast);
    let index = PinIndex::new(board);
    carry_exclusive_pure_sources(board, &written, &index, &mut moves);
    let (cut_edges, stranded) = crossing_edges(board, &index, &moves);
    NodeMovePlan {
        moves,
        cut_edges,
        stranded,
        function_names,
        written,
    }
}

type WrittenMoves = (
    BTreeMap<String, MoveDestination>,
    BTreeMap<MoveDestination, String>,
);

fn written_moves(board: &Board, ast: &BoardAst) -> WrittenMoves {
    let mut moves = BTreeMap::new();
    let mut names = BTreeMap::new();
    for (index, func) in ast.functions.iter().enumerate() {
        let Some(destination) = function_destination(board, ast, index) else {
            continue;
        };
        for anchor in foreign_statements(board, &func.body, &destination) {
            moves.entry(anchor).or_insert_with(|| destination.clone());
            names
                .entry(destination.clone())
                .or_insert_with(|| func.name.clone());
        }
    }
    (moves, names)
}

/// Where the anchored statements of the `function` at `index` belong. An unanchored function
/// that shares its name with a live Function layer is that layer to the planner; its statements
/// are left to the planner's own scope checks rather than guessed at here.
fn function_destination(board: &Board, ast: &BoardAst, index: usize) -> Option<MoveDestination> {
    let func = &ast.functions[index];
    if let Some(anchor) = func.anchor.as_deref() {
        return board
            .layers
            .get(anchor)
            .filter(|layer| matches!(layer.r#type, LayerType::Function))
            .map(|_| MoveDestination::Layer(anchor.to_string()));
    }
    let name = flow_like_ast::to_camel_case(&func.name);
    let named_like_a_live_layer = board.layers.values().any(|layer| {
        matches!(layer.r#type, LayerType::Function)
            && flow_like_ast::to_camel_case(&layer.name) == name
    });
    (!named_like_a_live_layer).then_some(MoveDestination::NewFunction(index))
}

/// Anchored statements of `body` whose nodes live outside `destination` and can move there.
fn foreign_statements(board: &Board, body: &Block, destination: &MoveDestination) -> Vec<String> {
    let mut anchors = Vec::new();
    collect_statement_anchors(body, false, &mut anchors);
    anchors.retain(|anchor| {
        find_board_node(board, anchor).is_some_and(|node| {
            let owned = match destination {
                MoveDestination::Layer(layer) => {
                    board_node_semantic_scope(board, node).as_deref() == Some(layer.as_str())
                }
                MoveDestination::NewFunction(_) => false,
            };
            !owned && !is_trigger_entry(node) && !reads_a_foreign_local(board, node, destination)
        })
    });
    anchors
}

fn collect_statement_anchors(block: &Block, into_handlers: bool, out: &mut Vec<String>) {
    for stmt in &block.stmts {
        match stmt {
            Stmt::Handler(event) => {
                if into_handlers {
                    out.extend(event.anchor.clone());
                    collect_statement_anchors(&event.body, true, out);
                }
                continue;
            }
            Stmt::Collapse(group) => {
                collect_statement_anchors(&group.body, into_handlers, out);
                continue;
            }
            Stmt::Local(_) | Stmt::Comment(_) => continue,
            _ => out.extend(stmt.anchor().map(str::to_string)),
        }
        match stmt {
            Stmt::Branch { arms, .. } => {
                for arm in arms {
                    collect_statement_anchors(&arm.body, into_handlers, out);
                }
            }
            Stmt::Loop { body, .. } => collect_statement_anchors(body, into_handlers, out),
            _ => {}
        }
    }
}

fn document_statement_anchors(ast: &BoardAst) -> HashSet<String> {
    let mut out = Vec::new();
    for event in &ast.events {
        out.extend(event.anchor.clone());
        collect_statement_anchors(&event.body, true, &mut out);
    }
    for func in &ast.functions {
        collect_statement_anchors(&func.body, true, &mut out);
    }
    for block in &ast.detached {
        collect_statement_anchors(block, true, &mut out);
    }
    out.into_iter().collect()
}

/// A pure node (no execution pins) feeding only moved nodes moves with the first of them. One
/// that also feeds a node staying behind stays, and the moved consumer gets its own copy from the
/// planner. Sources the text places explicitly keep that placement.
fn carry_exclusive_pure_sources(
    board: &Board,
    anchored: &HashSet<String>,
    index: &PinIndex,
    moves: &mut BTreeMap<String, MoveDestination>,
) {
    loop {
        let mut carried: BTreeMap<String, MoveDestination> = BTreeMap::new();
        for (consumer, destination) in moves.iter() {
            for producer in data_producers(board, index, consumer) {
                if !carried.contains_key(&producer)
                    && can_carry(board, anchored, index, moves, consumer, &producer)
                {
                    carried.insert(producer, destination.clone());
                }
            }
        }
        if carried.is_empty() {
            return;
        }
        moves.extend(carried);
    }
}

/// The nodes feeding `node_id`'s data inputs, behind any presentational bridges.
fn data_producers(board: &Board, index: &PinIndex, node_id: &str) -> Vec<String> {
    let Some(node) = find_board_node(board, node_id) else {
        return Vec::new();
    };
    let producers = node
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Input && !is_exec_pin(pin))
        .flat_map(|pin| pin.depends_on.iter())
        .flat_map(|source| index.real_endpoints(source, true))
        .filter_map(|pin| index.node_of(&pin).map(str::to_string))
        .collect::<BTreeSet<_>>();
    producers.into_iter().collect()
}

fn can_carry(
    board: &Board,
    anchored: &HashSet<String>,
    index: &PinIndex,
    moves: &BTreeMap<String, MoveDestination>,
    consumer: &str,
    producer: &str,
) -> bool {
    let (Some(consumer), Some(producer)) = (
        find_board_node(board, consumer),
        find_board_node(board, producer),
    ) else {
        return false;
    };
    !moves.contains_key(&producer.id)
        && !anchored.contains(&producer.id)
        && !producer.pins.values().any(is_exec_pin)
        && !is_trigger_entry(producer)
        && board_node_semantic_scope(board, producer) == board_node_semantic_scope(board, consumer)
        && moves
            .get(&consumer.id)
            .is_some_and(|destination| !reads_a_foreign_local(board, producer, destination))
        && feeds_only_moved(index, producer, moves)
}

/// A node bound to another layer's local variable cannot leave that layer: the variable stays
/// behind, and its name resolves to something else, or nothing, at the destination.
fn reads_a_foreign_local(board: &Board, node: &Node, destination: &MoveDestination) -> bool {
    node_pin_literal_string(node, "var_ref").is_some_and(|variable| {
        board.layers.values().any(|layer| {
            layer.variables.contains_key(&variable)
                && !matches!(destination, MoveDestination::Layer(id) if *id == layer.id)
        })
    })
}

fn feeds_only_moved(
    index: &PinIndex,
    producer: &Node,
    moves: &BTreeMap<String, MoveDestination>,
) -> bool {
    producer
        .pins
        .values()
        .filter(|pin| pin.pin_type == PinType::Output)
        .flat_map(|pin| pin.connected_to.iter())
        .flat_map(|target| index.real_endpoints(target, false))
        .all(|target| {
            index
                .node_of(&target)
                .is_some_and(|node| moves.contains_key(node))
        })
}

fn scope_token(
    board: &Board,
    moves: &BTreeMap<String, MoveDestination>,
    node: &Node,
) -> ScopeToken {
    match moves.get(&node.id) {
        Some(MoveDestination::Layer(layer)) => ScopeToken::Live(Some(layer.clone())),
        Some(MoveDestination::NewFunction(index)) => ScopeToken::New(*index),
        None => ScopeToken::Live(board_node_semantic_scope(board, node)),
    }
}

/// Every direct edge of a moved node whose other end does not end up in the same scope, and the
/// execution endpoints those cuts leave behind.
fn crossing_edges(
    board: &Board,
    index: &PinIndex,
    moves: &BTreeMap<String, MoveDestination>,
) -> (Vec<CutEdge>, Vec<StrandedExec>) {
    let mut cuts = Vec::new();
    let mut stranded = Vec::new();
    let mut seen_cuts = HashSet::new();
    let mut seen_stranded = HashSet::new();
    for (id, node) in moves
        .keys()
        .filter_map(|id| find_board_node(board, id).map(|node| (id, node)))
    {
        let token = scope_token(board, moves, node);
        let mut pins = node.pins.values().collect::<Vec<_>>();
        pins.sort_by(|left, right| left.id.cmp(&right.id));
        for pin in pins {
            let incoming = pin.pin_type == PinType::Input;
            let neighbors = if incoming {
                &pin.depends_on
            } else {
                &pin.connected_to
            };
            for neighbor in neighbors {
                let Some(owner) = crossing_owner(board, index, moves, &token, neighbor) else {
                    continue;
                };
                let cut = cut_between(id, pin, owner, neighbor, incoming);
                if !seen_cuts.insert((cut.from_pin.clone(), cut.to_pin.clone())) {
                    continue;
                }
                cuts.push(cut);
                if is_exec_pin(pin) {
                    stranded.extend(
                        stranded_ends(index, moves, id, neighbor, incoming)
                            .into_iter()
                            .filter(|end| {
                                seen_stranded.insert((end.owner.clone(), end.pin_id.clone()))
                            }),
                    );
                }
            }
        }
    }
    (cuts, stranded)
}

/// The owner of `neighbor` when the edge to it leaves `token`'s scope. An edge through a
/// Collapsed/Macro bridge always does: the moved node leaves that frame, and the planner
/// reconnects whatever the text still writes.
fn crossing_owner(
    board: &Board,
    index: &PinIndex,
    moves: &BTreeMap<String, MoveDestination>,
    token: &ScopeToken,
    neighbor: &str,
) -> Option<String> {
    match index.owners.get(neighbor)? {
        PinOwner::Node { node, .. } => {
            let other = find_board_node(board, node)?;
            (scope_token(board, moves, other) != *token).then(|| node.clone())
        }
        PinOwner::FunctionBoundary { layer, .. } => {
            (ScopeToken::Live(Some(layer.clone())) != *token).then(|| layer.clone())
        }
        PinOwner::Bridge { layer, .. } => Some(layer.clone()),
    }
}

fn cut_between(moved: &str, pin: &Pin, owner: String, neighbor: &str, incoming: bool) -> CutEdge {
    if incoming {
        CutEdge {
            from_owner: owner,
            from_pin: neighbor.to_string(),
            to_owner: moved.to_string(),
            to_pin: pin.id.clone(),
        }
    } else {
        CutEdge {
            from_owner: moved.to_string(),
            from_pin: pin.id.clone(),
            to_owner: owner,
            to_pin: neighbor.to_string(),
        }
    }
}

fn stranded_ends(
    index: &PinIndex,
    moves: &BTreeMap<String, MoveDestination>,
    moved: &str,
    neighbor: &str,
    incoming: bool,
) -> Vec<StrandedExec> {
    index
        .real_endpoints(neighbor, incoming)
        .into_iter()
        .filter_map(|pin_id| {
            let (owner, pin_name) = match index.owners.get(&pin_id)? {
                PinOwner::Node { node, name } if !moves.contains_key(node) => {
                    (node.clone(), name.clone())
                }
                PinOwner::FunctionBoundary { layer, name } => (layer.clone(), name.clone()),
                _ => return None,
            };
            Some(StrandedExec {
                owner,
                pin_id,
                pin_name,
                lost_successor: incoming,
                moved: moved.to_string(),
            })
        })
        .collect()
}

fn input_is_fed(board: &Board, pin_id: &str) -> bool {
    all_board_nodes(board)
        .into_iter()
        .find_map(|node| node.pins.get(pin_id))
        .or_else(|| {
            board
                .layers
                .values()
                .find_map(|layer| layer.pins.get(pin_id))
        })
        .is_some_and(|pin| !pin.depends_on.is_empty())
}

fn edit_pin(board: &mut Board, pin_id: &str, edit: impl FnOnce(&mut Pin)) {
    let layers = board.layers.values_mut().flat_map(|layer| {
        layer.pins.get_mut(pin_id).into_iter().chain(
            layer
                .nodes
                .values_mut()
                .filter_map(|node| node.pins.get_mut(pin_id)),
        )
    });
    let pin = board
        .nodes
        .values_mut()
        .filter_map(|node| node.pins.get_mut(pin_id))
        .chain(layers)
        .next();
    if let Some(pin) = pin {
        edit(pin);
    }
}

fn entity_label(board: &Board, id: &str) -> String {
    find_board_node(board, id)
        .map(|node| format!("`{}` ({id})", node.friendly_name))
        .or_else(|| {
            board
                .layers
                .get(id)
                .map(|layer| format!("the boundary of function `{}`", layer.name))
        })
        .unwrap_or_else(|| format!("`{id}`"))
}
