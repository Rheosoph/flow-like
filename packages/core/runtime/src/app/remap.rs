//! Moving an app's artifacts into a fresh id space.
//!
//! Forking an app (server) and duplicating one on the device (desktop) both
//! re-key every board, node, pin, layer, event, page and widget, and then have
//! to rewrite every reference to those ids — pin wiring, pin defaults, event
//! targets and the payloads inside pages and widgets. This module owns that
//! rewrite so the two callers cannot drift. Callers own the policy: which ids
//! get allocated, which artifacts travel, and whether secrets are stripped.

use crate::a2ui::{id_refs::IdRef, page_remap};
use crate::flow::board::Board;
use flow_like_types::proto;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const DERIVED_ID_LENGTH: usize = 24;
const DERIVED_ID_ALPHABET: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";

/// The destination id for `source_id` inside the id space identified by `seed`.
///
/// Deterministic, so re-running any step of a fork after a crash or a lost
/// commit produces the very same ids. The output has the shape of
/// [`flow_like_types::create_id`] (a leading letter followed by base-36
/// digits, 24 characters) so nothing downstream can tell the two apart.
pub fn derive_id(seed: &str, source_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(seed.as_bytes());
    hasher.update(&[0]);
    hasher.update(source_id.as_bytes());
    let hash = hasher.finalize();
    let bytes = hash.as_bytes();

    let mut id = String::with_capacity(DERIVED_ID_LENGTH);
    id.push((b'a' + bytes[0] % 26) as char);
    let mut body = u128::from_be_bytes(bytes[1..17].try_into().expect("16 hash bytes"));
    for _ in 1..DERIVED_ID_LENGTH {
        id.push(DERIVED_ID_ALPHABET[(body % 36) as usize] as char);
        body /= 36;
    }
    id
}

/// Mapping table built while moving an app into a new id space. Returned to
/// callers and persisted on the user's enrollment so the server can later
/// translate original-app IDs (referenced by lesson payloads, app refs, etc.)
/// into the user-specific IDs in their forked copy.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ForkIdMap {
    pub source_app_id: String,
    pub app_id: String,
    /// Board IDs: source -> destination
    pub boards: HashMap<String, String>,
    /// Node IDs: source -> destination (flat across boards & layers)
    pub nodes: HashMap<String, String>,
    /// Pin IDs: source -> destination (flat across boards, layers, nodes)
    pub pins: HashMap<String, String>,
    /// Event IDs: source -> destination
    pub events: HashMap<String, String>,
    /// Page IDs: source -> destination
    pub pages: HashMap<String, String>,
    /// Layer IDs: source -> destination
    pub layers: HashMap<String, String>,
    /// Widget IDs: source -> destination
    #[serde(default)]
    pub widgets: HashMap<String, String>,
    /// Template IDs: source -> destination
    #[serde(default)]
    pub templates: HashMap<String, String>,
    /// Variable IDs: source -> destination (currently identity-mapped —
    /// pins do not directly reference variable IDs in the proto today,
    /// so the map is reserved for future use and reporting)
    #[serde(default)]
    pub variables: HashMap<String, String>,
    /// Role IDs: source -> destination. Includes both the system roles
    /// (Owner / Admin / Member) and any custom roles copied from source.
    #[serde(default)]
    pub roles: HashMap<String, String>,
    /// Seed every destination id in this map is derived from
    /// ([`derive_id`]). Never leaves the process.
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub seed: String,
}

impl ForkIdMap {
    /// The destination id for `src` in this id space.
    pub fn mint(&self, src: &str) -> String {
        derive_id(&self.seed, src)
    }

    /// The map without its node / pin / layer / variable entries, which run
    /// to thousands of pairs and are derivable from the top-level ids.
    pub fn top_level(&self) -> Self {
        Self {
            nodes: HashMap::new(),
            pins: HashMap::new(),
            layers: HashMap::new(),
            variables: HashMap::new(),
            ..self.clone()
        }
    }

