use super::{
    DirDiffSession, DirManifest, MANIFEST_PREFIX, MANIFEST_VERSION, ManifestEntry, ManifestKind,
};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_storage::object_store::{ObjectMeta, ObjectStore};
use flow_like_types::{Error, async_trait, json::json};
use futures::{StreamExt, stream};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

/// Bounded parallelism for marker probes and Blake3 content hashing.
const HASH_CONCURRENCY: usize = 16;

#[crate::register_node]
#[derive(Default)]
pub struct GetChangesNode {}

impl GetChangesNode {
    pub fn new() -> Self {
        GetChangesNode {}
    }
}

#[async_trait]
impl NodeLogic for GetChangesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_get_changes",
            "Diff Directory",
            "Diffs a folder against a manifest, emitting added, updated and deleted files while ignoring directory manifests. Auto mode trusts store ETags (hashing only weak/missing ones); Checksum mode always Blake3-hashes contents",
            "Data/Files/Operations",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "manifest",
            "Manifest",
            "FlowPath to this workflow's manifest file. It may have any name and need not exist yet; use a distinct name when workflows share a root",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "root",
            "Root",
            "Root folder to scan for changes",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "recursive",
            "Recursive",
            "Scan the root folder recursively",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "mode",
            "Change Detection",
            "Auto: trust store ETags, hashing only files with a missing/weak ETag (fast). Checksum: always Blake3-hash contents, ignoring ETags (correct on backends with mtime-based ETags such as local disk)",
            VariableType::String,
        )
        .set_default_value(Some(json!("Auto")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Auto".to_string(), "Checksum".to_string()])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "added",
            "Added",
            "Files present in the folder but not in the manifest",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "updated",
            "Updated",
            "Files whose ETag/hash changed since the manifest",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "deleted",
            "Deleted",
            "Files in the manifest that no longer exist in the folder",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "session",
            "Session",
            "Diff session carrying the next manifest, feed into 'Write Directory Manifest'",
            VariableType::Struct,
        )
        .set_schema::<DirDiffSession>();

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let manifest_path: FlowPath = context.evaluate_pin("manifest").await?;
        let root: FlowPath = context.evaluate_pin("root").await?;
        let recursive: bool = context.evaluate_pin("recursive").await?;
        let mode: String = context.evaluate_pin("mode").await?;
        let checksum = mode.eq_ignore_ascii_case("checksum");

        let previous_manifest = load_manifest(&manifest_path, context).await?;
        // Only diff against a manifest taken with the same scan scope; otherwise
        // out-of-scope files would be reported as deleted (a data-loss footgun if
        // the `deleted` pin drives a delete node).
        let scope_matches =
            previous_manifest.entries.is_empty() || previous_manifest.recursive == recursive;
        let mut previous: HashMap<String, ManifestEntry> = if scope_matches {
            previous_manifest
                .entries
                .into_iter()
                .map(|entry| (entry.path.clone(), entry))
                .collect()
        } else {
            context.log_message(
                "Manifest scan scope (recursive) changed; treating all files as new to avoid false deletions",
                LogLevel::Warn,
            );
            HashMap::new()
        };

        let runtime = root.to_runtime(context).await?;
        let store = runtime.store.clone();
        let generic = store.as_generic();

        let objects = if recursive {
            generic
                .list(Some(&runtime.path))
                .map(|result| result.map_err(Error::from))
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()?
        } else {
            generic
                .list_with_delimiter(Some(&runtime.path))
                .await?
                .objects
        };

        // An identical manifest path in a different store is a real file. Compare
        // normalized object-store paths so stray slashes never create phantom churn.
        let same_store = root.store_ref == manifest_path.store_ref;
        let manifest_key = flow_like_storage::Path::from(manifest_path.path.as_str());
        let mut ignored_manifest_keys = HashSet::new();
        if same_store {
            ignored_manifest_keys.insert(manifest_key.as_ref().to_string());
        }

        let (detected_manifest_keys, probe_failures) =
            detect_manifest_keys(generic.clone(), &objects, &ignored_manifest_keys).await;
        ignored_manifest_keys.extend(detected_manifest_keys);
        if probe_failures > 0 {
            context.log_message(
                &format!(
                    "Failed to inspect {} files for the directory-manifest marker; treating them as regular files",
                    probe_failures
                ),
                LogLevel::Warn,
            );
        }

        // A manifest from an older snapshot must be forgotten silently rather than
        // emitted as deleted now that manifests are excluded from directory state.
        previous.retain(|key, _| !ignored_manifest_keys.contains(key));

        let candidates: Vec<_> = objects
            .into_iter()
            .filter(|object| !ignored_manifest_keys.contains(object.location.as_ref()))
            .collect();

        // Only read file bodies when we cannot trust the ETag: Checksum mode, or
        // a missing/weak ETag. Trusted ETags come for free from the single list().
        let to_hash: Vec<(String, flow_like_storage::Path)> = candidates
            .iter()
            .filter(|object| checksum || !etag_trustworthy(&object.e_tag))
            .map(|object| {
                (
                    object.location.as_ref().to_string(),
                    object.location.clone(),
                )
            })
            .collect();

        let mut hashes: HashMap<String, String> = HashMap::with_capacity(to_hash.len());
        let mut hash_failed: HashSet<String> = HashSet::new();
        if !to_hash.is_empty() {
            let results: Vec<(String, flow_like_types::Result<String>)> = stream::iter(to_hash)
                .map(|(key, location)| {
                    let store = store.clone();
                    async move { (key, store.content_hash(&location).await) }
                })
                .buffer_unordered(HASH_CONCURRENCY)
                .collect()
                .await;

            for (key, result) in results {
                match result {
                    Ok(hash) => {
                        hashes.insert(key, hash);
                    }
                    Err(err) => {
                        context.log_message(
                            &format!("Failed to hash {}: {}", key, err),
                            LogLevel::Warn,
                        );
                        hash_failed.insert(key);
                    }
                }
            }
        }

        let mut added = Vec::new();
        let mut updated = Vec::new();
        let mut entries = Vec::with_capacity(candidates.len());

        for object in candidates {
            let key = object.location.as_ref().to_string();
            let entry = ManifestEntry {
                path: key.clone(),
                e_tag: object.e_tag,
                hash: hashes.remove(&key),
                size: object.size,
                last_modified: object.last_modified.to_string(),
            };

            // A file selected for hashing whose read failed must never be reported as
            // unchanged: treat it as changed rather than falling back to size/mtime,
            // which can miss same-size same-mtime edits (the case Checksum mode and
            // weak-ETag hashing exist to catch).
            let flow_path = root.with_path(&key);
            match previous.remove(&key) {
                Some(prev) if hash_failed.contains(&key) || entry_changed(&prev, &entry) => {
                    updated.push(flow_path)
                }
                Some(_) => {}
                None => added.push(flow_path),
            }

            entries.push(entry);
        }

        let deleted = previous
            .into_keys()
            .map(|key| root.with_path(&key))
            .collect::<Vec<FlowPath>>();

        entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));
        let mut ignored_manifest_paths = ignored_manifest_keys.into_iter().collect::<Vec<_>>();
        ignored_manifest_paths.sort_unstable();

        let session = DirDiffSession {
            manifest: manifest_path,
            root: Some(root),
            ignored_manifest_paths,
            manifest_content: DirManifest {
                kind: ManifestKind::Directory,
                version: MANIFEST_VERSION,
                recursive,
                entries,
            },
        };

        context.set_pin_value("added", json!(added)).await?;
        context.set_pin_value("updated", json!(updated)).await?;
        context.set_pin_value("deleted", json!(deleted)).await?;
        context.set_pin_value("session", json!(session)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

async fn detect_manifest_keys(
    store: Arc<dyn ObjectStore>,
    objects: &[ObjectMeta],
    known_manifest_keys: &HashSet<String>,
) -> (HashSet<String>, usize) {
    let prefix_len = MANIFEST_PREFIX.len() as u64;
    let probes = objects
        .iter()
        .filter(|object| {
            object.size >= prefix_len && !known_manifest_keys.contains(object.location.as_ref())
        })
        .map(|object| {
            (
                object.location.as_ref().to_string(),
                object.location.clone(),
            )
        })
        .collect::<Vec<_>>();

    let results = stream::iter(probes)
        .map(|(key, location)| {
            let store = store.clone();
            async move {
                let bytes = store.get_range(&location, 0..prefix_len).await;
                (key, bytes)
            }
        })
        .buffer_unordered(HASH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

    let mut manifest_keys = HashSet::new();
    let mut failures = 0;
    for (key, result) in results {
        match result {
            Ok(bytes) if bytes.as_ref().starts_with(MANIFEST_PREFIX) => {
                manifest_keys.insert(key);
            }
            Ok(_) => {}
            Err(_) => failures += 1,
        }
    }

    (manifest_keys, failures)
}

/// Reads and parses the manifest. A genuinely missing manifest (the store reports
/// `NotFound`) yields an empty one, so every file is reported as added; malformed
/// JSON is tolerated the same way with a warning. Any other store error (network,
/// permission, throttling, …) is propagated instead of silently producing an empty
/// baseline that would report every file as added.
pub(super) async fn load_manifest(
    manifest: &FlowPath,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<DirManifest> {
    let runtime = manifest.to_runtime(context).await?;
    match runtime.store.as_generic().head(&runtime.path).await {
        Ok(_) => {}
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => {
            return Ok(DirManifest::default());
        }
        Err(err) => return Err(Error::from(err)),
    }

    let bytes = manifest.get(context, false).await?;

    match flow_like_types::json::from_slice::<DirManifest>(&bytes) {
        Ok(manifest) if manifest.version <= MANIFEST_VERSION => Ok(manifest),
        Ok(manifest) => {
            context.log_message(
                &format!(
                    "Manifest version {} is newer than supported {}, treating as empty",
                    manifest.version, MANIFEST_VERSION
                ),
                LogLevel::Warn,
            );
            Ok(DirManifest::default())
        }
        Err(err) => {
            context.log_message(
                &format!("Failed to parse manifest, treating as empty: {}", err),
                LogLevel::Warn,
            );
            Ok(DirManifest::default())
        }
    }
}

/// Whether a file changed between two manifest entries, preferring the strongest
/// signal available: content hash, then ETag, then size/mtime as a last resort.
fn entry_changed(previous: &ManifestEntry, current: &ManifestEntry) -> bool {
    if let (Some(old), Some(new)) = (&previous.hash, &current.hash) {
        return old != new;
    }
    if let (Some(old), Some(new)) = (&previous.e_tag, &current.e_tag) {
        return old != new;
    }
    previous.size != current.size || previous.last_modified != current.last_modified
}

/// An ETag we can rely on for change detection: present, non-empty, and not a
/// weak validator (`W/"…"`), which does not guarantee byte-level equality.
fn etag_trustworthy(e_tag: &Option<String>) -> bool {
    match e_tag {
        Some(tag) => !tag.is_empty() && !tag.starts_with("W/"),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::{PutPayload, memory::InMemory, path::Path};
    use flow_like_types::Bytes;

    async fn put(store: &Arc<dyn ObjectStore>, path: &str, bytes: impl Into<Bytes>) {
        store
            .put(&Path::from(path), PutPayload::from_bytes(bytes.into()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn detects_all_marked_manifests_regardless_of_name() {
        let store: Arc<dyn ObjectStore> = Arc::new(InMemory::new());
        let manifest = flow_like_types::json::to_vec(&DirManifest::default()).unwrap();
        put(&store, "root/workflow-a.state", manifest.clone()).await;
        put(&store, "root/completely-different-name", manifest).await;
        put(
            &store,
            "root/data.json",
            br#"{"version":1,"recursive":true,"entries":[],"padding":"this is ordinary user data and must remain visible"}"#
                .to_vec(),
        )
        .await;
        put(&store, "root/short", b"regular".to_vec()).await;

        let objects = store
            .list(Some(&Path::from("root")))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let (manifest_keys, failures) =
            detect_manifest_keys(store, &objects, &HashSet::new()).await;

        assert_eq!(failures, 0);
        assert_eq!(
            manifest_keys,
            HashSet::from([
                "root/workflow-a.state".to_string(),
                "root/completely-different-name".to_string(),
            ])
        );
    }

    #[test]
    fn diff_node_version_tracks_the_session_schema_change() {
        let node = GetChangesNode::new().get_node();

        assert_eq!(node.version, Some(1));
    }
}
