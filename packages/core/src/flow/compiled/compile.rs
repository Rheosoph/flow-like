//! Board → CompiledBoard.
//!
//! The input board must be fully prepared (post `node_updates` + `cleanup`,
//! i.e. what `Board::from_loaded_proto` returns): compilation bakes that state
//! in, and the compiled artifact never runs `on_update` again.
//!
//! Reroute nodes are pure wire-bends; they are spliced out here so the
//! executed graph never pays their hop. Splicing rewrites the surviving pins'
//! adjacency on the id level before arena indices are assigned, and handles
//! reroute chains because every splice updates the shared working adjacency.

use super::codes;
use super::format::{
    CompiledBoard, CompiledFnRefs, CompiledLayer, CompiledLayerCache, CompiledNode,
    CompiledNodeWasm, CompiledPin, CompiledPinOptions, CompiledVariable, NONE_IDX,
};
use crate::flow::board::{Board, Layer, LayerType};
use crate::flow::node::Node;
use crate::flow::pin::{Pin, PinOptions};
use crate::flow::variable::Variable;
use crate::state::FlowNodeRegistryInner;
use ahash::{AHashMap, AHashSet};
use flow_like_types::{Result, anyhow};
use std::collections::BTreeSet;

const REROUTE_NODE_NAME: &str = "reroute";

struct WorkingPin<'a> {
    pin: &'a Pin,
    depends_on: BTreeSet<String>,
    connected_to: BTreeSet<String>,
    owner_node: Option<&'a str>,
    /// Catalog type (`Node.name`) of the owning node — keys the default used
    /// for metadata interning. `None` for layer relay pins.
    owner_node_type: Option<&'a str>,
    owner_layer: Option<&'a str>,
}

/// `None` when the value matches the catalog default byte-for-byte — the
/// reconstruction reinflates it from the registry the artifact is
/// fingerprint-bound to. Anything else (user edits, missing default) is
/// stored verbatim.
fn intern(value: &str, default_value: Option<&str>) -> Option<String> {
    match default_value {
        Some(default_value) if default_value == value => None,
        _ => Some(value.to_string()),
    }
}

/// The single catalog-default pin matching (name, direction), or `None` when
/// the match is absent or ambiguous (multi-pins) — ambiguity disables
/// interning for that pin entirely.
fn default_pin<'a>(default_node: Option<&'a Node>, pin: &Pin) -> Option<&'a Pin> {
    let default_node = default_node?;
    let mut candidates = default_node
        .pins
        .values()
        .filter(|p| p.name == pin.name && p.pin_type == pin.pin_type);
    let first = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    Some(first)
}

/// Compile without a catalog: every metadata string is stored verbatim.
/// Prefer [`compile_board_with_catalog`] wherever a registry is available —
/// it produces smaller artifacts by interning catalog-default metadata.
pub fn compile_board(board: &Board) -> Result<CompiledBoard> {
    compile_board_inner(board, None)
}

/// Compile against the registry the artifact will be fingerprint-bound to,
/// interning node/pin metadata that matches the catalog defaults.
pub fn compile_board_with_catalog(
    board: &Board,
    catalog: &FlowNodeRegistryInner,
) -> Result<CompiledBoard> {
    compile_board_inner(board, Some(catalog))
}

