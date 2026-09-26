use flow_like_catalog_data_github::get_catalog;
use flow_like_runtime::flow::{node::Node, pin::PinType, variable::VariableType};
use flow_like_types::{Value, json::json};
use std::collections::{BTreeMap, BTreeSet};

const REPOSITORY_NODES: &[(&str, &str, &str)] = &[
    ("data_github_fetch_repo", "Fetch Repository", "fetchRepo"),
    ("data_github_pull_repo", "Pull Repository", "pullRepo"),
    ("data_github_push_repo", "Push Repository", "pushRepo"),
    ("data_github_sync_repo", "Sync Repository", "syncRepo"),
    ("data_github_init_repo", "Init Repository", "initRepo"),
    (
        "data_github_add_repo_remote",
        "Add Repository Remote",
        "addRepoRemote",
    ),
    (
        "data_github_set_repo_remote_url",
        "Set Repository Remote URL",
        "setRepoRemoteUrl",
    ),
    (
        "data_github_remove_repo_remote",
        "Remove Repository Remote",
        "removeRepoRemote",
    ),
    (
        "data_github_local_status",
        "Repository Status",
        "repositoryStatus",
    ),
    (
        "data_github_local_diff",
        "Repository Diff",
        "diffRepository",
    ),
    ("data_github_local_log", "Repository Log", "repositoryLog"),
    (
        "data_github_list_local_branches",
        "List Local Branches",
        "listLocalBranches",
    ),
    (
        "data_github_switch_local_branch",
        "Switch Branch",
        "switchBranch",
    ),
    (
        "data_github_checkout_local_revision",
        "Checkout Revision",
        "checkoutRevision",
    ),
    (
        "data_github_create_local_branch",
        "Create Local Branch",
        "createLocalBranch",
    ),
    (
        "data_github_delete_local_branch",
        "Delete Local Branch",
        "deleteLocalBranch",
    ),
    ("data_github_stage_local_files", "Stage Files", "stageFiles"),
    (
        "data_github_unstage_local_files",
        "Unstage Files",
        "unstageFiles",
    ),
    (
        "data_github_commit_local_repository",
        "Commit Changes",
        "commitChanges",
    ),
    (
        "data_github_save_local_stash",
        "Stash Changes",
        "stashChanges",
    ),
    ("data_github_pop_local_stash", "Pop Stash", "popStash"),
    (
        "data_github_list_local_stashes",
        "List Stashes",
        "listStashes",
    ),
    (
        "data_github_reset_local_repository",
        "Reset Repository",
        "resetRepository",
    ),
    (
        "data_github_merge_local_repository",
        "Merge Branch",
        "mergeBranch",
    ),
    ("data_github_abort_local_merge", "Abort Merge", "abortMerge"),
    (
        "data_github_list_local_remotes",
        "List Remotes",
        "listRemotes",
    ),
    (
        "data_github_list_local_tags",
        "List Local Tags",
        "listLocalTags",
    ),
    (
        "data_github_create_local_tag",
        "Create Local Tag",
        "createLocalTag",
    ),
    (
        "data_github_delete_local_tag",
        "Delete Local Tag",
        "deleteLocalTag",
    ),
];

fn definitions() -> Vec<Node> {
    get_catalog().iter().map(|logic| logic.get_node()).collect()
}

#[test]
fn all_repository_nodes_are_registered_once() {
    let nodes = definitions();
    assert_eq!(REPOSITORY_NODES.len(), 29);
    let expected: BTreeSet<&str> = REPOSITORY_NODES.iter().map(|(name, _, _)| *name).collect();
    let actual: BTreeSet<&str> = nodes
        .iter()
        .filter(|node| node.category == "Data/GitHub/Repository")
        .map(|node| node.name.as_str())
        .collect();
    assert_eq!(actual, expected);

    for (name, title, alias) in REPOSITORY_NODES {
        let registered: Vec<&Node> = nodes.iter().filter(|node| node.name == *name).collect();
        assert_eq!(
            registered.len(),
            1,
            "{name} must be registered exactly once"
        );
        let node = registered[0];
        assert_eq!(node.friendly_name, *title, "{name}");
        assert_eq!(node.flowscript_namespace(), "github", "{name}");
        assert_eq!(node.flowscript_alias(), *alias, "{name}");
    }
}

