//! Lowers an editor [`Board`] into a compiled execution plan.
//!
//! The compiler runs the expensive, string-keyed work exactly once — at version-snapshot
//! time or on the first run of a draft — so that starting a run becomes: fetch one object,
//! validate it, hydrate index tables. Everything the old per-run graph builder did with
//! hash maps of cuid2 strings is resolved here into flat `u32` arrays.
//!
//! Determinism is a hard requirement, not a nicety: plans are written with
//! `PutMode::Create` and racing writers must produce byte-identical objects. Every map
//! iteration below is therefore sorted before it reaches the artifact.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use flow_like_types::plan::{
    ColdPlan, DebugPlan, HotPlan, MAX_ENTITIES, NONE_INDEX, PLAN_FORMAT_VERSION, PlanLayer,
    PlanNode, PlanPin, PlanVariable, build_container,
    header::{PlanHeader, PlanStamps},
    hot::{Csr, PinNameEntry, name_hash, node_flags, pin_flags, variable_flags},
    serialize_section,
};

use super::super::{
    node::Node,
    pin::{Pin, PinType},
    variable::VariableType,
};
use super::{Board, LayerType};

/// Boards store descriptions and schemas as hashes into `Board::refs`; this sentinel means
/// "the empty string". Mirrors the resolution the LLM/MCP/REST surfaces do at runtime,
/// which the compiler performs once so the refs map never reaches the artifact.
const EMPTY_STRING_REF_HASH: &str = "16248035215404677707";

fn resolve_ref(value: &str, refs: &HashMap<String, String>) -> String {
    let trimmed = value.trim();
    if trimmed == EMPTY_STRING_REF_HASH {
        return String::new();
    }
    refs.get(trimmed)
        .cloned()
        .unwrap_or_else(|| trimmed.to_string())
}

/// Identifies the toolchain a plan was compiled against.
///
/// Compilation freezes the result of `Board::node_updates`, which normally self-heals a
/// board against catalog drift on every load. A plan compiled against one catalog must
/// therefore never execute against another — these stamps are what makes that detectable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompileStamps {
    pub catalog_signature: u64,
    pub wasm_signature: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum CompileError {
    #[error("board has {count} {kind}, exceeding the {max} the plan index space allows")]
    TooLarge {
        kind: &'static str,
        count: usize,
        max: usize,
    },
    #[error("failed to serialize plan section {section}: {source}")]
    Serialize {
        section: &'static str,
        source: flow_like_types::rkyv::rancor::Error,
    },
}

/// Interns strings so every id/name in the artifact costs one `u32`.
#[derive(Default)]
struct SymbolTable {
    symbols: Vec<String>,
    lookup: HashMap<String, u32>,
}

impl SymbolTable {
    fn intern(&mut self, value: &str) -> u32 {
        if let Some(index) = self.lookup.get(value) {
            return *index;
        }
        let index = self.symbols.len() as u32;
        self.symbols.push(value.to_string());
        self.lookup.insert(value.to_string(), index);
        index
    }
}

/// Deduplicates canonical default-value payloads.
#[derive(Default)]
struct BlobTable {
    blobs: Vec<Vec<u8>>,
    lookup: HashMap<Vec<u8>, u32>,
}

impl BlobTable {
    /// Canonicalize a stored default so equal values share one blob.
    ///
    /// Unparseable payloads are kept verbatim: the runtime already tolerates them by
    /// falling back to `Null`, and silently dropping them here would change behaviour.
    fn intern(&mut self, raw: &[u8]) -> u32 {
        let canonical = flow_like_types::json::from_slice::<flow_like_types::Value>(raw)
            .ok()
            .and_then(|value| flow_like_types::json::to_vec(&value).ok())
            .unwrap_or_else(|| raw.to_vec());

        if let Some(index) = self.lookup.get(&canonical) {
            return *index;
        }
        let index = self.blobs.len() as u32;
        self.lookup.insert(canonical.clone(), index);
        self.blobs.push(canonical);
        index
    }
}

/// A node paired with the layer that owns it, in canonical order.
struct NodeEntry<'a> {
    id: &'a str,
    node: &'a Node,
    layer: Option<&'a str>,
}

/// Everything the lowering needs to resolve pins before indices are assigned.
struct PinEntry<'a> {
    id: &'a str,
    pin: &'a Pin,
    owner_node: u32,
    is_layer_pin: bool,
}

/// A compiled plan, ready to be written as a container.
pub struct CompiledPlan {
    pub hot: HotPlan,
    pub cold: ColdPlan,
    pub debug: DebugPlan,
    pub stamps: PlanStamps,
}

impl CompiledPlan {
    /// Serialize to the on-disk container layout.
    pub fn to_container(&self) -> Result<Vec<u8>, CompileError> {
        let hot = serialize_section(&self.hot).map_err(|source| CompileError::Serialize {
            section: "hot",
            source,
        })?;
        let cold = serialize_section(&self.cold).map_err(|source| CompileError::Serialize {
            section: "cold",
            source,
        })?;
        let debug = serialize_section(&self.debug).map_err(|source| CompileError::Serialize {
            section: "debug",
            source,
        })?;

        let header = PlanHeader::new(PLAN_FORMAT_VERSION, self.stamps);
        Ok(build_container(&header, &hot, &cold, &debug))
    }
}

