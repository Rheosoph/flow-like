use super::{
    list_issues::GitHubIssueUser,
    list_pull_requests::{GitHubPullRequest, parse_pr},
    provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubPullRequestFile {
    pub sha: String,
    pub filename: String,
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub changes: i64,
    pub patch: Option<String>,
}

fn parse_pr_file(file: &Value) -> Option<GitHubPullRequestFile> {
    Some(GitHubPullRequestFile {
        sha: file["sha"].as_str()?.to_string(),
        filename: file["filename"].as_str()?.to_string(),
        status: file["status"].as_str()?.to_string(),
        additions: file["additions"].as_i64().unwrap_or(0),
        deletions: file["deletions"].as_i64().unwrap_or(0),
        changes: file["changes"].as_i64().unwrap_or(0),
        patch: file["patch"].as_str().map(String::from),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubPullRequestReview {
    pub id: i64,
    pub user: GitHubIssueUser,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub submitted_at: Option<String>,
}

fn parse_review(review: &Value) -> Option<GitHubPullRequestReview> {
    let user = &review["user"];
    Some(GitHubPullRequestReview {
        id: review["id"].as_i64()?,
        user: GitHubIssueUser {
            id: user["id"].as_i64()?,
            login: user["login"].as_str()?.to_string(),
            avatar_url: user["avatar_url"].as_str()?.to_string(),
            html_url: user["html_url"].as_str()?.to_string(),
        },
        body: review["body"].as_str().map(String::from),
        state: review["state"].as_str()?.to_string(),
        html_url: review["html_url"].as_str()?.to_string(),
        submitted_at: review["submitted_at"].as_str().map(String::from),
    })
}

// =============================================================================
// Get Pull Request Node
// =============================================================================

/// A single inline review comment, in the shape GitHub's review API expects.
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ReviewComment {
    /// File the comment belongs to, relative to the repository root.
    pub path: String,
    /// The comment text.
    pub body: String,
    /// Line in the diff the comment is anchored to.
    pub line: Option<i64>,
    /// First line, when the comment spans a range.
    pub start_line: Option<i64>,
    /// Which side of the diff: LEFT for the old file, RIGHT for the new one.
    pub side: Option<String>,
}

#[crate::register_node]
#[derive(Default)]
pub struct GetGitHubPullRequestNode {}

impl GetGitHubPullRequestNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGitHubPullRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_pull_request",
            "Get Pull Request",
            "Get details about a specific pull request",
            "Data/GitHub",
        );
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
            "pr_number",
            "PR Number",
            "Pull request number",
            VariableType::Integer,
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
            "pull_request",
            "Pull Request",
            "Pull request details",
            VariableType::Struct,
        )
        .set_schema::<GitHubPullRequest>();

        node.add_output_pin("title", "Title", "PR title", VariableType::String);
        node.add_output_pin("body", "Body", "PR body", VariableType::String);
        node.add_output_pin(
            "state",
            "State",
            "PR state (open/closed)",
            VariableType::String,
        );
        node.add_output_pin(
            "mergeable",
            "Mergeable",
            "Whether the PR can be merged",
            VariableType::Boolean,
        );

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
        let pr_number: i64 = context.evaluate_pin("pr_number").await?;

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}/pulls/{}", owner, repo, pr_number));

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

                let pr_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(pr) = parse_pr(&pr_json) {
                    let mergeable = pr_json["mergeable"].as_bool().unwrap_or(false);

                    context
                        .set_pin_value("title", json!(pr.title.clone()))
                        .await?;
                    context
                        .set_pin_value("body", json!(pr.body.clone().unwrap_or_default()))
                        .await?;
                    context
                        .set_pin_value("state", json!(pr.state.clone()))
                        .await?;
                    context.set_pin_value("mergeable", json!(mergeable)).await?;
                    context.set_pin_value("pull_request", json!(pr)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse pull request", LogLevel::Error);
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
// Create Pull Request Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGitHubPullRequestNode {}

impl CreateGitHubPullRequestNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGitHubPullRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_create_pull_request",
            "Create Pull Request",
            "Create a new pull request",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

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
        node.add_input_pin("title", "Title", "Pull request title", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "head",
            "Head",
            "Branch containing changes (owner:branch for cross-repo)",
            VariableType::String,
        );
        node.add_input_pin("base", "Base", "Branch to merge into", VariableType::String);

        node.add_input_pin(
            "issue",
            "Issue",
            "Issue number to convert into a pull request",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "head_repo",
            "Head Repository",
            "Repository name containing the head branch when both repos are owned by the same organization",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "body",
            "Body",
            "Pull request description (Markdown supported)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "draft",
            "Draft",
            "Create as draft pull request",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "maintainer_can_modify",
            "Maintainer Can Modify",
            "Allow maintainers to modify the PR",
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
            "pull_request",
            "Pull Request",
            "Created pull request",
            VariableType::Struct,
        )
        .set_schema::<GitHubPullRequest>();

        node.add_output_pin(
            "pr_number",
            "PR Number",
            "Pull request number",
            VariableType::Integer,
        );
        node.add_output_pin("html_url", "URL", "Pull request URL", VariableType::String);

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(8)
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
        let title: String = context.evaluate_pin("title").await.unwrap_or_default();
        let head: String = context.evaluate_pin("head").await?;
        let base: String = context.evaluate_pin("base").await?;
        let issue: i64 = context.evaluate_pin("issue").await.unwrap_or(0);
        let head_repo: String = context.evaluate_pin("head_repo").await.unwrap_or_default();
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let draft: bool = context.evaluate_pin("draft").await.unwrap_or(false);
        let maintainer_can_modify: bool = context
            .evaluate_pin("maintainer_can_modify")
            .await
            .unwrap_or(true);

        if owner.is_empty() || repo.is_empty() || head.is_empty() || base.is_empty() {
            context.log_message(
                "Owner, repository, head, and base are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        if issue <= 0 && title.is_empty() {
            context.log_message(
                "Title is required unless an issue number is provided",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}/pulls", owner, repo));

        let mut request_body = json!({
            "head": head,
            "base": base,
            "draft": draft,
            "maintainer_can_modify": maintainer_can_modify
        });

        if issue > 0 {
            request_body["issue"] = json!(issue);
        } else {
            request_body["title"] = json!(title);
        }

        if !head_repo.is_empty() {
            request_body["head_repo"] = json!(head_repo);
        }

        if !body.is_empty() {
            request_body["body"] = json!(body);
        }

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

                let pr_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(pr) = parse_pr(&pr_json) {
                    context.log_message(
                        &format!("Created PR #{}: {}", pr.number, pr.title),
                        LogLevel::Info,
                    );
                    context.set_pin_value("pr_number", json!(pr.number)).await?;
                    context
                        .set_pin_value("html_url", json!(pr.html_url.clone()))
                        .await?;
                    context.set_pin_value("pull_request", json!(pr)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse created PR", LogLevel::Error);
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
// Update Pull Request Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct UpdateGitHubPullRequestNode {}

impl UpdateGitHubPullRequestNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateGitHubPullRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_update_pull_request",
            "Update Pull Request",
            "Update an existing pull request",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

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
            "pr_number",
            "PR Number",
            "Pull request number",
            VariableType::Integer,
        );
        node.add_input_pin(
            "title",
            "Title",
            "New title (leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "body",
            "Body",
            "New body (leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("state", "State", "New state", VariableType::String)
            .set_default_value(Some(json!("")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "".to_string(),
                        "open".to_string(),
                        "closed".to_string(),
                    ])
                    .build(),
            );
        node.add_input_pin(
            "base",
            "Base",
            "New base branch (leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "maintainer_can_modify",
            "Maintainer Can Modify",
            "Allow maintainers to modify the PR when connected",
            VariableType::Boolean,
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
            "pull_request",
            "Pull Request",
            "Updated pull request",
            VariableType::Struct,
        )
        .set_schema::<GitHubPullRequest>();

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(8)
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
        let pr_number: i64 = context.evaluate_pin("pr_number").await?;
        let title: String = context.evaluate_pin("title").await.unwrap_or_default();
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let state: String = context.evaluate_pin("state").await.unwrap_or_default();
        let base: String = context.evaluate_pin("base").await.unwrap_or_default();

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}/pulls/{}", owner, repo, pr_number));

        let mut request_body = json!({});
        if !title.is_empty() {
            request_body["title"] = json!(title);
        }
        if !body.is_empty() {
            request_body["body"] = json!(body);
        }
        if !state.is_empty() {
            request_body["state"] = json!(state);
        }
        if !base.is_empty() {
            request_body["base"] = json!(base);
        }
        if let Ok(maintainer_can_modify) =
            context.evaluate_pin::<bool>("maintainer_can_modify").await
        {
            request_body["maintainer_can_modify"] = json!(maintainer_can_modify);
        }

        let client = reqwest::Client::new();
        let response = client
            .patch(&url)
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

                let pr_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;
                if let Some(pr) = parse_pr(&pr_json) {
                    context.set_pin_value("pull_request", json!(pr)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse updated PR", LogLevel::Error);
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
// Merge Pull Request Node
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubMergeResult {
    pub sha: String,
    pub merged: bool,
    pub message: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct MergeGitHubPullRequestNode {}

impl MergeGitHubPullRequestNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MergeGitHubPullRequestNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_merge_pull_request",
            "Merge Pull Request",
            "Merge a pull request",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

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
            "pr_number",
            "PR Number",
            "Pull request number",
            VariableType::Integer,
        );

        node.add_input_pin(
            "commit_title",
            "Commit Title",
            "Title for the merge commit (leave empty for default)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "sha",
            "Head SHA",
            "Expected SHA of the pull request head to guard against merging stale changes",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "commit_message",
            "Commit Message",
            "Extra detail for merge commit (leave empty for default)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "merge_method",
            "Merge Method",
            "Method to use for merging",
            VariableType::String,
        )
        .set_default_value(Some(json!("merge")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "merge".to_string(),
                    "squash".to_string(),
                    "rebase".to_string(),
                ])
                .build(),
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
            "merge_sha",
            "Merge SHA",
            "SHA of merge commit",
            VariableType::String,
        );
        node.add_output_pin(
            "merged",
            "Merged",
            "Whether the PR was merged",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(5)
                .set_security(6)
                .set_performance(8)
                .set_governance(5)
                .set_reliability(8)
                .set_cost(8)
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
        let pr_number: i64 = context.evaluate_pin("pr_number").await?;
        let commit_title: String = context
            .evaluate_pin("commit_title")
            .await
            .unwrap_or_default();
        let sha: String = context.evaluate_pin("sha").await.unwrap_or_default();
        let commit_message: String = context
            .evaluate_pin("commit_message")
            .await
            .unwrap_or_default();
        let merge_method: String = context
            .evaluate_pin("merge_method")
            .await
            .unwrap_or_else(|_| "merge".to_string());

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/pulls/{}/merge",
            owner, repo, pr_number
        ));

        let mut request_body = json!({
            "merge_method": merge_method
        });

        if !commit_title.is_empty() {
            request_body["commit_title"] = json!(commit_title);
        }
        if !sha.is_empty() {
            request_body["sha"] = json!(sha);
        }
        if !commit_message.is_empty() {
            request_body["commit_message"] = json!(commit_message);
        }

        let client = reqwest::Client::new();
        let response = client
            .put(&url)
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

                let merge_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let sha = merge_json["sha"].as_str().unwrap_or_default();
                let merged = merge_json["merged"].as_bool().unwrap_or(false);

                context.log_message(
                    &format!("Merged PR #{} with SHA {}", pr_number, sha),
                    LogLevel::Info,
                );
                context.set_pin_value("merge_sha", json!(sha)).await?;
                context.set_pin_value("merged", json!(merged)).await?;
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
// List Pull Request Files Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubPullRequestFilesNode {}

impl ListGitHubPullRequestFilesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubPullRequestFilesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_pr_files",
            "List PR Files",
            "List files changed in a pull request",
            "Data/GitHub",
        );
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
            "pr_number",
            "PR Number",
            "Pull request number",
            VariableType::Integer,
        );

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
            "files",
            "Files",
            "Array of changed files",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubPullRequestFile>();

        node.add_output_pin(
            "count",
            "Count",
            "Number of files changed",
            VariableType::Integer,
        );
        node.add_output_pin(
            "additions",
            "Additions",
            "Total lines added",
            VariableType::Integer,
        );
        node.add_output_pin(
            "deletions",
            "Deletions",
            "Total lines deleted",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(8)
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
        let pr_number: i64 = context.evaluate_pin("pr_number").await?;
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/pulls/{}/files?per_page={}&page={}",
            owner,
            repo,
            pr_number,
            per_page.clamp(1, 100),
            page.max(1)
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

                let files_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let files: Vec<GitHubPullRequestFile> =
                    files_json.iter().filter_map(parse_pr_file).collect();

                let count = files.len() as i64;
                let additions: i64 = files.iter().map(|f| f.additions).sum();
                let deletions: i64 = files.iter().map(|f| f.deletions).sum();

                context.log_message(&format!("Found {} changed files", count), LogLevel::Info);
                context.set_pin_value("files", json!(files)).await?;
                context.set_pin_value("count", json!(count)).await?;
                context.set_pin_value("additions", json!(additions)).await?;
                context.set_pin_value("deletions", json!(deletions)).await?;
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
// List Pull Request Reviews Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubPullRequestReviewsNode {}

impl ListGitHubPullRequestReviewsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubPullRequestReviewsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_pr_reviews",
            "List PR Reviews",
            "List reviews on a pull request",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

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
            "pr_number",
            "PR Number",
            "Pull request number",
            VariableType::Integer,
        );

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
            "reviews",
            "Reviews",
            "Array of reviews",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubPullRequestReview>();

        node.add_output_pin("count", "Count", "Number of reviews", VariableType::Integer);

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
        let pr_number: i64 = context.evaluate_pin("pr_number").await?;
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/pulls/{}/reviews?per_page={}&page={}",
            owner,
            repo,
            pr_number,
            per_page.clamp(1, 100),
            page.max(1)
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

                let reviews_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let reviews: Vec<GitHubPullRequestReview> =
                    reviews_json.iter().filter_map(parse_review).collect();

                let count = reviews.len() as i64;

                context.log_message(&format!("Found {} reviews", count), LogLevel::Info);
                context.set_pin_value("reviews", json!(reviews)).await?;
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
// Create Pull Request Review Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGitHubPullRequestReviewNode {}

impl CreateGitHubPullRequestReviewNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGitHubPullRequestReviewNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_create_pr_review",
            "Create PR Review",
            "Create a review on a pull request",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

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
            "pr_number",
            "PR Number",
            "Pull request number",
            VariableType::Integer,
        );

        node.add_input_pin("body", "Body", "Review comment body", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "commit_id",
            "Commit ID",
            "SHA of the commit needing a review",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "comments",
            "Comments",
            "Inline review comments with path, position or line, and body",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<crate::data::github::pull_requests::ReviewComment>();

        node.add_input_pin("event", "Event", "Review event type", VariableType::String)
            .set_default_value(Some(json!("COMMENT")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "APPROVE".to_string(),
                        "REQUEST_CHANGES".to_string(),
                        "COMMENT".to_string(),
                    ])
                    .build(),
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

        node.add_output_pin("review", "Review", "Created review", VariableType::Struct)
            .set_schema::<GitHubPullRequestReview>();

        node.add_output_pin(
            "review_id",
            "Review ID",
            "ID of the created review",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(8)
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
        let pr_number: i64 = context.evaluate_pin("pr_number").await?;
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let commit_id: String = context.evaluate_pin("commit_id").await.unwrap_or_default();
        let comments: Value = context
            .evaluate_pin("comments")
            .await
            .unwrap_or_else(|_| json!([]));
        let event: String = context
            .evaluate_pin("event")
            .await
            .unwrap_or_else(|_| "COMMENT".to_string());

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/pulls/{}/reviews",
            owner, repo, pr_number
        ));

        let comments_count = comments.as_array().map(|items| items.len()).unwrap_or(0);

        if matches!(event.as_str(), "COMMENT" | "REQUEST_CHANGES")
            && body.is_empty()
            && comments_count == 0
        {
            context.log_message(
                "Body or inline comments are required for COMMENT and REQUEST_CHANGES reviews",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let mut request_body = json!({
            "event": event
        });

        if !body.is_empty() {
            request_body["body"] = json!(body);
        }
        if !commit_id.is_empty() {
            request_body["commit_id"] = json!(commit_id);
        }
        if comments_count > 0 {
            request_body["comments"] = comments;
        }

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

                let review_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(review) = parse_review(&review_json) {
                    context.log_message(
                        &format!("Created {} review on PR #{}", event, pr_number),
                        LogLevel::Info,
                    );
                    context.set_pin_value("review_id", json!(review.id)).await?;
                    context.set_pin_value("review", json!(review)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse created review", LogLevel::Error);
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