fn compile_board_inner(
    board: &Board,
    catalog: Option<&FlowNodeRegistryInner>,
) -> Result<CompiledBoard> {
    let mut layer_idx: AHashMap<&str, u32> = AHashMap::with_capacity(board.layers.len());
    for (i, layer_id) in sorted_keys(&board.layers).iter().enumerate() {
        layer_idx.insert(layer_id.as_str(), i as u32);
    }

    // Working adjacency over every pin in the board (node pins, function-layer
    // body pins, layer relay pins), keyed by pin id. Only Function-layer
    // bodies execute (matches the engine's LayerType filter) — collapsed
    // layers keep their nodes in `board.nodes`, and any stale copies inside
    // `layer.nodes` must not overwrite the live adjacency.
    let mut working: AHashMap<&str, WorkingPin> = AHashMap::new();
    let mut reroute_node_ids: AHashSet<&str> = AHashSet::new();
    let mut reroute_pin_pairs: Vec<(String, String)> = Vec::new();

    for (node_id, node) in &board.nodes {
        collect_node(
            &mut working,
            &mut reroute_node_ids,
            &mut reroute_pin_pairs,
            node_id,
            node,
        );
    }
    for layer in board.layers.values() {
        if !matches!(layer.r#type, LayerType::Function) {
            continue;
        }
        for (node_id, node) in &layer.nodes {
            collect_node(
                &mut working,
                &mut reroute_node_ids,
                &mut reroute_pin_pairs,
                node_id,
                node,
            );
        }
    }
    for layer in board.layers.values() {
        for pin in layer.pins.values() {
            working.entry(pin_key(pin)).or_insert(WorkingPin {
                pin,
                depends_on: pin.depends_on.clone(),
                connected_to: pin.connected_to.clone(),
                owner_node: None,
                owner_node_type: None,
                owner_layer: Some(layer.id.as_str()),
            });
        }
    }

    splice_reroutes(&mut working, &reroute_pin_pairs);

    // Arena assignment. Deterministic order: board nodes by id, then function
    // layer bodies by (layer id, node id), pins within a node by (index, id),
    // then layer relay pins by (layer id, pin index, pin id). Pins already
    // registered for a node are not re-registered for a layer (old layer
    // format relays a node pin directly).
    let mut arena = Arena {
        idx: AHashMap::with_capacity(working.len()),
        order: Vec::with_capacity(working.len()),
    };

    let mut compiled_nodes: Vec<CompiledNode> = Vec::new();

    for node_id in sorted_keys(&board.nodes) {
        if reroute_node_ids.contains(node_id.as_str()) {
            continue;
        }
        let node = &board.nodes[node_id];
        compiled_nodes.push(compile_node(
            &working, &mut arena, &layer_idx, catalog, node, NONE_IDX,
        ));
    }
    for layer_id in sorted_keys(&board.layers) {
        let layer = &board.layers[layer_id];
        if !matches!(layer.r#type, LayerType::Function) {
            continue;
        }
        let body_idx = layer_idx[layer_id.as_str()];
        for node_id in sorted_keys(&layer.nodes) {
            if reroute_node_ids.contains(node_id.as_str()) {
                continue;
            }
            let node = &layer.nodes[node_id];
            compiled_nodes.push(compile_node(
                &working, &mut arena, &layer_idx, catalog, node, body_idx,
            ));
        }
    }

    let mut compiled_layers: Vec<CompiledLayer> = Vec::with_capacity(board.layers.len());
    for layer_id in sorted_keys(&board.layers) {
        let layer = &board.layers[layer_id];
        let mut relay_indices = Vec::with_capacity(layer.pins.len());
        for pin in sorted_pins(&layer.pins) {
            let idx = arena.assign(&working, pin);
            if idx != NONE_IDX {
                relay_indices.push(idx);
            }
        }
        compiled_layers.push(compile_layer(layer, &layer_idx, relay_indices));
    }

    // Node indices are only known now; owner_node needs them.
    let mut node_arena_idx: AHashMap<&str, u32> = AHashMap::with_capacity(compiled_nodes.len());
    for (i, node) in compiled_nodes.iter().enumerate() {
        node_arena_idx.insert(node.id.as_str(), i as u32);
    }

    let mut compiled_pins: Vec<CompiledPin> = Vec::with_capacity(arena.order.len());
    for key in &arena.order {
        let wp = working
            .get(key)
            .ok_or_else(|| anyhow!("pin {key} vanished from working set during compilation"))?;
        compiled_pins.push(compile_pin(
            wp,
            &arena.idx,
            &node_arena_idx,
            &layer_idx,
            catalog,
        ));
    }

    let mut refs: Vec<(String, String)> = board
        .refs
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    refs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut variables: Vec<CompiledVariable> = Vec::with_capacity(board.variables.len());
    for variable_id in sorted_keys(&board.variables) {
        variables.push(compile_variable(&board.variables[variable_id]));
    }

    Ok(CompiledBoard {
        id: board.id.clone(),
        name: board.name.clone(),
        version: board.version,
        stage: codes::stage_code(&board.stage),
        log_level: codes::log_level_code(&board.log_level),
        execution_mode: codes::execution_mode_code(&board.execution_mode),
        refs,
        page_ids: {
            let mut ids = board.page_ids.clone();
            ids.sort();
            ids
        },
        variables,
        layers: compiled_layers,
        nodes: compiled_nodes,
        pins: compiled_pins,
    })
}

