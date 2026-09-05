use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

use super::ConfluenceUser;

/// Search for users in Confluence
#[crate::register_node]
#[derive(Default)]
pub struct SearchUsersNode {}

impl SearchUsersNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchUsersNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_search_users",
            "Search Users",
            "Search for users in Confluence",
            "Data/Atlassian/Confluence",
        );
        node.set_flowscript_name("confluence", "searchUsers");
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "query",
            "Query",
            "Search query for user name or email",
            VariableType::String,
        );

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of users to return (default: 25)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "users",
            "Users",
            "List of matching users",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluenceUser>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of users found",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(
            ATLASSIAN_PROVIDER_ID,
            vec!["read:confluence-user", "read:account"],
        );
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let query: String = context.evaluate_pin("query").await?;
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(25);

        if query.is_empty() {
            return Err(flow_like_types::anyhow!("Search query is required"));
        }

        let client = reqwest::Client::new();

        let users = if provider.is_cloud {
            // Cloud uses user search endpoint
            let url = provider.confluence_rest_api_url("/search/user");
            let escaped_query = query.replace('"', "\\\"");
            let cql = format!(
                "user.fullname~\"{}\" OR user.email~\"{}\"",
                escaped_query, escaped_query
            );
            let limit = limit.to_string();

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .query(&[("cql", cql.as_str()), ("limit", limit.as_str())])
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to search users: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|u| u.get("user"))
                .filter_map(super::parse_confluence_user)
                .collect::<Vec<_>>()
        } else {
            // Server v1 API
            let url = provider.confluence_rest_api_url("/user/list");
            let limit = limit.to_string();

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .query(&[("prefix", query.as_str()), ("limit", limit.as_str())])
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to search users: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(super::parse_confluence_user)
                .collect::<Vec<_>>()
        };

        let count = users.len() as i64;

        context.set_pin_value("users", json!(users)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Get the current user's profile
#[crate::register_node]
#[derive(Default)]
pub struct GetCurrentUserNode {}

impl GetCurrentUserNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetCurrentUserNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_get_current_user",
            "Get Current User",
            "Get the profile of the currently authenticated user",
            "Data/Atlassian/Confluence",
        );
        node.set_flowscript_name("confluence", "getCurrentUser");
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Exec In",
            "Execution input",
            VariableType::Execution,
        );
        node.add_output_pin(
            "exec_out",
            "Exec Out",
            "Execution output",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("user", "User", "Current user profile", VariableType::Struct)
            .set_schema::<ConfluenceUser>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-user"]);
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
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;

        let client = reqwest::Client::new();

        let url = provider.confluence_rest_api_url("/user/current");

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get current user: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let user = super::parse_confluence_user(&data);

        context.set_pin_value("user", json!(user)).await?;

        Ok(())
    }
}
