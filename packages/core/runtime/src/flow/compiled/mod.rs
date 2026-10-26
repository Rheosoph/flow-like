//! Compiled board artifacts: an execution-scoped, rkyv-serialized board
//! representation that skips proto decode + `node_updates` at run time.
//!
//! Pipeline:
//! - compile: post-`node_updates` [`crate::flow::board::Board`] → [`CompiledBoard`]
//! - artifact: [`CompiledBoard`] ⇄ bytes (envelope + compressed rkyv), see below
//! - template: [`CompiledBoard`] + node registry → shared, immutable
//!   run template the executor caches per (board version, registry fingerprint)
//!
//! Artifact envelope (all little-endian):
//! ```text
//! [0..4)   magic "FLCB"
//! [4..6)   format version u16
//! [6]      codec (1 = zstd; 0 = lz4, legacy)
//! [7]      reserved (0)
//! [8..40)  registry fingerprint (blake3), the registry the compile ran against
//! [40..44) uncompressed rkyv length u32
//! [44..]   compressed rkyv archive
//! ```
//! zstd level 3 was chosen empirically on a 1097-node board: 4.9x ratio vs
//! lz4's 2.8x, ~2 ms to compress and <1 ms to decompress the whole payload —
//! both invisible next to one storage round trip. The header is readable
//! without decompressing, so a loader can reject a stale artifact from the
//! first bytes alone.
//! API artifacts may append a Zstd skippable frame containing widget definitions.
//! The board archive and format version stay unchanged for older readers.

pub mod codes;
pub mod compile;
pub mod format;
pub mod prerun;
pub mod resolver;
pub mod template;
pub mod view;
pub mod widget_snapshot;

pub use compile::{compile_board, compile_board_with_catalog};
pub use format::{CompiledBoard, FORMAT_VERSION, MAGIC};
pub use prerun::{
    MANIFEST_FORMAT_VERSION, PrerunManifest, PrerunOAuthRequirement, PrerunVariable,
    decode_manifest, draft_manifest_path, draft_page_manifest_path, encode_manifest,
    legacy_manifest_path, manifest_path, version_page_manifest_path,
};
pub use resolver::{TemplateCache, persist_artifact, template_from_bytes};
pub use template::CompiledRunTemplate;
pub use widget_snapshot::{
    ArtifactWidgetEntry, ArtifactWidgetSnapshot, append_widget_snapshot, decode_widget_snapshot,
    static_widget_selectors,
};

use flow_like_storage::Path;
use flow_like_types::{Result, anyhow};

const HEADER_LEN: usize = 40;
/// Compressed payload is prefixed with its uncompressed u32 length.
const PAYLOAD_LEN_PREFIX: usize = 4;
const CODEC_LZ4: u8 = 0;
const CODEC_ZSTD: u8 = 1;
const ZSTD_LEVEL: i32 = 3;
/// Upper bound for the decompressed payload — rejects absurd length prefixes
/// from corrupt artifacts before allocating.
const MAX_DECOMPRESSED_LEN: usize = 512 * 1024 * 1024;

/// Directory holding a board's version artifacts (compiled board + prerun manifest).
fn version_artifact_dir(board_dir: &Path, board_id: &str) -> Path {
    board_dir
        .clone()
        .join("compiled")
        .join(board_id.to_string())
}

/// Compiled artifact of an immutable board version. Lives beside the version
/// snapshots inside the app's meta prefix, so deleting the app removes it.
pub fn artifact_path(board_dir: &Path, board_id: &str, version: (u32, u32, u32)) -> Path {
    version_artifact_dir(board_dir, board_id)
        .join(format!("{}_{}_{}.flcb", version.0, version.1, version.2))
}

/// Directory holding a board's content-addressed draft artifacts.
///
/// Contract: `tmp/` so storage lifecycle rules on that prefix reclaim stale
/// drafts; `{app_id}` inside it so per-app executor credentials can read
/// exact drafts read-only. Only the API writes here — that write asymmetry
/// is the trust anchor for `entry_authority_revision`. The desktop startup
/// sweep removes old entries from local stores, which have no lifecycle rules.
pub fn draft_artifact_dir(app_id: &str, board_id: &str) -> Path {
    Path::from("tmp")
        .join("apps")
        .join(app_id)
        .join("compiled")
        .join("drafts")
        .join(board_id)
}

