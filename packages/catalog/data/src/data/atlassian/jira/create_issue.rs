use super::{JiraIssue, parse_jira_issue};
use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::JsonSchema;
use flow_like_types::json::{Deserialize, Serialize};
use flow_like_types::{Value, async_trait, json::json, reqwest};

#[crate::register_node]
#[derive(Default)]
pub struct CreateJiraIssueNode {}

impl CreateJiraIssueNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateJiraIssueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_create_issue",
            "Create Jira Issue",
            "Create a new Jira issue",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "createIssue");
        node.add_icon("/flow/icons/jira.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the creation",
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
            "project_key",
            "Project Key",
            "The project key (e.g., PROJ)",
            VariableType::String,
        );

        node.add_input_pin(
            "issue_type",
            "Issue Type",
            "The issue type name (e.g., Bug, Story, Task)",
            VariableType::String,
        )
        .set_default_value(Some(json!("Task")));

        node.add_input_pin(
            "summary",
            "Summary",
            "Issue summary/title",
            VariableType::String,
        );

        node.add_input_pin(
            "description",
            "Description",
            "Issue description (plain text)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "priority",
            "Priority",
            "Issue priority name (e.g., Highest, High, Medium, Low, Lowest)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "assignee_id",
            "Assignee ID",
            "Account ID of the assignee (leave empty for unassigned)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "labels",
            "Labels",
            "Labels to assign to the issue",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "parent_key",
            "Parent Key",
            "Parent issue key for subtasks (e.g., PROJ-123)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when issue is created successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "issue",
            "Issue",
            "The created Jira issue",
            VariableType::Struct,
        )
        .set_schema::<JiraIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "issue_key",
            "Issue Key",
            "The key of the created issue (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
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
        let project_key: String = context.evaluate_pin("project_key").await?;
        let issue_type: String = context.evaluate_pin("issue_type").await?;
        let summary: String = context.evaluate_pin("summary").await?;
        let description: String = context.evaluate_pin("description").await?;
        let priority: String = context.evaluate_pin("priority").await?;
        let assignee_id: String = context.evaluate_pin("assignee_id").await?;
        let labels: Vec<String> = context.evaluate_pin("labels").await.unwrap_or_default();
        let parent_key: String = context.evaluate_pin("parent_key").await?;

        if project_key.is_empty() {
            context.log_message("Project key is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        if summary.is_empty() {
            context.log_message("Summary is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/issue");

        // Build the issue fields
        let mut fields = json!({
            "project": {
                "key": project_key
            },
            "summary": summary,
            "issuetype": {
                "name": issue_type
            }
        });

        // Add description (use ADF format for cloud)
        if !description.is_empty() {
            if provider.is_cloud {
                fields["description"] = json!({
                    "type": "doc",
                    "version": 1,
                    "content": [
                        {
                            "type": "paragraph",
                            "content": [
                                {
                                    "type": "text",
                                    "text": description
                                }
                            ]
                        }
                    ]
                });
            } else {
                fields["description"] = json!(description);
            }
        }

        if !priority.is_empty() {
            fields["priority"] = json!({ "name": priority });
        }

        if !assignee_id.is_empty() {
            if provider.is_cloud {
                fields["assignee"] = json!({ "accountId": assignee_id });
            } else {
                fields["assignee"] = json!({ "name": assignee_id });
            }
        }

        if !labels.is_empty() {
            fields["labels"] = json!(labels);
        }

        if !parent_key.is_empty() {
            fields["parent"] = json!({ "key": parent_key });
        }

        let body = json!({ "fields": fields });

        context.log_message(
            &format!("Creating Jira issue in project {}", project_key),
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

        let issue_key = data
            .get("key")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();

        context.log_message(&format!("Created issue: {}", issue_key), LogLevel::Debug);

        // Fetch the full issue details
        let issue = fetch_created_issue(&provider, &issue_key, context).await?;

        context.set_pin_value("issue", json!(issue)).await?;
        context.set_pin_value("issue_key", json!(issue_key)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

async fn fetch_created_issue(
    provider: &AtlassianProvider,
    issue_key: &str,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Option<JiraIssue>> {
    let client = reqwest::Client::new();
    let url = provider.jira_api_url(&format!("/issue/{}", issue_key));

    let response = client
        .get(&url)
        .header("Authorization", provider.auth_header())
        .header("Accept", "application/json")
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            context.log_message(
                &format!("Failed to fetch created issue details: {}", e),
                LogLevel::Warn,
            );
            return Ok(None);
        }
    };

    if !response.status().is_success() {
        return Ok(None);
    }

    let data: Value = match response.json().await {
        Ok(d) => d,
        Err(_) => return Ok(None),
    };

    Ok(parse_jira_issue(&data, &provider.base_url))
}

/// Input structure for batch issue creation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchIssueInput {
    pub project_key: String,
    pub issue_type: String,
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub assignee_id: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub parent_key: Option<String>,
}

/// Result of a batch issue creation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BatchIssueResult {
    pub success: bool,
    pub issue_key: Option<String>,
    pub error: Option<String>,
    pub input_index: usize,
}

/// Batch create Jira issues
#[crate::register_node]
#[derive(Default)]
pub struct BatchCreateIssuesNode {}

impl BatchCreateIssuesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BatchCreateIssuesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_batch_create_issues",
            "Batch Create Issues",
            "Create multiple Jira issues in a batch",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "batchCreateIssues");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the creation",
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
            "issues",
            "Issues",
            "Array of issues to create",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<BatchIssueInput>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when batch completes",
            VariableType::Execution,
        );

        node.add_output_pin(
            "results",
            "Results",
            "Results for each issue creation",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<BatchIssueResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "created_count",
            "Created Count",
            "Number of successfully created issues",
            VariableType::Integer,
        );

        node.add_output_pin(
            "failed_count",
            "Failed Count",
            "Number of failed issue creations",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:jira-work"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(7)
                .set_cost(6)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issues: Vec<BatchIssueInput> = context.evaluate_pin("issues").await?;

        if issues.is_empty() {
            context.set_pin_value("results", json!([])).await?;
            context.set_pin_value("created_count", json!(0)).await?;
            context.set_pin_value("failed_count", json!(0)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url("/issue");
        let mut results: Vec<BatchIssueResult> = Vec::with_capacity(issues.len());
        let mut created_count = 0i64;
        let mut failed_count = 0i64;

        for (index, issue_input) in issues.iter().enumerate() {
            let mut fields = json!({
                "project": { "key": issue_input.project_key },
                "summary": issue_input.summary,
                "issuetype": { "name": issue_input.issue_type }
            });

            if let Some(ref desc) = issue_input.description
                && !desc.is_empty()
            {
                if provider.is_cloud {
                    fields["description"] = json!({
                        "type": "doc",
                        "version": 1,
                        "content": [{
                            "type": "paragraph",
                            "content": [{
                                "type": "text",
                                "text": desc
                            }]
                        }]
                    });
                } else {
                    fields["description"] = json!(desc);
                }
            }

            if let Some(ref priority) = issue_input.priority
                && !priority.is_empty()
            {
                fields["priority"] = json!({ "name": priority });
            }

            if let Some(ref assignee) = issue_input.assignee_id
                && !assignee.is_empty()
            {
                if provider.is_cloud {
                    fields["assignee"] = json!({ "accountId": assignee });
                } else {
                    fields["assignee"] = json!({ "name": assignee });
                }
            }

            if let Some(ref labels) = issue_input.labels
                && !labels.is_empty()
            {
                fields["labels"] = json!(labels);
            }

            if let Some(ref parent) = issue_input.parent_key
                && !parent.is_empty()
            {
                fields["parent"] = json!({ "key": parent });
            }

            let body = json!({ "fields": fields });

            let response = client
                .post(&url)
                .header("Authorization", provider.auth_header())
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    if let Ok(data) = resp.json::<Value>().await {
                        let issue_key = data.get("key").and_then(|v| v.as_str()).map(String::from);
                        results.push(BatchIssueResult {
                            success: true,
                            issue_key,
                            error: None,
                            input_index: index,
                        });
                        created_count += 1;
                    } else {
                        results.push(BatchIssueResult {
                            success: false,
                            issue_key: None,
                            error: Some("Failed to parse response".to_string()),
                            input_index: index,
                        });
                        failed_count += 1;
                    }
                }
                Ok(resp) => {
                    let error_text = resp.text().await.unwrap_or_default();
                    results.push(BatchIssueResult {
                        success: false,
                        issue_key: None,
                        error: Some(error_text),
                        input_index: index,
                    });
                    failed_count += 1;
                }
                Err(e) => {
                    results.push(BatchIssueResult {
                        success: false,
                        issue_key: None,
                        error: Some(e.to_string()),
                        input_index: index,
                    });
                    failed_count += 1;
                }
            }
        }

        context.log_message(
            &format!(
                "Batch create: {} created, {} failed",
                created_count, failed_count
            ),
            LogLevel::Debug,
        );

        context.set_pin_value("results", json!(results)).await?;
        context
            .set_pin_value("created_count", json!(created_count))
            .await?;
        context
            .set_pin_value("failed_count", json!(failed_count))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
