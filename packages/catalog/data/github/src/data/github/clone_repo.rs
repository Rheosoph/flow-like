use super::provider::{GITHUB_PROVIDER_ID, GitHubProvider};
use crate::data::path::FlowPath;
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use flow_like_storage::files::store::FlowLikeStore;

#[crate::register_node]
#[derive(Default)]
pub struct CloneGitHubRepoNode {}

impl CloneGitHubRepoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CloneGitHubRepoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_clone_repo",
            "Clone Repository",
            "Clone a GitHub repository. Works with any FlowPath store type (local, S3, memory, etc.). For non-local stores, clones to a temp directory first, then copies files into the target store.",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "cloneRepo");
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider for authentication",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );

        node.add_input_pin(
            "target_dir",
            "Target Directory",
            "FlowPath directory to clone into (supports any store type)",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "branch",
            "Branch",
            "Branch to clone (leave empty for default branch)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "depth",
            "Depth",
            "Shallow clone depth (0 for full clone)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        node.add_input_pin(
            "include_git",
            "Include .git",
            "Copy .git metadata into non-local stores. Local clones always retain their Git metadata.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );

        node.add_output_pin(
            "repo_path",
            "Repository Path",
            "FlowPath to the cloned repository",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(5)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;
        context.set_pin_value("repo_path", json!(null)).await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let mut target_dir: FlowPath = context.evaluate_pin("target_dir").await?;
        let branch: String = context.evaluate_pin("branch").await.unwrap_or_default();
        let depth: i64 = context.evaluate_pin("depth").await.unwrap_or(0);
        let include_git: bool = context.evaluate_pin("include_git").await.unwrap_or(false);

        if let Err(error) = super::repository::validate_repo_component(&owner)
            .and_then(|_| super::repository::validate_repo_component(&repo))
        {
            context.log_message(&error.to_string(), LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }
        if depth < 0 {
            context.log_message("Depth must be zero or positive", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let store = target_dir.to_store(context).await?;
        let local_target = if let FlowLikeStore::Local(local) = &store {
            Some(super::repository::resolve_local_path(
                local,
                &target_dir.object_path().join(repo.as_str()),
            )?)
        } else {
            None
        };
        drop(store);

        if let Some(target_path) = local_target {
            // Local stores keep the working tree and its Git metadata.
            context.log_message(
                &format!("Cloning {}/{} to {:?} (local)", owner, repo, target_path),
                LogLevel::Info,
            );

            match run_git_clone(&provider, &owner, &repo, &target_path, &branch, depth).await {
                Ok(()) => {
                    target_dir.path = build_repo_subpath(&target_dir.path, &repo);
                    context
                        .set_pin_value("repo_path", json!(target_dir))
                        .await?;
                    context.log_message(
                        &format!("Successfully cloned {}/{}", owner, repo),
                        LogLevel::Info,
                    );
                    context.activate_exec_pin("exec_out").await?;
                }
                Err(e) => {
                    let safe = e.to_string().replace(&provider.access_token, "***");
                    context.log_message(&format!("Git clone failed: {}", safe), LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
        } else {
            // Universal path: clone to temp dir, copy into FlowPath store
            let temp_dir =
                std::env::temp_dir().join(format!("flow-like-clone-{}", uuid::Uuid::new_v4()));

            context.log_message(
                &format!("Cloning {}/{} via temp directory", owner, repo),
                LogLevel::Info,
            );

            match run_git_clone(&provider, &owner, &repo, &temp_dir, &branch, depth).await {
                Ok(()) => {
                    let copy_result =
                        copy_dir_to_flowpath(&temp_dir, &target_dir, &repo, include_git, context)
                            .await;
                    let _ = std::fs::remove_dir_all(&temp_dir);

                    match copy_result {
                        Ok(file_count) => {
                            target_dir.path = build_repo_subpath(&target_dir.path, &repo);
                            context
                                .set_pin_value("repo_path", json!(target_dir))
                                .await?;
                            context.log_message(
                                &format!(
                                    "Successfully cloned {}/{} ({} files)",
                                    owner, repo, file_count
                                ),
                                LogLevel::Info,
                            );
                            context.activate_exec_pin("exec_out").await?;
                        }
                        Err(e) => {
                            context.log_message(
                                &format!("Failed to copy cloned files to store: {}", e),
                                LogLevel::Error,
                            );
                            context.activate_exec_pin("error").await?;
                        }
                    }
                }
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&temp_dir);
                    let safe = e.to_string().replace(&provider.access_token, "***");
                    context.log_message(&format!("Git clone failed: {}", safe), LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "This node requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn build_repo_subpath(base: &str, repo: &str) -> String {
    if base.is_empty() {
        repo.to_string()
    } else {
        format!("{}/{}", base, repo)
    }
}

#[cfg(feature = "execute")]
async fn run_git_clone(
    provider: &GitHubProvider,
    owner: &str,
    repo: &str,
    target: &std::path::Path,
    branch: &str,
    depth: i64,
) -> flow_like_types::Result<()> {
    let provider = provider.clone();
    let clone_url = provider.clone_url(owner, repo);
    let target = target.to_owned();
    let branch = branch.to_owned();
    flow_like_types::tokio::task::spawn_blocking(move || {
        let mut parent = target
            .parent()
            .ok_or_else(|| flow_like_types::anyhow!("Clone target needs a parent directory"))?;
        while !parent.exists() {
            parent = parent
                .parent()
                .ok_or_else(|| flow_like_types::anyhow!("Clone target needs an existing parent"))?;
        }
        let depth = depth.to_string();
        let mut args = vec!["clone"];
        if !branch.is_empty() {
            super::git::validate_ref(&branch)?;
            args.extend(["--branch", branch.as_str()]);
        }
        if depth != "0" {
            args.extend(["--depth", depth.as_str()]);
        }
        args.extend([
            "--",
            clone_url.as_str(),
            target
                .to_str()
                .ok_or_else(|| flow_like_types::anyhow!("Clone target must be UTF-8"))?,
        ]);
        super::git::run_network(parent, &args, &provider)?;
        Ok(())
    })
    .await?
}

#[cfg(feature = "execute")]
async fn copy_dir_to_flowpath(
    source_dir: &std::path::Path,
    target: &FlowPath,
    repo_name: &str,
    include_git: bool,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<usize> {
    let mut stack = vec![source_dir.to_path_buf()];
    let mut file_count = 0usize;

    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)?;
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;

            let relative = path
                .strip_prefix(source_dir)
                .map_err(|e| flow_like_types::anyhow!("Path prefix error: {}", e))?;

            if !include_git && relative.starts_with(".git") {
                continue;
            }

            if file_type.is_dir() {
                stack.push(path);
            } else if file_type.is_file() {
                let bytes = std::fs::read(&path)?;
                let relative_str = relative.to_string_lossy().replace('\\', "/");
                let full_path = if target.path.is_empty() {
                    format!("{}/{}", repo_name, relative_str)
                } else {
                    format!("{}/{}/{}", target.path, repo_name, relative_str)
                };

                let file_path = FlowPath::new(
                    full_path,
                    target.store_ref.clone(),
                    target.cache_store_ref.clone(),
                );
                file_path.put(context, bytes, false).await?;
                file_count += 1;
            }
        }
    }

    Ok(file_count)
}
