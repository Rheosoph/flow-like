use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

use super::JiraUser;

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
            "data_atlassian_jira_get_current_user",
            "Get Current User",
            "Get the profile of the currently authenticated user",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");

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
            .set_schema::<JiraUser>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:me", "read:jira-user"]);
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
        let url = provider.jira_api_url("/myself");

        context.log_message(&format!("Jira API URL: {}", url), LogLevel::Debug);
        context.log_message(
            &format!("Auth type: {}", provider.auth_type),
            LogLevel::Debug,
        );

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        context.log_message(
            &format!("Response status: {}", response.status()),
            LogLevel::Debug,
        );

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            context.log_message(&format!("Error response: {}", error_text), LogLevel::Error);
            return Err(flow_like_types::anyhow!(
                "Failed to get current user: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let user = super::parse_jira_user(&data);

        context.set_pin_value("user", json!(user)).await?;

        Ok(())
    }
}

/// Changelog history item
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraChangelogItem {
    pub field: String,
    pub field_type: String,
    pub from: Option<String>,
    pub from_string: Option<String>,
    pub to: Option<String>,
    pub to_string: Option<String>,
}

/// Changelog entry
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraChangelog {
    pub id: String,
    pub author: Option<JiraUser>,
    pub created: String,
    pub items: Vec<JiraChangelogItem>,
}

/// Changelog result for one issue in a batch
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraBatchChangelogResult {
    pub issue_key: String,
    pub changelog: Vec<JiraChangelog>,
}

fn parse_changelog_item(value: &Value) -> Option<JiraChangelogItem> {
    let obj = value.as_object()?;

    Some(JiraChangelogItem {
        field: obj.get("field")?.as_str()?.to_string(),
        field_type: obj
            .get("fieldtype")
            .and_then(|f| f.as_str())
            .unwrap_or("")
            .to_string(),
        from: obj.get("from").and_then(|f| f.as_str()).map(String::from),
        from_string: obj
            .get("fromString")
            .and_then(|f| f.as_str())
            .map(String::from),
        to: obj.get("to").and_then(|t| t.as_str()).map(String::from),
        to_string: obj
            .get("toString")
            .and_then(|t| t.as_str())
            .map(String::from),
    })
}

fn parse_changelog(value: &Value) -> Option<JiraChangelog> {
    let obj = value.as_object()?;

    let items: Vec<JiraChangelogItem> = obj
        .get("items")?
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .filter_map(parse_changelog_item)
        .collect();

    Some(JiraChangelog {
        id: obj.get("id")?.as_str()?.to_string(),
        author: obj.get("author").and_then(super::parse_jira_user),
        created: obj.get("created")?.as_str()?.to_string(),
        items,
    })
}

/// Get changelog for an issue
#[crate::register_node]
#[derive(Default)]
pub struct GetChangelogNode {}

impl GetChangelogNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetChangelogNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_changelog",
            "Get Changelog",
            "Get the change history for an issue",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");

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
            "issue_key",
            "Issue Key",
            "The issue key (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum number of changelog entries (default: 100)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "changelog",
            "Changelog",
            "List of changelog entries",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraChangelog>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total",
            "Total",
            "Total number of changelog entries",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;
        let max_results: i64 = context.evaluate_pin("max_results").await.unwrap_or(100);

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!(
            "/issue/{}/changelog?maxResults={}",
            issue_key, max_results
        ));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get changelog: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let changelog: Vec<JiraChangelog> = data["values"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_changelog)
            .collect();

        let total = data["total"].as_i64().unwrap_or(changelog.len() as i64);

        context.set_pin_value("changelog", json!(changelog)).await?;
        context.set_pin_value("total", json!(total)).await?;

        Ok(())
    }
}

/// Get changelogs for multiple issues
#[crate::register_node]
#[derive(Default)]
pub struct BatchGetChangelogsNode {}

impl BatchGetChangelogsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BatchGetChangelogsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_batch_get_changelogs",
            "Batch Get Changelogs",
            "Get changelogs for multiple issues at once",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");
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
            "issue_keys",
            "Issue Keys",
            "Issue keys to fetch changelogs for",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "results",
            "Results",
            "Changelog entries grouped by issue key",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraBatchChangelogResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(4)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let keys: Vec<String> = context.evaluate_pin("issue_keys").await.unwrap_or_default();

        if keys.is_empty() {
            return Err(flow_like_types::anyhow!("Issue keys are required"));
        }

        let client = reqwest::Client::new();
        let mut results = Vec::with_capacity(keys.len());

        for key in keys {
            let url = provider.jira_api_url(&format!("/issue/{}/changelog?maxResults=100", key));

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(data) = resp.json::<Value>().await {
                        let changelog: Vec<JiraChangelog> = data["values"]
                            .as_array()
                            .unwrap_or(&vec![])
                            .iter()
                            .filter_map(parse_changelog)
                            .collect();
                        results.push(JiraBatchChangelogResult {
                            issue_key: key,
                            changelog,
                        });
                    } else {
                        results.push(JiraBatchChangelogResult {
                            issue_key: key,
                            changelog: Vec::new(),
                        });
                    }
                }
                _ => {
                    results.push(JiraBatchChangelogResult {
                        issue_key: key,
                        changelog: Vec::new(),
                    });
                }
            }
        }

        context.set_pin_value("results", json!(results)).await?;

        Ok(())
    }
}
