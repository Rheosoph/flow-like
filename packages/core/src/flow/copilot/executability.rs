//! Static executability lint for reconciled FlowScript command batches.
//!
//! Reconcile can produce a batch that is structurally clean (every call resolved, every pin
//! known) yet still yield a board that cannot actually run: an event whose exec chain was lost,
//! an impure node whose required input ends up with neither a connection nor a value, a variable
//! that is read but never written. This module lints the *projected result* of applying the batch
//! to the base board.
//!
//! The batch is deliberately NOT materialized through `flow::ast::apply`: the apply planner is
//! async, needs `Arc<FlowLikeState>` plus instantiated catalog `NodeLogic` for `on_update`, and
//! mutates a real board with rollback — none of which the synchronous evaluate/check path owns.
//! Instead a lightweight name-level projection is built from the base board, the catalog
//! metadata, and the command batch. This is the cheaper sound option because every resolver gap
//! degrades to silence, never to a finding.
//!
//! False-positive discipline:
//! - an unresolvable node reference aborts the whole lint (empty report);
//! - an unresolvable pin reference marks its node opaque and exempt from every check;
//! - edges with unknown pins still conduct execution reachability, so a resolution gap can never
//!   manufacture an "unreachable" finding;
//! - blocking checks only ever fire for entities created by this batch.

use std::collections::{HashMap, HashSet};

use flow_like_ast::to_camel_case;

use super::types::{BoardCommand, NodeMetadata, PlaceholderPinDef};
use crate::flow::ast::{
    FlowScriptDiagnostic, FlowScriptDiagnosticCode, FlowScriptDiagnosticFix,
    FlowScriptDiagnosticPhase, parse_pin_occurrence_ref,
};
use crate::flow::board::{Board, Layer, LayerType};
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinType};
use crate::flow::variable::VariableType;

const MAX_FINDINGS_PER_CHECK: usize = 10;
const DEFAULT_OUTPUT_PIN_ALIASES: &[&str] = &["result", "value", "output", "out"];
const VARIABLE_GET_NODE_TYPE: &str = "variable_get";
const VARIABLE_SET_NODE_TYPE: &str = "variable_set";
const VARIABLE_REF_PIN: &str = "var_ref";
/// Node types a generated board must never contain, with the repair that replaces them. Writing
/// into a surface's raw data model does not drive the rendered page: elements read their own
/// state, and package widget instances read typed contract inputs. Both are unreachable from a
/// `$.data.*` path write, so the node is rejected instead of merely discouraged.
const PROHIBITED_NODE_TYPES: &[(&str, &str)] = &[(
    "a2ui_data_update",
    "Replace it with the setter for the target element (`a2uiSetElementText`, `a2uiSetElementValue`, `a2uiSetMarkdownContent`, `a2uiSetBadgeContent`, `a2uiSetProgress`, `a2uiSetSelectValue`, `a2uiSetSliderValue`, `a2uiWriteCsvToTable`, `a2uiUpdateTable`, `a2uiPushCsvToChart`). For package widgets, pass the values as `dyn*` inputs of `a2uiInstantiateWidget` and push the instance into its container with `a2uiPushChild`/`a2uiPushToContainer`, or patch a live instance with `a2uiWidgetUpdateInputs`.",
)];

#[derive(Debug, Default)]
pub(crate) struct ExecutabilityReport {
    pub(crate) blocking: Vec<FlowScriptDiagnostic>,
    pub(crate) review_notes: Vec<FlowScriptDiagnostic>,
}

/// Lint the graph produced by applying `commands` to `board`. Returns an empty report whenever
/// the batch cannot be projected soundly.
pub(crate) fn lint_flowscript_executability(
    board: &Board,
    catalog: &[NodeMetadata],
    commands: &[BoardCommand],
) -> ExecutabilityReport {
    let Some(graph) = project_graph(board, catalog, commands) else {
        return ExecutabilityReport::default();
    };
    let reachable = exec_reachable(&graph);

    let mut report = ExecutabilityReport::default();
    push_capped(
        &mut report.review_notes,
        check_prohibited_nodes(&graph),
        "prohibited node",
    );
    push_capped(
        &mut report.blocking,
        check_missing_required_inputs(&graph, &reachable),
        "missing required input",
    );
    push_capped(
        &mut report.blocking,
        check_dead_event_entries(&graph, &reachable),
        "dead event entry",
    );
    push_capped(
        &mut report.review_notes,
        check_unreachable_impure_nodes(&graph, &reachable),
        "unreachable impure node",
    );
    push_capped(
        &mut report.review_notes,
        check_unset_variable_reads(&graph),
        "unset variable read",
    );
    push_capped(
        &mut report.review_notes,
        check_function_exec_tails(&graph),
        "unterminated function body",
    );
    let (unfed_new, unfed_existing) = check_unfed_function_returns(&graph);
    push_capped(&mut report.blocking, unfed_new, "unfed function return");
    push_capped(
        &mut report.review_notes,
        unfed_existing,
        "unfed function return",
    );
    report
}

#[derive(Debug)]
struct GraphPin {
    live_id: Option<String>,
    name: String,
    friendly_name: String,
    input: bool,
    exec: bool,
    has_value: bool,
}

#[derive(Debug)]
struct GraphNode {
    key: String,
    node_type: String,
    display: String,
    is_new: bool,
    is_layer: bool,
    is_function_layer: bool,
    opaque: bool,
    removed: bool,
    pins: Vec<GraphPin>,
    /// Ordinals into `pins` that are required per catalog metadata and carry no usable default.
    required_inputs: Vec<usize>,
}

impl GraphNode {
    fn exec_input_count(&self) -> usize {
        self.pins.iter().filter(|pin| pin.exec && pin.input).count()
    }

    fn exec_output_count(&self) -> usize {
        self.pins
            .iter()
            .filter(|pin| pin.exec && !pin.input)
            .count()
    }

    fn is_impure_node(&self) -> bool {
        !self.is_layer && self.exec_input_count() > 0
    }

