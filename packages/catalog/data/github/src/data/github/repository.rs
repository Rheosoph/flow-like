use crate::data::path::FlowPath;
#[cfg(feature = "execute")]
use flow_like::flow::execution::{LogLevel, context::ExecutionContext};
use flow_like::flow::{
    node::{Node, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::json::json;

pub(crate) fn node(id: &str, title: &str, description: &str, script: &str) -> Node {
    let mut node = Node::new(id, title, description, "Data/GitHub/Repository");
    node.set_flowscript_name("github", script);
    node.add_icon("/flow/icons/github.svg");
    node.set_long_running(true);
    node.add_input_pin(
        "exec_in",
        "Input",
        "Run the operation",
        VariableType::Execution,
    );
    node.add_input_pin(
        "repository",
        "Repository",
        "FlowPath to the root of a local Git working tree. Requires Git on the runtime host.",
        VariableType::Struct,
    )
    .set_schema::<FlowPath>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
    node.add_output_pin(
        "exec_out",
        "Success",
        "Operation completed",
        VariableType::Execution,
    );
    node.add_output_pin(
        "error",
        "Error",
        "Operation failed; inspect Error Message",
        VariableType::Execution,
    );
    node.add_output_pin(
        "repo_path",
        "Repository Path",
        "Local working tree for the next repository node",
        VariableType::Struct,
    )
    .set_schema::<FlowPath>();
    node.add_output_pin(
        "output",
        "Output",
        "Git output with credentials removed",
        VariableType::String,
    );
    node.add_output_pin(
        "error_message",
        "Error Message",
        "Failure details, empty on success",
        VariableType::String,
    );
    node.set_scores(
        NodeScores::new()
            .set_privacy(5)
            .set_security(5)
            .set_performance(7)
            .set_governance(5)
            .set_reliability(7)
            .set_cost(10)
            .build(),
    );
    node
}

#[cfg(feature = "execute")]
pub(crate) async fn local_path(
    context: &mut ExecutionContext,
    path: &FlowPath,
) -> flow_like_types::Result<std::path::PathBuf> {
    use flow_like_storage::files::store::FlowLikeStore;
    let store = path.to_store(context).await?;
    let FlowLikeStore::Local(local) = store else {
        flow_like_types::bail!(
            "Repository operations require a local filesystem store. Cloud and memory clones are file snapshots."
        );
    };
    resolve_local_path(&local, &path.object_path())
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn resolve_local_path(
    local: &flow_like_storage::files::store::local_store::LocalObjectStore,
    path: &flow_like_storage::Path,
) -> flow_like_types::Result<std::path::PathBuf> {
    let root = local
        .directory_to_filesystem(&flow_like_storage::Path::from(""))?
        .canonicalize()?;
    let target = local.directory_to_filesystem(path)?;
    validate_local_target(&root, &target)?;
    Ok(target)
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn validate_local_target(
    root: &std::path::Path,
    target: &std::path::Path,
) -> flow_like_types::Result<()> {
    let mut ancestor = target;
    while !ancestor.try_exists()? {
        // A dangling symlink must not become a path outside the store later.
        if std::fs::symlink_metadata(ancestor).is_ok() {
            flow_like_types::bail!("Repository path contains a dangling symlink");
        }
        ancestor = ancestor
            .parent()
            .ok_or_else(|| flow_like_types::anyhow!("Repository path has no existing parent"))?;
    }
    if target
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
        || !ancestor.canonicalize()?.starts_with(root)
    {
        flow_like_types::bail!("Repository path leaves the local filesystem store");
    }
    Ok(())
}

#[cfg(any(feature = "execute", test))]
pub(crate) fn validate_repo_component(value: &str) -> flow_like_types::Result<()> {
    if value.is_empty()
        || matches!(value, "." | "..")
        || value.starts_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        flow_like_types::bail!(
            "Owner and repository must be names containing only letters, digits, periods, hyphens, or underscores"
        );
    }
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) async fn begin(context: &mut ExecutionContext) -> flow_like_types::Result<()> {
    context.deactivate_exec_pin("exec_out").await?;
    context.deactivate_exec_pin("error").await?;
    context.set_pin_value("output", json!("")).await?;
    context.set_pin_value("error_message", json!("")).await?;
    context.set_pin_value("repo_path", json!(null)).await?;
    Ok(())
}

#[cfg(feature = "execute")]
pub(crate) async fn finish(
    context: &mut ExecutionContext,
    result: flow_like_types::Result<String>,
    path: &FlowPath,
) -> flow_like_types::Result<()> {
    match result {
        Ok(output) => {
            context
                .set_pin_value("output", json!(super::git::redact(&output)))
                .await?;
            context.set_pin_value("repo_path", json!(path)).await?;
            context.activate_exec_pin("exec_out").await?;
        }
        Err(error) => {
            let message = super::git::redact(&error.to_string());
            context.log_message(&message, LogLevel::Error);
            context
                .set_pin_value("error_message", json!(message))
                .await?;
            context.activate_exec_pin("error").await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_paths_resolve_through_the_local_store() {
        use flow_like_storage::{Path, files::store::local_store::LocalObjectStore};
        let dir =
            std::env::temp_dir().join(format!("flow-like repo % #123-{}", uuid::Uuid::new_v4()));
        let store = LocalObjectStore::new(dir.clone()).unwrap();
        let root = dir.canonicalize().unwrap();
        assert_eq!(resolve_local_path(&store, &Path::from("")).unwrap(), root);
        assert_eq!(
            resolve_local_path(&store, &Path::from("nested/repo")).unwrap(),
            root.join("nested/repo")
        );
        assert_eq!(
            resolve_local_path(&store, &Path::parse("build#123").unwrap()).unwrap(),
            root.join("build#123")
        );
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.parent().unwrap(), root.join("escape")).unwrap();
            assert!(resolve_local_path(&store, &Path::from("escape/repo")).is_err());
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_targets_stay_inside_store() {
        let root = std::env::temp_dir().canonicalize().unwrap();
        assert!(validate_local_target(&root, &root.join("flow-like-not-yet-created/repo")).is_ok());
        assert!(validate_local_target(&root, &root.join("../outside")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlink_targets_cannot_leave_store() {
        let root = std::env::temp_dir().join(format!("flow-like-path-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        std::os::unix::fs::symlink(root.parent().unwrap(), root.join("escape")).unwrap();
        assert!(validate_local_target(&root, &root.join("escape/new/repo")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