/// Compiled artifact of a floating draft, keyed by the source `.board`'s ETag
/// and the registry that resolved its node implementations.
/// Recreatable at any time and stored below `tmp/apps/{app_id}/` so app-scoped
/// executor credentials can read, but never write, exact drafts.
pub fn draft_artifact_path(
    app_id: &str,
    board_id: &str,
    e_tag: &str,
    registry_fingerprint: &[u8; 32],
) -> Path {
    let fingerprint = blake3::Hash::from_bytes(*registry_fingerprint).to_hex();
    draft_artifact_dir(app_id, board_id).join(format!(
        "{}_{}.flcb",
        draft_artifact_stem(e_tag),
        &fingerprint.as_str()[..16]
    ))
}

/// Execution source snapshot for a floating draft, keyed by the ETag of the
/// `.board` object that produced it. This lets an executor rebuild an exact
/// artifact after the floating source advances.
pub fn draft_source_path(app_id: &str, board_id: &str, e_tag: &str) -> Path {
    draft_artifact_dir(app_id, board_id).join(format!("{}.board", draft_artifact_stem(e_tag)))
}

/// File stem shared by every draft artifact of one `.board` etag.
fn draft_artifact_stem(e_tag: &str) -> String {
    // Version the namespace when the trust contract of a draft artifact
    // changes. Version 2 guarantees writers verified the loaded Board still
    // matched this ETag before placing bytes at the content-addressed path.
    // Version 3 rebuilds prerun manifests with complete OAuth scopes.
    //
    // Unlike version manifests, the draft manifest path carries no
    // `.v{MANIFEST_FORMAT_VERSION}` segment — this byte is its only format
    // namespace. Bumping `MANIFEST_FORMAT_VERSION` (prerun.rs) therefore
    // requires bumping this byte too, or a rolling deployment's old and new
    // writers would race on one draft manifest path.
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"flow-like/draft-artifact");
    hasher.update(&[3]);
    hasher.update(e_tag.as_bytes());
    hasher.finalize().to_hex().as_str()[..32].to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactHeader {
    pub format_version: u16,
    pub codec: u8,
    pub registry_fingerprint: [u8; 32],
}

/// Serialize a compiled board into a persistable artifact.
pub fn encode_artifact(board: &CompiledBoard, registry_fingerprint: &[u8; 32]) -> Result<Vec<u8>> {
    crate::flow::board::format::ensure_supported(board.required_board_format_version())?;
    let archive = rkyv::to_bytes::<rkyv::rancor::Error>(board)
        .map_err(|e| anyhow!("failed to serialize compiled board {}: {e}", board.id))?;
    let compressed = zstd::bulk::compress(&archive, ZSTD_LEVEL)
        .map_err(|e| anyhow!("failed to compress compiled board {}: {e}", board.id))?;

    let mut out = Vec::with_capacity(HEADER_LEN + PAYLOAD_LEN_PREFIX + compressed.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT_VERSION.to_le_bytes());
    out.push(CODEC_ZSTD);
    out.push(0);
    out.extend_from_slice(registry_fingerprint);
    out.extend_from_slice(&(archive.len() as u32).to_le_bytes());
    out.extend_from_slice(&compressed);
    Ok(out)
}

/// Read and validate the envelope header without touching the payload.
/// Fails on wrong magic or a format version this build does not understand.
pub fn peek_header(bytes: &[u8]) -> Result<ArtifactHeader> {
    if bytes.len() < HEADER_LEN {
        return Err(anyhow!(
            "compiled board artifact too short: {} bytes",
            bytes.len()
        ));
    }
    if bytes[0..4] != MAGIC {
        return Err(anyhow!("compiled board artifact has wrong magic"));
    }
    let format_version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if format_version != FORMAT_VERSION {
        return Err(anyhow!(
            "compiled board artifact format v{format_version}, this build reads v{FORMAT_VERSION}"
        ));
    }
    let codec = bytes[6];
    if codec != CODEC_ZSTD && codec != CODEC_LZ4 {
        return Err(anyhow!(
            "compiled board artifact uses unknown codec {codec}"
        ));
    }
    let mut registry_fingerprint = [0u8; 32];
    registry_fingerprint.copy_from_slice(&bytes[8..40]);
    Ok(ArtifactHeader {
        format_version,
        codec,
        registry_fingerprint,
    })
}

/// Decode an artifact back into an owned [`CompiledBoard`].
///
/// When `expected_fingerprint` is given, an artifact compiled against a
/// different registry is rejected — its minted dynamic pins and synced schemas
/// may not match the running catalog, so the caller must recompile instead.
pub fn decode_artifact(
    bytes: &[u8],
    expected_fingerprint: Option<&[u8; 32]>,
) -> Result<CompiledBoard> {
    let header = peek_header(bytes)?;
    if let Some(expected) = expected_fingerprint
        && &header.registry_fingerprint != expected
    {
        return Err(anyhow!(
            "compiled board artifact was built against a different node registry"
        ));
    }

    let board = decode_archive(bytes, &header)?;
    board.validate()?;
    Ok(board)
}