    fn is_entry_node(&self) -> bool {
        !self.is_layer && self.exec_output_count() > 0 && self.exec_input_count() == 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Endpoint {
    node: usize,
    pin: Option<usize>,
}

#[derive(Debug)]
struct ProjectedVariable {
    name: String,
    has_default: bool,
    /// Secret, exposed, or runtime-configured variables receive values outside the graph.
    external: bool,
}

#[derive(Debug, Default)]
struct ProjectedGraph {
    nodes: Vec<GraphNode>,
    edges: Vec<(Endpoint, Endpoint)>,
    variables: HashMap<String, ProjectedVariable>,
    variable_reads: Vec<(usize, String)>,
    variable_writes: HashSet<String>,
}

impl ProjectedGraph {
    fn input_is_satisfied(&self, node: usize, pin: usize) -> bool {
        self.nodes[node].pins[pin].has_value
            || self
                .edges
                .iter()
                .any(|(_, to)| to.node == node && (to.pin == Some(pin) || to.pin.is_none()))
    }

    fn has_outgoing_exec_edge(&self, node: usize) -> bool {
        self.edges.iter().any(|(from, _)| {
            from.node == node
                && from
                    .pin
                    .map(|pin| self.nodes[node].pins[pin].exec)
                    .unwrap_or(true)
        })
    }
}

fn pin_lookup_keys(value: &str) -> HashSet<String> {
    let camel = to_camel_case(value);
    [
        value.to_string(),
        value.to_lowercase(),
        camel.to_lowercase(),
        camel,
    ]
    .into_iter()
    .collect()
}

fn graph_pin_name_matches(pin: &GraphPin, requested: &str) -> bool {
    let requested = pin_lookup_keys(requested);
    pin_lookup_keys(&pin.name)
        .iter()
        .chain(pin_lookup_keys(&pin.friendly_name).iter())
        .any(|key| requested.contains(key))
}

/// Mirror of the apply planner's pin resolution, over projected pins. `want_input` is `None` for
/// layer boundaries, whose directions are intentionally inverted from inner-body edges.
fn resolve_graph_pin(node: &GraphNode, pin_ref: &str, want_input: Option<bool>) -> Option<usize> {
    let direction_ok = |pin: &GraphPin| want_input.is_none_or(|input| pin.input == input);
    if let Some(found) = node
        .pins
        .iter()
        .position(|pin| pin.live_id.as_deref() == Some(pin_ref) && direction_ok(pin))
    {
        return Some(found);
    }
    if let Some((name, occurrence)) = parse_pin_occurrence_ref(pin_ref) {
        return node
            .pins
            .iter()
            .enumerate()
            .filter(|(_, pin)| direction_ok(pin) && graph_pin_name_matches(pin, name))
            .nth(occurrence)
            .map(|(index, _)| index);
    }
    if let Some(found) = node
        .pins
        .iter()
        .position(|pin| direction_ok(pin) && graph_pin_name_matches(pin, pin_ref))
    {
        return Some(found);
    }
    if want_input != Some(true)
        && DEFAULT_OUTPUT_PIN_ALIASES
            .iter()
            .any(|alias| alias.eq_ignore_ascii_case(pin_ref))
    {
        let outputs = node
            .pins
            .iter()
            .enumerate()
            .filter(|(_, pin)| !pin.input && !pin.exec)
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        return match outputs.as_slice() {
            [single] => Some(*single),
            many => many.iter().copied().find(|index| {
                DEFAULT_OUTPUT_PIN_ALIASES
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(&node.pins[*index].name))
            }),
        };
    }
    None
}

fn json_bytes_carry_value(bytes: Option<&[u8]>) -> bool {
    bytes
        .and_then(|bytes| flow_like_types::json::from_slice::<flow_like_types::Value>(bytes).ok())
        .is_some_and(|value| !value.is_null())
}

fn metadata_default_carries_value(default_value: Option<&str>) -> bool {
    default_value.is_some_and(|value| {
        let trimmed = value.trim();
        !trimmed.is_empty() && trimmed != "null"
    })
}

fn live_pin_to_graph_pin(pin: &Pin) -> GraphPin {
    GraphPin {
        live_id: Some(pin.id.clone()),
        name: pin.name.clone(),
        friendly_name: pin.friendly_name.clone(),
        input: pin.pin_type == PinType::Input,
        exec: pin.data_type == VariableType::Execution,
        has_value: json_bytes_carry_value(pin.default_value.as_deref()),
    }
}

fn placeholder_pin_to_graph_pin(pin: &PlaceholderPinDef) -> GraphPin {
    GraphPin {
        live_id: None,
        name: pin.name.clone(),
        friendly_name: pin.friendly_name.clone(),
        input: pin.pin_type.eq_ignore_ascii_case("input"),
        exec: pin.data_type == "Execution",
        has_value: false,
    }
}

fn sorted_live_pins(pins: &HashMap<String, Pin>) -> Vec<&Pin> {
    let mut pins = pins.values().collect::<Vec<_>>();
    pins.sort_by_key(|pin| (pin.index, pin.id.clone()));
    pins
}

fn live_node_display(node: &Node) -> String {
    if node.friendly_name.trim().is_empty() {
        node.name.clone()
    } else {
        node.friendly_name.clone()
    }
}

fn node_variable_ref(node: &Node) -> Option<String> {
    let pin = sorted_live_pins(&node.pins).into_iter().find(|pin| {
        pin.pin_type == PinType::Input && pin_lookup_keys(&pin.name).contains(VARIABLE_REF_PIN)
    })?;
    match flow_like_types::json::from_slice::<flow_like_types::Value>(pin.default_value.as_deref()?)
        .ok()?
    {
        flow_like_types::Value::String(id) => Some(id),
        _ => None,
    }
}

/// Map catalog `required_inputs` onto concrete input pin ordinals, mirroring the reconciler's
/// claiming order. A requirement that resolves to a defaulted pin is dropped: the catalog says
/// it can be defaulted, so it cannot be a static blocker.
fn required_input_ordinals(meta: &NodeMetadata, pins: &[GraphPin]) -> Vec<usize> {
    let mut claimed = HashSet::new();
    let mut ordinals = Vec::new();
    for required in &meta.required_inputs {
        let matching = pins
            .iter()
            .enumerate()
            .filter(|(index, pin)| {
                !claimed.contains(index)
                    && pin.input
                    && !pin.exec
                    && graph_pin_name_matches(pin, required)
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        let Some(ordinal) = matching
            .iter()
            .copied()
            .find(|index| !pins[*index].has_value)
            .or_else(|| matching.first().copied())
        else {
            continue;
        };
        claimed.insert(ordinal);
        if pins[ordinal].has_value {
            continue;
        }
        ordinals.push(ordinal);
    }
    ordinals
}

struct GraphBuilder<'a> {
    catalog: &'a [NodeMetadata],
    graph: ProjectedGraph,
    by_key: HashMap<String, usize>,
    /// Friendly-name/ref aliases for existing entities; ambiguous aliases are dropped and any
    /// later use of one aborts the lint.
    aliases: HashMap<String, usize>,
    ambiguous_aliases: HashSet<String>,
    synthetic_keys: usize,
}

impl<'a> GraphBuilder<'a> {
    fn new(catalog: &'a [NodeMetadata]) -> Self {
        Self {
            catalog,
            graph: ProjectedGraph::default(),
            by_key: HashMap::new(),
            aliases: HashMap::new(),
            ambiguous_aliases: HashSet::new(),
            synthetic_keys: 0,
        }
    }

    fn register_alias(&mut self, alias: &str, index: usize) {
        let alias = alias.trim();
        if alias.is_empty() || self.ambiguous_aliases.contains(alias) {
            return;
        }
        match self.aliases.get(alias) {
            Some(existing) if *existing != index => {
                self.aliases.remove(alias);
                self.ambiguous_aliases.insert(alias.to_string());
            }
            Some(_) => {}
            None => {
                self.aliases.insert(alias.to_string(), index);
            }
        }
    }

    fn push_node(&mut self, node: GraphNode) -> usize {
        let index = self.graph.nodes.len();
        self.by_key.insert(node.key.clone(), index);
        self.graph.nodes.push(node);
        index
    }

    fn synthetic_key(&mut self) -> String {
        self.synthetic_keys += 1;
        format!("__executability_unref_{}", self.synthetic_keys)
    }

    fn index_existing_node(&mut self, node: &Node) {
        let pins = sorted_live_pins(&node.pins)
            .into_iter()
            .map(live_pin_to_graph_pin)
            .collect::<Vec<_>>();
        let index = self.push_node(GraphNode {
            key: node.id.clone(),
            node_type: node.name.clone(),
            display: live_node_display(node),
            is_new: false,
            is_layer: false,
            is_function_layer: false,
            opaque: false,
            removed: false,
            pins,
            required_inputs: Vec::new(),
        });
        self.register_alias(&node.friendly_name, index);
        if node.name == VARIABLE_GET_NODE_TYPE {
            if let Some(variable_id) = node_variable_ref(node) {
                self.graph.variable_reads.push((index, variable_id));
            }
        } else if node.name == VARIABLE_SET_NODE_TYPE
            && let Some(variable_id) = node_variable_ref(node)
        {
            self.graph.variable_writes.insert(variable_id);
        }
    }

    fn index_existing_layer(&mut self, layer: &Layer) {
        let pins = sorted_live_pins(&layer.pins)
            .into_iter()
            .map(live_pin_to_graph_pin)
            .collect::<Vec<_>>();
        let index = self.push_node(GraphNode {
            key: layer.id.clone(),
            node_type: "__layer".to_string(),
            display: layer.name.clone(),
            is_new: false,
            is_layer: true,
            is_function_layer: matches!(layer.r#type, LayerType::Function),
            opaque: false,
            removed: false,
            pins,
            required_inputs: Vec::new(),
        });
        self.register_alias(&layer.name, index);
    }

    fn index_board(&mut self, board: &Board) {
        for node in board.nodes.values() {
            self.index_existing_node(node);
        }
        for layer in board.layers.values() {
            for node in layer.nodes.values() {
                self.index_existing_node(node);
            }
        }
        for layer in board.layers.values() {
            self.index_existing_layer(layer);
        }
        for (name, node_id) in &board.refs {
            if let Some(index) = self.by_key.get(node_id).copied() {
                self.register_alias(name, index);
            }
        }
        for variable in board.variables.values().chain(
            board
                .layers
                .values()
                .flat_map(|layer| layer.variables.values()),
        ) {
            self.graph.variables.insert(
                variable.id.clone(),
                ProjectedVariable {
                    name: variable.name.clone(),
                    has_default: json_bytes_carry_value(variable.default_value.as_deref()),
                    external: variable.secret || variable.exposed || variable.runtime_configured,
                },
            );
        }

        let mut pin_owner = HashMap::new();
        for (index, node) in self.graph.nodes.iter().enumerate() {
            for (ordinal, pin) in node.pins.iter().enumerate() {
                if let Some(live_id) = &pin.live_id {
                    pin_owner.insert(
                        live_id.clone(),
                        Endpoint {
                            node: index,
                            pin: Some(ordinal),
                        },
                    );
                }
            }
        }
        let mut seen = HashSet::new();
        let mut record = |from: Endpoint, to: Endpoint, edges: &mut Vec<(Endpoint, Endpoint)>| {
            if seen.insert((from, to)) {
                edges.push((from, to));
            }
        };
        let all_pins = board
            .nodes
            .values()
            .flat_map(|node| node.pins.values())
            .chain(
                board
                    .layers
                    .values()
                    .flat_map(|layer| layer.nodes.values())
                    .flat_map(|node| node.pins.values()),
            )
            .chain(board.layers.values().flat_map(|layer| layer.pins.values()));
        // `connected_to`/`depends_on` are symmetric on healthy boards; scanning both directions
        // tolerates half-written legacy data without inventing edges (both sides must exist).
        for pin in all_pins {
            let Some(this) = pin_owner.get(&pin.id).copied() else {
                continue;
            };
            for target in &pin.connected_to {
                if let Some(other) = pin_owner.get(target).copied() {
                    record(this, other, &mut self.graph.edges);
                }
            }
            for source in &pin.depends_on {
                if let Some(other) = pin_owner.get(source).copied() {
                    record(other, this, &mut self.graph.edges);
                }
            }
        }
    }

    fn resolve_metadata(&self, node_type: &str) -> Option<&'a NodeMetadata> {
        let mut matches = self.catalog.iter().filter(|meta| meta.name == node_type);
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first)
    }

    fn add_new_node(
        &mut self,
        node_type: &str,
        ref_id: Option<&str>,
        friendly_name: Option<&str>,
        additional_pins: Option<&[PlaceholderPinDef]>,
    ) {
        let key = ref_id
            .map(str::to_string)
            .unwrap_or_else(|| self.synthetic_key());
        let meta = self.resolve_metadata(node_type);
        let mut pins = Vec::new();
        if let Some(meta) = meta {
            for input in &meta.inputs {
                pins.push(GraphPin {
                    live_id: None,
                    name: input.name.clone(),
                    friendly_name: input.friendly_name.clone(),
                    input: true,
                    exec: input.data_type == "Execution",
                    has_value: metadata_default_carries_value(input.default_value.as_deref()),
                });
            }
            for output in &meta.outputs {
                pins.push(GraphPin {
                    live_id: None,
                    name: output.name.clone(),
                    friendly_name: output.friendly_name.clone(),
                    input: false,
                    exec: output.data_type == "Execution",
                    has_value: false,
                });
            }
        }
        for pin in additional_pins.unwrap_or_default() {
            pins.push(placeholder_pin_to_graph_pin(pin));
        }
        let required_inputs = meta
            .map(|meta| required_input_ordinals(meta, &pins))
            .unwrap_or_default();
        let display = friendly_name
            .filter(|name| !name.trim().is_empty())
            .map(str::to_string)
            .or_else(|| meta.map(|meta| meta.friendly_name.clone()))
            .unwrap_or_else(|| node_type.to_string());
        self.push_node(GraphNode {
            key,
            node_type: node_type.to_string(),
            display,
            is_new: true,
            is_layer: false,
            is_function_layer: false,
            // Unknown catalog shape: exempt from every check, still usable for edges.
            opaque: meta.is_none(),
            removed: false,
            pins,
            required_inputs,
        });
    }

    fn add_new_layer(
        &mut self,
        name: &str,
        ref_id: Option<&str>,
        layer_type: Option<&str>,
        pins: Option<&[PlaceholderPinDef]>,
    ) {
        let key = ref_id
            .map(str::to_string)
            .unwrap_or_else(|| self.synthetic_key());
        self.push_node(GraphNode {
            key,
            node_type: "__layer".to_string(),
            display: name.to_string(),
            is_new: true,
            is_layer: true,
            is_function_layer: layer_type
                .is_some_and(|value| value.eq_ignore_ascii_case("function")),
            opaque: false,
            removed: false,
            pins: pins
                .unwrap_or_default()
                .iter()
                .map(placeholder_pin_to_graph_pin)
                .collect(),
            required_inputs: Vec::new(),
        });
    }

    fn add_placeholder(
        &mut self,
        name: &str,
        ref_id: Option<&str>,
        pins: Option<&[PlaceholderPinDef]>,
    ) {
        let key = ref_id
            .map(str::to_string)
            .unwrap_or_else(|| self.synthetic_key());
        self.push_node(GraphNode {
            key,
            node_type: "__placeholder".to_string(),
            display: name.to_string(),
            is_new: true,
            is_layer: false,
            is_function_layer: false,
            // Placeholders are sketches, never executable claims; exempt from all checks.
            opaque: true,
            removed: false,
            pins: pins
                .unwrap_or_default()
                .iter()
                .map(placeholder_pin_to_graph_pin)
                .collect(),
            required_inputs: Vec::new(),
        });
    }

    fn resolve_node_key(&self, node_ref: &str) -> Option<usize> {
        if let Some(index) = self.by_key.get(node_ref) {
            return Some(*index);
        }
        if self.ambiguous_aliases.contains(node_ref) {
            return None;
        }
        self.aliases.get(node_ref).copied()
    }

    /// Resolve one side of a connect/disconnect. `Ok(None)` in the pin slot means the node was
    /// found but the pin was not; the node has been marked opaque. `Err(())` aborts the lint.
    fn resolve_endpoint(
        &mut self,
        node_ref: &str,
        pin_ref: &str,
        want_input: bool,
    ) -> Result<Endpoint, ()> {
        let Some(index) = self.resolve_node_key(node_ref) else {
            return Err(());
        };
        let node = &self.graph.nodes[index];
        let direction = if node.is_layer {
            None
        } else {
            Some(want_input)
        };
        match resolve_graph_pin(node, pin_ref, direction) {
            Some(pin) => Ok(Endpoint {
                node: index,
                pin: Some(pin),
            }),
            None => {
                self.graph.nodes[index].opaque = true;
                Ok(Endpoint {
                    node: index,
                    pin: None,
                })
            }
        }
    }

    fn upsert_variable(&mut self, id: String, name: &str, has_default: bool, external: bool) {
        self.graph.variables.insert(
            id,
            ProjectedVariable {
                name: name.to_string(),
                has_default,
                external,
            },
        );
    }
}

fn command_value_carries_data(value: &flow_like_types::Value) -> bool {
    !value.is_null()
}

fn project_graph(
    board: &Board,
    catalog: &[NodeMetadata],
    commands: &[BoardCommand],
) -> Option<ProjectedGraph> {
    let mut builder = GraphBuilder::new(catalog);
    builder.index_board(board);

    // Pass 1: entities, so later references resolve independent of command ordering.
    for command in commands {
        match command {
            BoardCommand::AddNode {
                node_type,
                ref_id,
                friendly_name,
                additional_pins,
                ..
            } => builder.add_new_node(
                node_type,
                ref_id.as_deref(),
                friendly_name.as_deref(),
                additional_pins.as_deref(),
            ),
            BoardCommand::AddPlaceholder {
                name, ref_id, pins, ..
            } => builder.add_placeholder(name, ref_id.as_deref(), pins.as_deref()),
            BoardCommand::CreateLayer {
                name,
                ref_id,
                layer_type,
                pins,
                ..
            } => builder.add_new_layer(
                name,
                ref_id.as_deref(),
                layer_type.as_deref(),
                pins.as_deref(),
            ),
            BoardCommand::CreateVariable {
                variable_id,
                name,
                default_value,
                exposed,
                secret,
                runtime_configured,
                ..
            } => {
                let id = variable_id
                    .clone()
                    .unwrap_or_else(|| builder.synthetic_key());
                builder.upsert_variable(
                    id,
                    name,
                    default_value
                        .as_ref()
                        .is_some_and(command_value_carries_data),
                    exposed.unwrap_or(false)
                        || secret.unwrap_or(false)
                        || runtime_configured.unwrap_or(false),
                );
            }
            _ => {}
        }
    }

    // Pass 2: wiring and mutations.
    for command in commands {
        match command {
            BoardCommand::ConnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                let from = builder.resolve_endpoint(from_node, from_pin, false).ok()?;
                let to = builder.resolve_endpoint(to_node, to_pin, true).ok()?;
                builder.graph.edges.push((from, to));
            }
            BoardCommand::DisconnectPins {
                from_node,
                from_pin,
                to_node,
                to_pin,
                ..
            } => {
                let from = builder.resolve_endpoint(from_node, from_pin, false).ok()?;
                let to = builder.resolve_endpoint(to_node, to_pin, true).ok()?;
                // An unresolved side already marked its node opaque; keeping the stale edge is
                // the conservative direction (it can only suppress findings).
                if from.pin.is_some() && to.pin.is_some() {
                    builder
                        .graph
                        .edges
                        .retain(|(edge_from, edge_to)| !(*edge_from == from && *edge_to == to));
                }
            }
            BoardCommand::UpdateNodePin {
                node_id,
                pin_id,
                value,
                ..
            } => {
                let Some(index) = builder.resolve_node_key(node_id) else {
                    return None;
                };
                let node = &builder.graph.nodes[index];
                let direction = if node.is_layer { None } else { Some(true) };
                match resolve_graph_pin(node, pin_id, direction) {
                    Some(pin) => {
                        let node_type = node.node_type.clone();
                        let is_var_ref =
                            pin_lookup_keys(&node.pins[pin].name).contains(VARIABLE_REF_PIN);
                        builder.graph.nodes[index].pins[pin].has_value = true;
                        if is_var_ref && let flow_like_types::Value::String(variable_id) = value {
                            if node_type == VARIABLE_GET_NODE_TYPE {
                                builder
                                    .graph
                                    .variable_reads
                                    .push((index, variable_id.clone()));
                            } else if node_type == VARIABLE_SET_NODE_TYPE {
                                builder.graph.variable_writes.insert(variable_id.clone());
                            }
                        }
                    }
                    None => builder.graph.nodes[index].opaque = true,
                }
            }
            BoardCommand::RenameNode {
                node_id,
                friendly_name,
                ..
            } => {
                let Some(index) = builder.resolve_node_key(node_id) else {
                    return None;
                };
                builder.graph.nodes[index].display = friendly_name.clone();
                builder.register_alias(friendly_name, index);
            }
            BoardCommand::RemoveNode { node_id, .. }
            | BoardCommand::RemoveLayer {
                layer_id: node_id, ..
            } => {
                let Some(index) = builder.resolve_node_key(node_id) else {
                    return None;
                };
                builder.graph.nodes[index].removed = true;
                builder
                    .graph
                    .edges
                    .retain(|(from, to)| from.node != index && to.node != index);
            }
            BoardCommand::UpdateVariable {
                variable_id,
                default_value,
                clear_default_value,
                exposed,
                secret,
                runtime_configured,
                value,
                ..
            } => {
                if let Some(variable) = builder.graph.variables.get_mut(variable_id) {
                    if *clear_default_value {
                        variable.has_default = false;
                    } else if default_value
                        .as_ref()
                        .or(value.as_ref())
                        .is_some_and(command_value_carries_data)
                    {
                        variable.has_default = true;
                    }
                    if exposed == &Some(true)
                        || secret == &Some(true)
                        || runtime_configured == &Some(true)
                    {
                        variable.external = true;
                    }
                }
            }
            BoardCommand::RemoveVariable { variable_id, .. } => {
                builder.graph.variables.remove(variable_id);
            }
            // Position, grouping membership, comments, and fn-ref bookkeeping cannot change
            // static executability.
            BoardCommand::AddNode { .. }
            | BoardCommand::AddPlaceholder { .. }
            | BoardCommand::CreateLayer { .. }
            | BoardCommand::CreateVariable { .. }
            | BoardCommand::MoveNode { .. }
            | BoardCommand::UpdateLayerCache { .. }
            | BoardCommand::SetNodeFunctionRefs { .. }
            | BoardCommand::AddComment { .. }
            | BoardCommand::RemoveComment { .. } => {}
        }
    }

