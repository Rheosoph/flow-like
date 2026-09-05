//! Incremental board transfer.
//!
//! A board is split into independently versioned **parts**: a small `meta` record, the
//! `variables`, `comments`, `layers` and `refs` maps, and one node **segment** per effective
//! layer. Every part carries an opaque revision token (a digest of exactly the bytes that part
//! ships). Clients echo the tokens they hold; the server answers with only the parts whose token
//! differs. Tokens are never computed client-side, so their derivation is free to change without
//! a protocol bump.
//!
//! Three properties are load-bearing and each guards a specific corruption:
//!
//! - **Segments partition on `node.layer`, not `Layer.nodes`.** The canvas filters the flat
//!   `Board.nodes` map by `node.layer`; `Layer.nodes` is a legacy parallel map that is empty on
//!   real boards. Segmenting on it would ship empty segments while the real nodes never move.
//! - **A returned segment replaces the client's node set for that segment wholesale** — unless it
//!   is a *patch* (`base` set), which applies node upserts and removals onto exactly the segment
//!   revision named by `base`. Either way a node that changed layer (and therefore appears in two
//!   changed segments) cannot linger in the one it left.
//! - **Tokens digest the wire payload, not `Node::hash`.** `Node::hash` skips `pin.id`, `node.id`,
//!   `error`, `version` and more, and dynamic `on_update` pins re-mint ids without changing it. A
//!   token built from it could report "unchanged" while every pin id differed.
//!
//! Catalog hydration is decided **per node by comparison**: a node is shipped lean only when every
//! catalog-owned field, on the node and on each of its pins, is byte-identical to the registry
//! definition. Roughly forty catalog `on_update`s rewrite `data_type`/`value_type`/`friendly_name`
//! dynamically (`reroute`, `struct_*`, `a2ui_*` …); an allowlist would corrupt exactly those.
//!
//! # Tokens are 128 bits
//!
//! A token is the first [`TOKEN_BYTES`] (16) bytes of a blake3 digest, base64url without padding:
//! 22 characters instead of 64 hex. A manifest carries two tokens per layer (definition + segment)
//! and rides on **every** sync and every merged apply, so on a board with 80 layers the difference
//! is ~7 KB per request — millions of requests a day make that the dominant recurring byte cost
//! of the protocol, which is why it was not left at 256 bits "to be safe".
//!
//! Why 128 is not a safety trade-off here:
//! - A collision needs two revisions of the *same part of the same board* to truncate to the same
//!   16 bytes. Accidentally: ~1e-21 over a billion revisions of one segment (birthday bound
//!   n²/2¹²⁹). Deliberately: 2⁶⁴ blake3 evaluations against one's own board.
//! - The consequence of one would be that **one client keeps one stale part** until that part
//!   next changes or the board is reopened (the first load is always full). Tokens never enter
//!   an authorization or write decision, they are computed *after* secret filtering, and the
//!   server never trusts a client token as evidence about the stored board — so stored data cannot
//!   be affected.
//! - The primitives underneath already sit at this tier: the board cache is validated by S3 ETags
//!   (MD5, 128 bits) and node ids are ~128-bit random ids.
//!
//! Client-side verification ("apply, hash locally, compare") was considered and rejected: it
//! would require the client to reproduce the server's canonical bytes (f32 coordinates, u64
//! hashes beyond 2⁵³, optional-field omission, hydration reversal) — a cross-language canonical
//! form, i.e. a protocol of its own. A manifest checksum without content hashing only detects a
//! collision when nothing else changed in the same request, so it was not built either.
//!
//! # Patches and deltas (`BoardSyncRequest::patch`)
//!
//! A client that opts in may receive, for a changed segment, a **patch** instead of the whole
//! segment whenever the server can resolve the *client's* token to a segment it still holds in
//! memory (the "base"). The patch carries only the nodes whose payload differs from the base plus
//! the ids that vanished, and names the base token so the client can verify it applies onto what
//! it holds. Bases are opportunistic — an in-memory index fed by every snapshot built on that
//! process; a miss ships the whole segment exactly as before. Under the same opt-in the response
//! replaces the full manifest echo with a `manifest_delta` relative to the request.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::SystemTime;

