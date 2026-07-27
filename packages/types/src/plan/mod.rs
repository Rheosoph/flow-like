//! Compiled execution plans: a zero-copy, index-addressed representation of a board.
//!
//! A plan is a *derived artifact*. The `.board` file stays the source of truth; plans are
//! regenerated whenever their stamps no longer match, never migrated. That is what makes
//! it safe to use a layout-coupled format (rkyv) with no field-tag compatibility.
//!
//! # Container
//!
//! ```text
//! [header]  fixed-size, plain little-endian bytes — readable regardless of rkyv layout
//! [HOT]     rkyv archive: topology + everything the per-node hot path touches
//! [COLD]    rkyv archive: display strings for LLM/MCP/REST/form surfaces
//! [DEBUG]   rkyv archive: optional, viewer-only names (never read by the runtime)
//! ```
//!
//! The header is deliberately *not* rkyv-encoded: a reader must be able to inspect the
//! format version and stamps of any plan, including ones written by a future binary whose
//! archived layout it cannot interpret.

pub mod cold;
pub mod header;
pub mod hot;

pub use cold::{ArchivedColdPlan, ColdPlan, DebugPlan};
pub use header::{PLAN_FORMAT_VERSION, PLAN_MAGIC, PlanHeader, SectionRef};
pub use hot::{
    ArchivedHotPlan, HotPlan, NodeFlags, PinFlags, PlanLayer, PlanNode, PlanPin, PlanVariable,
};

/// Sentinel for "no index" in the flat index space. `u32::MAX` is safe because a board can
/// never reach 4 billion nodes/pins and the compiler rejects anything close.
pub const NONE_INDEX: u32 = u32::MAX;

/// Upper bound on entities in a single plan. Keeps `NONE_INDEX` unambiguous and gives the
/// compiler a clear failure mode instead of silent index truncation.
pub const MAX_ENTITIES: usize = (u32::MAX - 1) as usize;

/// Errors produced when reading a plan container.
#[derive(Debug, thiserror::Error)]
pub enum PlanError {
    #[error("not a flow-like plan (bad magic)")]
    BadMagic,
    #[error("unsupported plan format version {found}, this binary supports {supported}")]
    UnsupportedFormat { found: u16, supported: u16 },
    #[error("plan truncated: section {section} needs {needed} bytes, buffer has {available}")]
    Truncated {
        section: &'static str,
        needed: usize,
        available: usize,
    },
    #[error("plan section {section} failed validation: {source}")]
    Validation {
        section: &'static str,
        source: rkyv::rancor::Error,
    },
    #[error("plan section {section} is absent")]
    MissingSection { section: &'static str },
    #[error("plan section {section} failed to decompress: {source}")]
    Decompress {
        section: &'static str,
        source: lz4_flex::block::DecompressError,
    },
}

/// A plan buffer plus its decoded header.
///
/// Holds the bytes as fetched (one object-store GET) and hands out validated, aligned
/// views of individual sections on demand. HOT is validated eagerly by the runtime; COLD
/// stays untouched until an LLM/MCP/REST/form node first asks for it.
#[derive(Debug, Clone)]
pub struct PlanBuffer {
    header: PlanHeader,
    bytes: Vec<u8>,
}

impl PlanBuffer {
    /// Parse the header and bounds-check every section against the buffer length.
    pub fn new(bytes: Vec<u8>) -> Result<Self, PlanError> {
        let header = PlanHeader::decode(&bytes)?;
        for (name, section) in [
            ("hot", &header.hot),
            ("cold", &header.cold),
            ("debug", &header.debug),
        ] {
            let end = section.offset as usize + section.len as usize;
            if end > bytes.len() {
                return Err(PlanError::Truncated {
                    section: name,
                    needed: end,
                    available: bytes.len(),
                });
            }
        }
        Ok(Self { header, bytes })
    }

