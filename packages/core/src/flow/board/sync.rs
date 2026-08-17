//! Incremental board transfer.
//!
//! A board is split into independently versioned **parts**: a small `meta` record, the
//! `variables`, `comments`, `layers` and `refs` maps, and one node **segment** per effective
//! layer. Every part carries an opaque revision token (a canonical-JSON digest of exactly the
//! bytes that part ships). Clients echo the tokens they hold; the server answers with only the
//! parts whose token differs. Tokens are never computed client-side, so their derivation is free
//! to change without a protocol bump.
//!
//! Three properties are load-bearing and each guards a specific corruption:
//!
//! - **Segments partition on `node.layer`, not `Layer.nodes`.** The canvas filters the flat
//!   `Board.nodes` map by `node.layer`; `Layer.nodes` is a legacy parallel map that is empty on
//!   real boards. Segmenting on it would ship empty segments while the real nodes never move.
//! - **A returned segment replaces the client's node set for that segment wholesale.** It is never
//!   merged, so a node that changed layer (and therefore appears in two changed segments) cannot
//!   linger in the one it left.
//! - **Tokens digest the wire payload, not `Node::hash`.** `Node::hash` skips `pin.id`, `node.id`,
//!   `error`, `version` and more, and dynamic `on_update` pins re-mint ids without changing it. A
//!   token built from it could report "unchanged" while every pin id differed.
//!
//! Catalog hydration is decided **per node by comparison**: a node is shipped lean only when every
//! catalog-owned field, on the node and on each of its pins, is byte-identical to the registry
//! definition. Roughly forty catalog `on_update`s rewrite `data_type`/`value_type`/`friendly_name`
//! dynamically (`reroute`, `struct_*`, `a2ui_*` …); an allowlist would corrupt exactly those.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::SystemTime;

use flow_like_types::base64::engine::general_purpose::STANDARD as BASE64;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Board, Comment, ExecutionMode, ExecutionStage, Layer};
use crate::flow::execution::LogLevel;
use crate::flow::node::{FnRefs, Node, NodeScores, NodeWasm};
use crate::flow::pin::{Pin, PinOptions, PinType, ValueType};
use crate::flow::variable::{Variable, VariableType};

/// Segment id for nodes that live on the board root (`node.layer` is `None` or empty).
pub const ROOT_SEGMENT: &str = "__root__";

/// The effective layer a node is drawn on. `Some("")` and `None` are the same thing to the
/// canvas, so they must be the same segment here.
pub fn node_segment(node: &Node) -> &str {
    match node.layer.as_deref() {
        None | Some("") => ROOT_SEGMENT,
        Some(layer) => layer,
    }
}

/// Follow a compact board ref to its value. `fix_refs` content-addresses every node and pin
/// description (and schema) into `Board::refs`, so a placed value can only be compared with the
/// catalog after resolving it the way the client's `unrefValue` does. Cycles resolve to the last
/// key seen; a value that is not a key resolves to itself.
fn resolve_ref<'a>(value: &'a str, refs: &'a HashMap<String, String>) -> &'a str {
    let mut current = value;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(current) {
        match refs.get(current) {
            Some(next) => current = next,
            None => break,
        }
    }
    current
}

// ----------------------------------------------------------------------------------------------
// Wire types
// ----------------------------------------------------------------------------------------------

/// Board fields that are neither nodes, variables, comments, layers nor refs. Tiny, and it changes
/// on every edit (`updated_at`), which is exactly what lets the client's fingerprint move.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct BoardMeta {
    pub id: String,
    pub name: String,
    pub description: String,
    pub viewport: (f32, f32, f32),
    pub version: (u32, u32, u32),
    pub stage: ExecutionStage,
    pub log_level: LogLevel,
    pub execution_mode: ExecutionMode,
    pub page_ids: Vec<String>,
    pub hash: Option<u64>,
    pub created_at: SystemTime,
    pub updated_at: SystemTime,
}

