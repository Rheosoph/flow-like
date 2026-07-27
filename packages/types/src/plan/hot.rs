//! HOT section: topology and everything the per-node execution path touches.
//!
//! Structure-of-arrays, `u32` indices, CSR adjacency. Strings appear only where a runtime
//! consumer provably needs them (node type keys for registry resolution, node instance ids
//! for log records, pin names for the string-API shim) — and even those are interned and
//! resolved once at hydration, never inside a run.

use rkyv::{Archive, Deserialize, Serialize};

/// Bump on any change to a struct in this module.
pub const HOT_SECTION_VERSION: u16 = 2;

/// FNV-1a over the name bytes. Used to key the per-node pin-name lookup so a name
/// resolution is an integer binary search instead of a string hash into a locked map.
///
/// Deterministic and dependency-free by design: the same bytes must produce the same hash
/// in the compiler and in every runtime that reads the artifact.
pub const fn name_hash(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    hash
}

pub mod node_flags {
    pub const IS_PURE: u32 = 1 << 0;
    pub const CAN_REFERENCE_FNS: u32 = 1 << 1;
    pub const CAN_BE_REFERENCED_BY_FNS: u32 = 1 << 2;
    /// Node carries at least one output execution pin; lets the scheduler skip nodes that
    /// can never continue the flow without touching their pins.
    pub const HAS_EXEC_OUTPUT: u32 = 1 << 3;
}

pub mod pin_flags {
    pub const IS_LAYER_PIN: u16 = 1 << 0;
    pub const HAS_DEFAULT: u16 = 1 << 1;
    /// `data_type == Execution`. Cached as a flag so the scheduler can partition pins
    /// without decoding the type byte.
    pub const IS_EXEC: u16 = 1 << 2;
}

pub mod variable_flags {
    pub const SECRET: u8 = 1 << 0;
    pub const EXPOSED: u8 = 1 << 1;
    pub const RUNTIME_CONFIGURED: u8 = 1 << 2;
    pub const EDITABLE: u8 = 1 << 3;
}

/// Typed wrappers so callers do not pass raw bitmasks around.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeFlags(pub u32);

