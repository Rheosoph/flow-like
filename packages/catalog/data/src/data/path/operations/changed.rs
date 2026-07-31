use crate::data::path::FlowPath;
use flow_like_storage::object_store::path::Path;
use flow_like_types::{
    JsonSchema,
    json::{Deserialize, Serialize},
};

pub mod apply_changed;
pub mod get_changes;

/// On-disk manifest format version. Bumped for incompatible [`DirManifest`] changes.
pub const MANIFEST_VERSION: u32 = 1;
/// Stable prefix written at the start of every compactly serialized manifest.
///
/// The diff node uses a short ranged read to recognize manifests independently
/// of their file name.
pub const MANIFEST_PREFIX: &[u8] = br#"{"$flow_like_manifest":"flow-like.directory-manifest""#;

/// Stable content discriminator used to identify directory manifests by bytes,
/// independently of their object-store key.
#[derive(Clone, Debug, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum ManifestKind {
    #[default]
    #[serde(rename = "flow-like.directory-manifest")]
    Directory,
}

/// A single tracked file inside a manifest.
///
/// Change detection prefers a `hash` when present, else the store `e_tag`.
/// Trusted ETags avoid a full-content hash read; `hash` is populated in Checksum
/// mode or when the store reports a missing/weak ETag.
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
    /// Content marker used to exclude manifests from directory diffs regardless of name.
    #[serde(default, rename = "$flow_like_manifest")]
    pub kind: ManifestKind,
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
/// Carries the manifest destination, scan scope, ignored manifest keys, and the
/// fully-computed current snapshot. The writer can persist it wholesale or merge
/// selected paths into the latest stored baseline.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct DirDiffSession {
    /// Where the manifest lives / should be written.
    pub manifest: FlowPath,
    /// Root that was scanned. Required when committing selected paths.
    #[serde(default)]
    pub root: Option<FlowPath>,
    /// Manifest files omitted from the snapshot and safe to purge from older baselines.
    #[serde(default)]
    pub ignored_manifest_paths: Vec<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_manifest_serialization_starts_with_marker() {
        let bytes = flow_like_types::json::to_vec(&DirManifest::default()).unwrap();

        assert!(bytes.starts_with(MANIFEST_PREFIX));
    }

    #[test]
    fn legacy_unmarked_manifest_remains_readable() {
        let manifest: DirManifest =
            flow_like_types::json::from_slice(br#"{"version":1,"recursive":true,"entries":[]}"#)
                .unwrap();

        assert_eq!(manifest.kind, ManifestKind::Directory);
        assert_eq!(manifest.version, 1);
        assert!(manifest.recursive);
    }

    #[test]
    fn an_unknown_manifest_marker_is_rejected() {
        let result = flow_like_types::json::from_slice::<DirManifest>(
            br#"{"$flow_like_manifest":"other","version":1,"entries":[]}"#,
        );

        assert!(result.is_err());
    }

    #[test]
    fn legacy_diff_session_without_root_remains_readable() {
        let session: DirDiffSession = flow_like_types::json::from_slice(
            br#"{
                "manifest":{"path":"state.json","store_ref":"store","cache_store_ref":null},
                "manifest_content":{"version":1,"recursive":true,"entries":[]}
            }"#,
        )
        .unwrap();

        assert!(session.root.is_none());
        assert!(session.ignored_manifest_paths.is_empty());
        assert_eq!(session.manifest_content.kind, ManifestKind::Directory);
    }
}