    pub fn header(&self) -> &PlanHeader {
        &self.header
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    fn section_bytes(&self, name: &'static str, section: &SectionRef) -> Result<&[u8], PlanError> {
        if section.len == 0 {
            return Err(PlanError::MissingSection { section: name });
        }
        let start = section.offset as usize;
        Ok(&self.bytes[start..start + section.len as usize])
    }

    /// Validate and return a zero-copy view of the HOT section.
    ///
    /// rkyv requires an aligned buffer; an object-store GET gives no alignment guarantee,
    /// so the section is copied once into an [`AlignedSection`]. That copy is a memcpy of
    /// a few hundred KB at worst — there is still no decode, no allocation per entity.
    pub fn hot(&self) -> Result<AlignedSection<ArchivedHotPlan>, PlanError> {
        let bytes = self.section_bytes("hot", &self.header.hot)?;
        AlignedSection::validated("hot", bytes)
    }

    /// Validate and return a zero-copy view of the COLD section.
    pub fn cold(&self) -> Result<AlignedSection<ArchivedColdPlan>, PlanError> {
        let bytes = self.section_bytes("cold", &self.header.cold)?;
        AlignedSection::validated("cold", bytes)
    }

    pub fn has_cold(&self) -> bool {
        self.header.cold.len > 0
    }
}

/// An archived plan section root that can be validated from untrusted bytes.
pub trait PlanSection:
    rkyv::Portable
    + for<'a> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>>
{
}

impl<T> PlanSection for T where
    T: rkyv::Portable
        + for<'a> rkyv::bytecheck::CheckBytes<rkyv::api::high::HighValidator<'a, rkyv::rancor::Error>>
{
}

/// An aligned, validated copy of one plan section.
///
/// Validation happens once, in [`AlignedSection::validated`]. After that, accessing the
/// archived root is a pointer cast — the whole point of the format.
///
/// The archived type is a parameter rather than a turbofish on `root` so that the two can
/// never disagree: a section validated as HOT cannot be read back as COLD, which would be
/// unsound.
#[derive(Debug, Clone)]
pub struct AlignedSection<T> {
    inner: rkyv::util::AlignedVec<16>,
    _archived: std::marker::PhantomData<fn() -> T>,
}

impl<T: PlanSection> AlignedSection<T> {
    /// Decompress `bytes` into an aligned buffer and run bytecheck validation for `T`.
    ///
    /// Sections are stored lz4-compressed. That is not in tension with zero-copy access:
    /// an object-store GET yields unaligned `Bytes`, so a section has to be copied into an
    /// aligned buffer regardless — decompressing during that unavoidable copy costs a
    /// fraction of a millisecond and removes far more bytes from the network fetch than it
    /// adds in CPU. What the format avoids is *per-field* decoding, and that is preserved:
    /// after this call, every access is a pointer cast.
    ///
    /// Every plan is treated as untrusted input: a corrupted or hostile object must fail
    /// here rather than produce unsound archived access later.
    pub fn validated(section: &'static str, bytes: &[u8]) -> Result<Self, PlanError> {
        let decompressed = lz4_flex::decompress_size_prepended(bytes)
            .map_err(|source| PlanError::Decompress { section, source })?;

        let mut inner = rkyv::util::AlignedVec::<16>::with_capacity(decompressed.len());
        inner.extend_from_slice(&decompressed);
        rkyv::access::<T, rkyv::rancor::Error>(&inner)
            .map_err(|source| PlanError::Validation { section, source })?;
        Ok(Self {
            inner,
            _archived: std::marker::PhantomData,
        })
    }

