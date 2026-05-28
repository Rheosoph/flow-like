use super::{
    JiraIssue, build_jira_search_body, jira_search_has_more, jira_search_next_page_token,
    jira_search_total, parse_jira_issue,
};
use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

#[crate::register_node]
#[derive(Default)]
pub struct SearchJiraIssuesNode {}

impl SearchJiraIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SearchJiraIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_search_issues",
            "Search Jira Issues",
            "Search for Jira issues using JQL (Jira Query Language)",
            "Data/Atlassian/Jira",
        );
        node.add_icon("/flow/icons/jira.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the search",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Atlassian provider (from Atlassian node)",
            VariableType::Struct,
        )
        .set_schema::<AtlassianProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "jql",
            "JQL Query",
            "JQL query string (e.g., 'project = PROJ AND status = \"In Progress\"')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "max_results",
            "Max Results",
            "Maximum number of results to return (1-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_input_pin(
            "start_at",
            "Start At",
            "Index of the first result to return (server/Data Center pagination)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "fields",
            "Fields",
            "Fields to return (leave empty for default fields)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "next_page_token",
            "Next Page Token",
            "Cloud pagination token from a previous search response",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when search completes successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "issues",
            "Issues",
            "Array of Jira issues matching the query",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "total",
            "Total",
            "Total number of matching issues",
            VariableType::Integer,
        );

        node.add_output_pin(
            "has_more",
            "Has More",
            "Whether there are more results available",
            VariableType::Boolean,
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
                .set_performance(7)
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

        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let jql: String = context.evaluate_pin("jql").await?;
        let max_results: i64 = context.evaluate_pin("max_results").await?;
        let start_at: i64 = context.evaluate_pin("start_at").await?;
        let fields: Vec<String> = context.evaluate_pin("fields").await.unwrap_or_default();
        let next_page_token: String = context
            .evaluate_pin("next_page_token")
            .await
            .unwrap_or_default();

        let client = reqwest::Client::new();
        let url = provider.jira_search_api_url();
        let body = build_jira_search_body(
            &provider,
            jql.clone(),
            max_results,
            start_at,
            fields,
            next_page_token,
        );

        context.log_message(
            &format!("Searching Jira with JQL: {}", jql),
            LogLevel::Debug,
        );

        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .send()
            .await;

        let response = match response {
            Ok(r) => r,
            Err(e) => {
                context.log_message(&format!("Request failed: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            context.log_message(
                &format!("Jira API error {}: {}", status, error_text),
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let data: Value = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                context.log_message(&format!("Failed to parse response: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        let issues_data = data.get("issues").and_then(|v| v.as_array());

        let issues: Vec<JiraIssue> = issues_data
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| parse_jira_issue(v, &provider.base_url))
                    .collect()
            })
            .unwrap_or_default();

        let returned_count = issues.len() as i64;
        let total = jira_search_total(&data, returned_count);
        let has_more = jira_search_has_more(&data, start_at, returned_count, total);
        let next_page_token = jira_search_next_page_token(&data);

        context.log_message(
            &format!("Found {} issues (total: {})", returned_count, total),
            LogLevel::Debug,
        );

        context.set_pin_value("issues", json!(issues)).await?;
        context.set_pin_value("total", json!(total)).await?;
        context.set_pin_value("has_more", json!(has_more)).await?;
        context
            .set_pin_value("next_page_token", json!(next_page_token))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
