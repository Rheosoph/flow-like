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
pub struct GitHubCommitAuthor {
    pub name: String,
    pub email: String,
    pub date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubCommitUser {
    pub login: String,
    pub id: i64,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubCommit {
    pub sha: String,
    pub message: String,
    pub author: GitHubCommitAuthor,
    pub committer: GitHubCommitAuthor,
    pub html_url: String,
    pub github_author: Option<GitHubCommitUser>,
    pub github_committer: Option<GitHubCommitUser>,
    pub stats: Option<GitHubCommitStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubCommitStats {
    pub additions: i64,
    pub deletions: i64,
    pub total: i64,
}

fn parse_commit_user(user: &Value) -> Option<GitHubCommitUser> {
    Some(GitHubCommitUser {
        login: user["login"].as_str()?.to_string(),
        id: user["id"].as_i64()?,
        avatar_url: user["avatar_url"].as_str()?.to_string(),
        html_url: user["html_url"].as_str()?.to_string(),
    })
}

pub fn parse_commit(commit: &Value) -> Option<GitHubCommit> {
    let commit_obj = &commit["commit"];
    let author = &commit_obj["author"];
    let committer = &commit_obj["committer"];

    Some(GitHubCommit {
        sha: commit["sha"].as_str()?.to_string(),
        message: commit_obj["message"].as_str()?.to_string(),
        author: GitHubCommitAuthor {
            name: author["name"].as_str()?.to_string(),
            email: author["email"].as_str()?.to_string(),
            date: author["date"].as_str()?.to_string(),
        },
        committer: GitHubCommitAuthor {
            name: committer["name"].as_str()?.to_string(),
            email: committer["email"].as_str()?.to_string(),
            date: committer["date"].as_str()?.to_string(),
        },
        html_url: commit["html_url"].as_str()?.to_string(),
        github_author: commit.get("author").and_then(parse_commit_user),
        github_committer: commit.get("committer").and_then(parse_commit_user),
        stats: commit.get("stats").and_then(|s| {
            Some(GitHubCommitStats {
                additions: s["additions"].as_i64()?,
                deletions: s["deletions"].as_i64()?,
                total: s["total"].as_i64()?,
            })
        }),
    })
}

// =============================================================================
// List Commits Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubCommitsNode {}

impl ListGitHubCommitsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubCommitsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_commits",
            "List Commits",
            "List commits for a repository",
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
            "sha",
            "SHA/Branch",
            "SHA or branch name to start listing commits from",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "path",
            "Path",
            "Only commits containing this file path",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "author",
            "Author",
            "GitHub username or email to filter by",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "committer",
            "Committer",
            "GitHub username or email to filter by committer",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "since",
            "Since",
            "Only commits after this date (ISO 8601 format)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "until",
            "Until",
            "Only commits before this date (ISO 8601 format)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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
            "commits",
            "Commits",
            "Array of commits",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubCommit>();

        node.add_output_pin(
            "count",
            "Count",
            "Number of commits returned",
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
        let sha: String = context.evaluate_pin("sha").await.unwrap_or_default();
        let path: String = context.evaluate_pin("path").await.unwrap_or_default();
        let author: String = context.evaluate_pin("author").await.unwrap_or_default();
        let committer: String = context.evaluate_pin("committer").await.unwrap_or_default();
        let since: String = context.evaluate_pin("since").await.unwrap_or_default();
        let until: String = context.evaluate_pin("until").await.unwrap_or_default();
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let mut url = format!(
            "/repos/{}/{}/commits?per_page={}&page={}",
            owner,
            repo,
            per_page.clamp(1, 100),
            page.max(1)
        );

        if !sha.is_empty() {
            url.push_str(&format!("&sha={}", urlencoding::encode(&sha)));
        }
        if !path.is_empty() {
            url.push_str(&format!("&path={}", urlencoding::encode(&path)));
        }
        if !author.is_empty() {
            url.push_str(&format!("&author={}", urlencoding::encode(&author)));
        }
        if !committer.is_empty() {
            url.push_str(&format!("&committer={}", urlencoding::encode(&committer)));
        }
        if !since.is_empty() {
            url.push_str(&format!("&since={}", urlencoding::encode(&since)));
        }
        if !until.is_empty() {
            url.push_str(&format!("&until={}", urlencoding::encode(&until)));
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

                let commits_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let commits: Vec<GitHubCommit> =
                    commits_json.iter().filter_map(parse_commit).collect();

                let count = commits.len() as i64;

                context.log_message(&format!("Found {} commits", count), LogLevel::Info);
                context.set_pin_value("commits", json!(commits)).await?;
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
// Get Commit Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGitHubCommitNode {}

impl GetGitHubCommitNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGitHubCommitNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_commit",
            "Get Commit",
            "Get details about a specific commit",
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
            "sha",
            "SHA",
            "Commit SHA, branch, or tag",
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
            "commit",
            "Commit",
            "Commit information",
            VariableType::Struct,
        )
        .set_schema::<GitHubCommit>();

        node.add_output_pin("message", "Message", "Commit message", VariableType::String);
        node.add_output_pin(
            "additions",
            "Additions",
            "Lines added",
            VariableType::Integer,
        );
        node.add_output_pin(
            "deletions",
            "Deletions",
            "Lines deleted",
            VariableType::Integer,
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
        let sha: String = context.evaluate_pin("sha").await?;

        if owner.is_empty() || repo.is_empty() || sha.is_empty() {
            context.log_message("Owner, repository, and SHA are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/commits/{}",
            owner,
            repo,
            urlencoding::encode(&sha)
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

                let commit_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(commit) = parse_commit(&commit_json) {
                    context
                        .set_pin_value("message", json!(commit.message.clone()))
                        .await?;

                    let (additions, deletions) = commit
                        .stats
                        .as_ref()
                        .map(|s| (s.additions, s.deletions))
                        .unwrap_or((0, 0));

                    context.set_pin_value("additions", json!(additions)).await?;
                    context.set_pin_value("deletions", json!(deletions)).await?;
                    context.set_pin_value("commit", json!(commit)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse commit", LogLevel::Error);
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
// Compare Commits Node
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubCompareResult {
    pub status: String,
    pub ahead_by: i64,
    pub behind_by: i64,
    pub total_commits: i64,
    pub commits: Vec<GitHubCommit>,
}

#[crate::register_node]
#[derive(Default)]
pub struct CompareGitHubCommitsNode {}

impl CompareGitHubCommitsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CompareGitHubCommitsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_compare_commits",
            "Compare Commits",
            "Compare two commits, branches, or tags",
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
            "base",
            "Base",
            "Base branch, tag, or SHA",
            VariableType::String,
        );
        node.add_input_pin(
            "head",
            "Head",
            "Head branch, tag, or SHA",
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
            "comparison",
            "Comparison",
            "Comparison result",
            VariableType::Struct,
        )
        .set_schema::<GitHubCompareResult>();

        node.add_output_pin(
            "status",
            "Status",
            "Comparison status (ahead, behind, identical, diverged)",
            VariableType::String,
        );
        node.add_output_pin(
            "ahead_by",
            "Ahead By",
            "Number of commits ahead",
            VariableType::Integer,
        );
        node.add_output_pin(
            "behind_by",
            "Behind By",
            "Number of commits behind",
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
        let base: String = context.evaluate_pin("base").await?;
        let head: String = context.evaluate_pin("head").await?;

        if owner.is_empty() || repo.is_empty() || base.is_empty() || head.is_empty() {
            context.log_message(
                "Owner, repository, base, and head are required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!(
            "/repos/{}/{}/compare/{}...{}",
            owner,
            repo,
            urlencoding::encode(&base),
            urlencoding::encode(&head)
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

                let compare_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let status = compare_json["status"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string();
                let ahead_by = compare_json["ahead_by"].as_i64().unwrap_or(0);
                let behind_by = compare_json["behind_by"].as_i64().unwrap_or(0);
                let total_commits = compare_json["total_commits"].as_i64().unwrap_or(0);

                let commits: Vec<GitHubCommit> = compare_json["commits"]
                    .as_array()
                    .map(|arr| arr.iter().filter_map(parse_commit).collect())
                    .unwrap_or_default();

                let result = GitHubCompareResult {
                    status: status.clone(),
                    ahead_by,
                    behind_by,
                    total_commits,
                    commits,
                };

                context.set_pin_value("status", json!(status)).await?;
                context.set_pin_value("ahead_by", json!(ahead_by)).await?;
                context.set_pin_value("behind_by", json!(behind_by)).await?;
                context.set_pin_value("comparison", json!(result)).await?;
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