use flow_like_types::base64::Engine;
use flow_like_types::base64::engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{Board, Comment, ExecutionMode, ExecutionStage, Layer};
use crate::flow::execution::LogLevel;
use crate::flow::node::{FnRefs, Node, NodeScores, NodeWasm};
use crate::flow::pin::{Pin, PinOptions, PinType, ValueType};
use crate::flow::variable::{Variable, VariableType};

/// Segment id for nodes that live on the board root (`node.layer` is `None` or empty).
pub const ROOT_SEGMENT: &str = "__root__";

/// Bytes of the blake3 digest a part token keeps. See the module docs before changing this: the
/// value is a deliberate byte-cost decision, and every change forces one full refetch per client.
pub const TOKEN_BYTES: usize = 16;

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
///
/// Every field type is order-stable under serialisation (no `HashMap`/`HashSet`): segment tokens
/// stream this struct straight into the hasher, so an unordered map here would make tokens differ
/// between replicas and turn every sync into a resend.
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

/// A node on the sync wire. Same order-stability rule as [`SyncPin`]: no unordered maps.
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
    pub pins: BTreeMap<String, SyncPin>,

    /// `true` when catalog-owned fields (below, and on every pin) were omitted and the client
    /// must rebuild them from its catalog entry for `name`. The client can distinguish "omitted"
    /// from "genuinely absent" only through this flag, which is why it is explicit on the wire.
    ///
    /// Always `false` on a node stored in a snapshot; [`BoardSyncSnapshot::diff`] sets it on the
    /// shipped clone. Stored nodes are therefore pure payload and compare as revisions.
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
    pub required_oauth_scopes: Option<BTreeMap<String, Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alias: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receiver: Option<String>,
}

impl SyncNode {
    /// Full wire form of `node`, `hydrate == false`.
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
            required_oauth_scopes: node.required_oauth_scopes.as_ref().map(|scopes| {
                scopes
                    .iter()
                    .map(|(provider, scopes)| (provider.clone(), scopes.clone()))
                    .collect()
            }),
            namespace: node.namespace.clone(),
            alias: node.alias.clone(),
            receiver: node.receiver.clone(),
        }
    }

    /// Whether a client holding `catalog` could rebuild this node's catalog-owned fields.
    /// `refs` is the board's ref table, needed to compare content-addressed descriptions.
    pub fn hydratable(node: &Node, catalog: Option<&Node>, refs: &HashMap<String, String>) -> bool {
        catalog.is_some_and(|catalog| Self::matches_catalog(node, catalog, refs))
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
        self.namespace = None;
        self.alias = None;
        self.receiver = None;
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
            || node.namespace != catalog.namespace
            || node.alias != catalog.alias
            || node.receiver != catalog.receiver
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

/// One node segment on the wire and in a snapshot.
///
/// Without `base`, `nodes` is the complete node set of the segment and replaces the client's
/// wholesale. With `base`, this is a **patch** onto the client's segment revision `base`: `nodes`
/// are upserts, `removed` are ids that no longer exist. `hash` is always the token of the
/// resulting revision.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq)]
pub struct SyncSegment {
    pub hash: String,
    pub nodes: BTreeMap<String, SyncNode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Ids of nodes a hydrating client may rebuild from its catalog. Server-side only; the
    /// decision reaches the wire as [`SyncNode::hydrate`] on each shipped node.
    #[serde(skip)]
    #[schemars(skip)]
    pub hydratable: BTreeSet<String>,
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

/// The tokens of the current revision that differ from the ones in the request. Absent parts kept
/// their token; removed layers/segments are listed on the response's `dropped_*` fields. Applying
/// this onto the request's manifest yields the full manifest of the revision.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default, PartialEq)]
pub struct BoardSyncManifestDelta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variables: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comments: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub layers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
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
    /// The client understands segment patches (`SyncSegment::base`) and manifest deltas. Off for
    /// every client that predates them, which is what keeps them safe to serve.
    #[serde(default)]
    pub patch: bool,
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
            patch: false,
        }
    }
}

/// The parts that changed. Absent parts are unchanged; a segment or layer listed in the request
/// but in neither the changed map nor the dropped list is unchanged; one the client never listed
/// is always present.
///
/// Exactly one of `manifest` and `manifest_delta` is set: the full manifest for clients that did
/// not opt into patches, the delta for those that did.
#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug)]
pub struct BoardSyncResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<BoardSyncManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_delta: Option<BoardSyncManifestDelta>,
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
// Tokens
// ----------------------------------------------------------------------------------------------