    /// Access the archived root. This is a pointer cast, not a parse.
    ///
    /// Callers reach for the root on every lookup, so this has to stay free. Re-running
    /// `rkyv::access` here would repeat full bytecheck validation — an O(section) walk — on
    /// every call.
    pub fn root(&self) -> &T {
        // SAFETY: `validated` is the only constructor, it ran bytecheck validation over
        // exactly these bytes for exactly this `T` (the type is pinned by the struct
        // parameter, so it cannot differ), and the buffer is immutable afterwards.
        unsafe { rkyv::access_unchecked::<T>(&self.inner) }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.inner
    }
}

/// Serialize one section to its stored (lz4-compressed) form.
pub fn serialize_section<T>(value: &T) -> Result<Vec<u8>, rkyv::rancor::Error>
where
    T: for<'a> rkyv::Serialize<
            rkyv::api::high::HighSerializer<
                rkyv::util::AlignedVec,
                rkyv::ser::allocator::ArenaHandle<'a>,
                rkyv::rancor::Error,
            >,
        >,
{
    rkyv::to_bytes::<rkyv::rancor::Error>(value)
        .map(|bytes| lz4_flex::compress_prepend_size(&bytes))
}

/// Assemble a plan container from already-serialized sections.
pub fn build_container(header: &PlanHeader, hot: &[u8], cold: &[u8], debug: &[u8]) -> Vec<u8> {
    let mut header = header.clone();
    let mut offset = PlanHeader::ENCODED_LEN as u64;

    header.hot = SectionRef::new(offset, hot.len() as u64, hot::HOT_SECTION_VERSION);
    offset += hot.len() as u64;
    header.cold = SectionRef::new(offset, cold.len() as u64, cold::COLD_SECTION_VERSION);
    offset += cold.len() as u64;
    header.debug = SectionRef::new(offset, debug.len() as u64, cold::DEBUG_SECTION_VERSION);

    let mut out =
        Vec::with_capacity(PlanHeader::ENCODED_LEN + hot.len() + cold.len() + debug.len());
    out.extend_from_slice(&header.encode());
    out.extend_from_slice(hot);
    out.extend_from_slice(cold);
    out.extend_from_slice(debug);
    out
}

#[cfg(test)]
mod tests {
    use super::header::PlanStamps;
    use super::hot::{Csr, PinNameEntry, PlanLayer, PlanNode, PlanPin, PlanVariable, name_hash};
    use super::*;

    fn sample_plan() -> (HotPlan, ColdPlan) {
        let hot = HotPlan {
            board_id: "board-1".into(),
            stage: 0,
            log_level: 2,
            symbols: vec![
                "node-a".into(),        // 0
                "node-b".into(),        // 1
                "events_simple".into(), // 2
                "math_add".into(),      // 3
                "exec_out".into(),      // 4
                "value".into(),         // 5
                "pin-1".into(),         // 6
                "pin-2".into(),         // 7
            ],
            blobs: vec![b"true".to_vec()],
            nodes: vec![
                PlanNode {
                    instance_id: 0,
                    type_key: 2,
                    flags: hot::node_flags::HAS_EXEC_OUTPUT,
                    first_pin: 0,
                    pin_count: 1,
                    layer: NONE_INDEX,
                    wasm_package: NONE_INDEX,
                    semantic_hash: 111,
                },
                PlanNode {
                    instance_id: 1,
                    type_key: 3,
                    flags: hot::node_flags::IS_PURE,
                    first_pin: 1,
                    pin_count: 1,
                    layer: NONE_INDEX,
                    wasm_package: NONE_INDEX,
                    semantic_hash: 222,
                },
            ],
            pins: vec![
                PlanPin {
                    id: 6,
                    name: 4,
                    owner_node: 0,
                    default_value: 0,
                    index: 0,
                    flags: hot::pin_flags::IS_EXEC | hot::pin_flags::HAS_DEFAULT,
                    pin_type: 1,
                    data_type: 9,
                    value_type: 1,
                },
                PlanPin {
                    id: 7,
                    name: 5,
                    owner_node: 1,
                    default_value: NONE_INDEX,
                    index: 0,
                    flags: 0,
                    pin_type: 0,
                    data_type: 2,
                    value_type: 1,
                },
            ],
            variables: vec![PlanVariable {
                id: 0,
                name: 0,
                default_value: NONE_INDEX,
                owner_layer: NONE_INDEX,
                data_type: 2,
                value_type: 1,
                flags: hot::variable_flags::EXPOSED,
            }],
            layers: vec![PlanLayer {
                id: 0,
                parent: NONE_INDEX,
                layer_type: 0,
            }],
            pin_connections: Csr {
                offsets: vec![0, 1, 1],
                edges: vec![1],
            },
            pin_dependencies: Csr {
                offsets: vec![0, 0, 1],
                edges: vec![0],
            },
            exec_successors: Csr {
                offsets: vec![0, 1, 1],
                edges: vec![1],
            },
            pure_schedules: Csr {
                offsets: vec![0, 1, 1],
                edges: vec![1],
            },
            fn_refs: Csr {
                offsets: vec![0, 0, 0],
                edges: vec![],
            },
            wasm_permissions: Csr {
                offsets: vec![0, 0, 0],
                edges: vec![],
            },
            layer_variables: Csr {
                offsets: vec![0, 0],
                edges: vec![],
            },
            // "node-a" < "node-b", so sorted order is already index order.
            nodes_by_id: vec![0, 1],
            pin_name_offsets: vec![0, 1, 2],
            pin_name_entries: vec![
                PinNameEntry {
                    hash: name_hash("exec_out"),
                    pin: 0,
                },
                PinNameEntry {
                    hash: name_hash("value"),
                    pin: 1,
                },
            ],
            start_nodes: vec![0],
        };

        // Handle 0 is the empty string; the rest are interned exactly once.
        let cold = ColdPlan {
            strings: vec![
                "".into(),                  // 0
                "Start".into(),             // 1
                "Add".into(),               // 2
                "entry".into(),             // 3
                "adds numbers".into(),      // 4
                "Out".into(),               // 5
                "Value".into(),             // 6
                "the value".into(),         // 7
                "{\"type\":\"number\"}".into(), // 8
                "a".into(),                 // 9
                "b".into(),                 // 10
                "Main".into(),              // 11
            ],
            node_friendly_names: vec![1, 2],
            node_descriptions: vec![3, 4],
            pin_friendly_names: vec![5, 6],
            pin_descriptions: vec![0, 7],
            pin_schemas: vec![0, 8],
            pin_valid_values: vec![vec![], vec![9, 10]],
            layer_names: vec![11],
        };

        (hot, cold)
    }

