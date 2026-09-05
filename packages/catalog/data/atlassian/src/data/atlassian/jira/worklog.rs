use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

use super::JiraUser;

/// Jira work log entry
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraWorklog {
    pub id: String,
    pub author: Option<JiraUser>,
    pub update_author: Option<JiraUser>,
    pub comment: Option<String>,
    pub started: String,
    pub time_spent: String,
    pub time_spent_seconds: i64,
    pub created: String,
    pub updated: String,
}

fn parse_worklog(value: &Value, is_cloud: bool) -> Option<JiraWorklog> {
    let obj = value.as_object()?;

    Some(JiraWorklog {
        id: obj.get("id")?.as_str()?.to_string(),
        author: obj.get("author").and_then(super::parse_jira_user),
        update_author: obj.get("updateAuthor").and_then(super::parse_jira_user),
        comment: if is_cloud {
            // Cloud uses ADF format
            obj.get("comment")
                .and_then(|c| c.get("content"))
                .and_then(|content| content.as_array())
                .and_then(|arr| arr.first())
                .and_then(|p| p.get("content"))
                .and_then(|content| content.as_array())
                .and_then(|arr| arr.first())
                .and_then(|t| t.get("text"))
                .and_then(|t| t.as_str())
                .map(String::from)
        } else {
            obj.get("comment")
                .and_then(|c| c.as_str())
                .map(String::from)
        },
        started: obj.get("started")?.as_str()?.to_string(),
        time_spent: obj
            .get("timeSpent")
            .and_then(|t| t.as_str())
            .unwrap_or("0m")
            .to_string(),
        time_spent_seconds: obj
            .get("timeSpentSeconds")
            .and_then(|t| t.as_i64())
            .unwrap_or(0),
        created: obj.get("created")?.as_str()?.to_string(),
        updated: obj.get("updated")?.as_str()?.to_string(),
    })
}

/// Get work logs for a Jira issue
#[crate::register_node]
#[derive(Default)]
pub struct GetWorklogNode {}

impl GetWorklogNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetWorklogNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_worklog",
            "Get Worklog",
            "Get work log entries for a Jira issue",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "getWorklog");
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

        node.add_output_pin(
            "worklogs",
            "Worklogs",
            "List of work log entries",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraWorklog>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total_time_spent",
            "Total Time Spent",
            "Total time spent in seconds",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
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
        let issue_key: String = context.evaluate_pin("issue_key").await?;

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}/worklog", issue_key));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get worklog: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;

        let worklogs: Vec<JiraWorklog> = data["worklogs"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|w| parse_worklog(w, provider.is_cloud))
            .collect();

        let total_time: i64 = worklogs.iter().map(|w| w.time_spent_seconds).sum();

        context.set_pin_value("worklogs", json!(worklogs)).await?;
        context
            .set_pin_value("total_time_spent", json!(total_time))
            .await?;

        Ok(())
    }
}

/// Add a work log entry to a Jira issue
#[crate::register_node]
#[derive(Default)]
pub struct AddWorklogNode {}

impl AddWorklogNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddWorklogNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_add_worklog",
            "Add Worklog",
            "Add a work log entry to a Jira issue",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "addWorklog");
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
            "time_spent",
            "Time Spent",
            "Time spent in Jira format (e.g., '2h 30m', '1d', '30m')",
            VariableType::String,
        );

        node.add_input_pin(
            "comment",
            "Comment",
            "Optional comment for the work log",
            VariableType::String,
        );

        node.add_input_pin(
            "started",
            "Started",
            "When the work was started (ISO 8601 format, defaults to now)",
            VariableType::String,
        );

        node.add_output_pin(
            "worklog",
            "Worklog",
            "The created work log entry",
            VariableType::Struct,
        )
        .set_schema::<JiraWorklog>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
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
        let time_spent: String = context.evaluate_pin("time_spent").await?;
        let comment: String = context.evaluate_pin("comment").await.unwrap_or_default();
        let started: String = context.evaluate_pin("started").await.unwrap_or_default();

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        if time_spent.is_empty() {
            return Err(flow_like_types::anyhow!("Time spent is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}/worklog", issue_key));

        let mut body = json!({
            "timeSpent": time_spent
        });

        if !started.is_empty() {
            body["started"] = json!(started);
        }

        if !comment.is_empty() {
            if provider.is_cloud {
                body["comment"] = json!({
                    "type": "doc",
                    "version": 1,
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [
                                {
                                    "type": "text",
                                    "text": comment
                                }
                            ]
                        }
                    ]
                });
            } else {
                body["comment"] = json!(comment);
            }
        }

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
                "Failed to add worklog: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let worklog = parse_worklog(&data, provider.is_cloud);

        context.set_pin_value("worklog", json!(worklog)).await?;

        Ok(())
    }
}
