use super::{JiraComment, JiraIssue, parse_jira_comment, parse_jira_issue};
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
pub struct GetJiraIssueNode {}

impl GetJiraIssueNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetJiraIssueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_issue",
            "Get Jira Issue",
            "Get a single Jira issue by its key (e.g., PROJ-123)",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "getIssue");
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
            "issue_key",
            "Issue Key",
            "The issue key (e.g., PROJ-123) or ID",
            VariableType::String,
        );

        node.add_input_pin(
            "include_comments",
            "Include Comments",
            "Whether to fetch comments for the issue",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

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

        node.add_output_pin("issue", "Issue", "The Jira issue", VariableType::Struct)
            .set_schema::<JiraIssue>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "comments",
            "Comments",
            "Comments on the issue (if requested)",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraComment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:jira-work"]);
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
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;
        let include_comments: bool = context.evaluate_pin("include_comments").await?;

        if issue_key.is_empty() {
            context.log_message("Issue key is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}", issue_key));

        context.log_message(
            &format!("Fetching Jira issue: {}", issue_key),
            LogLevel::Debug,
        );

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

        let issue = match parse_jira_issue(&data, &provider.base_url) {
            Some(i) => i,
            None => {
                context.log_message("Failed to parse issue from response", LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        context.set_pin_value("issue", json!(issue)).await?;

        // Fetch comments if requested
        let comments: Vec<JiraComment> = if include_comments {
            fetch_comments(&provider, &issue_key, context).await?
        } else {
            Vec::new()
        };

        context.set_pin_value("comments", json!(comments)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

async fn fetch_comments(
    provider: &AtlassianProvider,
    issue_key: &str,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Vec<JiraComment>> {
    let client = reqwest::Client::new();
    let url = provider.jira_api_url(&format!("/issue/{}/comment", issue_key));

    let response = client
        .get(&url)
        .header("Authorization", provider.auth_header())
        .header("Accept", "application/json")
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            context.log_message(&format!("Failed to fetch comments: {}", e), LogLevel::Warn);
            return Ok(Vec::new());
        }
    };

    if !response.status().is_success() {
        context.log_message("Failed to fetch comments", LogLevel::Warn);
        return Ok(Vec::new());
    }

    let data: Value = match response.json().await {
        Ok(d) => d,
        Err(_) => return Ok(Vec::new()),
    };

    let comments = data
        .get("comments")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_jira_comment).collect())
        .unwrap_or_default();

    Ok(comments)
}