    Some(builder.graph)
}

/// Node-level execution reachability. Seeds are event-shaped entries (exec outputs, no exec
/// inputs), function layers (their bodies run when called), and opaque non-layer nodes (unknown
/// shape must never make downstream work look unreachable).
fn exec_reachable(graph: &ProjectedGraph) -> Vec<bool> {
    let mut reachable = vec![false; graph.nodes.len()];
    let mut queue = Vec::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        if node.removed {
            continue;
        }
        let seed = if node.is_layer {
            node.is_function_layer
        } else {
            node.opaque || node.is_entry_node()
        };
        if seed {
            reachable[index] = true;
            queue.push(index);
        }
    }
    while let Some(current) = queue.pop() {
        for (from, to) in &graph.edges {
            if from.node != current || reachable[to.node] || graph.nodes[to.node].removed {
                continue;
            }
            let conducts = match (from.pin, to.pin) {
                (Some(from_pin), Some(to_pin)) => {
                    graph.nodes[from.node].pins[from_pin].exec
                        && graph.nodes[to.node].pins[to_pin].exec
                }
                // Unknown pins conservatively conduct execution.
                _ => true,
            };
            if conducts {
                reachable[to.node] = true;
                queue.push(to.node);
            }
        }
    }
    reachable
}

/// Review note: a node of a prohibited type, whether this batch adds it or the base board already
/// carries it. This never blocks — a rejected batch would strand the whole edit over a node the
/// model can simply rewrite — so the finding rides back as an actionable note and the caller turns
/// it into an explicit instruction to write a corrected revision. Unlike the other checks this
/// needs neither reachability nor pin resolution, so an opaque node is still reported.
fn check_prohibited_nodes(graph: &ProjectedGraph) -> Vec<FlowScriptDiagnostic> {
    graph
        .nodes
        .iter()
        .filter(|node| !node.removed && !node.is_layer)
        .filter_map(|node| {
            let (_, repair) = PROHIBITED_NODE_TYPES
                .iter()
                .find(|(node_type, _)| *node_type == node.node_type)?;
            let origin = if node.is_new {
                "This batch adds it"
            } else {
                "This board already carries it"
            };
            Some(executability_diagnostic(
                FlowScriptDiagnosticCode::FsProhibitedNode,
                format!(
                    "`{}` (`{}`) does not change what the page renders: a `$.data.*` write is observed by neither elements nor widget instances. {origin}; replace it.",
                    node.display, node.node_type
                ),
                Some(node.display.clone()),
                None,
                repair,
            ))
        })
        .collect()
}