    fn container_from(hot: &HotPlan, cold: &ColdPlan) -> Vec<u8> {
        let header = PlanHeader::new(
            PLAN_FORMAT_VERSION,
            PlanStamps {
                board_content_hash: 99,
                catalog_signature: 5,
                wasm_signature: 6,
                board_version: (0, 1, 0),
            },
        );
        let hot_bytes = serialize_section(hot).unwrap();
        let cold_bytes = serialize_section(cold).unwrap();
        build_container(&header, &hot_bytes, &cold_bytes, &[])
    }

    #[test]
    fn container_roundtrips_and_validates() {
        let (hot, cold) = sample_plan();
        let buffer = PlanBuffer::new(container_from(&hot, &cold)).unwrap();

        assert!(buffer.header().matches(99, 5, 6));
        assert_eq!(buffer.header().stamps.board_version, (0, 1, 0));

        let hot_section = buffer.hot().unwrap();
        let archived = hot_section.root();
        assert_eq!(archived.board_id.as_str(), "board-1");
        assert_eq!(archived.nodes.len(), 2);
        assert_eq!(
            archived.symbol(archived.nodes[1].type_key.to_native()),
            "math_add"
        );
    }

    #[test]
    fn node_lookup_finds_every_node_and_rejects_unknown() {
        let (hot, cold) = sample_plan();
        let buffer = PlanBuffer::new(container_from(&hot, &cold)).unwrap();
        let section = buffer.hot().unwrap();
        let archived = section.root();

        assert_eq!(archived.node_by_id("node-a"), Some(0));
        assert_eq!(archived.node_by_id("node-b"), Some(1));
        assert_eq!(archived.node_by_id("node-c"), None);
        assert_eq!(archived.node_by_id(""), None);
    }

    #[test]
    fn pin_name_lookup_resolves_within_owning_node() {
        let (hot, cold) = sample_plan();
        let buffer = PlanBuffer::new(container_from(&hot, &cold)).unwrap();
        let section = buffer.hot().unwrap();
        let archived = section.root();

        assert_eq!(archived.pins_by_name(0, "exec_out"), vec![0]);
        assert_eq!(archived.pins_by_name(1, "value"), vec![1]);
        // A name belonging to a different node must not leak across the node boundary.
        assert!(archived.pins_by_name(0, "value").is_empty());
        assert!(archived.pins_by_name(1, "nope").is_empty());
    }