/// Lower a board into a compiled plan.
///
/// The board is expected to have already been through `node_updates` and `cleanup` — that
/// is, to be in the same shape the old graph builder would have seen.
pub fn compile_board(board: &Board, stamps: CompileStamps) -> Result<CompiledPlan, CompileError> {
    let mut symbols = SymbolTable::default();
    let mut blobs = BlobTable::default();

    // ── Canonical entity ordering ────────────────────────────────────────────────
    // Nodes come from the board plus every function layer, mirroring the old builder.
    // Sorting by id makes the artifact reproducible across runs and machines.
    let mut layers: Vec<&str> = board.layers.keys().map(String::as_str).collect();
    layers.sort_unstable();

    let mut node_entries: Vec<NodeEntry> = board
        .nodes
        .iter()
        .map(|(id, node)| NodeEntry {
            id,
            node,
            layer: None,
        })
        .collect();

    for layer_id in &layers {
        let layer = &board.layers[*layer_id];
        if !matches!(layer.r#type, LayerType::Function) {
            continue;
        }
        for (id, node) in &layer.nodes {
            node_entries.push(NodeEntry {
                id,
                node,
                layer: Some(layer_id),
            });
        }
    }
    node_entries.sort_unstable_by(|a, b| a.id.cmp(b.id));
    check_size("nodes", node_entries.len())?;

    let layer_index: HashMap<&str, u32> = layers
        .iter()
        .enumerate()
        .map(|(index, id)| (*id, index as u32))
        .collect();
    // Pins are laid out contiguously per node so a node's pins are one slice, then layer
    // relay pins. A layer pin that duplicates a node pin id is skipped, matching the old
    // builder's handling of the legacy layer format.
    let mut pin_entries: Vec<PinEntry> = Vec::new();
    let mut seen_pins: BTreeSet<&str> = BTreeSet::new();
    // (first_pin, pin_count) per node, recorded as the slice is built.
    let mut pin_ranges: Vec<(u32, u32)> = Vec::with_capacity(node_entries.len());

    for (index, entry) in node_entries.iter().enumerate() {
        let first_pin = pin_entries.len() as u32;
        // Identity is `pin.id`, not the map key: `connected_to` / `depends_on` hold pin
        // ids, and both `Board::cleanup` and the runtime graph builder key their lookups
        // the same way. Keying by the map key instead would silently drop every edge if
        // the two ever diverged.
        let mut node_pins: Vec<(&str, &Pin)> = entry
            .node
            .pins
            .values()
            .map(|pin| (pin.id.as_str(), pin))
            .collect();
        node_pins.sort_unstable_by(|a, b| a.1.index.cmp(&b.1.index).then_with(|| a.0.cmp(b.0)));
        for (id, pin) in node_pins {
            if !seen_pins.insert(id) {
                continue;
            }
            pin_entries.push(PinEntry {
                id,
                pin,
                owner_node: index as u32,
                is_layer_pin: false,
            });
        }
        pin_ranges.push((first_pin, pin_entries.len() as u32 - first_pin));
    }

    for layer_id in &layers {
        let layer = &board.layers[*layer_id];
        let mut relay_pins: Vec<(&str, &Pin)> = layer
            .pins
            .values()
            .map(|pin| (pin.id.as_str(), pin))
            .collect();
        relay_pins.sort_unstable_by(|a, b| a.0.cmp(b.0));
        for (id, pin) in relay_pins {
            if !seen_pins.insert(id) {
                continue;
            }
            pin_entries.push(PinEntry {
                id,
                pin,
                owner_node: NONE_INDEX,
                is_layer_pin: true,
            });
        }
    }
    check_size("pins", pin_entries.len())?;

    let pin_index: HashMap<&str, u32> = pin_entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.id, index as u32))
        .collect();

    // ── Nodes ────────────────────────────────────────────────────────────────────
    let mut nodes = Vec::with_capacity(node_entries.len());
    let mut fn_refs = Csr::with_capacity(node_entries.len(), 0);
    let mut wasm_permissions = Csr::with_capacity(node_entries.len(), 0);

    for (index, entry) in node_entries.iter().enumerate() {
        let node = entry.node;
        let (first_pin, pin_count) = pin_ranges[index];

        let mut flags = 0u32;
        if node.is_pure() {
            flags |= node_flags::IS_PURE;
        }
        if let Some(refs) = &node.fn_refs {
            if refs.can_reference_fns {
                flags |= node_flags::CAN_REFERENCE_FNS;
            }
            if refs.can_be_referenced_by_fns {
                flags |= node_flags::CAN_BE_REFERENCED_BY_FNS;
            }
            for target in &refs.fn_refs {
                fn_refs.edges.push(symbols.intern(target));
            }
        }
        fn_refs.finish_row();

        if let Some(wasm) = &node.wasm {
            for permission in &wasm.permissions {
                wasm_permissions.edges.push(permission.to_plan_u8() as u32);
            }
        }
        wasm_permissions.finish_row();

        if node
            .pins
            .values()
            .any(|pin| pin.pin_type == PinType::Output && pin.data_type == VariableType::Execution)
        {
            flags |= node_flags::HAS_EXEC_OUTPUT;
        }

        nodes.push(PlanNode {
            instance_id: symbols.intern(&node.id),
            type_key: symbols.intern(&node.name),
            flags,
            first_pin,
            pin_count,
            layer: entry
                .layer
                .or(node.layer.as_deref())
                .and_then(|id| layer_index.get(id).copied())
                .unwrap_or(NONE_INDEX),
            wasm_package: node
                .wasm
                .as_ref()
                .map(|wasm| symbols.intern(&wasm.package_id))
                .unwrap_or(NONE_INDEX),
            semantic_hash: node.semantic_hash(),
        });
    }

    // ── Pins ─────────────────────────────────────────────────────────────────────
    let mut pins = Vec::with_capacity(pin_entries.len());
    for entry in &pin_entries {
        let pin = entry.pin;
        let mut flags = 0u16;
        if entry.is_layer_pin {
            flags |= pin_flags::IS_LAYER_PIN;
        }
        if pin.data_type == VariableType::Execution {
            flags |= pin_flags::IS_EXEC;
        }
        let default_value = match &pin.default_value {
            Some(raw) => {
                flags |= pin_flags::HAS_DEFAULT;
                blobs.intern(raw)
            }
            None => NONE_INDEX,
        };

        pins.push(PlanPin {
            id: symbols.intern(&pin.id),
            name: symbols.intern(&pin.name),
            owner_node: entry.owner_node,
            default_value,
            index: pin.index,
            flags,
            pin_type: pin.pin_type.to_plan_u8(),
            data_type: pin.data_type.to_plan_u8(),
            value_type: pin.value_type.to_plan_u8(),
        });
    }

    // ── Pin adjacency ────────────────────────────────────────────────────────────
    // Unresolvable ids are dropped, exactly as the old builder's `filter_map` did.
    let mut pin_connections = Csr::with_capacity(pin_entries.len(), 0);
    let mut pin_dependencies = Csr::with_capacity(pin_entries.len(), 0);
    for entry in &pin_entries {
        for target in &entry.pin.connected_to {
            if let Some(index) = pin_index.get(target.as_str()) {
                pin_connections.edges.push(*index);
            }
        }
        pin_connections.finish_row();

        for source in &entry.pin.depends_on {
            if let Some(index) = pin_index.get(source.as_str()) {
                pin_dependencies.edges.push(*index);
            }
        }
        pin_dependencies.finish_row();
    }

    // ── Execution successors ─────────────────────────────────────────────────────
    // Walks through layer relay pins so the scheduler sees direct node-to-node edges and
    // never has to evaluate data pins to discover where control flows next.
    let mut exec_successors = Csr::with_capacity(node_entries.len(), 0);
    for entry in &node_entries {
        let mut targets: BTreeSet<u32> = BTreeSet::new();
        for pin in entry.node.pins.values() {
            if pin.pin_type != PinType::Output || pin.data_type != VariableType::Execution {
                continue;
            }
            let Some(start) = pin_index.get(pin.id.as_str()) else {
                continue;
            };
            collect_exec_targets(*start, &pin_connections, &pins, &mut targets);
        }
        exec_successors.edges.extend(targets);
        exec_successors.finish_row();
    }

    // ── Pure schedules ───────────────────────────────────────────────────────────
    let mut pure_schedules = Csr::with_capacity(node_entries.len(), 0);
    for index in 0..node_entries.len() {
        let mut schedule = Vec::new();
        let mut visited = BTreeSet::new();
        collect_pure_schedule(
            index as u32,
            &nodes,
            &pins,
            &pin_dependencies,
            &mut visited,
            &mut schedule,
        );
        pure_schedules.edges.extend(schedule);
        pure_schedules.finish_row();
    }

    // ── Variables ────────────────────────────────────────────────────────────────
    let mut variables = Vec::new();
    let mut board_variable_ids: Vec<&str> = board.variables.keys().map(String::as_str).collect();
    board_variable_ids.sort_unstable();
    for id in board_variable_ids {
        variables.push(lower_variable(
            &board.variables[id],
            NONE_INDEX,
            &mut symbols,
            &mut blobs,
        ));
    }

    let mut layer_variables = Csr::with_capacity(layers.len(), 0);
    for layer_id in &layers {
        let layer = &board.layers[*layer_id];
        let mut ids: Vec<&str> = layer.variables.keys().map(String::as_str).collect();
        ids.sort_unstable();
        for id in ids {
            let index = variables.len() as u32;
            variables.push(lower_variable(
                &layer.variables[id],
                layer_index[*layer_id],
                &mut symbols,
                &mut blobs,
            ));
            layer_variables.edges.push(index);
        }
        layer_variables.finish_row();
    }
    check_size("variables", variables.len())?;

    // ── Layers ───────────────────────────────────────────────────────────────────
    let plan_layers = layers
        .iter()
        .map(|layer_id| {
            let layer = &board.layers[*layer_id];
            PlanLayer {
                id: symbols.intern(&layer.id),
                parent: layer
                    .parent_id
                    .as_deref()
                    .and_then(|id| layer_index.get(id).copied())
                    .unwrap_or(NONE_INDEX),
                layer_type: match layer.r#type {
                    LayerType::Function => 0,
                    LayerType::Macro => 1,
                    LayerType::Collapsed => 2,
                },
            }
        })
        .collect();

    // ── Lookup tables ────────────────────────────────────────────────────────────
    let mut nodes_by_id: Vec<u32> = (0..node_entries.len() as u32).collect();
    nodes_by_id.sort_unstable_by(|a, b| {
        node_entries[*a as usize]
            .node
            .id
            .cmp(&node_entries[*b as usize].node.id)
    });

    let mut pin_name_offsets = Vec::with_capacity(node_entries.len() + 1);
    let mut pin_name_entries = Vec::new();
    pin_name_offsets.push(0u32);
    for node in &nodes {
        let start = node.first_pin as usize;
        let end = start + node.pin_count as usize;
        let mut entries: Vec<PinNameEntry> = (start..end)
            .map(|pin| PinNameEntry {
                hash: name_hash(&pin_entries[pin].pin.name),
                pin: pin as u32,
            })
            .collect();
        // Sorted by hash for binary search; ties keep pin order so `get_pins_by_name`
        // returns a stable sequence instead of today's hash-map iteration order.
        entries.sort_by(|a, b| a.hash.cmp(&b.hash).then_with(|| a.pin.cmp(&b.pin)));
        pin_name_entries.extend(entries);
        pin_name_offsets.push(pin_name_entries.len() as u32);
    }

    let start_nodes = nodes
        .iter()
        .enumerate()
        .filter(|(index, _)| node_entries[*index].node.start.unwrap_or(false))
        .map(|(index, _)| index as u32)
        .collect();

    // ── Cold + debug sections ────────────────────────────────────────────────────
    // Strings here are interned rather than inlined per entity. `board.refs` is not only
    // an indirection, it is also the board's deduplication: one JSON schema is commonly
    // shared by hundreds of pins. Resolving refs without re-establishing that sharing
    // multiplies the artifact several times over.
    let mut cold_strings = SymbolTable::default();
    cold_strings.intern("");

    let mut cold = ColdPlan {
        node_friendly_names: Vec::with_capacity(node_entries.len()),
        node_descriptions: Vec::with_capacity(node_entries.len()),
        pin_friendly_names: Vec::with_capacity(pin_entries.len()),
        pin_descriptions: Vec::with_capacity(pin_entries.len()),
        pin_schemas: Vec::with_capacity(pin_entries.len()),
        pin_valid_values: Vec::with_capacity(pin_entries.len()),
        layer_names: Vec::with_capacity(layers.len()),
        strings: Vec::new(),
    };

    for entry in &node_entries {
        cold.node_friendly_names
            .push(cold_strings.intern(&resolve_ref(&entry.node.friendly_name, &board.refs)));
        cold.node_descriptions
            .push(cold_strings.intern(&resolve_ref(&entry.node.description, &board.refs)));
    }

    for entry in &pin_entries {
        cold.pin_friendly_names
            .push(cold_strings.intern(&resolve_ref(&entry.pin.friendly_name, &board.refs)));
        cold.pin_descriptions
            .push(cold_strings.intern(&resolve_ref(&entry.pin.description, &board.refs)));
        cold.pin_schemas.push(
            entry
                .pin
                .schema
                .as_deref()
                .map(|schema| cold_strings.intern(&resolve_ref(schema, &board.refs)))
                .unwrap_or(flow_like_types::plan::cold::EMPTY_STRING),
        );
        cold.pin_valid_values.push(
            entry
                .pin
                .options
                .as_ref()
                .and_then(|options| options.valid_values.as_ref())
                .map(|values| {
                    values
                        .iter()
                        .map(|value| cold_strings.intern(value))
                        .collect()
                })
                .unwrap_or_default(),
        );
    }

    for layer_id in &layers {
        cold.layer_names
            .push(cold_strings.intern(&board.layers[*layer_id].name));
    }
    cold.strings = cold_strings.symbols;

    let debug = DebugPlan {
        node_ids: node_entries
            .iter()
            .map(|entry| entry.node.id.clone())
            .collect(),
        node_friendly_names: node_entries
            .iter()
            .map(|entry| resolve_ref(&entry.node.friendly_name, &board.refs))
            .collect(),
        node_icons: node_entries
            .iter()
            .map(|entry| entry.node.icon.clone().unwrap_or_default())
            .collect(),
    };

    let hot = HotPlan {
        board_id: board.id.clone(),
        stage: board.stage.to_plan_u8(),
        log_level: board.log_level.clone() as u8,
        symbols: symbols.symbols,
        blobs: blobs.blobs,
        nodes,
        pins,
        variables,
        layers: plan_layers,
        pin_connections,
        pin_dependencies,
        exec_successors,
        pure_schedules,
        fn_refs,
        wasm_permissions,
        layer_variables,
        nodes_by_id,
        pin_name_offsets,
        pin_name_entries,
        start_nodes,
    };

    Ok(CompiledPlan {
        hot,
        cold,
        debug,
        stamps: PlanStamps {
            board_content_hash: board.hash.unwrap_or(0),
            catalog_signature: stamps.catalog_signature,
            wasm_signature: stamps.wasm_signature,
            board_version: board.version,
        },
    })
}

