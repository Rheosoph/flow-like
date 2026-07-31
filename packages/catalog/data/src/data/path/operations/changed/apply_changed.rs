use super::{DirDiffSession, DirManifest, ManifestEntry, get_changes::load_manifest};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_storage::Path;
use flow_like_types::{anyhow, async_trait, json::json};
use std::collections::{HashMap, HashSet};

#[crate::register_node]
#[derive(Default)]
pub struct WriteManifestNode {}

impl WriteManifestNode {
    pub fn new() -> Self {
        WriteManifestNode {}
    }
}

#[async_trait]
impl NodeLogic for WriteManifestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "path_write_manifest",
            "Write Directory Manifest",
            "Commits all or selected paths from a directory diff session to its manifest, so the next diff only reports uncommitted changes",
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
            "session",
            "Session",
            "Diff session produced by 'Diff Directory'",
            VariableType::Struct,
        )
        .set_schema::<DirDiffSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "committed_paths",
            "Committed Paths",
            "Optional changed paths to commit. Leave disconnected to commit the full diff; connect an array to commit only those paths (an empty array commits none)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])))
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Output",
            "Done with the Execution",
            VariableType::Execution,
        );

        node.add_output_pin(
            "manifest",
            "Manifest",
            "FlowPath of the written manifest file",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

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

        let session: DirDiffSession = context.evaluate_pin("session").await?;
        let committed_paths = evaluate_committed_paths(context).await?;

        let manifest_content = match committed_paths {
            None => session.manifest_content,
            Some(paths) => {
                let root = session.root.as_ref().ok_or_else(|| {
                    anyhow!(
                        "Selective manifest commits require a new Diff Directory session; run the diff node again"
                    )
                })?;
                let selected_keys = selected_keys(root, paths)?;
                let previous = load_manifest(&session.manifest, context).await?;
                merge_selected_manifest(
                    previous,
                    session.manifest_content,
                    &selected_keys,
                    &session.ignored_manifest_paths,
                )?
            }
        };
        let bytes = flow_like_types::json::to_vec(&manifest_content)?;

        session.manifest.put(context, bytes, false).await?;

        context
            .set_pin_value("manifest", json!(session.manifest))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

async fn evaluate_committed_paths(
    context: &ExecutionContext,
) -> flow_like_types::Result<Option<Vec<FlowPath>>> {
    let pin = context.get_pin_by_name("committed_paths").await?;
    let has_override = context
        .context_pin_overrides
        .as_ref()
        .is_some_and(|overrides| overrides.contains_key(&pin.id));
    let provided = has_override
        || !pin.depends_on().is_empty()
        || pin.has_value().await
        || has_selected_default(pin.default_value.as_ref());

    if !provided {
        return Ok(None);
    }

    context
        .evaluate_pin::<Vec<FlowPath>>("committed_paths")
        .await
        .map(Some)
}

fn has_selected_default(value: Option<&flow_like_types::Value>) -> bool {
    value.is_some_and(|value| {
        value
            .as_array()
            .map(|paths| !paths.is_empty())
            .unwrap_or(true)
    })
}

fn selected_keys(
    root: &FlowPath,
    selected_paths: Vec<FlowPath>,
) -> flow_like_types::Result<HashSet<String>> {
    let root_path = Path::from(root.path.as_str());
    let mut selected = HashSet::with_capacity(selected_paths.len());

    for path in selected_paths {
        if path.store_ref != root.store_ref {
            return Err(anyhow!(
                "Cannot commit path '{}' from a different store than the diff root",
                path.path
            ));
        }

        let selected_path = path.path;
        let path = Path::parse(selected_path.as_str())
            .map_err(|err| anyhow!("Cannot commit invalid path '{}': {}", selected_path, err))?;
        if !path.prefix_matches(&root_path) {
            return Err(anyhow!(
                "Cannot commit path '{}' because it is outside the diff root '{}'",
                path,
                root_path
            ));
        }
        selected.insert(path.as_ref().to_string());
    }

    Ok(selected)
}

fn merge_selected_manifest(
    previous: DirManifest,
    current: DirManifest,
    selected_keys: &HashSet<String>,
    ignored_manifest_paths: &[String],
) -> flow_like_types::Result<DirManifest> {
    if !previous.entries.is_empty() && previous.recursive != current.recursive {
        return Err(anyhow!(
            "Cannot selectively commit after changing the recursive scan setting; commit the full diff first"
        ));
    }

    let mut entries: HashMap<String, ManifestEntry> = previous
        .entries
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect();
    for key in ignored_manifest_paths {
        entries.remove(key);
    }

    let mut current_entries: HashMap<String, ManifestEntry> = current
        .entries
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect();

    for key in selected_keys {
        match current_entries.remove(key) {
            Some(entry) => {
                entries.insert(key.clone(), entry);
            }
            None => {
                entries.remove(key);
            }
        }
    }

    let mut entries = entries.into_values().collect::<Vec<_>>();
    entries.sort_unstable_by(|left, right| left.path.cmp(&right.path));

    Ok(DirManifest {
        kind: current.kind,
        version: current.version,
        recursive: current.recursive,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::path::operations::changed::{MANIFEST_VERSION, ManifestKind};

    fn entry(path: &str, hash: &str) -> ManifestEntry {
        ManifestEntry {
            path: path.to_string(),
            e_tag: None,
            hash: Some(hash.to_string()),
            size: 1,
            last_modified: String::new(),
        }
    }

    fn manifest(recursive: bool, entries: Vec<ManifestEntry>) -> DirManifest {
        DirManifest {
            kind: ManifestKind::Directory,
            version: MANIFEST_VERSION,
            recursive,
            entries,
        }
    }

    #[test]
    fn selective_commit_merges_additions_updates_and_deletions() {
        let previous = manifest(
            true,
            vec![
                entry("root/delete-selected", "old"),
                entry("root/delete-unselected", "old"),
                entry("root/other-workflow.state", "old"),
                entry("root/update-selected", "old"),
                entry("root/update-unselected", "old"),
            ],
        );
        let current = manifest(
            true,
            vec![
                entry("root/add-selected", "new"),
                entry("root/add-unselected", "new"),
                entry("root/update-selected", "new"),
                entry("root/update-unselected", "new"),
            ],
        );
        let selected = HashSet::from([
            "root/add-selected".to_string(),
            "root/delete-selected".to_string(),
            "root/update-selected".to_string(),
        ]);

        let merged = merge_selected_manifest(
            previous,
            current,
            &selected,
            &["root/other-workflow.state".to_string()],
        )
        .unwrap();
        let entries = merged
            .entries
            .into_iter()
            .map(|entry| (entry.path, entry.hash.unwrap()))
            .collect::<HashMap<_, _>>();

        assert_eq!(entries.get("root/add-selected").unwrap(), "new");
        assert_eq!(entries.get("root/update-selected").unwrap(), "new");
        assert_eq!(entries.get("root/update-unselected").unwrap(), "old");
        assert_eq!(entries.get("root/delete-unselected").unwrap(), "old");
        assert!(!entries.contains_key("root/add-unselected"));
        assert!(!entries.contains_key("root/delete-selected"));
        assert!(!entries.contains_key("root/other-workflow.state"));
    }

    #[test]
    fn empty_selection_preserves_the_previous_entries() {
        let previous = manifest(true, vec![entry("root/file", "old")]);
        let current = manifest(true, vec![entry("root/file", "new")]);

        let merged = merge_selected_manifest(previous, current, &HashSet::new(), &[]).unwrap();

        assert_eq!(merged.entries.len(), 1);
        assert_eq!(merged.entries[0].hash.as_deref(), Some("old"));
    }

    #[test]
    fn selective_commit_rejects_a_changed_scan_scope() {
        let previous = manifest(false, vec![entry("root/file", "old")]);
        let current = manifest(true, vec![entry("root/file", "new")]);

        let result = merge_selected_manifest(
            previous,
            current,
            &HashSet::from(["root/file".to_string()]),
            &[],
        );

        assert!(result.is_err());
    }

    #[test]
    fn selected_paths_must_belong_to_the_diff_root() {
        let root = FlowPath::new("root".to_string(), "store".to_string(), None);
        let other_store = FlowPath::new("root/file".to_string(), "other".to_string(), None);
        let outside = FlowPath::new("other/file".to_string(), "store".to_string(), None);

        assert!(selected_keys(&root, vec![other_store]).is_err());
        assert!(selected_keys(&root, vec![outside]).is_err());
    }

    #[test]
    fn selected_paths_preserve_encoded_object_keys() {
        let root = FlowPath::new("root".to_string(), "store".to_string(), None);
        let encoded = FlowPath::new("root/a%23b.txt".to_string(), "store".to_string(), None);

        let selected = selected_keys(&root, vec![encoded]).unwrap();

        assert!(selected.contains("root/a%23b.txt"));
        assert!(!selected.contains("root/a%2523b.txt"));
    }

    #[test]
    fn writer_node_exposes_the_optional_array_selector() {
        let node = WriteManifestNode::new().get_node();
        let selector = node.get_pin_by_name("committed_paths").unwrap();

        assert_eq!(node.version, Some(1));
        assert_eq!(selector.value_type, ValueType::Array);
        assert!(selector.default_value.is_some());
    }

    #[test]
    fn only_non_empty_or_invalid_inline_defaults_select_paths() {
        assert!(!has_selected_default(None));
        assert!(!has_selected_default(Some(&json!([]))));
        assert!(has_selected_default(Some(&json!([{
            "path": "root/file",
            "store_ref": "store",
            "cache_store_ref": null
        }]))));
        assert!(has_selected_default(Some(&json!(null))));
    }
}