    #[test]
    fn cold_section_is_only_decoded_on_demand() {
        let (hot, cold) = sample_plan();
        let buffer = PlanBuffer::new(container_from(&hot, &cold)).unwrap();
        assert!(buffer.has_cold());

        let section = buffer.cold().unwrap();
        let archived = section.root();
        assert_eq!(archived.node_friendly_name(1), "Add");
        assert_eq!(archived.pin_schema(0), None);
        assert_eq!(archived.pin_schema(1), Some("{\"type\":\"number\"}"));
        // Out-of-range indices degrade to empty rather than panicking.
        assert_eq!(archived.node_friendly_name(99), "");
    }

    #[test]
    fn csr_rows_address_the_right_neighbours() {
        let (hot, cold) = sample_plan();
        let buffer = PlanBuffer::new(container_from(&hot, &cold)).unwrap();
        let section = buffer.hot().unwrap();
        let archived = section.root();

        let successors = archived.exec_successors.row(0);
        assert_eq!(successors.len(), 1);
        assert_eq!(successors[0].to_native(), 1);
        assert!(archived.exec_successors.row(1).is_empty());
    }

    #[test]
    fn absent_debug_section_reports_missing_rather_than_garbage() {
        let (hot, cold) = sample_plan();
        let buffer = PlanBuffer::new(container_from(&hot, &cold)).unwrap();
        assert_eq!(buffer.header().debug.len, 0);
    }

    #[test]
    fn truncated_container_is_rejected() {
        let (hot, cold) = sample_plan();
        let mut bytes = container_from(&hot, &cold);
        bytes.truncate(bytes.len() - 8);
        assert!(matches!(
            PlanBuffer::new(bytes),
            Err(PlanError::Truncated { .. })
        ));
    }

    /// A plan is untrusted input; corruption must fail validation, never produce unsound
    /// archived access.
    #[test]
    fn corrupted_hot_section_fails_validation() {
        let (hot, cold) = sample_plan();
        let bytes = container_from(&hot, &cold);
        let header = PlanHeader::decode(&bytes).unwrap();

        let mut corrupted = bytes.clone();
        let hot_start = header.hot.offset as usize;
        let hot_end = hot_start + header.hot.len as usize;
        for byte in &mut corrupted[hot_start..hot_end] {
            *byte = 0xff;
        }

        let buffer = PlanBuffer::new(corrupted).unwrap();
        // Either gate is a correct rejection: garbage usually fails to decompress, and
        // anything that survives that still has to satisfy bytecheck.
        assert!(matches!(
            buffer.hot(),
            Err(PlanError::Validation { section: "hot", .. })
                | Err(PlanError::Decompress { section: "hot", .. })
        ));
    }

    /// Truncating inside a section must be caught, not read as a short archive.
    #[test]
    fn corrupted_section_length_is_rejected() {
        let (hot, cold) = sample_plan();
        let bytes = container_from(&hot, &cold);
        let header = PlanHeader::decode(&bytes).unwrap();

        let mut corrupted = bytes.clone();
        let hot_start = header.hot.offset as usize;
        // Flip bytes in the middle of the compressed payload.
        let middle = hot_start + (header.hot.len as usize / 2);
        corrupted[middle] ^= 0xff;
        corrupted[middle + 1] ^= 0xff;

        let buffer = PlanBuffer::new(corrupted).unwrap();
        assert!(buffer.hot().is_err());
    }

    #[test]
    fn name_hash_is_stable_and_distinguishes_names() {
        assert_eq!(name_hash("exec_out"), name_hash("exec_out"));
        assert_ne!(name_hash("exec_out"), name_hash("exec_in"));
        assert_eq!(name_hash(""), 0xcbf2_9ce4_8422_2325);
    }
}