fn token_hasher(part: &'static str) -> blake3::Hasher {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like.board-sync/v2\0");
    hasher.update(part.as_bytes());
    hasher.update(b"\0");
    hasher
}

fn encode_token(digest: blake3::Hash) -> String {
    URL_SAFE_NO_PAD.encode(&digest.as_bytes()[..TOKEN_BYTES])
}

/// Opaque revision token for a part whose serialisation may contain unordered maps: the value is
/// first canonicalised (sorted keys) so two replicas serialising the same `HashMap` agree, then
/// streamed into the hasher.
pub fn part_token<T: Serialize>(part: &'static str, value: &T) -> flow_like_types::Result<String> {
    let value = super::commands::canonicalize_json(flow_like_types::json::to_value(value)?);
    let mut hasher = token_hasher(part);
    serde_json::to_writer(&mut hasher, &value)?;
    Ok(encode_token(hasher.finalize()))
}

/// Token for a value whose serialisation is already order-stable (`BTreeMap`/`BTreeSet`/structs
/// only — [`SyncSegment::nodes`]). Streams straight into the hasher: no intermediate `Value`, no
/// intermediate `String`.
fn ordered_part_token<T: Serialize>(
    part: &'static str,
    value: &T,
) -> flow_like_types::Result<String> {
    let mut hasher = token_hasher(part);
    serde_json::to_writer(&mut hasher, value)?;
    Ok(encode_token(hasher.finalize()))
}

// ----------------------------------------------------------------------------------------------
// Snapshot: computed once per board revision, diffed per request
// ----------------------------------------------------------------------------------------------

/// Resolves a segment token the *client* holds to the segment it denotes, if this process still
/// has it. Returning `None` is always correct (the whole segment ships); returning a segment whose
/// `hash` differs from the token is a caller bug and is ignored.
pub type SegmentBaseResolver<'a> = &'a dyn Fn(&str) -> Option<Arc<SyncSegment>>;

