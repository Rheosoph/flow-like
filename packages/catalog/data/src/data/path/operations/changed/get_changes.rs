use super::{DirDiffSession, DirManifest, MANIFEST_VERSION, ManifestEntry};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Error, async_trait, json::json};
use futures::{StreamExt, stream};
use std::collections::{HashMap, HashSet};

/// Bounded parallelism when Blake3-hashing file bodies (Checksum mode / weak ETags).
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
            "Diffs a folder against a manifest, emitting added, updated and deleted files. Auto mode trusts store ETags (hashing only weak/missing ones); Checksum mode always Blake3-hashes contents",
            "Data/Files/Operations",
        );
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
            "FlowPath to the manifest file. May not exist yet — everything is then reported as added",
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

        // Skip the manifest file itself only when it lives in the same store as the
        // scanned root; an identical path in a different store is a real file. Compare
        // normalized object-store paths so a stray leading/trailing slash never leaks
        // the manifest back into the scan as phantom churn.
        let same_store = root.store_ref == manifest_path.store_ref;
        let manifest_key = flow_like_storage::Path::from(manifest_path.path.as_str());
        let candidates: Vec<_> = objects
            .into_iter()
            .filter(|object| !same_store || object.location != manifest_key)
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

        let session = DirDiffSession {
            manifest: manifest_path,
            manifest_content: DirManifest {
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

/// Reads and parses the manifest. A genuinely missing manifest (the store reports
/// `NotFound`) yields an empty one, so every file is reported as added; malformed
/// JSON is tolerated the same way with a warning. Any other store error (network,
/// permission, throttling, …) is propagated instead of silently producing an empty
/// baseline that would report every file as added.
async fn load_manifest(
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
