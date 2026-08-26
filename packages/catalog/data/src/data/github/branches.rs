use super::provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubBranch {
    pub name: String,
    pub sha: String,
    pub protected: bool,
}

fn parse_branch(branch: &Value) -> Option<GitHubBranch> {
    Some(GitHubBranch {
        name: branch["name"].as_str()?.to_string(),
        sha: branch["commit"]["sha"].as_str()?.to_string(),
        protected: branch["protected"].as_bool().unwrap_or(false),
    })
}

// =============================================================================
// List Branches Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubBranchesNode {}

impl ListGitHubBranchesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubBranchesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_branches",
            "List Branches",
            "List branches for a repository",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "listBranches");
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

        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );

        node.add_input_pin(
            "protected_only",
            "Protected Only",
            "Only list protected branches",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "per_page",
            "Per Page",
            "Results per page (max 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(30)));

        node.add_input_pin("page", "Page", "Page number", VariableType::Integer)
            .set_default_value(Some(json!(1)));

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
            "branches",
            "Branches",
            "Array of branches",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubBranch>();

        node.add_output_pin(
            "count",
            "Count",
            "Number of branches returned",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(8)
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
        let protected_only: bool = context
            .evaluate_pin("protected_only")
            .await
            .unwrap_or(false);
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let mut url = format!(
            "/repos/{}/{}/branches?per_page={}&page={}",
            owner,
            repo,
            per_page.clamp(1, 100),
            page.max(1)
        );

        if protected_only {
            url.push_str("&protected=true");
        }

        let full_url = provider.api_url(&url);

        let client = reqwest::Client::new();
        let response = client
            .get(&full_url)
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

                let branches_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let branches: Vec<GitHubBranch> =
                    branches_json.iter().filter_map(parse_branch).collect();

                let count = branches.len() as i64;

                context.log_message(&format!("Found {} branches", count), LogLevel::Info);
                context.set_pin_value("branches", json!(branches)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Get Branch Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGitHubBranchNode {}

impl GetGitHubBranchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGitHubBranchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_branch",
            "Get Branch",
            "Get details about a specific branch",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "getBranch");
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

        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );
        node.add_input_pin("branch", "Branch", "Branch name", VariableType::String);

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
            "branch_info",
            "Branch",
            "Branch information",
            VariableType::Struct,
        )
        .set_schema::<GitHubBranch>();

        node.add_output_pin("sha", "SHA", "Latest commit SHA", VariableType::String);

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
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
        let branch: String = context.evaluate_pin("branch").await?;

        if owner.is_empty() || repo.is_empty() || branch.is_empty() {
            context.log_message(
                "Owner, repository, and branch are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/branches/{}",
            owner,
            repo,
            urlencoding::encode(&branch)
        ));

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

                let branch_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(branch_info) = parse_branch(&branch_json) {
                    context
                        .set_pin_value("sha", json!(branch_info.sha.clone()))
                        .await?;
                    context
                        .set_pin_value("branch_info", json!(branch_info))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse branch", LogLevel::Error);
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

// =============================================================================
// Create Branch Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGitHubBranchNode {}

impl CreateGitHubBranchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGitHubBranchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_create_branch",
            "Create Branch",
            "Create a new branch from a reference (branch name or SHA)",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "createBranch");
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

        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );
        node.add_input_pin(
            "branch",
            "Branch Name",
            "Name for the new branch",
            VariableType::String,
        );
        node.add_input_pin(
            "from_sha",
            "From SHA",
            "SHA to create branch from (get from Get Branch node)",
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
            "ref",
            "Ref",
            "Created reference (refs/heads/...)",
            VariableType::String,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
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
        let branch: String = context.evaluate_pin("branch").await?;
        let from_sha: String = context.evaluate_pin("from_sha").await?;

        if owner.is_empty() || repo.is_empty() || branch.is_empty() || from_sha.is_empty() {
            context.log_message(
                "Owner, repository, branch name, and source SHA are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}/git/refs", owner, repo));

        let request_body = json!({
            "ref": format!("refs/heads/{}", branch),
            "sha": from_sha
        });

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .json(&request_body)
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

                let ref_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let ref_name = ref_json["ref"].as_str().unwrap_or_default();
                context.log_message(&format!("Created branch: {}", ref_name), LogLevel::Info);
                context.set_pin_value("ref", json!(ref_name)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// Delete Branch Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteGitHubBranchNode {}

impl DeleteGitHubBranchNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteGitHubBranchNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_delete_branch",
            "Delete Branch",
            "Delete a branch from a repository",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "deleteBranch");
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

        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );
        node.add_input_pin("branch", "Branch", "Branch to delete", VariableType::String);

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

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(6)
                .set_performance(9)
                .set_governance(5)
                .set_reliability(8)
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
        let branch: String = context.evaluate_pin("branch").await?;

        if owner.is_empty() || repo.is_empty() || branch.is_empty() {
            context.log_message(
                "Owner, repository, and branch are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/git/refs/heads/{}",
            owner,
            repo,
            urlencoding::encode(&branch)
        ));

        let client = reqwest::Client::new();
        let response = client
            .delete(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status() == 204 || resp.status().is_success() {
                    context.log_message(&format!("Deleted branch: {}", branch), LogLevel::Info);
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
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