/// A resolver for callers without a base index.
pub fn no_segment_bases(_token: &str) -> Option<Arc<SyncSegment>> {
    None
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
    segments: HashMap<String, Arc<SyncSegment>>,
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

/// [`node_refs`] for a stored (full-form) wire node — what a patch has at hand.
fn sync_node_refs(node: &SyncNode, refs: &HashMap<String, String>, out: &mut Vec<String>) {
    if let Some(description) = &node.description {
        collect_ref_chain(description, refs, out);
    }
    for pin in node.pins.values() {
        if let Some(description) = &pin.description {
            collect_ref_chain(description, refs, out);
        }
        if let Some(schema) = &pin.schema {
            collect_ref_chain(schema, refs, out);
        }
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
        Self::from_board_incremental(board, catalog, None)
    }

    /// [`Self::from_board`], reusing the token of every segment whose node payload is identical
    /// to `previous` — the snapshot of an earlier revision of the same board, when the caller
    /// still has one. Reuse is decided by comparing the payloads, never by trusting `previous`, so
    /// any earlier revision (or none) is a valid input; only the hashing cost changes.
    pub fn from_board_incremental(
        board: &Board,
        catalog: &[Node],
        previous: Option<&BoardSyncSnapshot>,
    ) -> flow_like_types::Result<Self> {
        let catalog_by_name: HashMap<&str, &Node> = catalog
            .iter()
            .map(|node| (node.name.as_str(), node))
            .collect();

        let mut buckets: HashMap<String, BTreeMap<String, SyncNode>> = HashMap::new();
        for (id, node) in &board.nodes {
            buckets
                .entry(node_segment(node).to_string())
                .or_default()
                .insert(id.clone(), SyncNode::from_node(node));
        }

        let mut segments = HashMap::with_capacity(buckets.len());
        let mut segment_tokens = BTreeMap::new();
        let mut segment_refs = HashMap::with_capacity(buckets.len());
        for (segment_id, nodes) in buckets {
            // The token identifies the payload — never how one client receives it (hydration is
            // decided below and never enters it) and never which catalog was in force.
            let hash = match previous.and_then(|previous| previous.segments.get(&segment_id)) {
                Some(earlier) if earlier.nodes == nodes => earlier.hash.clone(),
                _ => ordered_part_token("segment", &nodes)?,
            };
            let mut hydratable = BTreeSet::new();
            let mut reached = Vec::new();
            for id in nodes.keys() {
                if let Some(node) = board.nodes.get(id) {
                    let catalog = catalog_by_name.get(node.name.as_str()).copied();
                    if SyncNode::hydratable(node, catalog, &board.refs) {
                        hydratable.insert(id.clone());
                    }
                    node_refs(node, &board.refs, &mut reached);
                }
            }
            segment_tokens.insert(segment_id.clone(), hash.clone());
            segment_refs.insert(segment_id.clone(), dedup(reached));
            segments.insert(
                segment_id,
                Arc::new(SyncSegment {
                    hash,
                    nodes,
                    base: None,
                    removed: Vec::new(),
                    hydratable,
                }),
            );
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

    /// The segments of this revision, keyed by segment id. Feed these into a base index so later
    /// requests holding one of their tokens can be answered with patches.
    pub fn segments(&self) -> impl Iterator<Item = (&String, &Arc<SyncSegment>)> {
        self.segments.iter()
    }

    /// The segment of this revision with token `token`, if any. Linear in the number of layers;
    /// suitable as a base resolver over a handful of recent snapshots.
    pub fn segment_by_token(&self, token: &str) -> Option<Arc<SyncSegment>> {
        self.segments
            .values()
            .find(|segment| segment.hash == token)
            .cloned()
    }

    fn ship_node(node: &SyncNode, hydratable: bool, hydrate: bool) -> SyncNode {
        let mut shipped = node.clone();
        if hydrate && hydratable {
            shipped.hydrate = true;
            shipped.lean()
        } else {
            shipped.full()
        }
    }

    fn manifest_delta(&self, request: &BoardSyncRequest) -> BoardSyncManifestDelta {
        let manifest = &self.manifest;
        let differs = |held: &Option<String>, current: &String| {
            (held.as_ref() != Some(current)).then(|| current.clone())
        };
        BoardSyncManifestDelta {
            meta: differs(&request.meta, &manifest.meta),
            variables: differs(&request.variables, &manifest.variables),
            comments: differs(&request.comments, &manifest.comments),
            layers: manifest
                .layers
                .iter()
                .filter(|(id, token)| request.layers.get(*id) != Some(token))
                .map(|(id, token)| (id.clone(), token.clone()))
                .collect(),
            segments: manifest
                .segments
                .iter()
                .filter(|(id, token)| request.segments.get(*id) != Some(token))
                .map(|(id, token)| (id.clone(), token.clone()))
                .collect(),
        }
    }

    /// The parts of this revision that `request` does not already hold, plus every ref those
    /// parts reference. `resolve_base` turns a segment token the client holds into the segment it
    /// denotes so a changed segment can ship as a patch; pass [`no_segment_bases`] to always ship
    /// whole segments.
    pub fn diff(
        &self,
        request: &BoardSyncRequest,
        resolve_base: SegmentBaseResolver<'_>,
    ) -> BoardSyncResponse {
        let changed = |held: &Option<String>, current: &str| held.as_deref() != Some(current);
        let manifest = &self.manifest;
        let mut needed_refs: Vec<&str> = Vec::new();
        let mut patch_refs: Vec<String> = Vec::new();

        let mut segments: HashMap<String, SyncSegment> = HashMap::new();
        for (id, segment) in &self.segments {
            let held = request.segments.get(id);
            if held == Some(&segment.hash) {
                continue;
            }
            let base = held
                .filter(|_| request.patch)
                .and_then(|token| resolve_base(token))
                .filter(|base| Some(&base.hash) == held);

            let shipped = match base {
                Some(base) => {
                    let mut nodes = BTreeMap::new();
                    for (node_id, node) in &segment.nodes {
                        if base.nodes.get(node_id) == Some(node) {
                            continue;
                        }
                        sync_node_refs(node, &self.refs, &mut patch_refs);
                        nodes.insert(
                            node_id.clone(),
                            Self::ship_node(
                                node,
                                segment.hydratable.contains(node_id),
                                request.hydrate,
                            ),
                        );
                    }
                    let removed = base
                        .nodes
                        .keys()
                        .filter(|node_id| !segment.nodes.contains_key(*node_id))
                        .cloned()
                        .collect();
                    SyncSegment {
                        hash: segment.hash.clone(),
                        nodes,
                        base: Some(base.hash.clone()),
                        removed,
                        hydratable: BTreeSet::new(),
                    }
                }
                None => {
                    if let Some(keys) = self.segment_refs.get(id) {
                        needed_refs.extend(keys.iter().map(String::as_str));
                    }
                    SyncSegment {
                        hash: segment.hash.clone(),
                        nodes: segment
                            .nodes
                            .iter()
                            .map(|(node_id, node)| {
                                (
                                    node_id.clone(),
                                    Self::ship_node(
                                        node,
                                        segment.hydratable.contains(node_id),
                                        request.hydrate,
                                    ),
                                )
                            })
                            .collect(),
                        base: None,
                        removed: Vec::new(),
                        hydratable: BTreeSet::new(),
                    }
                }
            };
            segments.insert(id.clone(), shipped);
        }

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

        needed_refs.extend(patch_refs.iter().map(String::as_str));
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

        let (manifest, manifest_delta) = if request.patch {
            (None, Some(self.manifest_delta(request)))
        } else {
            (Some(manifest.clone()), None)
        };

        BoardSyncResponse {
            manifest,
            manifest_delta,
            meta: changed(&request.meta, &self.manifest.meta).then(|| self.meta.clone()),
            variables,
            comments: changed(&request.comments, &self.manifest.comments)
                .then(|| self.comments.clone()),
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

    fn manifest_of(response: &BoardSyncResponse) -> BoardSyncManifest {
        response.manifest.clone().expect("full manifest")
    }

    fn patching(manifest: &BoardSyncManifest) -> BoardSyncRequest {
        BoardSyncRequest {
            patch: true,
            ..BoardSyncRequest::from_manifest(manifest, false)
        }
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
        let response = snapshot.diff(&request, &no_segment_bases);
        assert!(response.meta.is_none());
        assert!(response.variables.is_none());
        assert!(response.refs.is_empty());
        assert!(response.layers.is_empty());
        assert!(response.segments.is_empty());
        assert!(response.dropped_segments.is_empty());
        assert_eq!(manifest_of(&response), snapshot.manifest);
        assert!(response.manifest_delta.is_none());
    }

    #[test]
    fn empty_request_returns_everything() {
        let catalog = catalog_node();
        let board = board_with(vec![placed(&catalog, None), placed(&catalog, Some("l"))]);
        let snapshot = BoardSyncSnapshot::from_board(&board, &[]).expect("snapshot");
        let response = snapshot.diff(&BoardSyncRequest::default(), &no_segment_bases);
        assert!(response.meta.is_some());
        assert!(response.variables.is_some());
        assert_eq!(response.segments.len(), 2);
        assert!(response.segments.values().all(|s| s.base.is_none()));
    }

    #[test]
    fn tokens_are_128_bit_base64url() {
        let catalog = catalog_node();
        let board = board_with(vec![placed(&catalog, None)]);
        let snapshot = BoardSyncSnapshot::from_board(&board, &[]).expect("snapshot");
        for token in [
            &snapshot.manifest.meta,
            &snapshot.manifest.segments[ROOT_SEGMENT],
        ] {
            assert_eq!(token.len(), 22, "16 bytes → 22 unpadded base64url chars");
            assert!(
                token
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                "url-safe alphabet only: {token}"
            );
        }
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
        let full = before.diff(&BoardSyncRequest::default(), &no_segment_bases);
        assert!(full.refs.contains_key("desc-a") && full.refs.contains_key("desc-b"));
        assert!(
            !full.refs.contains_key("orphan"),
            "unreferenced refs never ship"
        );

        // Move `a` one pixel: only the root segment changes, so only its refs come back.
        let mut edited = board.clone();
        edited.nodes.get_mut(&a.id).unwrap().coordinates = Some((1.0, 0.0, 0.0));
        let after = BoardSyncSnapshot::from_board(&edited, &[]).expect("after");
        let response = after.diff(
            &BoardSyncRequest::from_manifest(&before.manifest, false),
            &no_segment_bases,
        );
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
        let response = third.diff(
            &BoardSyncRequest::from_manifest(&after.manifest, false),
            &no_segment_bases,
        );
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
        let response = after.diff(
            &BoardSyncRequest::from_manifest(&before.manifest, false),
            &no_segment_bases,
        );
        assert_eq!(response.layers.keys().collect::<Vec<_>>(), vec!["l2"]);
        assert!(
            response.segments.is_empty(),
            "the layer's nodes did not change"
        );
        assert!(response.dropped_layers.is_empty());

        let mut removed = renamed.clone();
        removed.layers.remove("l1");
        let third = BoardSyncSnapshot::from_board(&removed, &[]).expect("third");
        let response = third.diff(
            &BoardSyncRequest::from_manifest(&after.manifest, false),
            &no_segment_bases,
        );
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
        let response = after.diff(
            &BoardSyncRequest::from_manifest(&before.manifest, false),
            &no_segment_bases,
        );
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
        let response = snapshot.diff(&request, &no_segment_bases);
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
    fn incremental_build_reuses_tokens_and_agrees_with_a_full_build() {
        let catalog = catalog_node();
        let a = placed(&catalog, None);
        let b = placed(&catalog, Some("l"));
        let board = board_with(vec![a.clone(), b]);
        let previous = BoardSyncSnapshot::from_board(&board, &[]).expect("previous");

        let mut edited = board.clone();
        edited.nodes.get_mut(&a.id).unwrap().coordinates = Some((5.0, 5.0, 0.0));
        let full = BoardSyncSnapshot::from_board(&edited, &[]).expect("full");
        let incremental = BoardSyncSnapshot::from_board_incremental(&edited, &[], Some(&previous))
            .expect("incremental");
        assert_eq!(full.manifest, incremental.manifest);
        assert_eq!(
            previous.manifest.segments["l"], incremental.manifest.segments["l"],
            "untouched segment keeps its token"
        );
        assert_ne!(
            previous.manifest.segments[ROOT_SEGMENT],
            incremental.manifest.segments[ROOT_SEGMENT]
        );
        // A previous snapshot of an unrelated board is harmless: reuse is by comparison.
        let unrelated = BoardSyncSnapshot::from_board(
            &board_with(vec![placed(&catalog, Some("elsewhere"))]),
            &[],
        )
        .expect("unrelated");
        let against_unrelated =
            BoardSyncSnapshot::from_board_incremental(&edited, &[], Some(&unrelated))
                .expect("incremental vs unrelated");
        assert_eq!(full.manifest, against_unrelated.manifest);
    }

    #[test]
    fn a_known_base_turns_a_changed_segment_into_a_node_patch() {
        let catalog = catalog_node();
        let moved = placed(&catalog, None);
        let stays = placed(&catalog, None);
        let gone = placed(&catalog, None);
        let board = board_with(vec![moved.clone(), stays.clone(), gone.clone()]);
        let before = BoardSyncSnapshot::from_board(&board, &[]).expect("before");

        let mut edited = board.clone();
        edited.nodes.get_mut(&moved.id).unwrap().coordinates = Some((9.0, 9.0, 0.0));
        edited.nodes.remove(&gone.id);
        let added = placed(&catalog, None);
        edited.nodes.insert(added.id.clone(), added.clone());
        let after = BoardSyncSnapshot::from_board(&edited, &[]).expect("after");

        let resolver = |token: &str| before.segment_by_token(token);
        let response = after.diff(&patching(&before.manifest), &resolver);
        let root = &response.segments[ROOT_SEGMENT];
        assert_eq!(
            root.base.as_deref(),
            Some(before.manifest.segments[ROOT_SEGMENT].as_str())
        );
        assert_eq!(root.hash, after.manifest.segments[ROOT_SEGMENT]);
        let mut upserted: Vec<_> = root.nodes.keys().cloned().collect();
        upserted.sort();
        let mut expected = vec![moved.id.clone(), added.id.clone()];
        expected.sort();
        assert_eq!(upserted, expected, "only the changed and the new node ship");
        assert_eq!(root.removed, vec![gone.id.clone()]);

        // The manifest comes back as a delta relative to the request.
        assert!(response.manifest.is_none());
        let delta = response.manifest_delta.expect("delta");
        assert_eq!(delta.segments.len(), 1);
        assert_eq!(
            delta.segments[ROOT_SEGMENT],
            after.manifest.segments[ROOT_SEGMENT]
        );
        assert!(delta.layers.is_empty());
        assert!(delta.variables.is_none());
        assert!(delta.meta.is_none(), "same fixed timestamps");
    }

    #[test]
    fn patches_need_the_opt_in_and_a_matching_base() {
        let catalog = catalog_node();
        let node = placed(&catalog, None);
        let board = board_with(vec![node.clone(), placed(&catalog, None)]);
        let before = BoardSyncSnapshot::from_board(&board, &[]).expect("before");
        let mut edited = board.clone();
        edited.nodes.get_mut(&node.id).unwrap().coordinates = Some((1.0, 1.0, 0.0));
        let after = BoardSyncSnapshot::from_board(&edited, &[]).expect("after");
        let resolver = |token: &str| before.segment_by_token(token);

        let legacy = after.diff(
            &BoardSyncRequest::from_manifest(&before.manifest, false),
            &resolver,
        );
        let root = &legacy.segments[ROOT_SEGMENT];
        assert!(root.base.is_none(), "no opt-in, no patch");
        assert_eq!(root.nodes.len(), 2);
        assert!(legacy.manifest.is_some() && legacy.manifest_delta.is_none());

        // A resolver that answers with the wrong segment for a token is ignored.
        let lying = |_: &str| after.segment_by_token(&after.manifest.segments[ROOT_SEGMENT]);
        let response = after.diff(&patching(&before.manifest), &lying);
        assert!(response.segments[ROOT_SEGMENT].base.is_none());
        assert_eq!(response.segments[ROOT_SEGMENT].nodes.len(), 2);

        // Unknown token: whole segment.
        let response = after.diff(&patching(&before.manifest), &no_segment_bases);
        assert!(response.segments[ROOT_SEGMENT].base.is_none());
    }

    #[test]
    fn a_patch_ships_only_the_refs_its_upserts_need() {
        let catalog = catalog_node();
        let mut a = placed(&catalog, None);
        let mut b = placed(&catalog, None);
        let mut board = board_with(vec![]);
        board.refs.insert("desc-a".into(), "A".into());
        board.refs.insert("desc-b".into(), "B".into());
        a.description = "desc-a".into();
        b.description = "desc-b".into();
        board.nodes.insert(a.id.clone(), a.clone());
        board.nodes.insert(b.id.clone(), b.clone());
        let before = BoardSyncSnapshot::from_board(&board, &[]).expect("before");
        let mut edited = board.clone();
        edited.nodes.get_mut(&a.id).unwrap().coordinates = Some((2.0, 0.0, 0.0));
        let after = BoardSyncSnapshot::from_board(&edited, &[]).expect("after");
        let resolver = |token: &str| before.segment_by_token(token);
        let response = after.diff(&patching(&before.manifest), &resolver);
        assert_eq!(response.refs.keys().collect::<Vec<_>>(), vec!["desc-a"]);
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
        let root = &snapshot.segments[ROOT_SEGMENT];
        assert!(
            root.nodes.values().all(|node| !node.hydrate),
            "stored = payload"
        );
        assert!(root.hydratable.contains(&pristine.id));
        assert!(
            root.hydratable.contains(&renamed.id),
            "friendly_name is instance data, not a disqualifier"
        );
        assert!(!root.hydratable.contains(&retyped.id), "dynamic pin type");
        assert!(!root.hydratable.contains(&older.id), "version mismatch");
        assert!(
            !root.hydratable.contains(&extra_pin.id),
            "pin missing from catalog"
        );

        let hydrated = snapshot.diff(
            &BoardSyncRequest {
                hydrate: true,
                ..Default::default()
            },
            &no_segment_bases,
        );
        let lean = &hydrated.segments[ROOT_SEGMENT].nodes[&pristine.id];
        assert!(lean.hydrate);
        assert!(lean.description.is_none());
        assert_eq!(lean.friendly_name, "Demo", "renamable, so it always ships");
        assert!(lean.pins.values().all(|p| p.data_type.is_none()));
        let full = &hydrated.segments[ROOT_SEGMENT].nodes[&renamed.id];
        assert!(full.hydrate, "a rename alone must not disqualify hydration");
        assert_eq!(full.friendly_name, "Call something");

        let plain = snapshot.diff(&BoardSyncRequest::default(), &no_segment_bases);
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
        assert!(
            snapshot.segments[ROOT_SEGMENT]
                .hydratable
                .contains(&node.id)
        );
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
        assert!(
            !snapshot.segments[ROOT_SEGMENT]
                .hydratable
                .contains(&node.id)
        );
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
        let response = snapshot.diff(&BoardSyncRequest::default(), &no_segment_bases);
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

    /// Timing harness, not a test:
    /// `cargo test -p flow-like-runtime --release board_sync_timing -- --ignored --nocapture`.
    /// Prints the cost of a full build, an incremental rebuild after a one-node edit, and the two
    /// diff shapes on a synthetic 1000-node / 80-layer board.
    #[test]
    #[ignore]
    fn board_sync_timing() {
        use std::time::Instant;
        let catalog = catalog_node();
        let mut nodes = Vec::with_capacity(1000);
        for index in 0..1000 {
            let layer = if index % 5 == 0 {
                None
            } else {
                Some(format!("layer-{}", index % 80))
            };
            let mut node = placed(&catalog, layer.as_deref());
            node.coordinates = Some((index as f32, (index * 2) as f32, 0.0));
            for pin in node.pins.values_mut() {
                pin.default_value = Some(format!("\"value-{index}\"").into_bytes());
                pin.schema = Some(format!("schema-{}", index % 10));
            }
            nodes.push(node);
        }
        let mut board = board_with(nodes.clone());
        for index in 0..10 {
            board.refs.insert(
                format!("schema-{index}"),
                format!(
                    "{{\"type\":\"object\",\"properties\":{{\"p{index}\":{{\"type\":\"string\"}}}}}}"
                ),
            );
        }
        for index in 0..80 {
            let id = format!("layer-{index}");
            board.layers.insert(
                id.clone(),
                Layer::new(
                    id,
                    format!("Layer {index}"),
                    super::super::LayerType::Collapsed,
                ),
            );
        }

        let started = Instant::now();
        let previous = BoardSyncSnapshot::from_board(&board, &[]).expect("full");
        let full_build = started.elapsed();

        let mut edited = board.clone();
        let edited_id = nodes[7].id.clone();
        edited.nodes.get_mut(&edited_id).unwrap().coordinates = Some((-1.0, -1.0, 0.0));
        let started = Instant::now();
        let rebuilt = BoardSyncSnapshot::from_board(&edited, &[]).expect("full again");
        let full_rebuild = started.elapsed();
        let started = Instant::now();
        let incremental = BoardSyncSnapshot::from_board_incremental(&edited, &[], Some(&previous))
            .expect("incremental");
        let incremental_rebuild = started.elapsed();
        assert_eq!(rebuilt.manifest, incremental.manifest);

        let request = BoardSyncRequest::from_manifest(&previous.manifest, false);
        let started = Instant::now();
        let whole = incremental.diff(&request, &no_segment_bases);
        let whole_diff = started.elapsed();
        let whole_bytes = flow_like_types::json::to_vec(&whole).expect("json").len();

        let resolver = |token: &str| previous.segment_by_token(token);
        let started = Instant::now();
        let patched = incremental.diff(&patching(&previous.manifest), &resolver);
        let patch_diff = started.elapsed();
        let patch_bytes = flow_like_types::json::to_vec(&patched).expect("json").len();

        let manifest_bytes = flow_like_types::json::to_vec(&previous.manifest)
            .expect("json")
            .len();
        println!(
            "nodes={} layers={}",
            edited.nodes.len(),
            edited.layers.len()
        );
        println!("full build           {full_build:?}");
        println!("full rebuild         {full_rebuild:?}");
        println!("incremental rebuild  {incremental_rebuild:?}");
        println!("diff, whole segment  {whole_diff:?} → {whole_bytes} bytes");
        println!("diff, node patch     {patch_diff:?} → {patch_bytes} bytes");
        println!("manifest             {manifest_bytes} bytes");
    }
}
