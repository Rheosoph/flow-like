use super::{
    list_issues::{GitHubIssue, parse_issue},
    provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

#[crate::register_node]
#[derive(Default)]
pub struct SearchGitHubIssuesNode {}

impl SearchGitHubIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchGitHubIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_search_issues",
            "Search Issues",
            "Search for issues across GitHub repositories",
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

        node.add_input_pin(
            "owner",
            "Owner",
            "Repository owner (optional)",
            VariableType::String,
        )
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
            "repo",
            "Repository",
            "Repository name (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "query",
            "Query",
            "Search query. Use GitHub search syntax",
            VariableType::String,
        );

        node.add_input_pin(
            "state",
            "State",
            "Filter by state: open, closed",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "type",
            "Type",
            "Filter by type: issue, pr",
            VariableType::String,
        )
        .set_default_value(Some(json!("issue")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["".to_string(), "issue".to_string(), "pr".to_string()])
                .build(),
        );

        node.add_input_pin(
            "author",
            "Author",
            "Filter by author username",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "assignee",
            "Assignee",
            "Filter by assignee username",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "labels",
            "Labels",
            "Filter by labels (comma-separated)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "sort",
            "Sort",
            "Sort by: comments, reactions, reactions-+1, interactions, created, updated",
            VariableType::String,
        )
        .set_default_value(Some(json!("best-match")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "best-match".to_string(),
                    "comments".to_string(),
                    "reactions".to_string(),
                    "reactions-+1".to_string(),
                    "reactions--1".to_string(),
                    "reactions-smile".to_string(),
                    "reactions-thinking_face".to_string(),
                    "reactions-heart".to_string(),
                    "reactions-tada".to_string(),
                    "interactions".to_string(),
                    "created".to_string(),
                    "updated".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "order",
            "Order",
            "Sort order: asc, desc",
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
            "issues",
            "Issues",
            "Array of matching issues",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total_count",
            "Total Count",
            "Total number of matching issues",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(7)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await.unwrap_or_default();
        let repo: String = context.evaluate_pin("repo").await.unwrap_or_default();
        let query: String = context.evaluate_pin("query").await?;
        let state: String = context.evaluate_pin("state").await.unwrap_or_default();
        let issue_type: String = context
            .evaluate_pin("type")
            .await
            .unwrap_or_else(|_| "issue".to_string());
        let author: String = context.evaluate_pin("author").await.unwrap_or_default();
        let assignee: String = context.evaluate_pin("assignee").await.unwrap_or_default();
        let labels: String = context.evaluate_pin("labels").await.unwrap_or_default();
        let sort: String = context
            .evaluate_pin("sort")
            .await
            .unwrap_or_else(|_| "best-match".to_string());
        let order: String = context
            .evaluate_pin("order")
            .await
            .unwrap_or_else(|_| "desc".to_string());
        let per_page: i64 = context.evaluate_pin("per_page").await.unwrap_or(30);
        let page: i64 = context.evaluate_pin("page").await.unwrap_or(1);

        if query.is_empty() {
            context.log_message("Search query is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let mut q = query.clone();

        if !owner.is_empty() && !repo.is_empty() {
            q.push_str(&format!(" repo:{}/{}", owner, repo));
        } else if !owner.is_empty() {
            q.push_str(&format!(" user:{}", owner));
        }

        if !state.is_empty() {
            q.push_str(&format!(" state:{}", state));
        }

        if !issue_type.is_empty() {
            q.push_str(&format!(" type:{}", issue_type));
        }

        if !author.is_empty() {
            q.push_str(&format!(" author:{}", author));
        }

        if !assignee.is_empty() {
            q.push_str(&format!(" assignee:{}", assignee));
        }

        if !labels.is_empty() {
            for label in labels.split(',') {
                let label = label.trim();
                if !label.is_empty() {
                    q.push_str(&format!(" label:{}", label));
                }
            }
        }

        let mut url = format!(
            "/search/issues?q={}&per_page={}&page={}",
            urlencoding::encode(&q),
            per_page.clamp(1, 100),
            page.max(1)
        );

        if sort != "best-match" {
            url.push_str(&format!("&sort={}&order={}", sort, order));
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

                let search_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let total_count = search_json["total_count"].as_i64().unwrap_or(0);
                let items = search_json["items"].as_array();

                let issues: Vec<GitHubIssue> = items
                    .map(|arr| arr.iter().filter_map(parse_issue).collect())
                    .unwrap_or_default();

                context.log_message(
                    &format!("Found {} issues (showing {})", total_count, issues.len()),
                    LogLevel::Info,
                );
                context.set_pin_value("issues", json!(issues)).await?;
                context
                    .set_pin_value("total_count", json!(total_count))
                    .await?;
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
