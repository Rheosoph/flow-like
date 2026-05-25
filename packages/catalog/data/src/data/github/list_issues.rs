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
pub struct GitHubIssueUser {
    pub id: i64,
    pub login: String,
    pub avatar_url: String,
    pub html_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubLabel {
    pub id: i64,
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubMilestone {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub state: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubIssue {
    pub id: i64,
    pub number: i64,
    pub title: String,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub user: GitHubIssueUser,
    pub labels: Vec<GitHubLabel>,
    pub assignees: Vec<GitHubIssueUser>,
    pub milestone: Option<GitHubMilestone>,
    pub comments: i64,
    pub is_pull_request: bool,
    pub created_at: String,
    pub updated_at: String,
    pub closed_at: Option<String>,
}

pub fn parse_issue(issue: &Value) -> Option<GitHubIssue> {
    let user = &issue["user"];
    let is_pr = issue.get("pull_request").is_some();

    Some(GitHubIssue {
        id: issue["id"].as_i64()?,
        number: issue["number"].as_i64()?,
        title: issue["title"].as_str()?.to_string(),
        body: issue["body"].as_str().map(String::from),
        state: issue["state"].as_str()?.to_string(),
        html_url: issue["html_url"].as_str()?.to_string(),
        user: GitHubIssueUser {
            id: user["id"].as_i64()?,
            login: user["login"].as_str()?.to_string(),
            avatar_url: user["avatar_url"].as_str()?.to_string(),
            html_url: user["html_url"].as_str()?.to_string(),
        },
        labels: issue["labels"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| {
                        Some(GitHubLabel {
                            id: l["id"].as_i64()?,
                            name: l["name"].as_str()?.to_string(),
                            color: l["color"].as_str()?.to_string(),
                            description: l["description"].as_str().map(String::from),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        assignees: issue["assignees"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|a| {
                        Some(GitHubIssueUser {
                            id: a["id"].as_i64()?,
                            login: a["login"].as_str()?.to_string(),
                            avatar_url: a["avatar_url"].as_str()?.to_string(),
                            html_url: a["html_url"].as_str()?.to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        milestone: issue["milestone"].as_object().and_then(|_| {
            Some(GitHubMilestone {
                id: issue["milestone"]["id"].as_i64()?,
                number: issue["milestone"]["number"].as_i64()?,
                title: issue["milestone"]["title"].as_str()?.to_string(),
                state: issue["milestone"]["state"].as_str()?.to_string(),
                description: issue["milestone"]["description"].as_str().map(String::from),
            })
        }),
        comments: issue["comments"].as_i64().unwrap_or(0),
        is_pull_request: is_pr,
        created_at: issue["created_at"].as_str()?.to_string(),
        updated_at: issue["updated_at"].as_str()?.to_string(),
        closed_at: issue["closed_at"].as_str().map(String::from),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct ListGitHubIssuesNode {}

impl ListGitHubIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListGitHubIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_list_issues",
            "List Issues",
            "List issues for a repository",
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
            "state",
            "State",
            "Issue state: open, closed, all",
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
            "milestone",
            "Milestone",
            "Milestone number, none, or *",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "labels",
            "Labels",
            "Comma-separated list of label names",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "assignee",
            "Assignee",
            "Filter by assignee username. Use * for any, none for no assignee",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "creator",
            "Creator",
            "Filter by issue creator username",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "mentioned",
            "Mentioned",
            "Filter by mentioned username",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "issue_type",
            "Issue Type",
            "Filter by issue type",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "issue_field_values",
            "Issue Field Values",
            "Filter by issue field values",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin("sort", "Sort", "Sort field", VariableType::String)
            .set_default_value(Some(json!("created")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec![
                        "created".to_string(),
                        "updated".to_string(),
                        "comments".to_string(),
                    ])
                    .build(),
            );

        node.add_input_pin(
            "direction",
            "Direction",
            "Sort direction",
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

        node.add_output_pin("issues", "Issues", "Array of issues", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<GitHubIssue>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of issues returned",
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
        let labels: String = context.evaluate_pin("labels").await.unwrap_or_default();
        let milestone: String = context.evaluate_pin("milestone").await.unwrap_or_default();
        let assignee: String = context.evaluate_pin("assignee").await.unwrap_or_default();
        let creator: String = context.evaluate_pin("creator").await.unwrap_or_default();
        let mentioned: String = context.evaluate_pin("mentioned").await.unwrap_or_default();
        let issue_type: String = context.evaluate_pin("issue_type").await.unwrap_or_default();
        let issue_field_values: String = context
            .evaluate_pin("issue_field_values")
            .await
            .unwrap_or_default();
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
            "/repos/{}/{}/issues?state={}&per_page={}&page={}&sort={}&direction={}",
            owner,
            repo,
            state,
            per_page.clamp(1, 100),
            page.max(1),
            sort,
            direction
        );

        if !milestone.is_empty() {
            url.push_str(&format!("&milestone={}", urlencoding::encode(&milestone)));
        }

        if !labels.is_empty() {
            url.push_str(&format!("&labels={}", urlencoding::encode(&labels)));
        }

        if !assignee.is_empty() {
            url.push_str(&format!("&assignee={}", urlencoding::encode(&assignee)));
        }

        if !creator.is_empty() {
            url.push_str(&format!("&creator={}", urlencoding::encode(&creator)));
        }

        if !mentioned.is_empty() {
            url.push_str(&format!("&mentioned={}", urlencoding::encode(&mentioned)));
        }

        if !issue_type.is_empty() {
            url.push_str(&format!("&type={}", urlencoding::encode(&issue_type)));
        }

        if !issue_field_values.is_empty() {
            url.push_str(&format!(
                "&issue_field_values={}",
                urlencoding::encode(&issue_field_values)
            ));
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

                let issues_json: Vec<Value> = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let issues: Vec<GitHubIssue> = issues_json.iter().filter_map(parse_issue).collect();

                let count = issues.len() as i64;

                context.log_message(&format!("Found {} issues", count), LogLevel::Info);
                context.set_pin_value("issues", json!(issues)).await?;
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