/// Decode the binary archive after its header has been checked. Callers still
/// validate arena indices before constructing runtime state.
fn decode_archive(bytes: &[u8], header: &ArtifactHeader) -> Result<CompiledBoard> {
    let aligned = match header.codec {
        CODEC_ZSTD => {
            let payload = &bytes[HEADER_LEN..];
            if payload.len() < PAYLOAD_LEN_PREFIX {
                return Err(anyhow!("compiled board artifact payload truncated"));
            }
            let len = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
            if len > MAX_DECOMPRESSED_LEN {
                return Err(anyhow!(
                    "compiled board artifact claims an implausible {len}-byte payload"
                ));
            }

            // Decode Zstd directly into the aligned allocation rkyv needs,
            // avoiding a second archive-sized buffer and copy.
            let mut aligned = rkyv::util::AlignedVec::<16>::with_capacity(len);
            aligned.resize(len, 0);
            let written = zstd::bulk::decompress_to_buffer(
                &payload[PAYLOAD_LEN_PREFIX..],
                aligned.as_mut_slice(),
            )
            .map_err(|e| anyhow!("failed to decompress compiled board artifact: {e}"))?;
            aligned.resize(written, 0);
            aligned
        }
        _ => {
            // Preserve the legacy decoder's length handling, including its
            // acceptance of payloads larger than the Zstd limit.
            let decompressed = lz4_flex::decompress_size_prepended(&bytes[HEADER_LEN..])
                .map_err(|e| anyhow!("failed to decompress compiled board artifact: {e}"))?;
            let mut aligned = rkyv::util::AlignedVec::<16>::new();
            aligned.extend_from_slice(&decompressed);
            aligned
        }
    };

    let archived = rkyv::access::<rkyv::Archived<CompiledBoard>, rkyv::rancor::Error>(&aligned)
        .map_err(|e| anyhow!("compiled board artifact failed validation: {e}"))?;
    rkyv::deserialize::<CompiledBoard, rkyv::rancor::Error>(archived)
        .map_err(|e| anyhow!("failed to deserialize compiled board artifact: {e}"))
}

#[cfg(test)]
mod artifact_loading_tests {
    use super::*;
    use crate::flow::board::Board;
    use crate::flow::node::Node;
    use crate::flow::pin::ValueType;
    use crate::flow::variable::{Variable, VariableType};
    use crate::state::FlowNodeRegistryInner;

    fn artifact(board: &CompiledBoard, codec: u8) -> Vec<u8> {
        let current = encode_artifact(board, &[7; 32]).unwrap();
        if codec == CODEC_ZSTD {
            return current;
        }
        let archive = rkyv::to_bytes::<rkyv::rancor::Error>(board).unwrap();
        let mut legacy = current[..HEADER_LEN].to_vec();
        legacy[6] = CODEC_LZ4;
        legacy.extend_from_slice(&lz4_flex::compress_prepend_size(&archive));
        legacy
    }

