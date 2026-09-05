use super::{JiraProject, parse_jira_project};
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
pub struct ListJiraProjectsNode {}

impl ListJiraProjectsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListJiraProjectsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_list_projects",
            "List Jira Projects",
            "List all accessible Jira projects",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "listProjects");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the request",
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
            "max_results",
            "Max Results",
            "Maximum number of projects to return (1-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(50)));

        node.add_input_pin(
            "start_at",
            "Start At",
            "Index of the first result to return (for pagination)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "query",
            "Query",
            "Filter projects by name or key (partial match)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when request completes successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "projects",
            "Projects",
            "Array of Jira projects",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraProject>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of projects returned",
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
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let max_results: i64 = context.evaluate_pin("max_results").await?;
        let start_at: i64 = context.evaluate_pin("start_at").await?;
        let query: String = context.evaluate_pin("query").await?;

        let client = reqwest::Client::new();

        // Build URL with query parameters
        let mut url = provider.jira_api_url("/project/search");
        let mut params = vec![
            format!("maxResults={}", max_results.clamp(1, 100)),
            format!("startAt={}", start_at.max(0)),
        ];

        if !query.is_empty() {
            params.push(format!("query={}", urlencoding::encode(&query)));
        }

        if !params.is_empty() {
            url = format!("{}?{}", url, params.join("&"));
        }

        context.log_message("Fetching Jira projects", LogLevel::Debug);

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/json")
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

        // Handle both API versions - cloud uses "values", server uses array directly
        let projects_data = data
            .get("values")
            .and_then(|v| v.as_array())
            .or_else(|| data.as_array());

        let projects: Vec<JiraProject> = projects_data
            .map(|arr| arr.iter().filter_map(parse_jira_project).collect())
            .unwrap_or_default();

        let count = projects.len() as i64;

        context.log_message(&format!("Found {} projects", count), LogLevel::Debug);

        context.set_pin_value("projects", json!(projects)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
