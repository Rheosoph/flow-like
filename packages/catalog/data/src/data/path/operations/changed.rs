use crate::data::path::FlowPath;
use flow_like_storage::object_store::path::Path;
use flow_like_types::{
    JsonSchema,
    json::{Deserialize, Serialize},
};

pub mod apply_changed;
pub mod get_changes;

/// On-disk manifest format version. Bumped when [`DirManifest`] changes shape.
pub const MANIFEST_VERSION: u32 = 1;

/// A single tracked file inside a manifest.
///
/// Change detection prefers a `hash` when present, else the store `e_tag` (free
/// from a single `list()` call, so file contents are never re-read). `hash` is
/// populated in Checksum mode or when the store reports a missing/weak ETag.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct ManifestEntry {
    /// Object-store key of the file (path within the root's store).
    pub path: String,
    /// ETag reported by the store; used when no content hash is recorded.
    pub e_tag: Option<String>,
    /// Blake3 content hash; the strongest signal when present.
    pub hash: Option<String>,
    /// File size in bytes.
    pub size: u64,
    /// Last-modified timestamp as reported by the store.
    pub last_modified: String,
}

/// Snapshot of every file found under a root folder at a point in time.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema)]
pub struct DirManifest {
    #[serde(default)]
    pub version: u32,
    /// Whether the snapshot was taken recursively. Diffs against a manifest with a
    /// different scope skip deletion detection to avoid falsely reporting
    /// out-of-scope files as deleted.
    #[serde(default)]
    pub recursive: bool,
    #[serde(default)]
    pub entries: Vec<ManifestEntry>,
}

/// Hand-off produced by the diff node and consumed by the writer node.
///
/// Carries both the manifest destination and the fully-computed next manifest,
/// so the writer only needs this session to persist the new state.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DirDiffSession {
    /// Where the manifest lives / should be written.
    pub manifest: FlowPath,
    /// The next manifest to persist, reflecting the current directory state.
    pub manifest_content: DirManifest,
}

#[derive(Clone, Debug)]
pub enum PathChange {
    Edited {
        path: Path,
        old_hash: String,
        new_hash: String,
    },
    Removed {
        path: Path,
        hash: String,
    },
    Added {
        path: Path,
        hash: String,
    },
    Renamed {
        old_path: Path,
        new_path: Path,
        hash: String,
    },
    Moved {
        old_path: Path,
        new_path: Path,
        hash: String,
    },
}
