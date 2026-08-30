use super::provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitHubUser {
    pub id: i64,
    pub login: String,
    pub name: Option<String>,
    pub email: Option<String>,
    pub avatar_url: String,
    pub html_url: String,
    pub user_type: String,
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub blog: Option<String>,
    pub twitter_username: Option<String>,
    pub public_repos: i64,
    pub public_gists: i64,
    pub followers: i64,
    pub following: i64,
    pub created_at: String,
    pub updated_at: String,
}

fn parse_user(user: &Value) -> Option<GitHubUser> {
    Some(GitHubUser {
        id: user["id"].as_i64()?,
        login: user["login"].as_str()?.to_string(),
        name: user["name"].as_str().map(String::from),
        email: user["email"].as_str().map(String::from),
        avatar_url: user["avatar_url"].as_str()?.to_string(),
        html_url: user["html_url"].as_str()?.to_string(),
        user_type: user["type"].as_str().unwrap_or("User").to_string(),
        bio: user["bio"].as_str().map(String::from),
        company: user["company"].as_str().map(String::from),
        location: user["location"].as_str().map(String::from),
        blog: user["blog"].as_str().map(String::from),
        twitter_username: user["twitter_username"].as_str().map(String::from),
        public_repos: user["public_repos"].as_i64().unwrap_or(0),
        public_gists: user["public_gists"].as_i64().unwrap_or(0),
        followers: user["followers"].as_i64().unwrap_or(0),
        following: user["following"].as_i64().unwrap_or(0),
        created_at: user["created_at"].as_str()?.to_string(),
        updated_at: user["updated_at"].as_str()?.to_string(),
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct GetGitHubUserNode {}

impl GetGitHubUserNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGitHubUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_get_user",
            "Get User",
            "Get information about a GitHub user, or the authenticated user if no username provided",
            "Data/GitHub",
        );
        node.set_flowscript_name("github", "getUser");
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
            "username",
            "Username",
            "GitHub username. Leave empty to get authenticated user",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

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

        node.add_output_pin("user", "User", "User details", VariableType::Struct)
            .set_schema::<GitHubUser>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["read:user"]);
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
        let username: String = context.evaluate_pin("username").await.unwrap_or_default();

        let url = if username.is_empty() {
            provider.api_url("/user")
        } else {
            provider.api_url(&format!("/users/{}", username))
        };

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

                let user_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(user) = parse_user(&user_json) {
                    context.log_message(&format!("Retrieved user: {}", user.login), LogLevel::Info);
                    context.set_pin_value("user", json!(user)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse user data", LogLevel::Error);
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
