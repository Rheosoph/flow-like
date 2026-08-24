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
pub struct GitHubPrUser {
    pub id: i64,
    pub login: String,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubPrBranch {
    pub label: String,
    #[serde(rename = "ref")]
    pub ref_name: String,
    pub sha: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubPullRequest {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub diff_url: String,
    pub patch_url: String,
    pub user: GitHubPrUser,
    pub head: GitHubPrBranch,
    pub base: GitHubPrBranch,
    pub draft: bool,
    pub merged: bool,
    pub mergeable: Option<bool>,
    pub commits: i64,
    pub additions: i64,
    pub deletions: i64,
    pub changed_files: i64,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
    pub merged_at: Option<String>,
}

pub fn parse_pr(pr: &Value) -> Option<GitHubPullRequest> {
    let user = &pr["user"];
    let head = &pr["head"];
    let base = &pr["base"];

    Some(GitHubPullRequest {
        id: pr["id"].as_i64()?,
        number: pr["number"].as_i64()?,
        title: pr["title"].as_str()?.to_string(),
        body: pr["body"].as_str().map(String::from),
        state: pr["state"].as_str()?.to_string(),
        html_url: pr["html_url"].as_str()?.to_string(),
        diff_url: pr["diff_url"].as_str().unwrap_or_default().to_string(),
        patch_url: pr["patch_url"].as_str().unwrap_or_default().to_string(),
        user: GitHubPrUser {
            id: user["id"].as_i64()?,
            login: user["login"].as_str()?.to_string(),
            avatar_url: user["avatar_url"].as_str()?.to_string(),
            html_url: user["html_url"].as_str()?.to_string(),
        },
        head: GitHubPrBranch {
            label: head["label"].as_str().unwrap_or_default().to_string(),
            ref_name: head["ref"].as_str().unwrap_or_default().to_string(),
            sha: head["sha"].as_str().unwrap_or_default().to_string(),
        },
        base: GitHubPrBranch {
            label: base["label"].as_str().unwrap_or_default().to_string(),
            ref_name: base["ref"].as_str().unwrap_or_default().to_string(),
            sha: base["sha"].as_str().unwrap_or_default().to_string(),
        },
        draft: pr["draft"].as_bool().unwrap_or(false),
        merged: pr["merged"].as_bool().unwrap_or(false),
        mergeable: pr["mergeable"].as_bool(),
        commits: pr["commits"].as_i64().unwrap_or(0),
        additions: pr["additions"].as_i64().unwrap_or(0),
        deletions: pr["deletions"].as_i64().unwrap_or(0),
        changed_files: pr["changed_files"].as_i64().unwrap_or(0),
        created_at: pr["created_at"].as_str()?.to_string(),
        updated_at: pr["updated_at"].as_str()?.to_string(),
        closed_at: pr["closed_at"].as_str().map(String::from),
        merged_at: pr["merged_at"].as_str().map(String::from),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubPullRequestsNode {}

impl ListGitHubPullRequestsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubPullRequestsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_pull_requests",
            "List Pull Requests",
            "List pull requests for a repository",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "listPullRequests");
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
            "state",
            "State",
            "PR state: open, closed, all",
            VariableType::String,
        )
        .set_default_value(Some(json!("open")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "open".to_string(),
                    "closed".to_string(),
                    "all".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "head",
            "Head",
            "Filter by head user or head user:ref (e.g., user:branch-name)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "base",
            "Base",
            "Filter by base branch name",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "sort",
            "Sort",
            "Sort by: created, updated, popularity, long-running",
            VariableType::String,
        )
        .set_default_value(Some(json!("created")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "created".to_string(),
                    "updated".to_string(),
                    "popularity".to_string(),
                    "long-running".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "direction",
            "Direction",
            "Sort direction: asc, desc",
            VariableType::String,
        )
        .set_default_value(Some(json!("desc")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["asc".to_string(), "desc".to_string()])
                .build(),
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
            "pull_requests",
            "Pull Requests",
            "Array of pull requests",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubPullRequest>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of pull requests returned",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
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
        let state: String = context
            .evaluate_pin("state")
            .await
            .unwrap_or_else(|_| "open".to_string());
        let head: String = context.evaluate_pin("head").await.unwrap_or_default();
        let base: String = context.evaluate_pin("base").await.unwrap_or_default();
        let sort: String = context
            .evaluate_pin("sort")
            .await
            .unwrap_or_else(|_| "created".to_string());
        let direction: String = context
            .evaluate_pin("direction")
            .await
            .unwrap_or_else(|_| "desc".to_string());
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if owner.is_empty() || repo.is_empty() {
            context.log_message("Owner and repository are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let mut url = format!(
            "/repos/{}/{}/pulls?state={}&sort={}&direction={}&per_page={}&page={}",
            owner,
            repo,
            state,
            sort,
            direction,
            per_page.clamp(1, 100),
            page.max(1)
        );

        if !head.is_empty() {
            url.push_str(&format!("&head={}", urlencoding::encode(&head)));
        }

        if !base.is_empty() {
            url.push_str(&format!("&base={}", urlencoding::encode(&base)));
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

                let prs_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let prs: Vec<GitHubPullRequest> = prs_json.iter().filter_map(parse_pr).collect();

                let count = prs.len() as i64;

                context.log_message(&format!("Found {} pull requests", count), LogLevel::Info);
                context.set_pin_value("pull_requests", json!(prs)).await?;
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
