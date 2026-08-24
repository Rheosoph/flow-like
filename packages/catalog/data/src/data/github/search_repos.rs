use super::{
    list_repos::GitHubRepository,
    provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
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
pub struct SearchGitHubReposNode {}

impl SearchGitHubReposNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchGitHubReposNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_search_repos",
            "Search Repositories",
            "Search for repositories on GitHub",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "searchRepos");
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
            "Search query. Use GitHub search syntax",
            VariableType::String,
        );

        node.add_input_pin(
            "language",
            "Language",
            "Filter by programming language",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "user",
            "User/Org",
            "Filter by user or organization",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin("topic", "Topic", "Filter by topic", VariableType::String)
            .set_default_value(Some(json!("")));

        node.add_input_pin(
            "sort",
            "Sort",
            "Sort by: stars, forks, help-wanted-issues, updated",
            VariableType::String,
        )
        .set_default_value(Some(json!("best-match")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "best-match".to_string(),
                    "stars".to_string(),
                    "forks".to_string(),
                    "help-wanted-issues".to_string(),
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
            "repos",
            "Repositories",
            "Array of repositories",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<GitHubRepository>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total_count",
            "Total Count",
            "Total number of matching repositories",
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
        let language: String = context.evaluate_pin("language").await.unwrap_or_default();
        let user: String = context.evaluate_pin("user").await.unwrap_or_default();
        let topic: String = context.evaluate_pin("topic").await.unwrap_or_default();
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

        if !language.is_empty() {
            q.push_str(&format!(" language:{}", language));
        }

        if !user.is_empty() {
            q.push_str(&format!(" user:{}", user));
        }

        if !topic.is_empty() {
            q.push_str(&format!(" topic:{}", topic));
        }

        let mut url = format!(
            "/search/repositories?q={}&per_page={}&page={}",
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

                let repos: Vec<GitHubRepository> = items
                    .map(|arr| arr.iter().filter_map(parse_repo).collect())
                    .unwrap_or_default();

                context.log_message(
                    &format!(
                        "Found {} repositories (showing {})",
                        total_count,
                        repos.len()
                    ),
                    LogLevel::Info,
                );
                context.set_pin_value("repos", json!(repos)).await?;
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
