use super::{
    JiraIssue, build_jira_search_body, jira_search_next_page_token, jira_search_total,
    parse_jira_issue,
};
use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

/// Get all issues for a specific Jira project
#[crate::register_node]
#[derive(Default)]
pub struct GetProjectIssuesNode {}

impl GetProjectIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetProjectIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_project_issues",
            "Get Project Issues",
            "Get all issues for a specific Jira project",
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
        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
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
            "The project key (e.g., PROJ)",
            VariableType::String,
        );

        node.add_input_pin(
            "jql_filter",
            "JQL Filter",
            "Additional JQL filter to apply (optional, will be combined with project filter)",
            VariableType::String,
        );

        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum number of issues to return (default 50, max 100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_input_pin(
            "start_at",
            "Start At",
            "Index to start at for server/Data Center pagination",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "next_page_token",
            "Next Page Token",
            "Cloud pagination token from a previous response",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "issues",
            "Issues",
            "List of issues in the project",
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

        node.add_output_pin(
            "count",
            "Count",
            "Number of issues returned in this response",
            VariableType::Integer,
        );

        node.add_output_pin(
            "next_page_token",
            "Next Page Token",
            "Cloud pagination token for the next page",
            VariableType::String,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let project_key: String = context.evaluate_pin("project_key").await?;
        let jql_filter: String = context.evaluate_pin("jql_filter").await.unwrap_or_default();
        let max_results: i64 = context.evaluate_pin("max_results").await.unwrap_or(50);
        let start_at: i64 = context.evaluate_pin("start_at").await.unwrap_or(0);
        let next_page_token: String = context
            .evaluate_pin("next_page_token")
            .await
            .unwrap_or_default();

        if project_key.is_empty() {
            context.log_message("Project key is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();

        // Build JQL query
        let jql = if jql_filter.is_empty() {
            format!("project = {}", project_key)
        } else {
            format!("project = {} AND ({})", project_key, jql_filter)
        };

        let url = provider.jira_search_api_url();
        let fields = vec![
            "summary".to_string(),
            "description".to_string(),
            "status".to_string(),
            "assignee".to_string(),
            "reporter".to_string(),
            "priority".to_string(),
            "issuetype".to_string(),
            "created".to_string(),
            "updated".to_string(),
            "labels".to_string(),
            "comment".to_string(),
        ];
        let body = build_jira_search_body(
            &provider,
            jql,
            max_results,
            start_at,
            fields,
            next_page_token,
        );
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
            context.log_message(
                &format!("Failed to get project issues: {} - {}", status, error_text),
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let data: Value = response.json().await?;

        let issues: Vec<JiraIssue> = data["issues"]
            .as_array()
            .unwrap_or(&vec![])
            .iter()
            .filter_map(|issue| parse_jira_issue(issue, &provider.base_url))
            .collect();

        let count = issues.len() as i64;
        let total = jira_search_total(&data, count);
        let next_page_token = jira_search_next_page_token(&data);

        context.set_pin_value("issues", json!(issues)).await?;
        context.set_pin_value("total", json!(total)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context
            .set_pin_value("next_page_token", json!(next_page_token))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