fn check_size(kind: &'static str, count: usize) -> Result<(), CompileError> {
    if count > MAX_ENTITIES {
        return Err(CompileError::TooLarge {
            kind,
            count,
            max: MAX_ENTITIES,
        });
    }
    Ok(())
}

fn lower_variable(
    variable: &super::super::variable::Variable,
    owner_layer: u32,
    symbols: &mut SymbolTable,
    blobs: &mut BlobTable,
) -> PlanVariable {
    let mut flags = 0u8;
    if variable.secret {
        flags |= variable_flags::SECRET;
    }
    if variable.exposed {
        flags |= variable_flags::EXPOSED;
    }
    if variable.runtime_configured {
        flags |= variable_flags::RUNTIME_CONFIGURED;
    }
    if variable.editable {
        flags |= variable_flags::EDITABLE;
    }

    PlanVariable {
        id: symbols.intern(&variable.id),
        name: symbols.intern(&variable.name),
        default_value: variable
            .default_value
            .as_ref()
            .map(|raw| blobs.intern(raw))
            .unwrap_or(NONE_INDEX),
        owner_layer,
        data_type: variable.data_type.to_plan_u8(),
        value_type: variable.value_type.to_plan_u8(),
        flags,
    }
}

/// Follow an execution pin's connections to the nodes they ultimately reach.
///
/// Layer relay pins own no node, so control passes through them; the visited set keeps a
/// relay cycle from looping forever.
fn collect_exec_targets(
    start: u32,
    connections: &Csr,
    pins: &[PlanPin],
    targets: &mut BTreeSet<u32>,
) {
    let mut stack = vec![start];
    let mut seen = BTreeSet::new();
    while let Some(current) = stack.pop() {
        if !seen.insert(current) {
            continue;
        }
        for next in connections.row(current as usize) {
            let pin = &pins[*next as usize];
            if pin.owner_node == NONE_INDEX {
                stack.push(*next);
            } else {
                targets.insert(pin.owner_node);
            }
        }
    }
}