impl BoardMeta {
    pub fn from_board(board: &Board) -> Self {
        Self {
            id: board.id.clone(),
            name: board.name.clone(),
            description: board.description.clone(),
            viewport: board.viewport,
            version: board.version,
            stage: board.stage.clone(),
            log_level: board.log_level,
            execution_mode: board.execution_mode.clone(),
            page_ids: board.page_ids.clone(),
            hash: board.hash,
            created_at: board.created_at,
            updated_at: board.updated_at,
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

mod base64_bytes {
    use super::BASE64;
    use flow_like_types::base64::Engine;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(value: &Option<Vec<u8>>, s: S) -> Result<S::Ok, S::Error> {
        match value {
            Some(bytes) => BASE64.encode(bytes).serialize(s),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<u8>>, D::Error> {
        let encoded: Option<String> = Option::deserialize(d)?;
        encoded
            .map(|text| BASE64.decode(text).map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// A pin on the sync wire. Graph identity is always present; catalog-owned metadata is present
/// unless the owning node was shipped lean (see [`SyncNode::hydrate`]).
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SyncPin {
    pub id: String,
    pub name: String,
    pub index: u16,
    /// Kept on the wire even for lean nodes: on the server this is a compact ref key while the
    /// catalog holds the expanded schema, and letting those two representations diverge between
    /// client and server is not worth the ~2% it would save.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub connected_to: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub depends_on: BTreeSet<String>,
    /// Base64 on the wire: as a JSON array of bytes this field alone was 10% of a full board.
    #[serde(
        default,
        with = "base64_bytes",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<String>")]
    pub default_value: Option<Vec<u8>>,

    // Catalog-owned. `None` on a lean node means "take it from the catalog pin of the same name".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub friendly_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_type: Option<PinType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_type: Option<VariableType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_type: Option<ValueType>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<PinOptions>,
}

impl SyncPin {
    fn from_pin(pin: &Pin) -> Self {
        Self {
            id: pin.id.clone(),
            name: pin.name.clone(),
            index: pin.index,
            schema: pin.schema.clone(),
            connected_to: pin.connected_to.clone(),
            depends_on: pin.depends_on.clone(),
            default_value: pin.default_value.clone(),
            friendly_name: Some(pin.friendly_name.clone()),
            description: Some(pin.description.clone()),
            pin_type: Some(pin.pin_type.clone()),
            data_type: Some(pin.data_type.clone()),
            value_type: Some(pin.value_type.clone()),
            options: pin.options.clone(),
        }
    }

    fn strip_catalog_fields(&mut self) {
        self.friendly_name = None;
        self.description = None;
        self.pin_type = None;
        self.data_type = None;
        self.value_type = None;
        self.options = None;
    }

    /// Whether every catalog-owned field equals the catalog pin, so the client can rebuild
    /// this pin from `catalog` without loss.
    fn matches_catalog(pin: &Pin, catalog: &Pin, refs: &HashMap<String, String>) -> bool {
        pin.friendly_name == catalog.friendly_name
            && resolve_ref(&pin.description, refs) == resolve_ref(&catalog.description, refs)
            && pin.pin_type == catalog.pin_type
            && pin.data_type == catalog.data_type
            && pin.value_type == catalog.value_type
            && pin.options == catalog.options
    }
}

/// A node on the sync wire.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SyncNode {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coordinates: Option<(f32, f32, f32)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<u64>,
    /// `fn_refs.fn_refs` is per-instance wiring, so the whole struct always ships.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fn_refs: Option<FnRefs>,
    /// `wasm.package_id` is instance identity, so it always ships.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm: Option<NodeWasm>,
    /// Users can rename nodes, so this is instance data and always ships.
    pub friendly_name: String,
    pub pins: HashMap<String, SyncPin>,

    /// `true` when catalog-owned fields (below, and on every pin) were omitted and the client
    /// must rebuild them from its catalog entry for `name`. The client can distinguish "omitted"
    /// from "genuinely absent" only through this flag, which is why it is explicit on the wire.
    #[serde(rename = "h", default, skip_serializing_if = "is_false")]
    pub hydrate: bool,

    // Catalog-owned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scores: Option<NodeScores>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_running: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_callback: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_offline: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_providers: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub required_oauth_scopes: Option<HashMap<String, Vec<String>>>,
}

impl SyncNode {
    /// Full wire form of `node`. `hydrate` starts `false`; [`Self::mark_hydratable`] decides it
    /// after the segment token is taken, so the token never depends on the server's catalog.
    pub fn from_node(node: &Node) -> Self {
        Self {
            id: node.id.clone(),
            name: node.name.clone(),
            version: node.version,
            coordinates: node.coordinates,
            layer: node.layer.clone(),
            comment: node.comment.clone(),
            start: node.start,
            error: node.error.clone(),
            hash: node.hash,
            fn_refs: node.fn_refs.clone(),
            wasm: node.wasm.clone(),
            pins: node
                .pins
                .iter()
                .map(|(id, pin)| (id.clone(), SyncPin::from_pin(pin)))
                .collect(),
            friendly_name: node.friendly_name.clone(),
            hydrate: false,
            description: Some(node.description.clone()),
            category: Some(node.category.clone()),
            icon: node.icon.clone(),
            docs: node.docs.clone(),
            scores: node.scores.clone(),
            long_running: node.long_running,
            event_callback: node.event_callback,
            only_offline: Some(node.only_offline),
            oauth_providers: node.oauth_providers.clone(),
            required_oauth_scopes: node.required_oauth_scopes.clone(),
        }
    }

    /// Record whether a client holding `catalog` could rebuild this node's catalog-owned fields.
    /// `refs` is the board's ref table, needed to compare content-addressed descriptions.
    pub fn mark_hydratable(
        &mut self,
        node: &Node,
        catalog: Option<&Node>,
        refs: &HashMap<String, String>,
    ) {
        self.hydrate = catalog.is_some_and(|catalog| Self::matches_catalog(node, catalog, refs));
    }

    /// The node as shipped to a client that asked for hydration: catalog-owned fields removed
    /// when — and only when — [`Self::hydrate`] says the client can rebuild them.
    pub fn lean(mut self) -> Self {
        if !self.hydrate {
            return self;
        }
        self.description = None;
        self.category = None;
        self.icon = None;
        self.docs = None;
        self.scores = None;
        self.long_running = None;
        self.event_callback = None;
        self.only_offline = None;
        self.oauth_providers = None;
        self.required_oauth_scopes = None;
        for pin in self.pins.values_mut() {
            pin.strip_catalog_fields();
        }
        self
    }

    /// The node as shipped to a client that did not ask for hydration.
    pub fn full(mut self) -> Self {
        self.hydrate = false;
        self
    }

    /// Exact per-node eligibility. Any dynamic mutation of a catalog-owned field, on the node or
    /// on any pin, disqualifies the node — no allowlist of "dynamic" node types is consulted.
    fn matches_catalog(node: &Node, catalog: &Node, refs: &HashMap<String, String>) -> bool {
        if node.version != catalog.version
            || resolve_ref(&node.description, refs) != resolve_ref(&catalog.description, refs)
            || node.category != catalog.category
            || node.icon != catalog.icon
            || node.docs != catalog.docs
            || node.scores != catalog.scores
            || node.long_running != catalog.long_running
            || node.event_callback != catalog.event_callback
            || node.only_offline != catalog.only_offline
            || node.oauth_providers != catalog.oauth_providers
            || node.required_oauth_scopes != catalog.required_oauth_scopes
        {
            return false;
        }

        // The client rebuilds pins by name, so a name must map to exactly one catalog pin.
        let mut by_name: HashMap<&str, Vec<&Pin>> = HashMap::new();
        for pin in catalog.pins.values() {
            by_name.entry(pin.name.as_str()).or_default().push(pin);
        }
        node.pins.values().all(|pin| {
            matches!(
                by_name.get(pin.name.as_str()).map(Vec::as_slice),
                Some([catalog_pin]) if SyncPin::matches_catalog(pin, catalog_pin, refs)
            )
        })
    }
}

/// One node segment: the nodes of one effective layer plus the token identifying this revision
/// of that set.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SyncSegment {
    pub hash: String,
    pub nodes: HashMap<String, SyncNode>,
}

/// Every part token of one board revision. The client stores this verbatim and echoes it as the
/// next request.
///
/// `refs` has no token on purpose: refs are content-addressed and ride along with whatever
/// parts ship (see [`BoardSyncResponse::refs`]), so the client never needs to prove which keys
/// it holds.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct BoardSyncManifest {
    pub meta: String,
    pub variables: String,
    pub comments: String,
    /// One token per layer *definition* (pins, variables, coordinates, cache …), keyed by layer
    /// id — independent of the layer's node segment, so a rename ships one small record.
    pub layers: BTreeMap<String, String>,
    pub segments: BTreeMap<String, String>,
}

/// What the client holds. Every field is optional so a client with nothing (first load) or an
/// older client that only tracks segments degrades to "send everything".
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct BoardSyncRequest {
    #[serde(default)]
    pub meta: Option<String>,
    #[serde(default)]
    pub variables: Option<String>,
    #[serde(default)]
    pub comments: Option<String>,
    #[serde(default)]
    pub layers: BTreeMap<String, String>,
    #[serde(default)]
    pub segments: BTreeMap<String, String>,
    /// The client holds a node catalog for this app and will rebuild catalog-owned fields.
    #[serde(default)]
    pub hydrate: bool,
}

impl BoardSyncRequest {
    pub fn from_manifest(manifest: &BoardSyncManifest, hydrate: bool) -> Self {
        Self {
            meta: Some(manifest.meta.clone()),
            variables: Some(manifest.variables.clone()),
            comments: Some(manifest.comments.clone()),
            layers: manifest.layers.clone(),
            segments: manifest.segments.clone(),
            hydrate,
        }
    }
}

/// The parts that changed. Absent parts are unchanged; a segment or layer listed in the request
/// but in neither the changed map nor the dropped list is unchanged; one the client never listed
/// is always present.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BoardSyncResponse {
    pub manifest: BoardSyncManifest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<BoardMeta>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<HashMap<String, Variable>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<HashMap<String, Comment>>,
    /// Changed or new layer definitions, keyed by layer id.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub layers: HashMap<String, Layer>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_layers: Vec<String>,
    /// Exactly the `Board::refs` entries referenced by the parts in this response — every node and
    /// pin description, pin schema and variable schema they carry. Refs are content-addressed, so
    /// the client upserts these into its table and needs nothing else to resolve what it received.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub refs: HashMap<String, String>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub segments: HashMap<String, SyncSegment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dropped_segments: Vec<String>,
}

// ----------------------------------------------------------------------------------------------
// Snapshot: computed once per board revision, diffed per request
// ----------------------------------------------------------------------------------------------

/// Opaque revision token for one part: blake3 over the canonical JSON of the payload. Canonical
/// so two replicas serialising the same `HashMap` in different orders agree.
pub fn part_token<T: Serialize>(part: &'static str, value: &T) -> flow_like_types::Result<String> {
    let value = super::commands::canonicalize_json(flow_like_types::json::to_value(value)?);
    let encoded = canonical_json::ser::to_string(&value)?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.board-sync/v1\0");
    hasher.update(part.as_bytes());
    hasher.update(b"\0");
    hasher.update(encoded.as_bytes());
    Ok(hasher.finalize().to_hex().to_string())
}

/// A board split into parts, each with its token. Build once per board revision (the API caches
/// it on the storage ETag) and answer any number of manifests from it with [`Self::diff`].
#[derive(Clone, Debug)]
pub struct BoardSyncSnapshot {
    pub manifest: BoardSyncManifest,
    meta: BoardMeta,
    variables: HashMap<String, Variable>,
    /// Ref keys the variables part reaches.
    variable_refs: Vec<String>,
    comments: HashMap<String, Comment>,
    layers: HashMap<String, Layer>,
    /// Ref keys each layer definition reaches.
    layer_refs: HashMap<String, Vec<String>>,
    refs: HashMap<String, String>,
    segments: HashMap<String, SyncSegment>,
    /// Ref keys each segment reaches.
    segment_refs: HashMap<String, Vec<String>>,
}

/// Every ref key a placed value may point at, following chains so a flattened key still
/// arrives with its target.
fn collect_ref_chain(value: &str, refs: &HashMap<String, String>, out: &mut Vec<String>) {
    let mut current = value;
    let mut hops = 0;
    while let Some(next) = refs.get(current) {
        out.push(current.to_string());
        current = next;
        hops += 1;
        if hops > 16 {
            break;
        }
    }
}

fn pin_refs(pin: &Pin, refs: &HashMap<String, String>, out: &mut Vec<String>) {
    collect_ref_chain(&pin.description, refs, out);
    if let Some(schema) = &pin.schema {
        collect_ref_chain(schema, refs, out);
    }
}

fn node_refs(node: &Node, refs: &HashMap<String, String>, out: &mut Vec<String>) {
    collect_ref_chain(&node.description, refs, out);
    for pin in node.pins.values() {
        pin_refs(pin, refs, out);
    }
}

fn variable_refs(variable: &Variable, refs: &HashMap<String, String>, out: &mut Vec<String>) {
    if let Some(schema) = &variable.schema {
        collect_ref_chain(schema, refs, out);
    }
}

fn layer_refs(layer: &Layer, refs: &HashMap<String, String>, out: &mut Vec<String>) {
    for pin in layer.pins.values() {
        pin_refs(pin, refs, out);
    }
    for variable in layer.variables.values() {
        variable_refs(variable, refs, out);
    }
    for node in layer.nodes.values() {
        node_refs(node, refs, out);
    }
}

fn dedup(mut keys: Vec<String>) -> Vec<String> {
    keys.sort();
    keys.dedup();
    keys
}

impl BoardSyncSnapshot {
    /// `catalog` is the registry the client's `getCatalog` sees for this app (built-in plus the
    /// app's WASM nodes); it decides hydration eligibility and nothing else. Pass an empty slice
    /// to disable hydration.
    pub fn from_board(board: &Board, catalog: &[Node]) -> flow_like_types::Result<Self> {
        let catalog_by_name: HashMap<&str, &Node> = catalog
            .iter()
            .map(|node| (node.name.as_str(), node))
            .collect();

        let mut buckets: HashMap<String, HashMap<String, SyncNode>> = HashMap::new();
        for (id, node) in &board.nodes {
            buckets
                .entry(node_segment(node).to_string())
                .or_default()
                .insert(id.clone(), SyncNode::from_node(node));
        }

        let mut segments = HashMap::with_capacity(buckets.len());
        let mut segment_tokens = BTreeMap::new();
        let mut segment_refs = HashMap::with_capacity(buckets.len());
        for (segment_id, mut nodes) in buckets {
            // Token first: it must identify the revision, not how one client receives it.
            let hash = part_token("segment", &nodes)?;
            let mut reached = Vec::new();
            for (id, sync_node) in nodes.iter_mut() {
                if let Some(node) = board.nodes.get(id) {
                    sync_node.mark_hydratable(
                        node,
                        catalog_by_name.get(node.name.as_str()).copied(),
                        &board.refs,
                    );
                    node_refs(node, &board.refs, &mut reached);
                }
            }
            segment_tokens.insert(segment_id.clone(), hash.clone());
            segment_refs.insert(segment_id.clone(), dedup(reached));
            segments.insert(segment_id, SyncSegment { hash, nodes });
        }

        let mut layer_tokens = BTreeMap::new();
        let mut layer_ref_keys = HashMap::with_capacity(board.layers.len());
        for (id, layer) in &board.layers {
            layer_tokens.insert(id.clone(), part_token("layer", layer)?);
            let mut reached = Vec::new();
            layer_refs(layer, &board.refs, &mut reached);
            layer_ref_keys.insert(id.clone(), dedup(reached));
        }

        let mut variable_ref_keys = Vec::new();
        for variable in board.variables.values() {
            variable_refs(variable, &board.refs, &mut variable_ref_keys);
        }

        let meta = BoardMeta::from_board(board);
        let manifest = BoardSyncManifest {
            meta: part_token("meta", &meta)?,
            variables: part_token("variables", &board.variables)?,
            comments: part_token("comments", &board.comments)?,
            layers: layer_tokens,
            segments: segment_tokens,
        };

        Ok(Self {
            manifest,
            meta,
            variables: board.variables.clone(),
            variable_refs: dedup(variable_ref_keys),
            comments: board.comments.clone(),
            layers: board.layers.clone(),
            layer_refs: layer_ref_keys,
            refs: board.refs.clone(),
            segments,
            segment_refs,
        })
    }

    /// The parts of this revision that `request` does not already hold, plus every ref those
    /// parts reference.
    pub fn diff(&self, request: &BoardSyncRequest) -> BoardSyncResponse {
        let changed = |held: &Option<String>, current: &str| held.as_deref() != Some(current);
        let manifest = &self.manifest;
        let mut needed_refs: Vec<&str> = Vec::new();

        let segments: HashMap<String, SyncSegment> = self
            .segments
            .iter()
            .filter(|(id, segment)| request.segments.get(*id) != Some(&segment.hash))
            .map(|(id, segment)| {
                if let Some(keys) = self.segment_refs.get(id) {
                    needed_refs.extend(keys.iter().map(String::as_str));
                }
                let nodes = segment
                    .nodes
                    .iter()
                    .map(|(node_id, node)| {
                        let node = node.clone();
                        let node = if request.hydrate {
                            node.lean()
                        } else {
                            node.full()
                        };
                        (node_id.clone(), node)
                    })
                    .collect();
                (
                    id.clone(),
                    SyncSegment {
                        hash: segment.hash.clone(),
                        nodes,
                    },
                )
            })
            .collect();

        let dropped_segments = request
            .segments
            .keys()
            .filter(|id| !self.segments.contains_key(*id))
            .cloned()
            .collect();

        let layers: HashMap<String, Layer> = self
            .layers
            .iter()
            .filter(|(id, _)| request.layers.get(*id) != manifest.layers.get(*id))
            .map(|(id, layer)| {
                if let Some(keys) = self.layer_refs.get(id) {
                    needed_refs.extend(keys.iter().map(String::as_str));
                }
                (id.clone(), layer.clone())
            })
            .collect();

        let dropped_layers = request
            .layers
            .keys()
            .filter(|id| !self.layers.contains_key(*id))
            .cloned()
            .collect();

        let variables = changed(&request.variables, &manifest.variables).then(|| {
            needed_refs.extend(self.variable_refs.iter().map(String::as_str));
            self.variables.clone()
        });

        needed_refs.sort_unstable();
        needed_refs.dedup();
        let refs = needed_refs
            .into_iter()
            .filter_map(|key| {
                self.refs
                    .get(key)
                    .map(|value| (key.to_string(), value.clone()))
            })
            .collect();

        BoardSyncResponse {
            manifest: manifest.clone(),
            meta: changed(&request.meta, &manifest.meta).then(|| self.meta.clone()),
            variables,
            comments: changed(&request.comments, &manifest.comments).then(|| self.comments.clone()),
            layers,
            dropped_layers,
            refs,
            segments,
            dropped_segments,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::pin::PinType;
    use crate::flow::variable::VariableType;
    use flow_like_storage::Path;

    fn pin(name: &str, pin_type: PinType) -> Pin {
        let mut node = Node::new("tmp", "tmp", "", "");
        match pin_type {
            PinType::Input => node.add_input_pin(name, name, "", VariableType::String),
            PinType::Output => node.add_output_pin(name, name, "", VariableType::String),
        };
        node.pins.into_values().next().expect("one pin")
    }

    fn catalog_node() -> Node {
        let mut node = Node::new("demo", "Demo", "A demo node", "Test");
        node.set_version(3);
        node.add_input_pin("value", "Value", "", VariableType::String);
        node.add_output_pin("result", "Result", "", VariableType::String);
        node
    }

    fn placed(catalog: &Node, layer: Option<&str>) -> Node {
        let mut node = catalog.clone();
        node.id = flow_like_types::create_id();
        node.layer = layer.map(str::to_string);
        node
    }

    fn board_with(nodes: Vec<Node>) -> Board {
        let mut board = Board::new_detached(Some("b".into()), Path::default());
        // Fixed timestamps so two boards built in different instants have the same `meta`.
        board.created_at = SystemTime::UNIX_EPOCH;
        board.updated_at = SystemTime::UNIX_EPOCH;
        for node in nodes {
            board.nodes.insert(node.id.clone(), node);
        }
        board
    }

    #[test]
    fn root_segment_absorbs_empty_and_missing_layer() {
        let catalog = catalog_node();
        let a = placed(&catalog, None);
        let b = placed(&catalog, Some(""));
        let c = placed(&catalog, Some("layer-1"));
        let snapshot =
            BoardSyncSnapshot::from_board(&board_with(vec![a, b, c]), &[]).expect("snapshot");
        assert_eq!(snapshot.segments[ROOT_SEGMENT].nodes.len(), 2);
        assert_eq!(snapshot.segments["layer-1"].nodes.len(), 1);
    }

    #[test]
    fn unchanged_manifest_yields_empty_diff() {
        let catalog = catalog_node();
        let board = board_with(vec![placed(&catalog, None), placed(&catalog, Some("l"))]);
        let snapshot = BoardSyncSnapshot::from_board(&board, &[]).expect("snapshot");
        let request = BoardSyncRequest::from_manifest(&snapshot.manifest, false);
        let response = snapshot.diff(&request);
        assert!(response.meta.is_none());
        assert!(response.variables.is_none());
        assert!(response.refs.is_empty());
        assert!(response.layers.is_empty());
        assert!(response.segments.is_empty());
        assert!(response.dropped_segments.is_empty());
    }

    #[test]
    fn empty_request_returns_everything() {
        let catalog = catalog_node();
        let board = board_with(vec![placed(&catalog, None), placed(&catalog, Some("l"))]);
        let snapshot = BoardSyncSnapshot::from_board(&board, &[]).expect("snapshot");
        let response = snapshot.diff(&BoardSyncRequest::default());
        assert!(response.meta.is_some());
        assert!(response.variables.is_some());
        assert_eq!(response.segments.len(), 2);
    }

    #[test]
    fn refs_ride_along_with_exactly_the_parts_that_ship() {
        // Two root nodes each carrying a distinct ref, and one layer-1 node with its own.
        let catalog = catalog_node();
        let mut a = placed(&catalog, None);
        let mut b = placed(&catalog, Some("l"));
        let mut board = board_with(vec![]);
        board.refs.insert("desc-a".into(), "A's description".into());
        board.refs.insert("desc-b".into(), "B's description".into());
        board
            .refs
            .insert("orphan".into(), "nobody references me".into());
        a.description = "desc-a".into();
        b.description = "desc-b".into();
        board.nodes.insert(a.id.clone(), a.clone());
        board.nodes.insert(b.id.clone(), b.clone());

        let before = BoardSyncSnapshot::from_board(&board, &[]).expect("before");
        let full = before.diff(&BoardSyncRequest::default());
        assert!(full.refs.contains_key("desc-a") && full.refs.contains_key("desc-b"));
        assert!(
            !full.refs.contains_key("orphan"),
            "unreferenced refs never ship"
        );

        // Move `a` one pixel: only the root segment changes, so only its refs come back.
        let mut edited = board.clone();
        edited.nodes.get_mut(&a.id).unwrap().coordinates = Some((1.0, 0.0, 0.0));
        let after = BoardSyncSnapshot::from_board(&edited, &[]).expect("after");
        let response = after.diff(&BoardSyncRequest::from_manifest(&before.manifest, false));
        assert_eq!(
            response.segments.keys().collect::<Vec<_>>(),
            vec![ROOT_SEGMENT]
        );
        assert_eq!(response.refs.keys().collect::<Vec<_>>(), vec!["desc-a"]);

        // A brand-new description on the moved node arrives with the segment that needs it.
        let mut renamed = edited.clone();
        renamed
            .refs
            .insert("desc-a2".into(), "A's new description".into());
        renamed.nodes.get_mut(&a.id).unwrap().description = "desc-a2".into();
        let third = BoardSyncSnapshot::from_board(&renamed, &[]).expect("third");
        let response = third.diff(&BoardSyncRequest::from_manifest(&after.manifest, false));
        assert!(response.refs.contains_key("desc-a2"));
        assert!(
            !response.refs.contains_key("desc-b"),
            "layer l did not change"
        );
    }

    #[test]
    fn layer_definitions_are_versioned_per_layer() {
        let catalog = catalog_node();
        let mut board = board_with(vec![placed(&catalog, Some("l1"))]);
        board.layers.insert(
            "l1".into(),
            Layer::new(
                "l1".into(),
                "First".into(),
                super::super::LayerType::Collapsed,
            ),
        );
        board.layers.insert(
            "l2".into(),
            Layer::new(
                "l2".into(),
                "Second".into(),
                super::super::LayerType::Collapsed,
            ),
        );
        let before = BoardSyncSnapshot::from_board(&board, &[]).expect("before");
        assert_eq!(before.manifest.layers.len(), 2);

        let mut renamed = board.clone();
        renamed.layers.get_mut("l2").unwrap().name = "Second (renamed)".into();
        let after = BoardSyncSnapshot::from_board(&renamed, &[]).expect("after");
        let response = after.diff(&BoardSyncRequest::from_manifest(&before.manifest, false));
        assert_eq!(response.layers.keys().collect::<Vec<_>>(), vec!["l2"]);
        assert!(
            response.segments.is_empty(),
            "the layer's nodes did not change"
        );
        assert!(response.dropped_layers.is_empty());

        let mut removed = renamed.clone();
        removed.layers.remove("l1");
        let third = BoardSyncSnapshot::from_board(&removed, &[]).expect("third");
        let response = third.diff(&BoardSyncRequest::from_manifest(&after.manifest, false));
        assert_eq!(response.dropped_layers, vec!["l1".to_string()]);
        assert!(response.layers.is_empty());
    }

    #[test]
    fn moving_a_node_between_layers_changes_both_segments_and_nothing_else() {
        let catalog = catalog_node();
        let mut node = placed(&catalog, None);
        let other = placed(&catalog, Some("l"));
        let before =
            BoardSyncSnapshot::from_board(&board_with(vec![node.clone(), other.clone()]), &[])
                .expect("before");
        node.layer = Some("l".into());
        let after =
            BoardSyncSnapshot::from_board(&board_with(vec![node, other]), &[]).expect("after");
        let response = after.diff(&BoardSyncRequest::from_manifest(&before.manifest, false));
        assert!(response.meta.is_none());
        assert_eq!(response.segments.len(), 1, "layer l gained a node");
        assert!(response.segments.contains_key("l"));
        assert_eq!(response.dropped_segments, vec![ROOT_SEGMENT.to_string()]);
    }

    #[test]
    fn a_stale_client_segment_the_server_no_longer_has_is_reported_dropped() {
        let catalog = catalog_node();
        let snapshot =
            BoardSyncSnapshot::from_board(&board_with(vec![placed(&catalog, None)]), &[])
                .expect("snapshot");
        let mut request = BoardSyncRequest::from_manifest(&snapshot.manifest, false);
        request.segments.insert("ghost".into(), "x".into());
        let response = snapshot.diff(&request);
        assert_eq!(response.dropped_segments, vec!["ghost".to_string()]);
    }

    #[test]
    fn re_minted_pin_id_changes_the_segment_token_even_when_node_hash_does_not() {
        // Node::hash never reads a pin's id (it only orders pins by it, which a single pin
        // cannot expose); the segment token must cover it.
        let mut single = Node::new("single", "Single", "", "Test");
        single.set_version(1);
        single.add_input_pin("value", "Value", "", VariableType::String);
        let mut node = placed(&single, None);
        node.hash();
        let before =
            BoardSyncSnapshot::from_board(&board_with(vec![node.clone()]), &[]).expect("before");
        let (old_id, mut pin) = node.pins.drain().next().expect("a pin");
        pin.id = "re-minted".into();
        node.pins.insert(pin.id.clone(), pin);
        let old_hash = node.hash;
        node.hash();
        assert_eq!(
            old_hash, node.hash,
            "precondition: Node::hash ignores pin ids"
        );
        assert_ne!(old_id, "re-minted");
        let after = BoardSyncSnapshot::from_board(&board_with(vec![node]), &[]).expect("after");
        assert_ne!(
            before.manifest.segments[ROOT_SEGMENT],
            after.manifest.segments[ROOT_SEGMENT]
        );
    }

    #[test]
    fn token_is_independent_of_hash_map_iteration_order() {
        let catalog = catalog_node();
        let nodes: Vec<Node> = (0..8).map(|_| placed(&catalog, None)).collect();
        let a = BoardSyncSnapshot::from_board(&board_with(nodes.clone()), &[]).expect("a");
        let reversed: Vec<Node> = nodes.into_iter().rev().collect();
        let b = BoardSyncSnapshot::from_board(&board_with(reversed), &[]).expect("b");
        assert_eq!(a.manifest, b.manifest);
    }

    #[test]
    fn hydration_is_exact_comparison_not_an_allowlist() {
        let catalog = catalog_node();
        let pristine = placed(&catalog, None);
        let mut renamed = placed(&catalog, None);
        renamed.friendly_name = "Call something".into();
        let mut retyped = placed(&catalog, None);
        for pin in retyped.pins.values_mut() {
            pin.data_type = VariableType::Integer;
        }
        let mut older = placed(&catalog, None);
        older.version = Some(2);
        let mut extra_pin = placed(&catalog, None);
        let dynamic = pin("minted", PinType::Input);
        extra_pin.pins.insert(dynamic.id.clone(), dynamic);

        let board = board_with(vec![
            pristine.clone(),
            renamed.clone(),
            retyped.clone(),
            older.clone(),
            extra_pin.clone(),
        ]);
        let snapshot = BoardSyncSnapshot::from_board(&board, std::slice::from_ref(&catalog))
            .expect("snapshot");
        let root = &snapshot.segments[ROOT_SEGMENT].nodes;
        assert!(root[&pristine.id].hydrate);
        assert!(
            root[&renamed.id].hydrate,
            "friendly_name is instance data, not a disqualifier"
        );
        assert!(!root[&retyped.id].hydrate, "dynamic pin type");
        assert!(!root[&older.id].hydrate, "version mismatch");
        assert!(!root[&extra_pin.id].hydrate, "pin missing from catalog");

        let hydrated = snapshot.diff(&BoardSyncRequest {
            hydrate: true,
            ..Default::default()
        });
        let lean = &hydrated.segments[ROOT_SEGMENT].nodes[&pristine.id];
        assert!(lean.hydrate);
        assert!(lean.description.is_none());
        assert_eq!(lean.friendly_name, "Demo", "renamable, so it always ships");
        assert!(lean.pins.values().all(|p| p.data_type.is_none()));
        let full = &hydrated.segments[ROOT_SEGMENT].nodes[&renamed.id];
        assert!(full.hydrate, "a rename alone must not disqualify hydration");
        assert_eq!(full.friendly_name, "Call something");

        let plain = snapshot.diff(&BoardSyncRequest::default());
        let node = &plain.segments[ROOT_SEGMENT].nodes[&pristine.id];
        assert!(
            !node.hydrate,
            "a client that did not ask must never be told to hydrate"
        );
        assert!(node.description.is_some());
    }

    #[test]
    fn hydration_token_does_not_depend_on_encoding() {
        let catalog = catalog_node();
        let board = board_with(vec![placed(&catalog, None)]);
        let with = BoardSyncSnapshot::from_board(&board, std::slice::from_ref(&catalog))
            .expect("with catalog");
        let without = BoardSyncSnapshot::from_board(&board, &[]).expect("without catalog");
        assert_eq!(
            with.manifest.segments[ROOT_SEGMENT], without.manifest.segments[ROOT_SEGMENT],
            "the token identifies the revision, not how a particular client receives it"
        );
    }

    #[test]
    fn content_addressed_descriptions_still_match_the_catalog() {
        // `fix_refs` replaces every description with a ref key; the eligibility check must
        // compare what the key resolves to, or no persisted board would ever hydrate.
        let catalog = catalog_node();
        let mut node = placed(&catalog, None);
        let mut board = board_with(vec![]);
        board
            .refs
            .insert("desc-key".into(), catalog.description.clone());
        node.description = "desc-key".into();
        for pin in node.pins.values_mut() {
            let key = format!("pin-{}", pin.name);
            board.refs.insert(key.clone(), pin.description.clone());
            pin.description = key;
        }
        board.nodes.insert(node.id.clone(), node.clone());
        let snapshot = BoardSyncSnapshot::from_board(&board, std::slice::from_ref(&catalog))
            .expect("snapshot");
        assert!(snapshot.segments[ROOT_SEGMENT].nodes[&node.id].hydrate);
    }

    #[test]
    fn same_name_pins_in_both_directions_disable_hydration() {
        let mut catalog = Node::new("odd", "Odd", "", "Test");
        catalog.set_version(1);
        catalog.add_input_pin("value", "Value", "", VariableType::String);
        catalog.add_output_pin("value", "Value", "", VariableType::String);
        let node = placed(&catalog, None);
        let snapshot = BoardSyncSnapshot::from_board(
            &board_with(vec![node.clone()]),
            std::slice::from_ref(&catalog),
        )
        .expect("snapshot");
        assert!(!snapshot.segments[ROOT_SEGMENT].nodes[&node.id].hydrate);
    }

    #[test]
    fn default_value_crosses_the_wire_as_base64() {
        let catalog = catalog_node();
        let mut node = placed(&catalog, None);
        for pin in node.pins.values_mut() {
            pin.default_value = Some(vec![0x22, 0x68, 0x69, 0x22]);
        }
        let snapshot =
            BoardSyncSnapshot::from_board(&board_with(vec![node.clone()]), &[]).expect("snapshot");
        let response = snapshot.diff(&BoardSyncRequest::default());
        let json = flow_like_types::json::to_value(&response).expect("json");
        let pins = &json["segments"][ROOT_SEGMENT]["nodes"][&node.id]["pins"];
        let encoded = pins
            .as_object()
            .and_then(|pins| pins.values().next())
            .and_then(|pin| pin["default_value"].as_str())
            .expect("base64 default");
        assert_eq!(encoded, "ImhpIg==");
        let round: BoardSyncResponse =
            flow_like_types::json::from_value(json.clone()).expect("roundtrip");
        assert_eq!(flow_like_types::json::to_value(&round).expect("json"), json);
    }
}