fn pin_key(pin: &Pin) -> &str {
    pin.id.as_str()
}

fn register_node_pins<'a>(
    working: &mut AHashMap<&'a str, WorkingPin<'a>>,
    node_id: &'a str,
    node: &'a Node,
) {
    for pin in node.pins.values() {
        working.insert(
            pin_key(pin),
            WorkingPin {
                pin,
                depends_on: pin.depends_on.clone(),
                connected_to: pin.connected_to.clone(),
                owner_node: Some(node_id),
                owner_node_type: Some(node.name.as_str()),
                owner_layer: None,
            },
        );
    }
}

struct Arena<'a> {
    idx: AHashMap<&'a str, u32>,
    order: Vec<&'a str>,
}

impl<'a> Arena<'a> {
    /// Assign (or return) the arena index for a pin. Pins registered under a
    /// node earlier keep their index when a layer references them again (old
    /// layer format relays node pins directly).
    fn assign(&mut self, working: &AHashMap<&'a str, WorkingPin<'a>>, pin: &Pin) -> u32 {
        if let Some(existing) = self.idx.get(pin.id.as_str()) {
            return *existing;
        }
        let Some((key, _)) = working.get_key_value(pin.id.as_str()) else {
            return NONE_IDX;
        };
        let idx = self.order.len() as u32;
        self.idx.insert(key, idx);
        self.order.push(key);
        idx
    }
}

fn compile_node<'a>(
    working: &AHashMap<&'a str, WorkingPin<'a>>,
    arena: &mut Arena<'a>,
    layer_idx: &AHashMap<&str, u32>,
    catalog: Option<&FlowNodeRegistryInner>,
    node: &Node,
    body_layer: u32,
) -> CompiledNode {
    let mut pin_indices = Vec::with_capacity(node.pins.len());
    for pin in sorted_pins(&node.pins) {
        let idx = arena.assign(working, pin);
        if idx != NONE_IDX {
            pin_indices.push(idx);
        }
    }
    let default_node = catalog
        .and_then(|c| c.registry.get(&node.name))
        .map(|(default_node, _)| default_node);
    CompiledNode {
        id: node.id.clone(),
        name: node.name.clone(),
        friendly_name: intern(
            &node.friendly_name,
            default_node.map(|d| d.friendly_name.as_str()),
        ),
        description: intern(
            &node.description,
            default_node.map(|d| d.description.as_str()),
        ),
        category: intern(&node.category, default_node.map(|d| d.category.as_str())),
        start: node.start.unwrap_or(false),
        long_running: node.long_running.unwrap_or(false),
        event_callback: node.event_callback.unwrap_or(false),
        only_offline: node.only_offline,
        node_version: node.version.unwrap_or(u32::MAX),
        layer: node
            .layer
            .as_deref()
            .and_then(|l| layer_idx.get(l).copied())
            .unwrap_or(NONE_IDX),
        body_layer,
        pins: pin_indices,
        fn_refs: node.fn_refs.as_ref().map(|f| CompiledFnRefs {
            fn_refs: f.fn_refs.clone(),
            can_reference_fns: f.can_reference_fns,
            can_be_referenced_by_fns: f.can_be_referenced_by_fns,
        }),
        oauth_providers: node.oauth_providers.clone().unwrap_or_default(),
        required_oauth_scopes: node
            .required_oauth_scopes
            .as_ref()
            .map(|m| {
                let mut entries: Vec<(String, Vec<String>)> =
                    m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                entries
            })
            .unwrap_or_default(),
        wasm: node.wasm.as_ref().map(|w| CompiledNodeWasm {
            package_id: w.package_id.clone(),
            permissions: w
                .permissions
                .iter()
                .map(codes::node_permission_code)
                .collect(),
        }),
    }
}

