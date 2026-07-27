//! COLD section: display strings that runtime code genuinely reads, but only outside the
//! per-node hot path.
//!
//! These cannot be dropped — an audit of the catalog found real `run()`-time consumers:
//! LLM tool generation (node friendly name becomes the tool name; pin descriptions,
//! schemas and enum options become the tool's JSON schema), the lazy tool index, the MCP
//! and REST/OpenAPI surfaces, form building, and a couple of error paths. They are split
//! out so the common case never pays for them: the section is validated lazily, on first
//! access, and a plain flow that never touches an agent node never decodes it at all.
//!
//! Ref indirection is resolved at compile time. Boards store descriptions and schemas as
//! `board.refs` hashes that the runtime used to look up on every call; the compiler
//! inlines them here so the refs map does not exist in the artifact.
//!
//! Inlining alone would be a size disaster, because `board.refs` is also what deduplicates
//! those strings — one JSON schema is routinely shared by hundreds of pins. So this section
//! keeps its own interned string pool and stores `u32` handles, restoring the sharing the
//! ref table used to provide.

use rkyv::{Archive, Deserialize, Serialize};

pub const COLD_SECTION_VERSION: u16 = 1;
pub const DEBUG_SECTION_VERSION: u16 = 1;

/// Handle 0 is always the empty string, so "absent" needs no discriminant.
pub const EMPTY_STRING: u32 = 0;

/// Parallel arrays indexed by the same node/pin indices as the HOT section, holding
/// handles into [`ColdPlan::strings`].
#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct ColdPlan {
    /// Interned pool. Element 0 is the empty string by construction.
    pub strings: Vec<String>,

    pub node_friendly_names: Vec<u32>,
    pub node_descriptions: Vec<u32>,

    pub pin_friendly_names: Vec<u32>,
    pub pin_descriptions: Vec<u32>,
    /// JSON schema handles, already ref-resolved.
    pub pin_schemas: Vec<u32>,
    /// Enum options surfaced to LLM tooling and forms. An empty row means unconstrained.
    pub pin_valid_values: Vec<Vec<u32>>,

    pub layer_names: Vec<u32>,
}

impl Default for ColdPlan {
    fn default() -> Self {
        Self {
            strings: vec![String::new()],
            node_friendly_names: Vec::new(),
            node_descriptions: Vec::new(),
            pin_friendly_names: Vec::new(),
            pin_descriptions: Vec::new(),
            pin_schemas: Vec::new(),
            pin_valid_values: Vec::new(),
            layer_names: Vec::new(),
        }
    }
}

impl ArchivedColdPlan {
    pub fn string(&self, handle: u32) -> &str {
        self.strings
            .get(handle as usize)
            .map(|value| value.as_str())
            .unwrap_or("")
    }

    fn lookup(&self, table: &rkyv::vec::ArchivedVec<rkyv::rend::u32_le>, index: u32) -> &str {
        table
            .get(index as usize)
            .map(|handle| self.string(handle.to_native()))
            .unwrap_or("")
    }

    pub fn node_friendly_name(&self, node: u32) -> &str {
        self.lookup(&self.node_friendly_names, node)
    }

    pub fn node_description(&self, node: u32) -> &str {
        self.lookup(&self.node_descriptions, node)
    }

    pub fn pin_friendly_name(&self, pin: u32) -> &str {
        self.lookup(&self.pin_friendly_names, pin)
    }

    pub fn pin_description(&self, pin: u32) -> &str {
        self.lookup(&self.pin_descriptions, pin)
    }

    /// `None` when the pin carries no schema, matching `Pin::schema`.
    pub fn pin_schema(&self, pin: u32) -> Option<&str> {
        let schema = self.lookup(&self.pin_schemas, pin);
        (!schema.is_empty()).then_some(schema)
    }

    /// Enum options for a pin, empty when unconstrained.
    pub fn pin_valid_values(&self, pin: u32) -> Vec<&str> {
        self.pin_valid_values
            .get(pin as usize)
            .map(|row| row.iter().map(|h| self.string(h.to_native())).collect())
            .unwrap_or_default()
    }

    pub fn layer_name(&self, layer: u32) -> &str {
        self.lookup(&self.layer_names, layer)
    }
}

/// Viewer-only names. Never read by the execution runtime.
///
/// Log viewers today resolve node ids against the currently open `.board`, which shows
/// "Unknown Node" for runs of a board version that has since changed. Carrying names here
/// lets a viewer render historical runs faithfully; it is optional and may be empty.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Default)]
#[rkyv(derive(Debug))]
pub struct DebugPlan {
    pub node_ids: Vec<String>,
    pub node_friendly_names: Vec<String>,
    pub node_icons: Vec<String>,
}