impl NodeFlags {
    pub fn is_pure(self) -> bool {
        self.0 & node_flags::IS_PURE != 0
    }
    pub fn can_reference_fns(self) -> bool {
        self.0 & node_flags::CAN_REFERENCE_FNS != 0
    }
    pub fn can_be_referenced_by_fns(self) -> bool {
        self.0 & node_flags::CAN_BE_REFERENCED_BY_FNS != 0
    }
    pub fn has_exec_output(self) -> bool {
        self.0 & node_flags::HAS_EXEC_OUTPUT != 0
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PinFlags(pub u16);

impl PinFlags {
    pub fn is_layer_pin(self) -> bool {
        self.0 & pin_flags::IS_LAYER_PIN != 0
    }
    pub fn has_default(self) -> bool {
        self.0 & pin_flags::HAS_DEFAULT != 0
    }
    pub fn is_exec(self) -> bool {
        self.0 & pin_flags::IS_EXEC != 0
    }
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct PlanNode {
    /// Symbol index of the board instance id (cuid2). Log records are keyed by it and
    /// viewers join it against the `.board`, so it must stay byte-stable with the source.
    pub instance_id: u32,
    /// Symbol index of the node type key (`Node::name`) used for registry resolution.
    pub type_key: u32,
    pub flags: u32,
    /// Range into `HotPlan::pins`.
    pub first_pin: u32,
    pub pin_count: u32,
    /// Index into `HotPlan::layers`, or `NONE_INDEX`.
    pub layer: u32,
    /// Symbol index of the WASM package id, or `NONE_INDEX` for native nodes.
    pub wasm_package: u32,
    /// Precomputed [`crate`]-side semantic hash, replacing the runtime `Node::hash()`
    /// recompute that used to force layout fields to stay in the artifact.
    pub semantic_hash: u64,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct PlanPin {
    /// Symbol index of the pin id.
    pub id: u32,
    /// Symbol index of the pin name.
    pub name: u32,
    /// Owning node index, or `NONE_INDEX` for layer relay pins.
    pub owner_node: u32,
    /// Blob index of the canonical JSON default, or `NONE_INDEX`.
    pub default_value: u32,
    pub index: u16,
    pub flags: u16,
    pub pin_type: u8,
    pub data_type: u8,
    pub value_type: u8,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct PlanVariable {
    /// Symbol index of the variable id.
    pub id: u32,
    /// Symbol index of the variable name.
    ///
    /// No execution path resolves a variable by name — they are keyed by id everywhere —
    /// but a board projected from a plan needs it to stay faithful, and an unnamed
    /// variable is a debugging footgun. Interned, so a handful of names costs nothing.
    pub name: u32,
    /// Blob index of the canonical JSON default, or `NONE_INDEX`.
    pub default_value: u32,
    /// Owning layer index for function-local variables, or `NONE_INDEX` for board scope.
    pub owner_layer: u32,
    pub data_type: u8,
    pub value_type: u8,
    pub flags: u8,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct PlanLayer {
    /// Symbol index of the layer id.
    pub id: u32,
    /// Parent layer index, or `NONE_INDEX`.
    pub parent: u32,
    pub layer_type: u8,
}

/// One entry of a node's pin-name lookup table, sorted by `hash` then `pin`.
///
/// A node may legitimately carry several pins with the same name (that is how the editor
/// lets users add more inputs of a type), so lookups resolve to a contiguous range.
#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct PinNameEntry {
    pub hash: u64,
    pub pin: u32,
}

/// Compressed-sparse-row adjacency: neighbours of `i` are
/// `edges[offsets[i] as usize..offsets[i + 1] as usize]`.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Default)]
#[rkyv(derive(Debug))]
pub struct Csr {
    pub offsets: Vec<u32>,
    pub edges: Vec<u32>,
}

impl Csr {
    pub fn with_capacity(rows: usize, edges: usize) -> Self {
        let mut offsets = Vec::with_capacity(rows + 1);
        offsets.push(0);
        Self {
            offsets,
            edges: Vec::with_capacity(edges),
        }
    }

    /// Close the current row. Rows must be pushed in index order.
    pub fn finish_row(&mut self) {
        self.offsets.push(self.edges.len() as u32);
    }

    pub fn row(&self, index: usize) -> &[u32] {
        let start = self.offsets[index] as usize;
        let end = self.offsets[index + 1] as usize;
        &self.edges[start..end]
    }
}

impl ArchivedCsr {
    pub fn row(&self, index: usize) -> &[rkyv::rend::u32_le] {
        let start = self.offsets[index].to_native() as usize;
        let end = self.offsets[index + 1].to_native() as usize;
        &self.edges[start..end]
    }
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct HotPlan {
    pub board_id: String,
    pub stage: u8,
    pub log_level: u8,

    /// Interned strings. Every `*_symbol`/`id`/`name` field indexes into this.
    pub symbols: Vec<String>,
    /// Canonical JSON blobs for pin and variable defaults, parsed once at compile time.
    pub blobs: Vec<Vec<u8>>,

    pub nodes: Vec<PlanNode>,
    pub pins: Vec<PlanPin>,
    pub variables: Vec<PlanVariable>,
    pub layers: Vec<PlanLayer>,

    /// pin -> pins it feeds.
    pub pin_connections: Csr,
    /// pin -> pins it depends on.
    pub pin_dependencies: Csr,
    /// node -> nodes reachable through its output execution pins.
    pub exec_successors: Csr,
    /// impure node -> topologically ordered pure nodes to evaluate before it.
    pub pure_schedules: Csr,
    /// node -> symbol indices of the node ids it may call (`fn_refs`).
    pub fn_refs: Csr,
    /// node -> WASM sandbox permission grants, encoded via `NodePermission::to_plan_u8`.
    ///
    /// Security-relevant: a node whose grants were dropped would either be denied
    /// capabilities it legitimately declared, or — worse, if a consumer ever treats an
    /// empty set permissively — be handed more than it declared.
    pub wasm_permissions: Csr,
    /// layer -> variable indices scoped to it.
    pub layer_variables: Csr,

    /// Node indices sorted by instance-id string, for id -> index binary search.
    pub nodes_by_id: Vec<u32>,
    /// Per-node pin-name lookup, CSR-indexed by node.
    pub pin_name_offsets: Vec<u32>,
    pub pin_name_entries: Vec<PinNameEntry>,

    /// Nodes flagged as flow entry points in the editor. Actual entry selection is by
    /// payload id through `nodes_by_id`; this is a fallback and a validation aid.
    pub start_nodes: Vec<u32>,
}

impl ArchivedHotPlan {
    pub fn symbol(&self, index: u32) -> &str {
        &self.symbols[index as usize]
    }

    /// Resolve a node instance id to its index. `O(log n)` over the sorted id table.
    pub fn node_by_id(&self, id: &str) -> Option<u32> {
        let nodes_by_id = &self.nodes_by_id;
        let mut low = 0usize;
        let mut high = nodes_by_id.len();
        while low < high {
            let mid = (low + high) / 2;
            let node_index = nodes_by_id[mid].to_native();
            let candidate = self.symbol(self.nodes[node_index as usize].instance_id.to_native());
            match candidate.cmp(id) {
                std::cmp::Ordering::Less => low = mid + 1,
                std::cmp::Ordering::Greater => high = mid,
                std::cmp::Ordering::Equal => return Some(node_index),
            }
        }
        None
    }

    /// Resolve a pin name within a node to the pin indices carrying it, in stable order.
    ///
    /// Replaces the double-locked `pin_name_cache` map lookup: an integer binary search
    /// over a contiguous slice, with a string compare only to rule out hash collisions.
    pub fn pins_by_name(&self, node: u32, name: &str) -> Vec<u32> {
        let start = self.pin_name_offsets[node as usize].to_native() as usize;
        let end = self.pin_name_offsets[node as usize + 1].to_native() as usize;
        let entries = &self.pin_name_entries[start..end];
        let target = name_hash(name);

        let mut low = 0usize;
        let mut high = entries.len();
        while low < high {
            let mid = (low + high) / 2;
            if entries[mid].hash.to_native() < target {
                low = mid + 1;
            } else {
                high = mid;
            }
        }

        let mut out = Vec::new();
        for entry in &entries[low..] {
            if entry.hash.to_native() != target {
                break;
            }
            let pin_index = entry.pin.to_native();
            if self.symbol(self.pins[pin_index as usize].name.to_native()) == name {
                out.push(pin_index);
            }
        }
        out
    }
}
