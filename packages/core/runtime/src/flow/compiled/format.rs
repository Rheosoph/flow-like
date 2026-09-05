//! rkyv wire format for compiled boards.
//!
//! A `CompiledBoard` is the execution-scoped snapshot of a board *after*
//! `node_updates` + `cleanup` have run: dynamic pins are minted, schemas are
//! synced, reroutes are spliced out. Editor-only data (coordinates, comments,
//! icons, docs, scores, colors) is dropped. Everything read at execution time
//! survives — including node/pin display names and descriptions, which agentic
//! nodes turn into LLM/MCP/REST tool definitions at runtime.
//!
//! Layout invariants:
//! - All pins live in one global arena; edges are `u32` indices into it.
//!   Ids are retained (runtime lookups by id), but wiring never touches them.
//! - Nodes from function-layer bodies live in the same node arena, tagged with
//!   `body_layer`, mirroring how the engine flattens them into one graph.
//! - Enum fields are stored as explicit `u8` codes (not derived archived
//!   enums) so the on-disk representation stays under our control.

use rkyv::{Archive, Deserialize, Serialize};

/// Bump on ANY change to the structs in this file. A mismatch invalidates
/// every persisted artifact; loaders fall back to compiling from the proto.
pub const FORMAT_VERSION: u16 = 1;

pub const MAGIC: [u8; 4] = *b"FLCB";

/// Sentinel for "no node" / "no layer" references.
pub const NONE_IDX: u32 = u32::MAX;

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledBoard {
    pub id: String,
    /// Kept: surfaces in run bookkeeping (desktop run list).
    pub name: String,
    pub version: (u32, u32, u32),
    /// `ExecutionStage` code
    pub stage: u8,
    /// `LogLevel` code
    pub log_level: u8,
    /// `ExecutionMode` code
    pub execution_mode: u8,
    /// Schema-ref table (`board.refs`) — resolved at runtime by tool generation.
    pub refs: Vec<(String, String)>,
    pub page_ids: Vec<String>,
    pub variables: Vec<CompiledVariable>,
    pub layers: Vec<CompiledLayer>,
    pub nodes: Vec<CompiledNode>,
    /// Global pin arena. Order groups a node's pins contiguously.
    pub pins: Vec<CompiledPin>,
}

