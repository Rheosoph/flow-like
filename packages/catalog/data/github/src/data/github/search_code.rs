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
pub struct GitHubCodeSearchResult {
    pub name: String,
    pub path: String,
    pub sha: String,
    pub html_url: String,
    pub repository_full_name: String,
    pub repository_url: String,
    pub score: f64,
}

fn parse_code_result(item: &Value) -> Option<GitHubCodeSearchResult> {
    Some(GitHubCodeSearchResult {
        name: item["name"].as_str()?.to_string(),
        path: item["path"].as_str()?.to_string(),
        sha: item["sha"].as_str()?.to_string(),
        html_url: item["html_url"].as_str()?.to_string(),
        repository_full_name: item["repository"]["full_name"].as_str()?.to_string(),
        repository_url: item["repository"]["html_url"].as_str()?.to_string(),
        score: item["score"].as_f64().unwrap_or(0.0),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct SearchGitHubCodeNode {}

impl SearchGitHubCodeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchGitHubCodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_search_code",
            "Search Code",
            "Search for code across GitHub repositories",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "searchCode");
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
            "query",
            "Query",
            "Search query. Use GitHub code search syntax",
            VariableType::String,
        );

        node.add_input_pin(
            "repo",
            "Repository",
            "Limit to a specific repo (owner/repo format)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "language",
            "Language",
            "Filter by programming language",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin("path", "Path", "Filter by file path", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "extension",
            "Extension",
            "Filter by file extension (e.g., rs, ts, py)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin("sort", "Sort", "Sort field", VariableType::String)
            .set_default_value(Some(json!("")))
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["".to_string(), "indexed".to_string()])
                    .build(),
            );

        node.add_input_pin("order", "Order", "Sort order", VariableType::String)
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
            "results",
            "Results",
            "Array of code search results",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubCodeSearchResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total_count",
            "Total Count",
            "Total number of matching results (may be > returned)",
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
        let query: String = context.evaluate_pin("query").await?;
        let repo: String = context.evaluate_pin("repo").await.unwrap_or_default();
        let language: String = context.evaluate_pin("language").await.unwrap_or_default();
        let path: String = context.evaluate_pin("path").await.unwrap_or_default();
        let extension: String = context.evaluate_pin("extension").await.unwrap_or_default();
        let sort: String = context.evaluate_pin("sort").await.unwrap_or_default();
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

        if !repo.is_empty() {
            q.push_str(&format!(" repo:{}", repo));
        }

        if !language.is_empty() {
            q.push_str(&format!(" language:{}", language));
        }

        if !path.is_empty() {
            q.push_str(&format!(" path:{}", path));
        }

        if !extension.is_empty() {
            q.push_str(&format!(" extension:{}", extension));
        }

        let mut url = provider.api_url(&format!(
            "/search/code?q={}&per_page={}&page={}",
            urlencoding::encode(&q),
            per_page.clamp(1, 100),
            page.max(1)
        ));
        if !sort.is_empty() {
            url.push_str(&format!(
                "&sort={}&order={}",
                urlencoding::encode(&sort),
                urlencoding::encode(&order)
            ));
        }

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

                let search_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let total_count = search_json["total_count"].as_i64().unwrap_or(0);
                let items = search_json["items"].as_array();

                let results: Vec<GitHubCodeSearchResult> = items
                    .map(|arr| arr.iter().filter_map(parse_code_result).collect())
                    .unwrap_or_default();

                context.log_message(
                    &format!("Found {} results (showing {})", total_count, results.len()),
                    LogLevel::Info,
                );
                context.set_pin_value("results", json!(results)).await?;
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
