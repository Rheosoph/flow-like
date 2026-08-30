//! Compiled board artifacts: an execution-scoped, rkyv-serialized board
//! representation that skips proto decode + `node_updates` at run time.
//!
//! Pipeline:
//! - compile: post-`node_updates` [`crate::flow::board::Board`] → [`CompiledBoard`]
//! - artifact: [`CompiledBoard`] ⇄ bytes (envelope + lz4 + rkyv), see below
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

pub mod codes;
pub mod compile;
pub mod format;
pub mod prerun;
pub mod resolver;
pub mod template;
pub mod view;

pub use compile::{compile_board, compile_board_with_catalog};
pub use format::{CompiledBoard, FORMAT_VERSION, MAGIC};
pub use prerun::{
    MANIFEST_FORMAT_VERSION, PrerunManifest, PrerunOAuthRequirement, PrerunVariable,
    decode_manifest, draft_manifest_path, encode_manifest, manifest_path,
};
pub use resolver::{TemplateCache, persist_artifact};
pub use template::CompiledRunTemplate;

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
    board_dir.child("compiled").child(board_id.to_string())
}

/// Compiled artifact of an immutable board version. Lives beside the version
/// snapshots inside the app's meta prefix, so deleting the app removes it.
pub fn artifact_path(board_dir: &Path, board_id: &str, version: (u32, u32, u32)) -> Path {
    version_artifact_dir(board_dir, board_id)
        .child(format!("{}_{}_{}.flcb", version.0, version.1, version.2))
}

/// Directory holding a board's draft artifacts; writers purge it down to the
/// newest entry (local stores have no lifecycle rules).
pub fn draft_artifact_dir(app_id: &str, board_id: &str) -> Path {
    Path::from("tmp")
        .child("compiled")
        .child(app_id)
        .child(board_id)
}

/// Compiled artifact of a floating draft, keyed by the source `.board`'s etag.
/// Recreatable at any time — parked under the meta store's `tmp/` prefix so
/// bucket lifecycle rules may purge stale ones.
pub fn draft_artifact_path(app_id: &str, board_id: &str, e_tag: &str) -> Path {
    draft_artifact_dir(app_id, board_id).child(format!("{}.flcb", draft_artifact_stem(e_tag)))
}

/// File stem shared by every draft artifact of one `.board` etag.
fn draft_artifact_stem(e_tag: &str) -> String {
    blake3::hash(e_tag.as_bytes()).to_hex().as_str()[..32].to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactHeader {
    pub format_version: u16,
    pub codec: u8,
    pub registry_fingerprint: [u8; 32],
}

/// Serialize a compiled board into a persistable artifact.
pub fn encode_artifact(board: &CompiledBoard, registry_fingerprint: &[u8; 32]) -> Result<Vec<u8>> {
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

    let decompressed = match header.codec {
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
            zstd::bulk::decompress(&payload[PAYLOAD_LEN_PREFIX..], len)
                .map_err(|e| anyhow!("failed to decompress compiled board artifact: {e}"))?
        }
        _ => lz4_flex::decompress_size_prepended(&bytes[HEADER_LEN..])
            .map_err(|e| anyhow!("failed to decompress compiled board artifact: {e}"))?,
    };
    let mut aligned = rkyv::util::AlignedVec::<16>::new();
    aligned.extend_from_slice(&decompressed);

    let archived = rkyv::access::<rkyv::Archived<CompiledBoard>, rkyv::rancor::Error>(&aligned)
        .map_err(|e| anyhow!("compiled board artifact failed validation: {e}"))?;
    let board = rkyv::deserialize::<CompiledBoard, rkyv::rancor::Error>(archived)
        .map_err(|e| anyhow!("failed to deserialize compiled board artifact: {e}"))?;
    board.validate()?;
    Ok(board)
}