impl CompiledBoard {
    /// Bounds-check every arena index. rkyv validation guarantees structure,
    /// not semantics — a bit-flipped or mis-produced artifact can decode fine
    /// and then panic deep inside template build or per-run wiring. Loaders
    /// call this after decode so a bad artifact falls back to compiling from
    /// source instead of wedging the board in a panic loop.
    pub fn validate(&self) -> flow_like_types::Result<()> {
        let pins = self.pins.len() as u32;
        let nodes = self.nodes.len() as u32;
        let layers = self.layers.len() as u32;
        let ok = |idx: u32, bound: u32| idx == NONE_IDX || idx < bound;

        for (i, pin) in self.pins.iter().enumerate() {
            if !ok(pin.owner_node, nodes)
                || !ok(pin.owner_layer, layers)
                || pin.depends_on.iter().any(|&p| p >= pins)
                || pin.connected_to.iter().any(|&p| p >= pins)
            {
                return Err(flow_like_types::anyhow!(
                    "compiled board {}: pin {i} carries an out-of-range index",
                    self.id
                ));
            }
        }
        for (i, node) in self.nodes.iter().enumerate() {
            if !ok(node.layer, layers)
                || !ok(node.body_layer, layers)
                || node.pins.iter().any(|&p| p >= pins)
            {
                return Err(flow_like_types::anyhow!(
                    "compiled board {}: node {i} carries an out-of-range index",
                    self.id
                ));
            }
        }
        for (i, layer) in self.layers.iter().enumerate() {
            if !ok(layer.parent_layer, layers) || layer.pins.iter().any(|&p| p >= pins) {
                return Err(flow_like_types::anyhow!(
                    "compiled board {}: layer {i} carries an out-of-range index",
                    self.id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledPin {
    pub id: String,
    pub name: String,
    /// `None` = identical to the catalog default for this (node type, pin
    /// name, direction) — reinflated from the registry at reconstruction.
    /// Safe because the artifact is fingerprint-bound to that registry.
    pub friendly_name: Option<String>,
    /// `None` = identical to the catalog default (see `friendly_name`).
    pub description: Option<String>,
    /// `PinType` code (input/output)
    pub pin_type: u8,
    /// `VariableType` code
    pub data_type: u8,
    /// `ValueType` code
    pub value_type: u8,
    pub schema: Option<String>,
    pub options: Option<CompiledPinOptions>,
    /// serde_json bytes, identical to `Pin.default_value`
    pub default_value: Option<Vec<u8>>,
    pub index: u16,
    /// Owning node (arena index) — `NONE_IDX` for layer relay pins.
    pub owner_node: u32,
    /// Owning layer (arena index) for relay pins — `NONE_IDX` otherwise.
    pub owner_layer: u32,
    /// Upstream pins (arena indices).
    pub depends_on: Vec<u32>,
    /// Downstream pins (arena indices).
    pub connected_to: Vec<u32>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledPinOptions {
    pub sensitive: Option<bool>,
    pub valid_values: Option<Vec<String>>,
    pub range: Option<(f64, f64)>,
    pub step: Option<f64>,
    pub enforce_schema: Option<bool>,
    pub enforce_generic_value_type: Option<bool>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledNode {
    pub id: String,
    /// Catalog type key — resolves to `NodeLogic` through the registry.
    pub name: String,
    /// `None` = identical to the catalog default node — reinflated from the
    /// registry at reconstruction (fingerprint-bound, so always available).
    /// User renames always serialize as `Some`.
    pub friendly_name: Option<String>,
    /// `None` = identical to the catalog default (see `friendly_name`).
    pub description: Option<String>,
    /// Kept because it feeds `semantic_hash` (lazy-tool embedding index key).
    /// `None` = identical to the catalog default.
    pub category: Option<String>,
    pub start: bool,
    pub long_running: bool,
    pub event_callback: bool,
    pub only_offline: bool,
    /// Node schema version (`Node.version`); `u32::MAX` = unversioned.
    pub node_version: u32,
    /// Layer this node is tagged with (`Node.layer`) — `NONE_IDX` if none.
    pub layer: u32,
    /// Function layer whose body contains this node — `NONE_IDX` for
    /// top-level board nodes.
    pub body_layer: u32,
    /// This node's pins (arena indices).
    pub pins: Vec<u32>,
    pub fn_refs: Option<CompiledFnRefs>,
    pub oauth_providers: Vec<String>,
    pub required_oauth_scopes: Vec<(String, Vec<String>)>,
    pub wasm: Option<CompiledNodeWasm>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledFnRefs {
    pub fn_refs: Vec<String>,
    pub can_reference_fns: bool,
    pub can_be_referenced_by_fns: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledNodeWasm {
    pub package_id: String,
    /// `NodePermission` codes
    pub permissions: Vec<u8>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledVariable {
    pub id: String,
    pub name: String,
    /// serde_json bytes, identical to `Variable.default_value`
    pub default_value: Option<Vec<u8>>,
    /// `VariableType` code
    pub data_type: u8,
    /// `ValueType` code
    pub value_type: u8,
    pub exposed: bool,
    pub secret: bool,
    pub editable: bool,
    pub runtime_configured: bool,
    pub schema: Option<String>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledLayer {
    pub id: String,
    /// Kept: `layer.name` appears in call_function error/log lines.
    pub name: String,
    pub parent_layer: u32,
    /// `LayerType` code
    pub layer_type: u8,
    pub variables: Vec<CompiledVariable>,
    /// Boundary relay pins (arena indices).
    pub pins: Vec<u32>,
    pub cache: Option<CompiledLayerCache>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct CompiledLayerCache {
    pub enabled: bool,
    pub prefix: String,
    pub ttl_seconds: Option<u64>,
    /// `LayerCacheScope` code
    pub scope: u8,
}