/// BLOCKING: a required input (per catalog metadata, no default anywhere) on a reachable, newly
/// added impure node with neither a connection nor a written value cannot execute.
fn check_missing_required_inputs(
    graph: &ProjectedGraph,
    reachable: &[bool],
) -> Vec<FlowScriptDiagnostic> {
    let mut findings = Vec::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        if !node.is_new
            || node.opaque
            || node.removed
            || node.is_layer
            || !node.is_impure_node()
            || !reachable[index]
            // Nodes whose `on_update` mints pins have no statically knowable requirement
            // surface, so a required-input finding on them is always noise.
            || crate::flow::node::mints_pins_on_update(&node.node_type)
        {
            continue;
        }
        for ordinal in &node.required_inputs {
            if graph.input_is_satisfied(index, *ordinal) {
                continue;
            }
            let pin_name = to_camel_case(&graph.nodes[index].pins[*ordinal].name);
            findings.push(executability_diagnostic(
                FlowScriptDiagnosticCode::FsUnresolvedArgument,
                format!(
                    "Executability: required input `{pin_name}` of `{}` (`{}`) has no connection and no value, so this node cannot run.",
                    node.display, node.node_type
                ),
                Some(node.display.clone()),
                Some(pin_name),
                "Connect a value to this required input or set an explicit literal for it.",
            ));
        }
    }
    findings
}

/// BLOCKING: a newly added event entry whose exec output chains to nothing, while the same batch
/// leaves newly added impure work unreachable, registers a workflow that silently does nothing.
/// The unreachable-impure conjunction is what distinguishes a lost exec chain from an event whose
/// authored body is intentionally pure.
fn check_dead_event_entries(
    graph: &ProjectedGraph,
    reachable: &[bool],
) -> Vec<FlowScriptDiagnostic> {
    let stranded_new_impure = graph.nodes.iter().enumerate().any(|(index, node)| {
        node.is_new && !node.opaque && !node.removed && node.is_impure_node() && !reachable[index]
    });
    if !stranded_new_impure {
        return Vec::new();
    }
    graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            node.is_new
                && !node.opaque
                && !node.removed
                && node.is_entry_node()
                && !graph.has_outgoing_exec_edge(*index)
        })
        .map(|(_, node)| {
            executability_diagnostic(
                FlowScriptDiagnosticCode::FsExecutionEntryUnconnected,
                format!(
                    "Executability: event entry `{}` (`{}`) has no outgoing execution connection while other new impure nodes are unreachable; this event would register and then silently do nothing.",
                    node.display, node.node_type
                ),
                Some(node.display.clone()),
                None,
                "Chain the event's exec output to the first impure call of its body.",
            )
        })
        .collect()
}

