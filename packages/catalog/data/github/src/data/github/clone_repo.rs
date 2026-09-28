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
            "Clone a GitHub repository into any FlowPath store. Cloud and memory stores retain the checkout and Git metadata for subsequent repository operations.",
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
            "Retain Git metadata so repository operations can use cloud and memory clones. Disable for a file-only snapshot. Local clones always retain their Git metadata.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

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
        let include_git: bool = context.evaluate_pin("include_git").await.unwrap_or(true);

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
        if let Err(error) =
            super::git::ensure_origin_allowed(context.execution_environment(), &provider).await
        {
            context.log_message(&error.to_string(), LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        target_dir.path = target_dir.object_path().join(repo.as_str()).to_string();
        context.log_message(
            &format!("Cloning {}/{} into {}", owner, repo, target_dir.path),
            LogLevel::Info,
        );
        let result = async {
            let workspace = super::repository::Workspace::open(context, &target_dir).await?;
            let (workspace, result) =
                run_git_clone(&provider, &owner, &repo, workspace, &branch, depth).await?;
            if let Err(error) = result {
                if let Err(release) = workspace.discard().await {
                    return Err(flow_like_types::anyhow!(
                        "{error}; could not release repository storage lock: {release}"
                    ));
                }
                return Err(error);
            }
            workspace.persist_filtered(include_git).await
        }
        .await;

        match result {
            Ok(()) => {
                context
                    .set_pin_value("repo_path", json!(target_dir))
                    .await?;
                context.log_message(
                    &format!("Successfully cloned {}/{}", owner, repo),
                    LogLevel::Info,
                );
                context.activate_exec_pin("exec_out").await?;
            }
            Err(error) => {
                let safe = error.to_string().replace(&provider.access_token, "***");
                context.log_message(&format!("Git clone failed: {}", safe), LogLevel::Error);
                context.activate_exec_pin("error").await?;
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
async fn run_git_clone(
    provider: &GitHubProvider,
    owner: &str,
    repo: &str,
    workspace: super::repository::Workspace,
    branch: &str,
    depth: i64,
) -> flow_like_types::Result<(super::repository::Workspace, flow_like_types::Result<()>)> {
    let provider = provider.clone();
    let clone_url = provider.clone_url(owner, repo);
    let branch = branch.to_owned();
    flow_like_types::tokio::task::spawn_blocking(move || {
        let result = (|| {
            let target = workspace.path();
            let mut parent = target
                .parent()
                .ok_or_else(|| flow_like_types::anyhow!("Clone target needs a parent directory"))?;
            while !parent.exists() {
                parent = parent.parent().ok_or_else(|| {
                    flow_like_types::anyhow!("Clone target needs an existing parent")
                })?;
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
        })();
        (workspace, result)
    })
    .await
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clone_retains_git_metadata_by_default() {
        let node = CloneGitHubRepoNode::new().get_node();
        let include_git = node.get_pin_by_name("include_git").unwrap();
        let value: flow_like_types::Value =
            flow_like_types::json::from_slice(include_git.default_value.as_deref().unwrap())
                .unwrap();
        assert_eq!(value, json!(true));
    }
}