    pub fn translate_board(&self, src: &str) -> String {
        self.boards
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
    pub fn translate_node(&self, src: &str) -> String {
        self.nodes
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
    pub fn translate_event(&self, src: &str) -> String {
        self.events
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
    pub fn translate_page(&self, src: &str) -> String {
        self.pages
            .get(src)
            .cloned()
            .unwrap_or_else(|| src.to_string())
    }
}

/// Re-key a board (or template, or archived board version) and every
/// reference inside it. Secret variable values are kept — callers crossing a
/// security boundary follow up with [`strip_board_secrets`].
///
/// Validation runs before any map is touched, so a board this build cannot
/// read leaves `maps` exactly as it was.
pub fn remap_board(
    mut board: proto::Board,
    maps: &mut ForkIdMap,
) -> flow_like_types::Result<proto::Board> {
    Board::validate_proto_types(&board)?;
    board.format_version = Board::required_proto_format_version(&board);
    // Host receipts belong to the source board's persistence boundary and must never be copied
    // into a fork where their identities and replay claims are invalid.
    board.internal_refs.clear();
    let new_board_id = maps
        .boards
        .get(&board.id)
        .cloned()
        .unwrap_or_else(|| maps.mint(&board.id));
    maps.boards
        .entry(board.id.clone())
        .or_insert_with(|| new_board_id.clone());

    register_board_ids(&board, maps);

    let mut new_nodes = HashMap::with_capacity(board.nodes.len());
    for (_, mut node) in board.nodes.drain() {
        rewrite_node(&mut node, maps);
        new_nodes.insert(node.id.clone(), node);
    }
    board.nodes = new_nodes;

    let mut new_layers = HashMap::with_capacity(board.layers.len());
    for (_, mut layer) in board.layers.drain() {
        let new_layer_id = maps
            .layers
            .get(&layer.id)
            .cloned()
            .unwrap_or_else(|| maps.mint(&layer.id));
        layer.id = new_layer_id.clone();
        if let Some(parent) = layer.parent_id.as_ref() {
            layer.parent_id = maps.layers.get(parent).cloned();
        }
        let mut layer_nodes = HashMap::with_capacity(layer.nodes.len());
        for (_, mut node) in layer.nodes.drain() {
            rewrite_node(&mut node, maps);
            layer_nodes.insert(node.id.clone(), node);
        }
        layer.nodes = layer_nodes;

        let mut layer_pins = HashMap::with_capacity(layer.pins.len());
        for (_, mut pin) in layer.pins.drain() {
            rewrite_pin_top(&mut pin, maps);
            layer_pins.insert(pin.id.clone(), pin);
        }
        layer.pins = layer_pins;
        new_layers.insert(new_layer_id, layer);
    }
    board.layers = new_layers;

    board.id = new_board_id;
    board.page_ids = board
        .page_ids
        .iter()
        .map(|p| maps.translate_page(p))
        .collect();

    Ok(board)
}

/// Clears `default_value` on every variable marked `secret = true`, both at
/// board level and inside each layer. Secrets must never travel into a
/// fork — even when the caller is the source-app owner — because the
/// destination may live in a different security boundary (different org,
/// different deployment, anonymous public download).
pub fn strip_board_secrets(board: &mut proto::Board) {
    strip_secret_values(&mut board.variables);
    for layer in board.layers.values_mut() {
        strip_secret_values(&mut layer.variables);
    }
}

/// Allocate destination ids for every node, pin and layer of `board` without
/// rewriting it. [`remap_board`] does this for its own board; a caller that
/// registers every board up front also resolves references *between* boards
/// (a pin default naming a node elsewhere) regardless of remap order.
///
/// Layers must be registered before any node is rewritten — nodes inside a
/// function/collapsed layer carry `node.layer = Some(layer_id)`, and a layer
/// missing from `maps.layers` at rewrite time would orphan the node from its
/// function and empty the layer when the desktop rebuilds `layer.nodes`.
pub fn register_board_ids(board: &proto::Board, maps: &mut ForkIdMap) {
    register_node_pin_ids(&board.nodes, maps);
    for layer in board.layers.values() {
        let layer_id = maps.mint(&layer.id);
        maps.layers.entry(layer.id.clone()).or_insert(layer_id);
        if let Some(parent) = layer.parent_id.as_ref() {
            let parent_id = maps.mint(parent);
            maps.layers.entry(parent.clone()).or_insert(parent_id);
        }
        register_node_pin_ids(&layer.nodes, maps);
        register_pin_ids(&layer.pins, maps);
    }
}

fn register_node_pin_ids(nodes: &HashMap<String, proto::Node>, maps: &mut ForkIdMap) {
    for node in nodes.values() {
        let node_id = maps.mint(&node.id);
        maps.nodes.entry(node.id.clone()).or_insert(node_id);
        register_pin_ids(&node.pins, maps);
    }
}

fn register_pin_ids(pins: &HashMap<String, proto::Pin>, maps: &mut ForkIdMap) {
    for pin in pins.values() {
        let pin_id = maps.mint(&pin.id);
        maps.pins.entry(pin.id.clone()).or_insert(pin_id);
    }
}

fn rewrite_node(node: &mut proto::Node, maps: &ForkIdMap) {
    node.id = maps.translate_node(&node.id);
    if let Some(layer) = node.layer.as_ref() {
        // Layers were pre-registered in remap_board, so a missing entry
        // means a stale pointer; preserve the original so the desktop
        // can surface the reference rather than silently orphan the node.
        node.layer = Some(maps.layers.get(layer).cloned().unwrap_or(layer.clone()));
    }
    // Agent / Call Reference style nodes carry a list of function-target
    // node ids in `fn_refs.fn_refs`. These are global node ids; without
    // translation, the destination would point at the source's nodes.
    if let Some(fn_refs) = node.fn_refs.as_mut() {
        fn_refs.fn_refs = fn_refs
            .fn_refs
            .iter()
            .map(|id| maps.nodes.get(id).cloned().unwrap_or(id.clone()))
            .collect();
    }
    let mut new_pins = HashMap::with_capacity(node.pins.len());
    for (_, mut pin) in node.pins.drain() {
        rewrite_pin_top(&mut pin, maps);
        new_pins.insert(pin.id.clone(), pin);
    }
    node.pins = new_pins;
}

fn rewrite_pin_top(pin: &mut proto::Pin, maps: &ForkIdMap) {
    pin.id = maps.pins.get(&pin.id).cloned().unwrap_or(pin.id.clone());
    pin.connected_to = pin
        .connected_to
        .iter()
        .map(|p| maps.pins.get(p).cloned().unwrap_or(p.clone()))
        .collect();
    pin.depends_on = pin
        .depends_on
        .iter()
        .map(|p| maps.pins.get(p).cloned().unwrap_or(p.clone()))
        .collect();
    // Pin default values frequently encode a target id chosen by the
    // user — Call Function holds a layer id in `function_layer_id`,
    // Call Reference holds a node id in `fn_ref`, Goto / page-link
    // nodes hold page or event ids. Translate every JSON string we
    // recognize so those references land on the destination's id space.
    rewrite_default_value_ids(&mut pin.default_value, maps);
}

/// Walks a JSON-encoded pin `default_value` and rewrites every string
/// whose contents match a known source id (node, layer, event, page,
/// pin) to the destination id from `maps`. Strings that don't match
/// anything in the maps are left untouched. Empty bytes / non-JSON
/// payloads are no-ops so non-string defaults (numbers, structs that
/// don't reference ids) keep working.
pub fn rewrite_default_value_ids(default_value: &mut Vec<u8>, maps: &ForkIdMap) {
    if default_value.is_empty() {
        return;
    }
    let mut value: flow_like_types::Value = match serde_json::from_slice(default_value) {
        Ok(v) => v,
        Err(_) => return,
    };
    if !translate_ids_in_json(&mut value, maps) {
        return;
    }
    if let Ok(bytes) = serde_json::to_vec(&value) {
        *default_value = bytes;
    }
}

/// Recursively visits a JSON value and rewrites any string equal to a
/// known source id, plus any page-scoped element reference whose page
/// head is a known source page. Returns whether anything changed so
/// callers can skip a re-encode when the payload is untouched.
fn translate_ids_in_json(value: &mut flow_like_types::Value, maps: &ForkIdMap) -> bool {
    match value {
        flow_like_types::Value::String(s) => {
            let translated = lookup_id(s, maps).or_else(|| translate_element_ref(s, maps));
            if let Some(translated) = translated {
                *s = translated;
                true
            } else {
                false
            }
        }
        flow_like_types::Value::Array(items) => {
            let mut changed = false;
            for item in items.iter_mut() {
                if translate_ids_in_json(item, maps) {
                    changed = true;
                }
            }
            changed
        }
        flow_like_types::Value::Object(map) => {
            let mut changed = false;
            for (_k, v) in map.iter_mut() {
                if translate_ids_in_json(v, maps) {
                    changed = true;
                }
            }
            changed
        }
        _ => false,
    }
}

/// Whole-string translation of any id a pin default may name.
///
/// `widgets` is in this chain because the `widget_selector` pin of
/// `a2ui_instantiate_widget` stores the bare project widget id (see
/// `WidgetVariable` / `widget-select.tsx`, which commits
/// `selector: widgetId`), and every copied widget gets a fresh id — so
/// without this the copied node resolves against the source app's widget
/// and fails with "Widget '…' not found". Package widget selectors are
/// immune by construction: they are encoded `pkg:{package_id}/{widget_id}`,
/// never a key of `maps.widgets`, and package ids are global and must never
/// be rewritten. Legacy name-based selectors are likewise untouched.
///
/// `roles` is here because the `role` pin of the project-user nodes
/// accepts "Role ID or exact role name" — an id needs translating, a
/// name is not a map key and passes through.
///
/// Deliberately absent: `templates` (no board artifact stores a
/// template id) and `variables` (variable ids are preserved verbatim
/// by `remap_board`, so `var_ref` defaults must keep resolving against
/// the unchanged `board.variables` keys).
fn lookup_id(src: &str, maps: &ForkIdMap) -> Option<String> {
    maps.nodes
        .get(src)
        .or_else(|| maps.layers.get(src))
        .or_else(|| maps.events.get(src))
        .or_else(|| maps.pages.get(src))
        .or_else(|| maps.pins.get(src))
        .or_else(|| maps.boards.get(src))
        .or_else(|| maps.widgets.get(src))
        .or_else(|| maps.roles.get(src))
        .cloned()
}

/// UI element references are composite: the element picker stores
/// `"{page_id}/{component_id}"` in the `element_ref` pin default (see
/// `ElementSelect`), and the runtime keys its `_elements` payload the
/// same way — the prerun manifest lists the refs a board reads, and
/// `ExecutionContext::read_element` resolves them by exact key first,
/// then by `/{component_id}` suffix on the shipped map.
///
/// Component ids are page-scoped and survive a remap unchanged, but the
/// page head does not, so `lookup_id` never matches the composite
/// string as a whole. Without this pass every `Get Element` /
/// `Set Element …` node in a copied app keeps pointing at the source
/// app's page and silently resolves to "element not found".
fn translate_element_ref(src: &str, maps: &ForkIdMap) -> Option<String> {
    let (page_id, component_id) = src.split_once('/')?;
    if component_id.is_empty() {
        return None;
    }
    let new_page_id = maps.pages.get(page_id)?;
    Some(format!("{}/{}", new_page_id, component_id))
}

/// Re-key an event and its board / node / page targets. Secret variable
/// values are kept — callers crossing a security boundary follow up with
/// [`strip_event_secrets`].
pub fn remap_event(event: &mut proto::Event, maps: &ForkIdMap) {
    event.id = maps
        .events
        .get(&event.id)
        .cloned()
        .unwrap_or_else(|| maps.mint(&event.id));
    event.board_id = maps.translate_board(&event.board_id);
    event.node_id = maps.translate_node(&event.node_id);
    if let Some(default_page) = event.default_page_id.as_ref() {
        event.default_page_id = Some(maps.translate_page(default_page));
    }
    if let Some(canary) = event.canary.as_mut() {
        canary.board_id = maps.translate_board(&canary.board_id);
        canary.node_id = maps.translate_node(&canary.node_id);
    }
    for variant in event.variants.iter_mut() {
        variant.board_id = maps.translate_board(&variant.board_id);
        variant.node_id = maps.translate_node(&variant.node_id);
        if let Some(page) = variant.default_page_id.as_ref() {
            variant.default_page_id = Some(maps.translate_page(page));
        }
    }
    for input in event.inputs.iter_mut() {
        if let Some(new_pin) = maps.pins.get(&input.id) {
            input.id = new_pin.clone();
        }
    }
}

/// Clears `default_value` on every secret-marked variable inside an event
/// proto, including the canary's and every variant's variables. The event's
/// `config` bytes are intentionally NOT touched here — token sites (HTTP
/// auth_token, PAT, OAuth) are the caller's policy.
pub fn strip_event_secrets(event: &mut proto::Event) {
    strip_secret_values(&mut event.variables);
    if let Some(canary) = event.canary.as_mut() {
        strip_secret_values(&mut canary.variables);
    }
    for variant in event.variants.iter_mut() {
        strip_secret_values(&mut variant.variables);
    }
}

fn strip_secret_values(variables: &mut HashMap<String, proto::Variable>) {
    for var in variables.values_mut() {
        if var.secret {
            var.default_value.clear();
        }
    }
}

/// Apply the id translations to a page in place and give it `new_page_id`.
///
/// Which references exist inside a page payload is
/// [`page_remap`]'s inventory, shared with template instantiation; this
/// function supplies only the policy — what each source id becomes.
///
/// Returns one entry per payload that could not be rewritten and therefore
/// still points at the source app.
pub fn remap_page(page: &mut proto::Page, new_page_id: &str, maps: &ForkIdMap) -> Vec<String> {
    page.id = new_page_id.to_string();
    let mut by_field = fork_field_translator(maps);
    let mut by_literal = fork_literal_translator(maps);
    let mut translators = page_remap::IdTranslators {
        by_field: &mut by_field,
        by_literal: &mut by_literal,
    };
    page_remap::remap_page_refs(page, &mut translators)
}

/// Run the shared widget pass over a JSON-serialized `.widget` document.
pub fn remap_widget_json(widget: &mut flow_like_types::Value, maps: &ForkIdMap) -> Vec<String> {
    let mut by_field = fork_field_translator(maps);
    let mut by_literal = fork_literal_translator(maps);
    let mut translators = page_remap::IdTranslators {
        by_field: &mut by_field,
        by_literal: &mut by_literal,
    };
    page_remap::remap_widget_json(widget, &mut translators)
}

/// Resolve a reference the a2ui walker found under a recognized field
/// name. Translation stays opt-in per value: a name only resolves when
/// the embedded string is actually a key of the corresponding map, so a
/// user-authored `nodeId` in unrelated game state is left alone, and a
/// value already on the destination's id space is a no-op.
pub fn fork_field_translator(maps: &ForkIdMap) -> impl FnMut(IdRef, &str) -> Option<String> + '_ {
    move |kind, id| match kind {
        IdRef::Node => maps.nodes.get(id).cloned(),
        IdRef::Board => maps.boards.get(id).cloned(),
        IdRef::Page => maps.pages.get(id).cloned(),
        IdRef::Widget => maps.widgets.get(id).cloned(),
        IdRef::Event => maps.events.get(id).cloned(),
        IdRef::App => {
            (id == maps.source_app_id && !maps.source_app_id.is_empty() && !maps.app_id.is_empty())
                .then(|| maps.app_id.clone())
        }
    }
}

/// Resolve a reference that arrived without a field name — a widget
/// customization value, an exposed prop's default. `lookup_id` is the
/// same whole-string pass pin defaults take, so the two agree on what
/// counts as an id, and composite element references
/// (`{page_id}/{component_id}`) follow the page they name.
fn fork_literal_translator(maps: &ForkIdMap) -> impl FnMut(&str) -> Option<String> + '_ {
    move |id| lookup_id(id, maps).or_else(|| translate_element_ref(id, maps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_ids_are_stable_and_scoped_to_the_seed() {
        let a = derive_id("job-a", "src");
        assert_eq!(a, derive_id("job-a", "src"));
        assert_ne!(a, derive_id("job-b", "src"));
        assert_ne!(a, derive_id("job-a", "other"));
    }

    #[test]
    fn derived_ids_look_like_cuids() {
        let random = flow_like_types::create_id();
        for id in [derive_id("seed", "x"), derive_id("", "y")] {
            assert_eq!(id.len(), random.len());
            assert!(id.as_bytes()[0].is_ascii_lowercase());
            assert!(
                id.bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            );
        }
    }

    #[test]
    fn the_separator_keeps_prefixes_apart() {
        assert_ne!(derive_id("ab", "c"), derive_id("a", "bc"));
    }

    #[test]
    fn remapping_keeps_secrets_until_the_caller_strips_them() {
        let secret = proto::Variable {
            id: "token".into(),
            secret: true,
            default_value: b"shh".to_vec(),
            ..Default::default()
        };
        let board = proto::Board {
            id: "src_board".into(),
            variables: HashMap::from([(secret.id.clone(), secret.clone())]),
            ..Default::default()
        };
        let mut maps = ForkIdMap {
            seed: "seed".into(),
            ..Default::default()
        };

        let mut remapped = remap_board(board, &mut maps).expect("remap board");
        assert_eq!(remapped.variables["token"].default_value, b"shh".to_vec());
        strip_board_secrets(&mut remapped);
        assert!(remapped.variables["token"].default_value.is_empty());

        let mut event = proto::Event {
            id: "src_event".into(),
            board_id: "src_board".into(),
            variables: HashMap::from([(secret.id.clone(), secret)]),
            ..Default::default()
        };
        remap_event(&mut event, &maps);
        assert_eq!(event.board_id, remapped.id);
        assert_eq!(event.variables["token"].default_value, b"shh".to_vec());
        strip_event_secrets(&mut event);
        assert!(event.variables["token"].default_value.is_empty());
    }
}