#[test]
fn github_aliases_are_unique_and_preserve_api_entry_points() {
    let nodes = definitions();
    let mut aliases = BTreeMap::new();
    for node in &nodes {
        let key = (node.flowscript_namespace(), node.flowscript_alias());
        assert!(
            aliases.insert(key.clone(), node.name.as_str()).is_none(),
            "Duplicate FlowScript entry point {}.{}",
            key.0,
            key.1
        );
    }

    for (alias, name) in [
        ("cloneRepo", "data_github_clone_repo"),
        ("listBranches", "data_github_list_branches"),
        ("getBranch", "data_github_get_branch"),
        ("createBranch", "data_github_create_branch"),
        ("deleteBranch", "data_github_delete_branch"),
        ("listCommits", "data_github_list_commits"),
        ("getCommit", "data_github_get_commit"),
        ("mergePullRequest", "data_github_merge_pull_request"),
    ] {
        assert_eq!(
            aliases.get(&("github".to_string(), alias.to_string())),
            Some(&name),
            "Existing API entry point github.{alias} must retain its node"
        );
    }
}

#[test]
fn repository_nodes_expose_the_shared_execution_and_path_contract() {
    let nodes = definitions();
    for (name, _, _) in REPOSITORY_NODES {
        let node = nodes.iter().find(|node| node.name == *name).unwrap();
        for (pin_name, direction, data_type) in [
            ("exec_in", PinType::Input, VariableType::Execution),
            ("repository", PinType::Input, VariableType::Struct),
            ("exec_out", PinType::Output, VariableType::Execution),
            ("error", PinType::Output, VariableType::Execution),
            ("repo_path", PinType::Output, VariableType::Struct),
            ("output", PinType::Output, VariableType::String),
            ("error_message", PinType::Output, VariableType::String),
        ] {
            let pin = node
                .get_pin_by_name(pin_name)
                .unwrap_or_else(|| panic!("{name} is missing {pin_name}"));
            assert_eq!(pin.pin_type, direction, "{name}.{pin_name}");
            assert_eq!(pin.data_type, data_type, "{name}.{pin_name}");
        }
        let repository = node.get_pin_by_name("repository").unwrap();
        let output = node.get_pin_by_name("repo_path").unwrap();
        assert!(
            repository.schema.is_some(),
            "{name}.repository must have a FlowPath schema"
        );
        assert_eq!(
            repository.schema, output.schema,
            "{name} must pass its FlowPath to the next node"
        );
        assert_eq!(
            repository
                .options
                .as_ref()
                .and_then(|options| options.enforce_schema),
            Some(true),
            "{name}.repository must enforce its FlowPath schema"
        );
    }
}

fn default_value(node: &Node, pin_name: &str) -> Value {
    let bytes = node
        .get_pin_by_name(pin_name)
        .unwrap()
        .default_value
        .as_deref()
        .unwrap();
    flow_like_types::json::from_slice(bytes).unwrap()
}

#[test]
fn destructive_operations_require_explicit_choices() {
    let nodes = definitions();
    let by_name = |name: &str| nodes.iter().find(|node| node.name == name).unwrap();
    let reset = by_name("data_github_reset_local_repository");
    assert_eq!(default_value(reset, "revision"), json!("HEAD"));
    assert_eq!(default_value(reset, "mode"), json!("mixed"));
    assert_eq!(default_value(reset, "allow_destructive"), json!(false));
    assert_eq!(
        reset
            .get_pin_by_name("mode")
            .unwrap()
            .options
            .as_ref()
            .unwrap()
            .valid_values
            .as_deref(),
        Some(["soft".to_string(), "mixed".to_string(), "hard".to_string()].as_slice())
    );
    for (name, pin, expected) in [
        ("data_github_switch_local_branch", "require_clean", true),
        ("data_github_checkout_local_revision", "require_clean", true),
        ("data_github_delete_local_branch", "force", false),
        (
            "data_github_merge_local_repository",
            "fast_forward_only",
            true,
        ),
        ("data_github_stage_local_files", "all", false),
        ("data_github_unstage_local_files", "all", false),
    ] {
        assert_eq!(
            default_value(by_name(name), pin),
            json!(expected),
            "{name}.{pin}"
        );
    }
    assert!(
        by_name("data_github_push_repo")
            .get_pin_by_name("force")
            .is_none()
    );
}