fn collect_node<'a>(
    working: &mut AHashMap<&'a str, WorkingPin<'a>>,
    reroute_node_ids: &mut AHashSet<&'a str>,
    reroute_pin_pairs: &mut Vec<(String, String)>,
    node_id: &'a str,
    node: &'a Node,
) {
    register_node_pins(working, node_id, node);
    if let Some(pair) = splicable_reroute(node) {
        reroute_node_ids.insert(node_id);
        reroute_pin_pairs.push(pair);
    }
}

/// The (input pin id, output pin id) of a reroute that may be spliced out.
///
/// Not splicable — the node is kept and executed like any other — when it is
/// a WASM node that merely shares the name, when its pin shape is not the
/// catalog reroute's exactly-one-input/one-output, or when its input carries a
/// default literal with no upstream (the reroute then acts as a value source
/// the splice would silently drop).
fn splicable_reroute(node: &Node) -> Option<(String, String)> {
    if node.name != REROUTE_NODE_NAME || node.wasm.is_some() || node.pins.len() != 2 {
        return None;
    }
    let mut input = None;
    let mut output = None;
    for pin in node.pins.values() {
        match pin.pin_type {
            crate::flow::pin::PinType::Input => input = Some(pin),
            crate::flow::pin::PinType::Output => output = Some(pin),
        }
    }
    let (input, output) = (input?, output?);
    if input.depends_on.is_empty() && input.default_value.is_some() {
        return None;
    }
    Some((input.id.clone(), output.id.clone()))
}

fn splice_reroutes(working: &mut AHashMap<&str, WorkingPin>, reroutes: &[(String, String)]) {
    for (in_pin_id, out_pin_id) in reroutes {
        let upstream: Vec<String> = working
            .get(in_pin_id.as_str())
            .map(|p| p.depends_on.iter().cloned().collect())
            .unwrap_or_default();
        let downstream: Vec<String> = working
            .get(out_pin_id.as_str())
            .map(|p| p.connected_to.iter().cloned().collect())
            .unwrap_or_default();

        for up_id in &upstream {
            if let Some(up) = working.get_mut(up_id.as_str()) {
                up.connected_to.remove(in_pin_id);
                up.connected_to.extend(downstream.iter().cloned());
            }
        }
        for down_id in &downstream {
            if let Some(down) = working.get_mut(down_id.as_str()) {
                down.depends_on.remove(out_pin_id);
                down.depends_on.extend(upstream.iter().cloned());
            }
        }
        if let Some(p) = working.get_mut(in_pin_id.as_str()) {
            p.depends_on.clear();
        }
        if let Some(p) = working.get_mut(out_pin_id.as_str()) {
            p.connected_to.clear();
        }
    }
}