/// Review note: a newly added impure node no event or function entry can ever execute.
fn check_unreachable_impure_nodes(
    graph: &ProjectedGraph,
    reachable: &[bool],
) -> Vec<FlowScriptDiagnostic> {
    graph
        .nodes
        .iter()
        .enumerate()
        .filter(|(index, node)| {
            node.is_new
                && !node.opaque
                && !node.removed
                && node.is_impure_node()
                && !reachable[*index]
        })
        .map(|(_, node)| {
            executability_diagnostic(
                FlowScriptDiagnosticCode::FsExecutionEntryUnconnected,
                format!(
                    "Executability note: impure node `{}` (`{}`) is not reachable from any event or function entry through execution wiring, so it will never run.",
                    node.display, node.node_type
                ),
                Some(node.display.clone()),
                None,
                "Wire this node into an event body or a called function, or remove it.",
            )
        })
        .collect()
}

/// Review note: a newly added variable read whose variable has no usable default, is not supplied
/// externally (secret/exposed/runtime-configured), and is never written anywhere in the projected
/// graph will always yield an empty value.
fn check_unset_variable_reads(graph: &ProjectedGraph) -> Vec<FlowScriptDiagnostic> {
    let mut flagged = HashSet::new();
    let mut findings = Vec::new();
    for (node_index, variable_id) in &graph.variable_reads {
        let reader = &graph.nodes[*node_index];
        if !reader.is_new || reader.removed || !flagged.insert(variable_id.clone()) {
            continue;
        }
        let Some(variable) = graph.variables.get(variable_id) else {
            continue;
        };
        if variable.has_default || variable.external || graph.variable_writes.contains(variable_id)
        {
            continue;
        }
        findings.push(executability_diagnostic(
            FlowScriptDiagnosticCode::FsVariableUnresolved,
            format!(
                "Executability note: variable `{}` is read but has no default value and is never set anywhere on the resulting board.",
                variable.name
            ),
            Some(variable.name.clone()),
            None,
            "Give the variable a default value or set it before reading it.",
        ));
    }
    findings
}

/// Review note: a newly created impure function layer whose boundary exec output receives no
/// connection has a body that can never terminate normally.
fn check_function_exec_tails(graph: &ProjectedGraph) -> Vec<FlowScriptDiagnostic> {
    let mut findings = Vec::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        if !node.is_new || !node.is_function_layer || node.removed || node.opaque {
            continue;
        }
        let has_exec_in = node.pins.iter().any(|pin| pin.exec && pin.input);
        let Some(exec_out) = node.pins.iter().position(|pin| pin.exec && !pin.input) else {
            continue;
        };
        if !has_exec_in {
            continue;
        }
        // Any unknown-pin edge touching this layer means we cannot see its boundary soundly.
        let touched_unknown = graph.edges.iter().any(|(from, to)| {
            (from.node == index && from.pin.is_none()) || (to.node == index && to.pin.is_none())
        });
        if touched_unknown {
            continue;
        }
        let terminated = graph
            .edges
            .iter()
            .any(|(_, to)| to.node == index && to.pin == Some(exec_out));
        if !terminated {
            findings.push(executability_diagnostic(
                FlowScriptDiagnosticCode::FsHelperExecutionTailUnconnected,
                format!(
                    "Executability note: function `{}` never reaches its boundary exec output, so its body cannot terminate normally.",
                    node.display
                ),
                Some(node.display.clone()),
                None,
                "Chain the last impure call of the function body to the function's exec output.",
            ));
        }
    }
    findings
}

/// A function layer's declared non-exec boundary return pin with neither an incoming edge from
/// the body nor a stored value returns nothing to callers — the exact silently-broken shape of a
/// function whose `return` statement was never wired. BLOCKING for layers created by this batch;
/// a review note for pre-existing layers so a repair loop keeps seeing the defect without being
/// unable to commit unrelated work.
fn check_unfed_function_returns(
    graph: &ProjectedGraph,
) -> (Vec<FlowScriptDiagnostic>, Vec<FlowScriptDiagnostic>) {
    let mut new_findings = Vec::new();
    let mut existing_findings = Vec::new();
    for (index, node) in graph.nodes.iter().enumerate() {
        if !node.is_function_layer || node.removed || node.opaque {
            continue;
        }
        // Any unknown-pin edge touching this layer means we cannot see its boundary soundly.
        let touched_unknown = graph.edges.iter().any(|(from, to)| {
            (from.node == index && from.pin.is_none()) || (to.node == index && to.pin.is_none())
        });
        if touched_unknown {
            continue;
        }
        for (ordinal, pin) in node.pins.iter().enumerate() {
            if pin.exec || pin.input || graph.input_is_satisfied(index, ordinal) {
                continue;
            }
            let pin_name = to_camel_case(&pin.name);
            let finding = executability_diagnostic(
                FlowScriptDiagnosticCode::FsFunctionReturnMismatch,
                format!(
                    "Executability{}: declared return pin `{pin_name}` of function `{}` has no incoming connection from the body, so callers receive nothing for it.",
                    if node.is_new { "" } else { " note" },
                    node.display
                ),
                Some(node.display.clone()),
                Some(pin_name),
                "Add a `return` statement (or wire a body output) feeding this declared return pin, or remove the unused return from the function signature.",
            );
            if node.is_new {
                new_findings.push(finding);
            } else {
                existing_findings.push(finding);
            }
        }
    }
    (new_findings, existing_findings)
}

fn push_capped(
    target: &mut Vec<FlowScriptDiagnostic>,
    mut findings: Vec<FlowScriptDiagnostic>,
    check: &str,
) {
    if findings.len() > MAX_FINDINGS_PER_CHECK {
        let truncated = findings.len() - MAX_FINDINGS_PER_CHECK;
        let code = findings[0].code;
        findings.truncate(MAX_FINDINGS_PER_CHECK);
        findings.push(executability_diagnostic(
            code,
            format!("Executability: {truncated} additional {check} finding(s) were truncated."),
            None,
            None,
            "Repair the listed findings first; the remainder will surface on the next check.",
        ));
    }
    target.append(&mut findings);
}

