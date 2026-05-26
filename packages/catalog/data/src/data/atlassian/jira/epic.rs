use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

use super::{JiraIssue, build_jira_search_body};

/// Link an issue to an epic
#[crate::register_node]
#[derive(Default)]
pub struct LinkToEpicNode {}

impl LinkToEpicNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LinkToEpicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_link_to_epic",
            "Link to Epic",
            "Link an issue to an epic (adds issue to epic's child issues)",
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
            "issue_key",
            "Issue Key",
            "The issue key to link to the epic (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_input_pin(
            "epic_key",
            "Epic Key",
            "The epic key to link the issue to (e.g., PROJ-100)",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the linking was successful",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(
            ATLASSIAN_PROVIDER_ID,
            vec!["write:jira-work", "write:epic:jira-software"],
        );
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;
        let epic_key: String = context.evaluate_pin("epic_key").await?;

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        if epic_key.is_empty() {
            return Err(flow_like_types::anyhow!("Epic key is required"));
        }

        let client = reqwest::Client::new();

        // For cloud, use the parent field; for server, use epic link custom field
        if provider.is_cloud {
            // Cloud: Update the parent field
            let url = provider.jira_api_url(&format!("/issue/{}", issue_key));
            let body = json!({
                "fields": {
                    "parent": {
                        "key": epic_key
                    }
                }
            });

            let response = client
                .put(&url)
                .header("Authorization", provider.auth_header())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to link issue to epic: {} - {}",
                    status,
                    error_text
                ));
            }
        } else {
            // Server/DC: Use agile endpoint
            let url = provider.jira_agile_api_url(&format!("/epic/{}/issue", epic_key));
            let body = json!({
                "issues": [issue_key]
            });

            let response = client
                .post(&url)
                .header("Authorization", provider.auth_header())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to link issue to epic: {} - {}",
                    status,
                    error_text
                ));
            }
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}

/// Remove an issue from an epic
#[crate::register_node]
#[derive(Default)]
pub struct UnlinkFromEpicNode {}

impl UnlinkFromEpicNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UnlinkFromEpicNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_unlink_from_epic",
            "Unlink from Epic",
            "Remove an issue from its epic",
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
            "issue_key",
            "Issue Key",
            "The issue key to unlink from its epic (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the unlinking was successful",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(
            ATLASSIAN_PROVIDER_ID,
            vec!["write:jira-work", "write:epic:jira-software"],
        );
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let client = reqwest::Client::new();

        if provider.is_cloud {
            // Cloud: Clear the parent field
            let url = provider.jira_api_url(&format!("/issue/{}", issue_key));
            let body = json!({
                "fields": {
                    "parent": null
                }
            });

            let response = client
                .put(&url)
                .header("Authorization", provider.auth_header())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to unlink issue from epic: {} - {}",
                    status,
                    error_text
                ));
            }
        } else {
            // Server/DC: Use agile endpoint
            let url = provider.jira_agile_api_url("/epic/none/issue");
            let body = json!({
                "issues": [issue_key]
            });

            let response = client
                .post(&url)
                .header("Authorization", provider.auth_header())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to unlink issue from epic: {} - {}",
                    status,
                    error_text
                ));
            }
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}

/// Get issues linked to an epic
#[crate::register_node]
#[derive(Default)]
pub struct GetEpicIssuesNode {}

impl GetEpicIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetEpicIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_epic_issues",
            "Get Epic Issues",
            "Get all issues linked to an epic",
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
            "epic_key",
            "Epic Key",
            "The epic key (e.g., PROJ-100)",
            VariableType::String,
        );

        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum number of issues to return (default: 50)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "issues",
            "Issues",
            "Issues linked to the epic",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of issues found",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:epic:jira-software"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
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
        let epic_key: String = context.evaluate_pin("epic_key").await?;
        let max_results: i64 = context.evaluate_pin("max_results").await.unwrap_or(50);

        if epic_key.is_empty() {
            return Err(flow_like_types::anyhow!("Epic key is required"));
        }

        let client = reqwest::Client::new();

        // Use JQL search to find issues with this epic as parent
        let jql = if provider.is_cloud {
            format!("parent = {}", epic_key)
        } else {
            format!("\"Epic Link\" = {}", epic_key)
        };

        let url = provider.jira_search_api_url();
        let body =
            build_jira_search_body(&provider, jql, max_results, 0, Vec::new(), String::new());

        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get epic issues: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let issues: Vec<JiraIssue> = data["issues"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|i| super::parse_jira_issue(i, &provider.base_url))
            .collect();

        let count = issues.len() as i64;

        context.set_pin_value("issues", json!(issues)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}