fn compile_pin(
    wp: &WorkingPin,
    arena_idx: &AHashMap<&str, u32>,
    node_arena_idx: &AHashMap<&str, u32>,
    layer_idx: &AHashMap<&str, u32>,
    catalog: Option<&FlowNodeRegistryInner>,
) -> CompiledPin {
    let map_edges = |edges: &BTreeSet<String>| -> Vec<u32> {
        let mut mapped: Vec<u32> = edges
            .iter()
            .filter_map(|id| arena_idx.get(id.as_str()).copied())
            .collect();
        mapped.sort_unstable();
        mapped
    };

    let pin = wp.pin;
    let default_node = wp
        .owner_node_type
        .and_then(|name| catalog.and_then(|c| c.registry.get(name)))
        .map(|(default_node, _)| default_node);
    let catalog_pin = default_pin(default_node, pin);
    CompiledPin {
        id: pin.id.clone(),
        name: pin.name.clone(),
        friendly_name: intern(
            &pin.friendly_name,
            catalog_pin.map(|p| p.friendly_name.as_str()),
        ),
        description: intern(
            &pin.description,
            catalog_pin.map(|p| p.description.as_str()),
        ),
        pin_type: codes::pin_type_code(&pin.pin_type),
        data_type: codes::variable_type_code(&pin.data_type),
        value_type: codes::value_type_code(&pin.value_type),
        schema: pin.schema.clone(),
        options: pin.options.as_ref().map(compile_pin_options),
        default_value: pin.default_value.clone(),
        index: pin.index,
        owner_node: wp
            .owner_node
            .and_then(|id| node_arena_idx.get(id).copied())
            .unwrap_or(NONE_IDX),
        owner_layer: wp
            .owner_layer
            .and_then(|id| layer_idx.get(id).copied())
            .unwrap_or(NONE_IDX),
        depends_on: map_edges(&wp.depends_on),
        connected_to: map_edges(&wp.connected_to),
    }
}

fn compile_pin_options(options: &PinOptions) -> CompiledPinOptions {
    CompiledPinOptions {
        sensitive: options.sensitive,
        valid_values: options.valid_values.clone(),
        range: options.range,
        step: options.step,
        enforce_schema: options.enforce_schema,
        enforce_generic_value_type: options.enforce_generic_value_type,
    }
}

fn compile_variable(variable: &Variable) -> CompiledVariable {
    CompiledVariable {
        id: variable.id.clone(),
        name: variable.name.clone(),
        default_value: variable.default_value.clone(),
        data_type: codes::variable_type_code(&variable.data_type),
        value_type: codes::value_type_code(&variable.value_type),
        exposed: variable.exposed,
        secret: variable.secret,
        editable: variable.editable,
        runtime_configured: variable.runtime_configured,
        schema: variable.schema.clone(),
    }
}

fn compile_layer(
    layer: &Layer,
    layer_idx: &AHashMap<&str, u32>,
    relay_indices: Vec<u32>,
) -> CompiledLayer {
    let mut variables: Vec<CompiledVariable> = Vec::with_capacity(layer.variables.len());
    for variable_id in sorted_keys(&layer.variables) {
        variables.push(compile_variable(&layer.variables[variable_id]));
    }
    CompiledLayer {
        id: layer.id.clone(),
        name: layer.name.clone(),
        parent_layer: layer
            .parent_id
            .as_deref()
            .and_then(|p| layer_idx.get(p).copied())
            .unwrap_or(NONE_IDX),
        layer_type: codes::layer_type_code(&layer.r#type),
        variables,
        pins: relay_indices,
        cache: layer.cache.as_ref().map(|c| CompiledLayerCache {
            enabled: c.enabled,
            prefix: c.prefix.clone(),
            ttl_seconds: c.ttl_seconds,
            scope: codes::layer_cache_scope_code(&c.scope),
        }),
    }
}

fn sorted_keys<V>(map: &std::collections::HashMap<String, V>) -> Vec<&String> {
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort();
    keys
}

fn sorted_pins(pins: &std::collections::HashMap<String, Pin>) -> Vec<&Pin> {
    let mut sorted: Vec<&Pin> = pins.values().collect();
    sorted.sort_by(|a, b| a.index.cmp(&b.index).then_with(|| a.id.cmp(&b.id)));
    sorted
}