fn executability_diagnostic(
    code: FlowScriptDiagnosticCode,
    message: String,
    scope: Option<String>,
    pin: Option<String>,
    fix_summary: &str,
) -> FlowScriptDiagnostic {
    let id_material = format!(
        "EXECUTABILITY|{}|{}|{}|{message}",
        code.as_str(),
        scope.as_deref().unwrap_or_default(),
        pin.as_deref().unwrap_or_default(),
    );
    let digest = blake3::hash(id_material.as_bytes()).to_hex().to_string();
    FlowScriptDiagnostic {
        id: format!("FSD-{}", &digest[..16]),
        code,
        phase: FlowScriptDiagnosticPhase::Validation,
        message,
        source_span: None,
        spans: Vec::new(),
        additional_sites: 0,
        ast_path: None,
        scope,
        expected: None,
        actual: None,
        declaration: None,
        pin,
        fix: Some(FlowScriptDiagnosticFix {
            summary: fix_summary.to_string(),
            declaration_search: None,
            catalog_declarations: Vec::new(),
            companion_declarations: Vec::new(),
        }),
        caused_by: None,
        occurrences: 1,
        related_messages: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::SystemTime;

    use flow_like_storage::Path;
    use serde_json::json;

    use super::super::ir_tools::{
        CheckFlowScriptArgs, CommitFlowScriptArgs, FlowIrDraftMode, FlowIrDraftStore,
        WriteFlowScriptArgs,
    };
    use super::super::types::PinMetadata;
    use super::*;
    use crate::flow::board::{ExecutionMode, ExecutionStage};
    use crate::flow::execution::LogLevel;
    use crate::flow::pin::ValueType;

    fn pin_meta(name: &str, data_type: &str) -> PinMetadata {
        PinMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: String::new(),
            data_type: data_type.to_string(),
            value_type: "Normal".to_string(),
            default_value: None,
            schema: None,
            is_generic: data_type == "Generic",
            valid_values: None,
            enforce_schema: false,
        }
    }

    fn meta(name: &str, inputs: Vec<PinMetadata>, outputs: Vec<PinMetadata>) -> NodeMetadata {
        NodeMetadata {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: name.to_string(),
            inputs,
            outputs,
            category: None,
            required_inputs: Vec::new(),
            companion_nodes: Vec::new(),
            capability_tags: Vec::new(),
        }
    }

    fn empty_board() -> Board {
        Board {
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
        }
    }

    fn catalog() -> Vec<NodeMetadata> {
        let mut fetch = meta(
            "http_fetch",
            vec![pin_meta("exec_in", "Execution"), pin_meta("url", "String")],
            vec![
                pin_meta("exec_out", "Execution"),
                pin_meta("body", "String"),
            ],
        );
        fetch.required_inputs = vec!["url".to_string()];
        let mut log = meta(
            "log_info",
            vec![
                pin_meta("exec_in", "Execution"),
                pin_meta("message", "String"),
            ],
            vec![pin_meta("exec_out", "Execution")],
        );
        log.required_inputs = vec!["message".to_string()];
        vec![
            meta(
                "events_simple",
                Vec::new(),
                vec![pin_meta("exec_out", "Execution")],
            ),
            fetch,
            log,
            meta(
                "string_concat",
                vec![pin_meta("left", "String"), pin_meta("right", "String")],
                vec![pin_meta("string", "String")],
            ),
            meta(
                "variable_get",
                vec![pin_meta("var_ref", "String")],
                vec![pin_meta("value_ref", "Generic")],
            ),
            meta(
                "variable_set",
                vec![
                    pin_meta("exec_in", "Execution"),
                    pin_meta("var_ref", "String"),
                    pin_meta("value", "Generic"),
                ],
                vec![pin_meta("exec_out", "Execution")],
            ),
            meta(
                "string_format",
                vec![
                    pin_meta("exec_in", "Execution"),
                    pin_meta("format_string", "String"),
                ],
                vec![
                    pin_meta("exec_out", "Execution"),
                    pin_meta("string", "String"),
                ],
            ),
            meta(
                "a2ui_data_update",
                vec![
                    pin_meta("exec_in", "Execution"),
                    pin_meta("surface_id", "String"),
                    pin_meta("path", "String"),
                    pin_meta("value", "Generic"),
                ],
                vec![pin_meta("exec_out", "Execution")],
            ),
        ]
    }

    fn add_node(node_type: &str, ref_id: &str) -> BoardCommand {
        BoardCommand::AddNode {
            node_type: node_type.to_string(),
            ref_id: Some(ref_id.to_string()),
            position: None,
            friendly_name: None,
            additional_pins: None,
            target_layer: None,
            summary: None,
        }
    }

    fn connect(from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) -> BoardCommand {
        BoardCommand::ConnectPins {
            from_node: from_node.to_string(),
            from_pin: from_pin.to_string(),
            to_node: to_node.to_string(),
            to_pin: to_pin.to_string(),
            summary: None,
        }
    }

    fn update_pin(node: &str, pin: &str, value: serde_json::Value) -> BoardCommand {
        BoardCommand::UpdateNodePin {
            node_id: node.to_string(),
            pin_id: pin.to_string(),
            value,
            summary: None,
        }
    }

    fn create_variable(id: &str, name: &str, default: Option<serde_json::Value>) -> BoardCommand {
        BoardCommand::CreateVariable {
            variable_id: Some(id.to_string()),
            name: name.to_string(),
            data_type: "String".to_string(),
            value_type: "Normal".to_string(),
            default_value: default,
            description: None,
            category: None,
            schema: None,
            exposed: None,
            secret: None,
            editable: None,
            runtime_configured: None,
            target_layer: None,
            summary: None,
        }
    }

    fn assert_clean(report: &ExecutabilityReport) {
        assert!(
            report.blocking.is_empty(),
            "expected no blocking findings: {:#?}",
            report.blocking
        );
        assert!(
            report.review_notes.is_empty(),
            "expected no review notes: {:#?}",
            report.review_notes
        );
    }

    #[test]
    fn clean_batch_produces_no_findings() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            add_node("log_info", "$2"),
            update_pin("$1", "url", json!("https://example.com")),
            connect("$0", "exec_out", "$1", "exec_in"),
            connect("$1", "exec_out", "$2", "exec_in"),
            connect("$1", "body", "$2", "message"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_clean(&report);
    }

    #[test]
    fn missing_required_input_on_reachable_impure_node_blocks() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_eq!(report.blocking.len(), 1, "{:#?}", report.blocking);
        let finding = &report.blocking[0];
        assert_eq!(finding.code, FlowScriptDiagnosticCode::FsUnresolvedArgument);
        assert!(finding.message.starts_with("Executability:"));
        assert!(finding.message.contains("`url`"), "{}", finding.message);
    }

    #[test]
    fn required_input_satisfied_by_connection_is_silent() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("string_concat", "$1"),
            add_node("http_fetch", "$2"),
            update_pin("$1", "left", json!("https://")),
            update_pin("$1", "right", json!("example.com")),
            connect("$1", "string", "$2", "url"),
            connect("$0", "exec_out", "$2", "exec_in"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_clean(&report);
    }

    #[test]
    fn defaulted_required_input_is_silent() {
        let mut catalog = catalog();
        let fetch = catalog
            .iter_mut()
            .find(|meta| meta.name == "http_fetch")
            .expect("fetch metadata");
        fetch
            .inputs
            .iter_mut()
            .find(|input| input.name == "url")
            .expect("url pin")
            .default_value = Some("\"https://example.com\"".to_string());
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog, &commands);
        assert_clean(&report);
    }

    #[test]
    fn dynamic_pin_node_types_are_exempt_from_required_inputs() {
        let mut catalog = catalog();
        catalog
            .iter_mut()
            .find(|meta| meta.name == "string_format")
            .expect("string_format metadata")
            .required_inputs = vec!["format_string".to_string()];
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("string_format", "$1"),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog, &commands);
        assert_clean(&report);
    }

    #[test]
    fn unresolvable_pin_reference_marks_node_opaque_and_suppresses_findings() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            connect("$0", "exec_out", "$1", "exec_in"),
            // A dynamic pin the static catalog cannot see: the node's true shape is unknown.
            update_pin("$1", "minted_by_on_update", json!("value")),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_clean(&report);
    }

    #[test]
    fn unresolvable_node_reference_aborts_the_whole_lint() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            connect("$0", "exec_out", "$1", "exec_in"),
            connect("$99", "body", "$1", "url"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_clean(&report);
    }

    #[test]
    fn lost_exec_chain_blocks_event_entry_and_notes_unreachable_node() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            update_pin("$1", "url", json!("https://example.com")),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_eq!(report.blocking.len(), 1, "{:#?}", report.blocking);
        assert_eq!(
            report.blocking[0].code,
            FlowScriptDiagnosticCode::FsExecutionEntryUnconnected
        );
        assert!(
            report.blocking[0].message.contains("silently do nothing"),
            "{}",
            report.blocking[0].message
        );
        assert_eq!(report.review_notes.len(), 1, "{:#?}", report.review_notes);
        assert!(
            report.review_notes[0]
                .message
                .starts_with("Executability note:"),
            "{}",
            report.review_notes[0].message
        );
    }

    #[test]
    fn pure_only_body_does_not_flag_the_event_entry() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("string_concat", "$1"),
            update_pin("$1", "left", json!("a")),
            update_pin("$1", "right", json!("b")),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_clean(&report);
    }

    #[test]
    fn new_data_update_node_notes_setter_repair_without_blocking() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("a2ui_data_update", "$1"),
            update_pin("$1", "surface_id", json!("main")),
            update_pin("$1", "path", json!("data/sources")),
            update_pin("$1", "value", json!("[]")),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert!(
            report.blocking.is_empty(),
            "a prohibited node must not strand the batch: {:#?}",
            report.blocking
        );
        assert_eq!(report.review_notes.len(), 1, "{:#?}", report.review_notes);
        let finding = &report.review_notes[0];
        assert_eq!(finding.code, FlowScriptDiagnosticCode::FsProhibitedNode);
        assert!(
            finding.message.contains("This batch adds it"),
            "{}",
            finding.message
        );
        let fix = finding.fix.as_ref().expect("prohibited node carries a fix");
        assert!(
            fix.summary.contains("a2uiSetElementText"),
            "{}",
            fix.summary
        );
        assert!(
            fix.summary.contains("a2uiInstantiateWidget"),
            "{}",
            fix.summary
        );
    }

    #[test]
    fn existing_data_update_node_notes_replacement_without_blocking() {
        let mut board = empty_board();
        let mut legacy = Node::new("a2ui_data_update", "Data Update", "", "UI/Data");
        legacy.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        legacy.add_input_pin("path", "Path", "", VariableType::String);
        legacy.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        board.nodes.insert(legacy.id.clone(), legacy);
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            update_pin("$1", "url", json!("https://example.com")),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&board, &catalog(), &commands);
        assert!(
            report.blocking.is_empty(),
            "an inherited node must not wedge unrelated edits: {:#?}",
            report.blocking
        );
        assert_eq!(report.review_notes.len(), 1, "{:#?}", report.review_notes);
        let note = &report.review_notes[0];
        assert_eq!(note.code, FlowScriptDiagnosticCode::FsProhibitedNode);
        assert!(
            note.message.contains("This board already carries it"),
            "{}",
            note.message
        );
    }

    #[test]
    fn existing_disconnected_impure_node_is_not_flagged() {
        let mut board = empty_board();
        let mut stale = Node::new("http_fetch", "Stale Fetch", "", "web");
        stale.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        stale.add_input_pin("url", "Url", "", VariableType::String);
        stale.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        board.nodes.insert(stale.id.clone(), stale);
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("http_fetch", "$1"),
            update_pin("$1", "url", json!("https://example.com")),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&board, &catalog(), &commands);
        assert_clean(&report);
    }

    #[test]
    fn base_board_connections_seed_and_satisfy_reachability() {
        let mut board = empty_board();
        let mut event = Node::new("events_simple", "On Start", "", "events");
        event.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        let mut worker = Node::new("log_info", "Existing Log", "", "logging");
        worker.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        worker.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        let event_out = event
            .pins
            .values()
            .find(|pin| pin.name == "exec_out")
            .expect("event exec out")
            .id
            .clone();
        let worker_in = worker
            .pins
            .values()
            .find(|pin| pin.name == "exec_in")
            .expect("worker exec in")
            .id
            .clone();
        event
            .pins
            .get_mut(&event_out)
            .expect("event pin")
            .connected_to
            .insert(worker_in.clone());
        worker
            .pins
            .get_mut(&worker_in)
            .expect("worker pin")
            .depends_on
            .insert(event_out);
        let worker_id = worker.id.clone();
        board.nodes.insert(event.id.clone(), event);
        board.nodes.insert(worker_id.clone(), worker);

        // New impure work chained from the EXISTING node must count as reachable.
        let commands = vec![
            add_node("http_fetch", "$0"),
            update_pin("$0", "url", json!("https://example.com")),
            connect(&worker_id, "exec_out", "$0", "exec_in"),
        ];
        let report = lint_flowscript_executability(&board, &catalog(), &commands);
        assert_clean(&report);
    }

    #[test]
    fn variable_read_without_set_or_default_surfaces_one_note() {
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("log_info", "$1"),
            add_node("variable_get", "$2"),
            create_variable("var_target", "targetFolder", None),
            update_pin("$2", "var_ref", json!("var_target")),
            connect("$2", "value_ref", "$1", "message"),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert!(report.blocking.is_empty(), "{:#?}", report.blocking);
        assert_eq!(report.review_notes.len(), 1, "{:#?}", report.review_notes);
        assert_eq!(
            report.review_notes[0].code,
            FlowScriptDiagnosticCode::FsVariableUnresolved
        );
        assert!(
            report.review_notes[0].message.contains("targetFolder"),
            "{}",
            report.review_notes[0].message
        );
    }

    #[test]
    fn variable_read_with_default_set_write_or_external_source_is_silent() {
        let base = |variable: BoardCommand, extra: Vec<BoardCommand>| {
            let mut commands = vec![
                add_node("events_simple", "$0"),
                add_node("log_info", "$1"),
                add_node("variable_get", "$2"),
                variable,
                update_pin("$2", "var_ref", json!("var_target")),
                connect("$2", "value_ref", "$1", "message"),
                connect("$0", "exec_out", "$1", "exec_in"),
            ];
            commands.extend(extra);
            commands
        };

        let with_default = base(
            create_variable("var_target", "targetFolder", Some(json!("inbox"))),
            Vec::new(),
        );
        assert_clean(&lint_flowscript_executability(
            &empty_board(),
            &catalog(),
            &with_default,
        ));

        let with_set = base(
            create_variable("var_target", "targetFolder", None),
            vec![
                add_node("variable_set", "$3"),
                update_pin("$3", "var_ref", json!("var_target")),
                update_pin("$3", "value", json!("inbox")),
                connect("$1", "exec_out", "$3", "exec_in"),
            ],
        );
        assert_clean(&lint_flowscript_executability(
            &empty_board(),
            &catalog(),
            &with_set,
        ));

        let mut secret = create_variable("var_target", "targetFolder", None);
        if let BoardCommand::CreateVariable { secret: flag, .. } = &mut secret {
            *flag = Some(true);
        }
        let with_secret = base(secret, Vec::new());
        assert_clean(&lint_flowscript_executability(
            &empty_board(),
            &catalog(),
            &with_secret,
        ));
    }

    #[test]
    fn function_layer_without_exec_tail_surfaces_one_note() {
        let function_pins = vec![
            PlaceholderPinDef {
                name: "exec_in".to_string(),
                friendly_name: "Exec In".to_string(),
                description: None,
                pin_type: "Input".to_string(),
                data_type: "Execution".to_string(),
                value_type: None,
                schema: None,
                enforce_schema: false,
            },
            PlaceholderPinDef {
                name: "exec_out".to_string(),
                friendly_name: "Exec Out".to_string(),
                description: None,
                pin_type: "Output".to_string(),
                data_type: "Execution".to_string(),
                value_type: None,
                schema: None,
                enforce_schema: false,
            },
        ];
        let create_layer = BoardCommand::CreateLayer {
            name: "sendReport".to_string(),
            ref_id: Some("$0".to_string()),
            layer_type: Some("Function".to_string()),
            node_ids: Vec::new(),
            pins: Some(function_pins),
            position: None,
            color: None,
            target_layer: None,
            cache: None,
            summary: None,
        };
        let mut commands = vec![
            create_layer,
            add_node("http_fetch", "$1"),
            update_pin("$1", "url", json!("https://example.com")),
            connect("$0", "exec_in", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert!(report.blocking.is_empty(), "{:#?}", report.blocking);
        assert_eq!(report.review_notes.len(), 1, "{:#?}", report.review_notes);
        assert_eq!(
            report.review_notes[0].code,
            FlowScriptDiagnosticCode::FsHelperExecutionTailUnconnected
        );
        assert!(
            report.review_notes[0].message.contains("sendReport"),
            "{}",
            report.review_notes[0].message
        );

        commands.push(connect("$1", "exec_out", "$0", "exec_out"));
        let terminated = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_clean(&terminated);
    }

    fn placeholder_pin(name: &str, pin_type: &str, data_type: &str) -> PlaceholderPinDef {
        PlaceholderPinDef {
            name: name.to_string(),
            friendly_name: name.to_string(),
            description: None,
            pin_type: pin_type.to_string(),
            data_type: data_type.to_string(),
            value_type: None,
            schema: None,
            enforce_schema: false,
        }
    }

    fn create_function_layer_with_return(ref_id: &str, name: &str) -> BoardCommand {
        BoardCommand::CreateLayer {
            name: name.to_string(),
            ref_id: Some(ref_id.to_string()),
            layer_type: Some("Function".to_string()),
            node_ids: Vec::new(),
            pins: Some(vec![
                placeholder_pin("exec_in", "Input", "Execution"),
                placeholder_pin("exec_out", "Output", "Execution"),
                placeholder_pin("owner_sub", "Output", "String"),
            ]),
            position: None,
            color: None,
            target_layer: None,
            cache: None,
            summary: None,
        }
    }

    #[test]
    fn new_function_layer_with_unfed_return_pin_blocks() {
        let mut commands = vec![
            create_function_layer_with_return("$0", "getOwnerIdentity"),
            add_node("http_fetch", "$1"),
            update_pin("$1", "url", json!("https://example.com")),
            connect("$0", "exec_in", "$1", "exec_in"),
            connect("$1", "exec_out", "$0", "exec_out"),
        ];
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_eq!(report.blocking.len(), 1, "{:#?}", report.blocking);
        let finding = &report.blocking[0];
        assert_eq!(
            finding.code,
            FlowScriptDiagnosticCode::FsFunctionReturnMismatch
        );
        assert!(
            finding.message.contains("`ownerSub`") && finding.message.contains("getOwnerIdentity"),
            "{}",
            finding.message
        );
        assert!(report.review_notes.is_empty(), "{:#?}", report.review_notes);

        commands.push(connect("$1", "body", "$0", "owner_sub"));
        let fed = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_clean(&fed);
    }

    /// Base board mirroring the applied uptime-monitor defect: a pre-existing Function layer
    /// whose declared return pin has no incoming edge. Returns the board and the body node id.
    fn board_with_unfed_return_layer(feed_return: bool) -> Board {
        let mut board = empty_board();
        let mut layer = crate::flow::board::Layer::new(
            "layer-owner".to_string(),
            "getOwnerIdentity".to_string(),
            LayerType::Function,
        );
        let mut template = Node::new("boundary", "Boundary", "", "test");
        template.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        template.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        let return_pin_id = template
            .add_output_pin("owner_sub", "owner_sub", "", VariableType::String)
            .id
            .clone();
        for pin in template.pins.into_values() {
            layer.pins.insert(pin.id.clone(), pin);
        }

        let mut body = Node::new("http_fetch", "Fetch", "", "web");
        body.id = "body-fetch".to_string();
        body.layer = Some(layer.id.clone());
        body.add_input_pin("exec_in", "Exec In", "", VariableType::Execution);
        body.add_output_pin("exec_out", "Exec Out", "", VariableType::Execution);
        let body_out = body
            .add_output_pin("body", "Body", "", VariableType::String)
            .id
            .clone();
        if feed_return {
            body.pins
                .get_mut(&body_out)
                .expect("body output pin")
                .connected_to
                .insert(return_pin_id.clone());
            layer
                .pins
                .get_mut(&return_pin_id)
                .expect("layer return pin")
                .depends_on
                .insert(body_out);
        }
        board.nodes.insert(body.id.clone(), body);
        board.layers.insert(layer.id.clone(), layer);
        board
    }

    #[test]
    fn pre_existing_unfed_return_pin_surfaces_a_review_note() {
        let board = board_with_unfed_return_layer(false);
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("log_info", "$1"),
            update_pin("$1", "message", json!("hello")),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&board, &catalog(), &commands);
        assert!(report.blocking.is_empty(), "{:#?}", report.blocking);
        let note = report
            .review_notes
            .iter()
            .find(|note| note.code == FlowScriptDiagnosticCode::FsFunctionReturnMismatch)
            .unwrap_or_else(|| panic!("missing unfed-return note: {:#?}", report.review_notes));
        assert!(
            note.message.contains("`ownerSub`") && note.message.contains("getOwnerIdentity"),
            "{}",
            note.message
        );
    }

    #[test]
    fn pre_existing_fed_return_pin_is_silent() {
        let board = board_with_unfed_return_layer(true);
        let commands = vec![
            add_node("events_simple", "$0"),
            add_node("log_info", "$1"),
            update_pin("$1", "message", json!("hello")),
            connect("$0", "exec_out", "$1", "exec_in"),
        ];
        let report = lint_flowscript_executability(&board, &catalog(), &commands);
        assert!(report.blocking.is_empty(), "{:#?}", report.blocking);
        assert!(
            !report
                .review_notes
                .iter()
                .any(|note| { note.code == FlowScriptDiagnosticCode::FsFunctionReturnMismatch }),
            "{:#?}",
            report.review_notes
        );
    }

    #[test]
    fn findings_are_capped_with_a_truncation_note() {
        let mut commands = vec![add_node("events_simple", "$0")];
        for index in 1..=12 {
            let node_ref = format!("${index}");
            commands.push(add_node("http_fetch", &node_ref));
            commands.push(connect("$0", "exec_out", &node_ref, "exec_in"));
        }
        let report = lint_flowscript_executability(&empty_board(), &catalog(), &commands);
        assert_eq!(report.blocking.len(), 11, "{:#?}", report.blocking);
        assert!(
            report.blocking[10].message.contains("2 additional"),
            "{}",
            report.blocking[10].message
        );
    }

    #[test]
    fn clean_realistic_script_checks_valid_with_zero_executability_findings() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let source = r#"eventsSimple() {
    const page = httpFetch({ url: "https://example.com" })
    logInfo({ message: page.body })
}
"#;
        let written = store.write_flowscript(
            &board,
            &catalog(),
            WriteFlowScriptArgs {
                draft_id: "executability-clean".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.to_string(),
                allow_scope_reduction: false,
            },
        );
        assert!(written.diagnostics.is_empty(), "{written:#?}");
        let checked = store.check_flowscript(
            &board,
            &catalog(),
            CheckFlowScriptArgs {
                draft_id: "executability-clean".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert!(checked.diagnostics.is_empty(), "{checked:#?}");
        assert!(checked.review_notes.is_empty(), "{checked:#?}");
    }

    #[test]
    fn script_using_data_update_stays_valid_but_directs_another_revision() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let source = r#"eventsSimple() {
    a2uiDataUpdate({ surfaceId: "main", path: "data/sources", value: "[]" })
}
"#;
        let written = store.write_flowscript(
            &board,
            &catalog(),
            WriteFlowScriptArgs {
                draft_id: "prohibited-data-update".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.to_string(),
                allow_scope_reduction: false,
            },
        );
        assert!(written.diagnostics.is_empty(), "{written:#?}");
        let checked = store.check_flowscript(
            &board,
            &catalog(),
            CheckFlowScriptArgs {
                draft_id: "prohibited-data-update".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert!(checked.diagnostics.is_empty(), "{checked:#?}");
        let prohibited = checked
            .review_notes
            .iter()
            .find(|note| note.code == FlowScriptDiagnosticCode::FsProhibitedNode)
            .unwrap_or_else(|| panic!("{checked:#?}"));
        assert!(
            prohibited
                .fix
                .as_ref()
                .is_some_and(|fix| fix.summary.contains("a2uiInstantiateWidget")),
            "{prohibited:#?}"
        );
        // The model must read this as its own outstanding work, not as a human-review note.
        assert!(checked.message.contains("NOT DONE"), "{}", checked.message);
        assert!(
            checked.message.contains("Write a corrected revision now"),
            "{}",
            checked.message
        );
        assert!(
            !checked.message.contains("acceptance review note"),
            "{}",
            checked.message
        );

        let queued = store.commit_flowscript(
            &board,
            &catalog(),
            CommitFlowScriptArgs {
                draft_id: "prohibited-data-update".to_string(),
                expected_revision: 0,
                allow_deletions: false,
                remove_node_ids: Vec::new(),
                remove_variable_ids: Vec::new(),
                remove_layer_ids: Vec::new(),
                remove_comment_ids: Vec::new(),
            },
        );
        assert_eq!(queued.status, "queued", "{queued:#?}");
        assert!(!queued.commands.is_empty(), "{queued:#?}");
        assert!(queued.message.contains("NOT DONE"), "{}", queued.message);
    }

    #[test]
    fn board_variable_read_without_default_surfaces_note_and_stays_valid() {
        let store = FlowIrDraftStore::new();
        let board = empty_board();
        let source = r#"const targetFolder: string

eventsSimple() {
    logInfo({ message: targetFolder })
}
"#;
        let written = store.write_flowscript(
            &board,
            &catalog(),
            WriteFlowScriptArgs {
                draft_id: "executability-variable-note".to_string(),
                replace_existing: false,
                mode: FlowIrDraftMode::Additive,
                source: source.to_string(),
                allow_scope_reduction: false,
            },
        );
        assert!(written.diagnostics.is_empty(), "{written:#?}");
        let checked = store.check_flowscript(
            &board,
            &catalog(),
            CheckFlowScriptArgs {
                draft_id: "executability-variable-note".to_string(),
                expected_revision: 0,
            },
        );
        assert_eq!(checked.status, "valid", "{checked:#?}");
        assert!(checked.diagnostics.is_empty(), "{checked:#?}");
        assert!(
            checked.review_notes.iter().any(|note| {
                note.code == FlowScriptDiagnosticCode::FsVariableUnresolved
                    && note.message.contains("targetFolder")
            }),
            "{checked:#?}"
        );
    }

    #[test]
    fn stripped_exec_chain_from_real_reconcile_output_blocks() {
        let board = empty_board();
        let catalog = catalog();
        let source = r#"eventsSimple() {
    httpFetch({ url: "https://example.com" })
}
"#;
        let reconcile = crate::flow::ast::reconcile_text_with_catalog(&board, source, &catalog);
        assert!(
            reconcile.diagnostics.is_empty(),
            "{:?}",
            reconcile.diagnostics
        );
        let intact = lint_flowscript_executability(&board, &catalog, &reconcile.commands);
        assert_clean(&intact);

        // Simulate a lost exec splice: drop every exec-to-exec connect from the real batch.
        let stripped = reconcile
            .commands
            .into_iter()
            .filter(|command| {
                !matches!(
                    command,
                    BoardCommand::ConnectPins { from_pin, to_pin, .. }
                        if from_pin.contains("exec") && to_pin.contains("exec")
                )
            })
            .collect::<Vec<_>>();
        let report = lint_flowscript_executability(&board, &catalog, &stripped);
        assert!(
            report.blocking.iter().any(|finding| {
                finding.code == FlowScriptDiagnosticCode::FsExecutionEntryUnconnected
            }),
            "{report:#?}"
        );
    }
}
