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

/// Jira Sprint
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct JiraSprint {
    pub id: i64,
    pub name: String,
    pub state: String,
    pub goal: Option<String>,
    pub start_date: Option<String>,
    pub end_date: Option<String>,
    pub complete_date: Option<String>,
    pub origin_board_id: Option<i64>,
}

fn parse_sprint(value: &Value) -> Option<JiraSprint> {
    let obj = value.as_object()?;

    Some(JiraSprint {
        id: obj.get("id")?.as_i64()?,
        name: obj.get("name")?.as_str()?.to_string(),
        state: obj
            .get("state")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string(),
        goal: obj.get("goal").and_then(|g| g.as_str()).map(String::from),
        start_date: obj
            .get("startDate")
            .and_then(|d| d.as_str())
            .map(String::from),
        end_date: obj
            .get("endDate")
            .and_then(|d| d.as_str())
            .map(String::from),
        complete_date: obj
            .get("completeDate")
            .and_then(|d| d.as_str())
            .map(String::from),
        origin_board_id: obj.get("originBoardId").and_then(|b| b.as_i64()),
    })
}

/// Get sprints for a board
#[crate::register_node]
#[derive(Default)]
pub struct GetSprintsNode {}

impl GetSprintsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetSprintsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_sprints",
            "Get Sprints",
            "Get all sprints for a board",
            "Data/Atlassian/Jira/Agile",
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
            "board_id",
            "Board ID",
            "The board ID to get sprints from",
            VariableType::Integer,
        );

        node.add_input_pin(
            "state",
            "State",
            "Filter by sprint state: 'active', 'closed', 'future' (optional, comma-separated for multiple)",
            VariableType::String,
        );

        node.add_output_pin(
            "sprints",
            "Sprints",
            "List of sprints",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraSprint>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of sprints", VariableType::Integer);

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:sprint:jira-software"]);
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
        let board_id: i64 = context.evaluate_pin("board_id").await?;
        let state: String = context.evaluate_pin("state").await.unwrap_or_default();

        let client = reqwest::Client::new();

        let mut url = provider.jira_agile_api_url(&format!("/board/{}/sprint", board_id));
        if !state.is_empty() {
            url.push_str(&format!("?state={}", urlencoding::encode(&state)));
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
                "Failed to get sprints: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let sprints: Vec<JiraSprint> = data["values"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(parse_sprint)
            .collect();

        let count = sprints.len() as i64;

        context.set_pin_value("sprints", json!(sprints)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Get issues in a sprint
#[crate::register_node]
#[derive(Default)]
pub struct GetSprintIssuesNode {}

impl GetSprintIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetSprintIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_sprint_issues",
            "Get Sprint Issues",
            "Get all issues in a sprint",
            "Data/Atlassian/Jira/Agile",
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
            "sprint_id",
            "Sprint ID",
            "The sprint ID to get issues from",
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

        node.add_output_pin(
            "issues",
            "Issues",
            "Issues in the sprint",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total",
            "Total",
            "Total issues in sprint",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:sprint:jira-software"]);
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
        let sprint_id: i64 = context.evaluate_pin("sprint_id").await?;
        let jql: String = context.evaluate_pin("jql").await.unwrap_or_default();
        let max_results: i64 = context.evaluate_pin("max_results").await.unwrap_or(50);

        let client = reqwest::Client::new();

        let mut params = vec![format!("maxResults={}", max_results)];
        if !jql.is_empty() {
            params.push(format!("jql={}", urlencoding::encode(&jql)));
        }

        let url = format!(
            "{}?{}",
            provider.jira_agile_api_url(&format!("/sprint/{}/issue", sprint_id)),
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
                "Failed to get sprint issues: {} - {}",
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

/// Create a new sprint
#[crate::register_node]
#[derive(Default)]
pub struct CreateSprintNode {}

impl CreateSprintNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateSprintNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_create_sprint",
            "Create Sprint",
            "Create a new sprint on a board",
            "Data/Atlassian/Jira/Agile",
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

        node.add_input_pin("name", "Name", "Name of the sprint", VariableType::String);

        node.add_input_pin(
            "board_id",
            "Board ID",
            "The board ID to create the sprint on",
            VariableType::Integer,
        );

        node.add_input_pin(
            "goal",
            "Goal",
            "Sprint goal (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "start_date",
            "Start Date",
            "Sprint start date (ISO 8601, optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "end_date",
            "End Date",
            "Sprint end date (ISO 8601, optional)",
            VariableType::String,
        );

        node.add_output_pin(
            "sprint",
            "Sprint",
            "The created sprint",
            VariableType::Struct,
        )
        .set_schema::<JiraSprint>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:sprint:jira-software"]);
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
        let name: String = context.evaluate_pin("name").await?;
        let board_id: i64 = context.evaluate_pin("board_id").await?;
        let goal: String = context.evaluate_pin("goal").await.unwrap_or_default();
        let start_date: String = context.evaluate_pin("start_date").await.unwrap_or_default();
        let end_date: String = context.evaluate_pin("end_date").await.unwrap_or_default();

        if name.is_empty() {
            return Err(flow_like_types::anyhow!("Sprint name is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_agile_api_url("/sprint");

        let mut body = json!({
            "name": name,
            "originBoardId": board_id
        });

        if !goal.is_empty() {
            body["goal"] = json!(goal);
        }
        if !start_date.is_empty() {
            body["startDate"] = json!(start_date);
        }
        if !end_date.is_empty() {
            body["endDate"] = json!(end_date);
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
                "Failed to create sprint: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let sprint = parse_sprint(&data);

        context.set_pin_value("sprint", json!(sprint)).await?;

        Ok(())
    }
}

/// Update an existing sprint
#[crate::register_node]
#[derive(Default)]
pub struct UpdateSprintNode {}

impl UpdateSprintNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateSprintNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_update_sprint",
            "Update Sprint",
            "Update an existing sprint",
            "Data/Atlassian/Jira/Agile",
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
            "sprint_id",
            "Sprint ID",
            "The sprint ID to update",
            VariableType::Integer,
        );

        node.add_input_pin(
            "name",
            "Name",
            "New name for the sprint (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "goal",
            "Goal",
            "New sprint goal (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "state",
            "State",
            "New state: 'active', 'closed', 'future' (optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "start_date",
            "Start Date",
            "New start date (ISO 8601, optional)",
            VariableType::String,
        );

        node.add_input_pin(
            "end_date",
            "End Date",
            "New end date (ISO 8601, optional)",
            VariableType::String,
        );

        node.add_output_pin(
            "sprint",
            "Sprint",
            "The updated sprint",
            VariableType::Struct,
        )
        .set_schema::<JiraSprint>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:sprint:jira-software"]);
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
        let sprint_id: i64 = context.evaluate_pin("sprint_id").await?;
        let name: String = context.evaluate_pin("name").await.unwrap_or_default();
        let goal: String = context.evaluate_pin("goal").await.unwrap_or_default();
        let state: String = context.evaluate_pin("state").await.unwrap_or_default();
        let start_date: String = context.evaluate_pin("start_date").await.unwrap_or_default();
        let end_date: String = context.evaluate_pin("end_date").await.unwrap_or_default();

        let client = reqwest::Client::new();
        let url = provider.jira_agile_api_url(&format!("/sprint/{}", sprint_id));

        let mut body = json!({});

        if !name.is_empty() {
            body["name"] = json!(name);
        }
        if !goal.is_empty() {
            body["goal"] = json!(goal);
        }
        if !state.is_empty() {
            body["state"] = json!(state);
        }
        if !start_date.is_empty() {
            body["startDate"] = json!(start_date);
        }
        if !end_date.is_empty() {
            body["endDate"] = json!(end_date);
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
                "Failed to update sprint: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let sprint = parse_sprint(&data);

        context.set_pin_value("sprint", json!(sprint)).await?;

        Ok(())
    }
}

/// Move issues to a sprint
#[crate::register_node]
#[derive(Default)]
pub struct MoveToSprintNode {}

impl MoveToSprintNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MoveToSprintNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_move_to_sprint",
            "Move to Sprint",
            "Move issues to a sprint",
            "Data/Atlassian/Jira/Agile",
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
            "sprint_id",
            "Sprint ID",
            "The sprint ID to move issues to",
            VariableType::Integer,
        );

        node.add_input_pin(
            "issue_keys",
            "Issue Keys",
            "Issue keys to move",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_output_pin(
            "success",
            "Success",
            "Whether the move was successful",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:sprint:jira-software"]);
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
        let sprint_id: i64 = context.evaluate_pin("sprint_id").await?;
        let keys: Vec<String> = context.evaluate_pin("issue_keys").await.unwrap_or_default();

        if keys.is_empty() {
            return Err(flow_like_types::anyhow!("Issue keys are required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_agile_api_url(&format!("/sprint/{}/issue", sprint_id));

        let body = json!({
            "issues": keys
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
                "Failed to move issues to sprint: {} - {}",
                status,
                error_text
            ));
        }

        context.set_pin_value("success", json!(true)).await?;

        Ok(())
    }
}
