use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

use super::JiraIssue;

/// Jira Board
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraBoard {
    pub id: i64,
    pub name: String,
    pub board_type: String,
    pub project_key: Option<String>,
    pub project_name: Option<String>,
}

fn parse_board(value: &Value) -> Option<JiraBoard> {
    let obj = value.as_object()?;

    let location = obj.get("location").and_then(|l| l.as_object());

    Some(JiraBoard {
        id: obj.get("id")?.as_i64()?,
        name: obj.get("name")?.as_str()?.to_string(),
        board_type: obj
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .to_string(),
        project_key: location
            .and_then(|l| l.get("projectKey"))
            .and_then(|k| k.as_str())
            .map(String::from),
        project_name: location
            .and_then(|l| l.get("projectName"))
            .and_then(|n| n.as_str())
            .map(String::from),
    })
}

/// Get all agile boards
#[crate::register_node]
#[derive(Default)]
pub struct GetBoardsNode {}

impl GetBoardsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetBoardsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_boards",
            "Get Boards",
            "Get all agile boards (Scrum or Kanban)",
            "Data/Atlassian/Jira/Agile",
        );
        node.set_flowscript_name("jira", "getBoards");
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
            "project_key",
            "Project Key",
            "Filter boards by project key (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "board_type",
            "Board Type",
            "Filter by board type: 'scrum' or 'kanban' (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "name",
            "Name",
            "Filter boards by name (partial match, optional)",
            VariableType::String,
        );

        node.add_output_pin("boards", "Boards", "List of boards", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<JiraBoard>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of boards", VariableType::Integer);

        node.add_required_oauth_scopes(
            ATLASSIAN_PROVIDER_ID,
            vec!["read:board-scope:jira-software"],
        );
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
        let project_key: String = context
            .evaluate_pin("project_key")
            .await
            .unwrap_or_default();
        let board_type: String = context.evaluate_pin("board_type").await.unwrap_or_default();
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();

        let client = reqwest::Client::new();

        let mut params = vec![];
        if !project_key.is_empty() {
            params.push(format!(
                "projectKeyOrId={}",
                urlencoding::encode(&project_key)
            ));
        }
        if !board_type.is_empty() {
            params.push(format!("type={}", board_type.to_lowercase()));
        }
        if !name.is_empty() {
            params.push(format!("name={}", urlencoding::encode(&name)));
        }

        let mut url = provider.jira_agile_api_url("/board");
        if !params.is_empty() {
            url.push_str(&format!("?{}", params.join("&")));
        }

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get boards: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let boards: Vec<JiraBoard> = data["values"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_board)
            .collect();

        let count = boards.len() as i64;

        context.set_pin_value("boards", json!(boards)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Get issues for a specific board
#[crate::register_node]
#[derive(Default)]
pub struct GetBoardIssuesNode {}

impl GetBoardIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetBoardIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_board_issues",
            "Get Board Issues",
            "Get all issues on an agile board",
            "Data/Atlassian/Jira/Agile",
        );
        node.set_flowscript_name("jira", "getBoardIssues");
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
            "board_id",
            "Board ID",
            "The board ID to get issues from",
            VariableType::Integer,
        );

        node.add_input_pin(
            "jql",
            "JQL Filter",
            "Additional JQL filter (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum number of results (default: 50)",
            VariableType::Integer,
        );

        node.add_input_pin(
            "start_at",
            "Start At",
            "Index to start at for pagination (default: 0)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "issues",
            "Issues",
            "List of issues on the board",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total",
            "Total",
            "Total number of issues",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(
            ATLASSIAN_PROVIDER_ID,
            vec!["read:board-scope:jira-software"],
        );
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
        let board_id: i64 = context.evaluate_pin("board_id").await?;
        let jql: String = context.evaluate_pin("jql").await.unwrap_or_default();
        let max_results: i64 = context.evaluate_pin("max_results").await.unwrap_or(50);
        let start_at: i64 = context.evaluate_pin("start_at").await.unwrap_or(0);

        let client = reqwest::Client::new();

        let mut params = vec![
            format!("maxResults={}", max_results),
            format!("startAt={}", start_at),
        ];
        if !jql.is_empty() {
            params.push(format!("jql={}", urlencoding::encode(&jql)));
        }

        let url = format!(
            "{}?{}",
            provider.jira_agile_api_url(&format!("/board/{}/issue", board_id)),
            params.join("&")
        );

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get board issues: {} - {}",
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

        let total = data["total"].as_i64().unwrap_or(issues.len() as i64);

        context.set_pin_value("issues", json!(issues)).await?;
        context.set_pin_value("total", json!(total)).await?;

        Ok(())
    }
}

/// Get the backlog for a board
#[crate::register_node]
#[derive(Default)]
pub struct GetBacklogNode {}

impl GetBacklogNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetBacklogNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_backlog",
            "Get Backlog",
            "Get backlog issues for a board",
            "Data/Atlassian/Jira/Agile",
        );
        node.set_flowscript_name("jira", "getBacklog");
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
            "board_id",
            "Board ID",
            "The board ID to get backlog from",
            VariableType::Integer,
        );

        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum number of results (default: 50)",
            VariableType::Integer,
        );

        node.add_output_pin("issues", "Issues", "Backlog issues", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<JiraIssue>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total",
            "Total",
            "Total backlog items",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(
            ATLASSIAN_PROVIDER_ID,
            vec!["read:board-scope:jira-software"],
        );
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
        let board_id: i64 = context.evaluate_pin("board_id").await?;
        let max_results: i64 = context.evaluate_pin("max_results").await.unwrap_or(50);

        let client = reqwest::Client::new();

        let url = format!(
            "{}?maxResults={}",
            provider.jira_agile_api_url(&format!("/board/{}/backlog", board_id)),
            max_results
        );

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get backlog: {} - {}",
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

        let total = data["total"].as_i64().unwrap_or(issues.len() as i64);

        context.set_pin_value("issues", json!(issues)).await?;
        context.set_pin_value("total", json!(total)).await?;

        Ok(())
    }
}
