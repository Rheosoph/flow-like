use super::{
    list_repos::GitHubRepository,
    provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

fn parse_repo(repo: &Value) -> Option<GitHubRepository> {
    Some(GitHubRepository {
        id: repo["id"].as_i64()?,
        name: repo["name"].as_str()?.to_string(),
        full_name: repo["full_name"].as_str()?.to_string(),
        description: repo["description"].as_str().map(String::from),
        html_url: repo["html_url"].as_str()?.to_string(),
        clone_url: repo["clone_url"].as_str().unwrap_or_default().to_string(),
        ssh_url: repo["ssh_url"].as_str().unwrap_or_default().to_string(),
        private: repo["private"].as_bool().unwrap_or(false),
        fork: repo["fork"].as_bool().unwrap_or(false),
        archived: repo["archived"].as_bool().unwrap_or(false),
        default_branch: repo["default_branch"]
            .as_str()
            .unwrap_or("main")
            .to_string(),
        language: repo["language"].as_str().map(String::from),
        stargazers_count: repo["stargazers_count"].as_i64().unwrap_or(0),
        forks_count: repo["forks_count"].as_i64().unwrap_or(0),
        open_issues_count: repo["open_issues_count"].as_i64().unwrap_or(0),
        created_at: repo["created_at"].as_str()?.to_string(),
        updated_at: repo["updated_at"].as_str()?.to_string(),
        pushed_at: repo["pushed_at"].as_str().map(String::from),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct GetGitHubRepoNode {}

impl GetGitHubRepoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGitHubRepoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_repo",
            "Get Repository",
            "Get detailed information about a specific repository",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "getRepo");
        node.add_icon("/flow/icons/github.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "owner",
            "Owner",
            "Repository owner (user or organization)",
            VariableType::String,
        );

        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );

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
            "repository",
            "Repository",
            "Repository details",
            VariableType::Struct,
        )
        .set_schema::<GitHubRepository>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository name are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}", owner, repo));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let repo_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(repository) = parse_repo(&repo_json) {
                    context.log_message(
                        &format!("Retrieved repository: {}", repository.full_name),
                        LogLevel::Info,
                    );
                    context
                        .set_pin_value("repository", json!(repository))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse repository data", LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