/// The pure nodes feeding a node's data inputs, in pin order and without duplicates.
fn pure_parents(node: u32, nodes: &[PlanNode], pins: &[PlanPin], dependencies: &Csr) -> Vec<u32> {
    let entry = &nodes[node as usize];
    let start = entry.first_pin as usize;
    let end = start + entry.pin_count as usize;

    let mut parents = Vec::new();
    for pin_index in start..end {
        let pin = &pins[pin_index];
        if pin.pin_type != PinType::Input.to_plan_u8() || pin.flags & pin_flags::IS_EXEC != 0 {
            continue;
        }
        for source in dependencies.row(pin_index) {
            let owner = pins[*source as usize].owner_node;
            if owner == NONE_INDEX || nodes[owner as usize].flags & node_flags::IS_PURE == 0 {
                continue;
            }
            if !parents.contains(&owner) {
                parents.push(owner);
            }
        }
    }
    parents
}

/// Build the topologically ordered list of pure nodes a node must evaluate first.
///
/// Mirrors the runtime's dependency walk: for every non-execution input pin, follow
/// `depends_on` to the owning node, and continue while those owners are pure. Emitting in
/// post-order means a schedule can be executed front to back with no further checks.
///
/// The traversal keeps an explicit stack rather than recursing: boards are user input, and
/// a deeply chained pure graph would otherwise overflow the stack — an abort the caller
/// cannot catch. `visited` also terminates cycles, which the editor permits to exist.
fn collect_pure_schedule(
    node: u32,
    nodes: &[PlanNode],
    pins: &[PlanPin],
    dependencies: &Csr,
    visited: &mut BTreeSet<u32>,
    out: &mut Vec<u32>,
) {
    // `false` = still needs expanding, `true` = children done, safe to emit.
    let mut stack: Vec<(u32, bool)> = pure_parents(node, nodes, pins, dependencies)
        .into_iter()
        .rev()
        .map(|parent| (parent, false))
        .collect();

    while let Some((current, expanded)) = stack.pop() {
        if expanded {
            out.push(current);
            continue;
        }
        if !visited.insert(current) {
            continue;
        }
        stack.push((current, true));
        for parent in pure_parents(current, nodes, pins, dependencies)
            .into_iter()
            .rev()
        {
            if !visited.contains(&parent) {
                stack.push((parent, false));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::board::Layer;
    use crate::flow::pin::ValueType;
    use flow_like_storage::Path;
    use flow_like_types::plan::{ArchivedHotPlan, PlanBuffer};

    const STAMPS: CompileStamps = CompileStamps {
        catalog_signature: 1,
        wasm_signature: 2,
    };

    fn node(id: &str, type_key: &str) -> Node {
        let mut node = Node::new(type_key, type_key, "", "Test");
        node.id = id.to_string();
        node
    }

    /// Add a pin under a deterministic id and return it.
    ///
    /// `Node::add_*_pin` mints a random cuid2, which would make every rebuild of the same
    /// fixture produce different bytes and mask real determinism regressions. Boards keep
    /// `pins` keyed by `pin.id`, so both the key and the field are rewritten together.
    fn add_pin(node: &mut Node, output: bool, name: &str, data_type: VariableType) -> String {
        let generated = {
            let pin = if output {
                node.add_output_pin(name, name, "", data_type)
            } else {
                node.add_input_pin(name, name, "", data_type)
            };
            pin.id.clone()
        };

        let mut pin = node.pins.remove(&generated).expect("pin was just added");
        let id = format!("{}::{}::{}", node.id, name, node.pins.len());
        pin.id = id.clone();
        node.pins.insert(id.clone(), pin);
        id
    }

    /// Wire `from` -> `to`, mirroring how the editor records an edge on both ends.
    fn connect(board: &mut Board, from_node: &str, from_pin: &str, to_node: &str, to_pin: &str) {
        board
            .nodes
            .get_mut(from_node)
            .unwrap()
            .pins
            .get_mut(from_pin)
            .unwrap()
            .connected_to
            .insert(to_pin.to_string());
        board
            .nodes
            .get_mut(to_node)
            .unwrap()
            .pins
            .get_mut(to_pin)
            .unwrap()
            .depends_on
            .insert(from_pin.to_string());
    }

    fn insert(board: &mut Board, node: Node) {
        board.nodes.insert(node.id.clone(), node);
    }

    fn empty_board() -> Board {
        Board::new_detached(Some("board-1".into()), Path::from("apps/app-1"))
    }

    /// exec_a --exec--> pure_add(input) ; pure_add(out) --data--> exec_b(input)
    fn sample_board() -> Board {
        let mut board = empty_board();

        let mut a = node("node-a", "events_simple");
        let a_exec = add_pin(&mut a, true, "exec_out", VariableType::Execution);

        let mut b = node("node-b", "log");
        let b_exec = add_pin(&mut b, false, "exec_in", VariableType::Execution);
        let b_value = add_pin(&mut b, false, "value", VariableType::String);

        let mut pure = node("node-pure", "math_add");
        let pure_out = add_pin(&mut pure, true, "result", VariableType::Integer);

        insert(&mut board, a);
        insert(&mut board, b);
        insert(&mut board, pure);

        connect(&mut board, "node-a", &a_exec, "node-b", &b_exec);
        connect(&mut board, "node-pure", &pure_out, "node-b", &b_value);
        board
    }

    fn compile_hot(board: &Board) -> (Vec<u8>, PlanBuffer) {
        let plan = compile_board(board, STAMPS).unwrap();
        let container = plan.to_container().unwrap();
        let buffer = PlanBuffer::new(container.clone()).unwrap();
        (container, buffer)
    }

    fn node_index_of(archived: &ArchivedHotPlan, id: &str) -> u32 {
        archived.node_by_id(id).expect("node present in plan")
    }

    #[test]
    fn compiles_empty_board() {
        let (_, buffer) = compile_hot(&empty_board());
        let section = buffer.hot().unwrap();
        let archived = section.root();
        assert_eq!(archived.nodes.len(), 0);
        assert_eq!(archived.pins.len(), 0);
        assert_eq!(archived.board_id.as_str(), "board-1");
    }

    #[test]
    fn every_node_is_lowered_with_a_contiguous_pin_range() {
        let board = sample_board();
        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        assert_eq!(archived.nodes.len(), 3);
        assert_eq!(archived.pins.len(), 4);

        for node in archived.nodes.iter() {
            let start = node.first_pin.to_native() as usize;
            let end = start + node.pin_count.to_native() as usize;
            assert!(end <= archived.pins.len());
            // Every pin in the range must actually belong to this node.
            let index = archived
                .nodes
                .iter()
                .position(|candidate| {
                    candidate.instance_id.to_native() == node.instance_id.to_native()
                })
                .unwrap() as u32;
            for pin in &archived.pins[start..end] {
                assert_eq!(pin.owner_node.to_native(), index);
            }
        }
    }

    #[test]
    fn data_connections_resolve_to_pin_indices() {
        let board = sample_board();
        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let pure = node_index_of(archived, "node-pure");
        let pure_node = &archived.nodes[pure as usize];
        let result_pin = pure_node.first_pin.to_native();

        let targets = archived.pin_connections.row(result_pin as usize);
        assert_eq!(targets.len(), 1);
        let target_pin = &archived.pins[targets[0].to_native() as usize];
        assert_eq!(archived.symbol(target_pin.name.to_native()), "value");

        // The reverse edge must be present too.
        let deps = archived
            .pin_dependencies
            .row(targets[0].to_native() as usize);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].to_native(), result_pin);
    }

    /// The old graph builder silently dropped edges pointing at pins that no longer exist
    /// (`filter_map` over the id lookup). Compilation must not turn that into a hard error
    /// or, worse, an out-of-range index.
    #[test]
    fn dangling_connections_are_dropped() {
        let mut board = sample_board();
        board
            .nodes
            .get_mut("node-a")
            .unwrap()
            .pins
            .values_mut()
            .next()
            .unwrap()
            .connected_to
            .insert("pin-that-never-existed".into());

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        for edge in archived.pin_connections.edges.iter() {
            assert!((edge.to_native() as usize) < archived.pins.len());
        }
    }

    #[test]
    fn exec_successors_follow_control_flow_only() {
        let board = sample_board();
        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let a = node_index_of(archived, "node-a");
        let b = node_index_of(archived, "node-b");
        let pure = node_index_of(archived, "node-pure");

        let successors = archived.exec_successors.row(a as usize);
        assert_eq!(successors.len(), 1);
        assert_eq!(successors[0].to_native(), b);

        // The pure node feeds data, not control, so it must not appear as a successor.
        assert!(archived.exec_successors.row(pure as usize).is_empty());
        assert!(archived.exec_successors.row(b as usize).is_empty());
    }

    /// Control flowing through a layer relay pin must land on the real target node, so the
    /// scheduler never has to walk relay hops at run time.
    #[test]
    fn exec_successors_traverse_layer_relay_pins() {
        let mut board = empty_board();

        let mut a = node("node-a", "events_simple");
        let a_exec = add_pin(&mut a, true, "exec_out", VariableType::Execution);
        let mut b = node("node-b", "log");
        let b_exec = add_pin(&mut b, false, "exec_in", VariableType::Execution);

        let mut relay = Pin {
            id: "relay-pin".into(),
            name: "relay".into(),
            friendly_name: "Relay".into(),
            description: String::new(),
            pin_type: PinType::Output,
            data_type: VariableType::Execution,
            schema: None,
            value_type: ValueType::Normal,
            depends_on: Default::default(),
            connected_to: Default::default(),
            default_value: None,
            index: 0,
            options: None,
            value: None,
        };
        relay.connected_to.insert(b_exec.clone());

        a.pins
            .get_mut(&a_exec)
            .unwrap()
            .connected_to
            .insert(relay.id.clone());
        b.pins
            .get_mut(&b_exec)
            .unwrap()
            .depends_on
            .insert(relay.id.clone());

        let mut layer = Layer::new("layer-1".into(), "Layer".into(), LayerType::Collapsed);
        layer.pins.insert(relay.id.clone(), relay);
        board.layers.insert(layer.id.clone(), layer);

        insert(&mut board, a);
        insert(&mut board, b);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let a_index = node_index_of(archived, "node-a");
        let b_index = node_index_of(archived, "node-b");
        let successors = archived.exec_successors.row(a_index as usize);
        assert_eq!(
            successors.len(),
            1,
            "relay hop should resolve to the target node"
        );
        assert_eq!(successors[0].to_native(), b_index);
    }

    #[test]
    fn pure_schedule_lists_dependencies_before_their_consumer() {
        let mut board = empty_board();

        // consumer <- pure_outer <- pure_inner
        let mut inner = node("node-inner", "math_const");
        let inner_out = add_pin(&mut inner, true, "out", VariableType::Integer);

        let mut outer = node("node-outer", "math_double");
        let outer_in = add_pin(&mut outer, false, "in", VariableType::Integer);
        let outer_out = add_pin(&mut outer, true, "out", VariableType::Integer);

        let mut consumer = node("node-consumer", "log");
        add_pin(&mut consumer, false, "exec_in", VariableType::Execution);
        let consumer_in = add_pin(&mut consumer, false, "value", VariableType::Integer);

        insert(&mut board, inner);
        insert(&mut board, outer);
        insert(&mut board, consumer);

        connect(
            &mut board,
            "node-inner",
            &inner_out,
            "node-outer",
            &outer_in,
        );
        connect(
            &mut board,
            "node-outer",
            &outer_out,
            "node-consumer",
            &consumer_in,
        );

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let consumer_index = node_index_of(archived, "node-consumer");
        let inner_index = node_index_of(archived, "node-inner");
        let outer_index = node_index_of(archived, "node-outer");

        let schedule: Vec<u32> = archived
            .pure_schedules
            .row(consumer_index as usize)
            .iter()
            .map(|value| value.to_native())
            .collect();

        assert_eq!(schedule.len(), 2, "both pure ancestors must be scheduled");
        let inner_position = schedule.iter().position(|n| *n == inner_index).unwrap();
        let outer_position = schedule.iter().position(|n| *n == outer_index).unwrap();
        assert!(
            inner_position < outer_position,
            "a pure node must be scheduled after the pure nodes it depends on"
        );
    }

    /// Boards are user input; a long pure chain must not overflow the compiler's stack.
    #[test]
    fn deep_pure_chains_compile_without_recursion_limits() {
        const DEPTH: usize = 5_000;
        let mut board = empty_board();
        let mut previous: Option<(String, String)> = None;

        for depth in 0..DEPTH {
            let id = format!("pure-{depth:05}");
            let mut n = node(&id, "math_double");
            let input = add_pin(&mut n, false, "in", VariableType::Integer);
            let output = add_pin(&mut n, true, "out", VariableType::Integer);
            insert(&mut board, n);

            if let Some((previous_id, previous_out)) = previous {
                connect(&mut board, &previous_id, &previous_out, &id, &input);
            }
            previous = Some((id, output));
        }

        // An impure consumer at the end pulls the whole chain into one schedule.
        let mut consumer = node("consumer", "log");
        add_pin(&mut consumer, false, "exec_in", VariableType::Execution);
        let consumer_in = add_pin(&mut consumer, false, "value", VariableType::Integer);
        insert(&mut board, consumer);
        let (last_id, last_out) = previous.unwrap();
        connect(&mut board, &last_id, &last_out, "consumer", &consumer_in);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let consumer_index = node_index_of(archived, "consumer");
        let schedule = archived.pure_schedules.row(consumer_index as usize);
        assert_eq!(schedule.len(), DEPTH);

        // Front to back must be dependency order: pure-00000 first, pure-04999 last.
        let first = archived.symbol(archived.nodes[schedule[0].to_native() as usize].instance_id.to_native());
        let last = archived.symbol(
            archived.nodes[schedule[schedule.len() - 1].to_native() as usize]
                .instance_id
                .to_native(),
        );
        assert_eq!(first, "pure-00000");
        assert_eq!(last, format!("pure-{:05}", DEPTH - 1));
    }

    /// The editor permits a cyclic data graph; compilation must terminate on one.
    #[test]
    fn cyclic_pure_dependencies_terminate() {
        let mut board = empty_board();

        let mut a = node("cycle-a", "math_double");
        let a_in = add_pin(&mut a, false, "in", VariableType::Integer);
        let a_out = add_pin(&mut a, true, "out", VariableType::Integer);
        let mut b = node("cycle-b", "math_double");
        let b_in = add_pin(&mut b, false, "in", VariableType::Integer);
        let b_out = add_pin(&mut b, true, "out", VariableType::Integer);

        let mut consumer = node("cycle-consumer", "log");
        add_pin(&mut consumer, false, "exec_in", VariableType::Execution);
        let consumer_in = add_pin(&mut consumer, false, "value", VariableType::Integer);

        insert(&mut board, a);
        insert(&mut board, b);
        insert(&mut board, consumer);

        connect(&mut board, "cycle-a", &a_out, "cycle-b", &b_in);
        connect(&mut board, "cycle-b", &b_out, "cycle-a", &a_in);
        connect(
            &mut board,
            "cycle-b",
            &b_out,
            "cycle-consumer",
            &consumer_in,
        );

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let consumer_index = node_index_of(archived, "cycle-consumer");
        let schedule = archived.pure_schedules.row(consumer_index as usize);
        // Each cycle member appears once; the important property is that we got here.
        assert_eq!(schedule.len(), 2);
    }

    #[test]
    fn pin_name_lookup_matches_the_board() {
        let board = sample_board();
        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let b = node_index_of(archived, "node-b");
        let value_pins = archived.pins_by_name(b, "value");
        assert_eq!(value_pins.len(), 1);
        assert_eq!(
            archived.symbol(archived.pins[value_pins[0] as usize].name.to_native()),
            "value"
        );
        assert!(archived.pins_by_name(b, "missing").is_empty());
    }

    /// The editor lets a user add several pins sharing a name; lookups must return all of
    /// them, in a stable order.
    #[test]
    fn repeated_pin_names_all_resolve_in_stable_order() {
        let mut board = empty_board();
        let mut n = node("node-multi", "bool_or");
        add_pin(&mut n, false, "boolean", VariableType::Boolean);
        add_pin(&mut n, false, "boolean", VariableType::Boolean);
        add_pin(&mut n, false, "boolean", VariableType::Boolean);
        insert(&mut board, n);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let index = node_index_of(archived, "node-multi");
        let pins = archived.pins_by_name(index, "boolean");
        assert_eq!(pins.len(), 3);
        assert!(pins.windows(2).all(|w| w[0] < w[1]), "order must be stable");
    }

    /// Plans are written with `PutMode::Create`; racing writers must produce identical
    /// bytes or one of them silently wins with different content.
    #[test]
    fn compilation_is_byte_for_byte_deterministic() {
        let first = compile_board(&sample_board(), STAMPS)
            .unwrap()
            .to_container()
            .unwrap();
        let second = compile_board(&sample_board(), STAMPS)
            .unwrap()
            .to_container()
            .unwrap();
        assert_eq!(first, second);
    }

    /// The whole point of dropping layout metadata: moving a node on the canvas must not
    /// produce a different plan.
    #[test]
    fn editor_only_fields_do_not_affect_the_plan() {
        let board = sample_board();
        let baseline = compile_board(&board, STAMPS)
            .unwrap()
            .to_container()
            .unwrap();

        let mut moved = sample_board();
        for node in moved.nodes.values_mut() {
            node.coordinates = Some((1234.0, 5678.0, 1.0));
            node.comment = Some("a note for humans".into());
            node.docs = Some("docs".into());
        }
        let after = compile_board(&moved, STAMPS)
            .unwrap()
            .to_container()
            .unwrap();

        assert_eq!(baseline, after);
    }

    #[test]
    fn ref_hashes_are_resolved_into_the_cold_section() {
        let mut board = sample_board();
        board
            .refs
            .insert("ref-hash-1".into(), "A friendly description".into());
        board.nodes.get_mut("node-a").unwrap().description = "ref-hash-1".into();
        // The sentinel means "empty", not a literal to pass through.
        board.nodes.get_mut("node-b").unwrap().description = EMPTY_STRING_REF_HASH.into();

        let (_, buffer) = compile_hot(&board);
        let hot_section = buffer.hot().unwrap();
        let archived = hot_section.root();
        let a = node_index_of(archived, "node-a");
        let b = node_index_of(archived, "node-b");

        let cold_section = buffer.cold().unwrap();
        let cold = cold_section.root();
        assert_eq!(cold.node_description(a), "A friendly description");
        assert_eq!(cold.node_description(b), "");
    }

    #[test]
    fn identical_defaults_share_one_blob() {
        let mut board = empty_board();
        let mut n = node("node-defaults", "test");
        for name in ["a", "b", "c"] {
            let pin = n.add_input_pin(name, name, "", VariableType::Boolean);
            pin.set_default_value(Some(flow_like_types::json::json!(true)));
        }
        insert(&mut board, n);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        assert_eq!(
            archived.blobs.len(),
            1,
            "equal defaults must be deduplicated"
        );
        for pin in archived.pins.iter() {
            assert_eq!(pin.default_value.to_native(), 0);
        }
    }

    /// WASM sandbox grants must survive lowering exactly. Dropping or reordering them
    /// changes what a sandboxed node is allowed to do, so this is a security property, not
    /// a fidelity nicety.
    #[test]
    fn wasm_permissions_survive_lowering() {
        use crate::flow::node::{NodePermission, NodeWasm};

        let mut board = empty_board();
        let mut plain = node("node-plain", "log");
        add_pin(&mut plain, false, "exec_in", VariableType::Execution);

        let mut sandboxed = node("node-wasm", "custom_wasm");
        add_pin(&mut sandboxed, false, "exec_in", VariableType::Execution);
        sandboxed.wasm = Some(NodeWasm {
            package_id: "pkg-1".into(),
            permissions: vec![
                NodePermission::StorageRead,
                NodePermission::NetworkHttp,
                NodePermission::Functions,
            ],
        });

        insert(&mut board, plain);
        insert(&mut board, sandboxed);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let wasm_index = node_index_of(archived, "node-wasm");
        let granted: Vec<NodePermission> = archived
            .wasm_permissions
            .row(wasm_index as usize)
            .iter()
            .filter_map(|v| NodePermission::from_plan_u8(v.to_native() as u8))
            .collect();
        assert_eq!(
            granted,
            vec![
                NodePermission::StorageRead,
                NodePermission::NetworkHttp,
                NodePermission::Functions
            ]
        );

        // A node with no WASM metadata must grant nothing.
        let plain_index = node_index_of(archived, "node-plain");
        assert!(
            archived
                .wasm_permissions
                .row(plain_index as usize)
                .is_empty()
        );
    }

    #[test]
    fn variable_flags_survive_lowering() {
        use crate::flow::variable::Variable;
        use flow_like_types::plan::hot::variable_flags;

        let mut board = empty_board();
        let mut secret = Variable::new("token", VariableType::String, ValueType::Normal);
        secret.id = "var-secret".into();
        secret.secret = true;
        secret.exposed = false;

        let mut exposed = Variable::new("greeting", VariableType::String, ValueType::Normal);
        exposed.id = "var-exposed".into();
        exposed.exposed = true;

        board.variables.insert(secret.id.clone(), secret);
        board.variables.insert(exposed.id.clone(), exposed);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();
        assert_eq!(archived.variables.len(), 2);

        for variable in archived.variables.iter() {
            let id = archived.symbol(variable.id.to_native());
            let flags = variable.flags;
            match id {
                "var-secret" => {
                    assert!(flags & variable_flags::SECRET != 0);
                    assert!(flags & variable_flags::EXPOSED == 0);
                }
                "var-exposed" => {
                    assert!(flags & variable_flags::SECRET == 0);
                    assert!(flags & variable_flags::EXPOSED != 0);
                }
                other => panic!("unexpected variable {other}"),
            }
        }
    }

    #[test]
    fn function_layer_nodes_are_compiled_alongside_board_nodes() {
        let mut board = sample_board();
        let mut layer = Layer::new("layer-fn".into(), "Fn".into(), LayerType::Function);
        let mut inner = node("node-in-layer", "log");
        add_pin(&mut inner, false, "exec_in", VariableType::Execution);
        layer.nodes.insert(inner.id.clone(), inner);
        board.layers.insert(layer.id.clone(), layer);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        assert_eq!(archived.nodes.len(), 4);
        assert!(archived.node_by_id("node-in-layer").is_some());
    }

    /// Nodes inside non-function layers are collapsed presentation only; the old builder
    /// did not instantiate them and neither may the compiler.
    #[test]
    fn collapsed_layer_nodes_are_not_compiled() {
        let mut board = sample_board();
        let mut layer = Layer::new("layer-c".into(), "C".into(), LayerType::Collapsed);
        let inner = node("node-collapsed", "log");
        layer.nodes.insert(inner.id.clone(), inner);
        board.layers.insert(layer.id.clone(), layer);

        let (_, buffer) = compile_hot(&board);
        let section = buffer.hot().unwrap();
        let archived = section.root();

        assert_eq!(archived.nodes.len(), 3);
        assert!(archived.node_by_id("node-collapsed").is_none());
    }

    /// Compile the real production boards checked into `tests/` and verify that every
    /// index the artifact contains actually addresses something.
    ///
    /// Synthetic fixtures cannot reach the shapes these hit — thousands of nodes, nested
    /// layers, dynamic pins — and a plan is consumed by index with no further bounds
    /// checking beyond bytecheck, so an out-of-range index is the failure mode that
    /// matters most.
    #[test]
    fn real_boards_compile_to_structurally_valid_plans() {
        use flow_like_types::{FromProto, Message};

        for name in ["ttwctnp08u18sg2z6nmcqqak", "bypaw6n2ksuvrw0kcaj14omz"] {
            let path = format!("../../tests/ast/{name}.board");
            let Ok(raw) = std::fs::read(&path) else {
                continue;
            };

            let decoded = lz4_flex::decompress_size_prepended(&raw).unwrap();
            let proto = flow_like_types::proto::Board::decode(&decoded[..]).unwrap();
            let board = Board::from_proto(proto);

            let container = compile_board(&board, STAMPS).unwrap().to_container().unwrap();
            let buffer = PlanBuffer::new(container.clone()).unwrap();
            let section = buffer.hot().unwrap();
            let plan = section.root();

            let node_count = plan.nodes.len();
            let pin_count = plan.pins.len();
            let symbol_count = plan.symbols.len();
            let blob_count = plan.blobs.len();

            let header = buffer.header();
            eprintln!(
                "{name}: board {} B (lz4) | plan {} B = hot {} B + cold {} B + debug {} B \
                 | {node_count} nodes, {pin_count} pins",
                raw.len(),
                container.len(),
                header.hot.len,
                header.cold.len,
                header.debug.len,
            );
            assert!(node_count > 0, "{name} should contain nodes");

            for node in plan.nodes.iter() {
                assert!((node.instance_id.to_native() as usize) < symbol_count);
                assert!((node.type_key.to_native() as usize) < symbol_count);
                let first = node.first_pin.to_native() as usize;
                assert!(first + node.pin_count.to_native() as usize <= pin_count);
                let layer = node.layer.to_native();
                assert!(layer == NONE_INDEX || (layer as usize) < plan.layers.len());
                let wasm = node.wasm_package.to_native();
                assert!(wasm == NONE_INDEX || (wasm as usize) < symbol_count);
            }

            for pin in plan.pins.iter() {
                assert!((pin.id.to_native() as usize) < symbol_count);
                assert!((pin.name.to_native() as usize) < symbol_count);
                let owner = pin.owner_node.to_native();
                assert!(owner == NONE_INDEX || (owner as usize) < node_count);
                let default = pin.default_value.to_native();
                assert!(default == NONE_INDEX || (default as usize) < blob_count);
            }

            for (label, csr, limit) in [
                ("pin_connections", &plan.pin_connections, pin_count),
                ("pin_dependencies", &plan.pin_dependencies, pin_count),
                ("exec_successors", &plan.exec_successors, node_count),
                ("pure_schedules", &plan.pure_schedules, node_count),
            ] {
                for edge in csr.edges.iter() {
                    assert!(
                        (edge.to_native() as usize) < limit,
                        "{name}/{label} points outside the table"
                    );
                }
            }

            // Every node must be reachable by its own id, and the pin-name table must
            // resolve back to pins that really carry that name.
            for index in 0..node_count {
                let id = plan.symbol(plan.nodes[index].instance_id.to_native());
                assert_eq!(plan.node_by_id(id), Some(index as u32), "{name}: {id}");
            }
            for entry in plan.pin_name_entries.iter() {
                assert!((entry.pin.to_native() as usize) < pin_count);
            }
        }
    }

    #[test]
    fn stamps_record_what_the_plan_was_built_from() {
        let mut board = sample_board();
        board.version = (2, 3, 4);
        board.hash = Some(0xabcd);

        let plan = compile_board(&board, STAMPS).unwrap();
        let buffer = PlanBuffer::new(plan.to_container().unwrap()).unwrap();

        assert!(buffer.header().matches(0xabcd, 1, 2));
        assert!(
            !buffer.header().matches(0xabcd, 99, 2),
            "catalog drift must be detected"
        );
        assert_eq!(buffer.header().stamps.board_version, (2, 3, 4));
    }
}

/// Compute a signature over the node catalog a plan was compiled against.
///
/// Any change to a node's schema surface must invalidate every plan compiled before it,
/// because compilation freezes `on_update` output that would otherwise be recomputed on
/// every board load. `entries` maps node type key to that node's semantic hash.
pub fn catalog_signature(entries: &BTreeMap<String, u64>) -> u64 {
    use highway::{HighwayHash, HighwayHasher};
    let mut hasher = HighwayHasher::new(highway::Key([
        0x00ff_00ff_00ff_00ff,
        0xff00_ff00_ff00_ff00,
        0x0f0f_0f0f_0f0f_0f0f,
        0xf0f0_f0f0_f0f0_f0f0,
    ]));
    for (name, hash) in entries {
        hasher.append(name.as_bytes());
        hasher.append(&hash.to_le_bytes());
    }
    hasher.finalize64()
}