    #[test]
    fn both_codecs_keep_the_decoded_length_and_variable_defaults() {
        let mut board = Board::new_detached(Some("artifact-load".into()), Path::default());
        let mut variable = Variable::new("default", VariableType::String, ValueType::Normal);
        variable.set_default_value(serde_json::json!("large default".repeat(16_384)));
        let variable_id = variable.id.clone();
        let default = variable.default_value.clone();
        board.variables.insert(variable_id.clone(), variable);
        let compiled = compile_board(&board).unwrap();
        let registry = FlowNodeRegistryInner::new(0);

        for codec in [CODEC_ZSTD, CODEC_LZ4] {
            let bytes = artifact(&compiled, codec);
            let len = u32::from_le_bytes(bytes[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap());
            for capacity in [len, len + 17] {
                let mut bytes = bytes.clone();
                bytes[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&capacity.to_le_bytes());
                assert_eq!(decode_artifact(&bytes, Some(&[7; 32])).unwrap(), compiled);
                let template =
                    template_from_bytes(&bytes, &[7; 32], &registry, &Path::default()).unwrap();
                assert_eq!(
                    template.board.variables[&variable_id].default_value,
                    default
                );
                assert_eq!(template.variables[0].variable.default_value, default);
                assert_eq!(
                    template.variables[0].parsed_default.as_deref(),
                    Some(&serde_json::json!("large default".repeat(16_384)))
                );
            }
        }
    }

    #[test]
    fn both_codecs_reject_invalid_payload_lengths() {
        let board = Board::new_detached(None, Path::default());
        let compiled = compile_board(&board).unwrap();
        for codec in [CODEC_ZSTD, CODEC_LZ4] {
            let bytes = artifact(&compiled, codec);
            let len = u32::from_le_bytes(bytes[HEADER_LEN..HEADER_LEN + 4].try_into().unwrap());
            for capacity in [0, len - 1] {
                let mut malformed = bytes.clone();
                malformed[HEADER_LEN..HEADER_LEN + 4].copy_from_slice(&capacity.to_le_bytes());
                assert!(decode_artifact(&malformed, None).is_err(), "codec {codec}");
            }
            for end in [HEADER_LEN, HEADER_LEN + 3, bytes.len() - 1] {
                assert!(
                    decode_artifact(&bytes[..end], None).is_err(),
                    "codec {codec}"
                );
            }
        }
    }

    #[test]
    fn zstd_rejects_payload_lengths_above_its_limit() {
        let board = Board::new_detached(None, Path::default());
        let compiled = compile_board(&board).unwrap();
        let mut bytes = artifact(&compiled, CODEC_ZSTD);
        bytes[HEADER_LEN..HEADER_LEN + 4]
            .copy_from_slice(&(MAX_DECOMPRESSED_LEN as u32 + 1).to_le_bytes());
        assert!(
            decode_artifact(&bytes, None)
                .unwrap_err()
                .to_string()
                .contains("implausible")
        );
    }

    #[test]
    fn direct_template_loading_still_rejects_invalid_arena_indices() {
        let mut board = Board::new_detached(None, Path::default());
        let mut node = Node::new("test-node", "Test", "", "Tests");
        node.add_input_pin("value", "Value", "", VariableType::String);
        board.nodes.insert(node.id.clone(), node);
        let compiled = compile_board(&board).unwrap();
        let registry = FlowNodeRegistryInner::new(0);
        for corruption in 0..7 {
            let mut invalid = compiled.clone();
            match corruption {
                0 => invalid.nodes[0].body_layer = 42,
                1 => invalid.nodes[0].layer = 42,
                2 => invalid.nodes[0].pins.push(42),
                3 => invalid.pins[0].owner_node = 42,
                4 => invalid.pins[0].owner_layer = 42,
                5 => invalid.pins[0].depends_on.push(42),
                _ => invalid.pins[0].connected_to.push(42),
            }
            for codec in [CODEC_ZSTD, CODEC_LZ4] {
                let bytes = artifact(&invalid, codec);
                let decoded_error = decode_artifact(&bytes, None).unwrap_err();
                let template_error =
                    template_from_bytes(&bytes, &[7; 32], &registry, &Path::default())
                        .err()
                        .expect("arena validation must run before catalog instantiation");
                assert!(decoded_error.to_string().contains("out-of-range index"));
                assert!(template_error.to_string().contains("out-of-range index"));
            }
        }
    }

    #[test]
    fn direct_template_loading_still_validates_geometry_defaults() {
        use flow_like_types::geometry::{GeometryKind, marker};

        let mut board = Board::new_detached(None, Path::default());
        board
            .refs
            .insert("point".into(), marker(GeometryKind::Point).into());
        let mut variable = Variable::new("point", VariableType::Geometry, ValueType::Normal);
        variable.schema = Some("point".into());
        variable.set_default_value(serde_json::json!({
            "type": "Point", "coordinates": [13.4, 52.5]
        }));
        board.variables.insert(variable.id.clone(), variable);
        let mut compiled = compile_board(&board).unwrap();
        let registry = FlowNodeRegistryInner::new(0);
        let bytes = artifact(&compiled, CODEC_ZSTD);
        let template = template_from_bytes(&bytes, &[7; 32], &registry, &Path::default()).unwrap();
        assert_eq!(
            template.variables[0].variable.schema.as_deref(),
            Some(marker(GeometryKind::Point))
        );

        compiled.variables[0].default_value =
            Some(br#"{"type":"Point","coordinates":["invalid",52.5]}"#.to_vec());
        for codec in [CODEC_ZSTD, CODEC_LZ4] {
            let bytes = artifact(&compiled, codec);
            assert!(
                decode_artifact(&bytes, None).is_ok(),
                "the archive itself is valid"
            );
            assert!(template_from_bytes(&bytes, &[7; 32], &registry, &Path::default()).is_err());
        }
    }
}
