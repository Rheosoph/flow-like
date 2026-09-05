use super::{JiraIssue, parse_jira_issue};
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
pub struct UpdateJiraIssueNode {}

impl UpdateJiraIssueNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateJiraIssueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_update_issue",
            "Update Jira Issue",
            "Update an existing Jira issue's fields",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "updateIssue");
        node.add_icon("/flow/icons/jira.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the update",
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
            "The issue key (e.g., PROJ-123) to update",
            VariableType::String,
        );

        node.add_input_pin(
            "summary",
            "Summary",
            "New summary/title (leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "description",
            "Description",
            "New description (leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "priority",
            "Priority",
            "New priority name (leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "assignee_id",
            "Assignee ID",
            "New assignee account ID (leave empty to keep current, use 'unassigned' to remove)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "labels",
            "Labels",
            "New labels (replaces existing labels, leave empty to keep current)",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "comment",
            "Comment",
            "Add a comment while updating (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when update completes successfully",
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
            "The updated Jira issue",
            VariableType::Struct,
        )
        .set_schema::<JiraIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
        let issue_key: String = context.evaluate_pin("issue_key").await?;
        let summary: String = context.evaluate_pin("summary").await?;
        let description: String = context.evaluate_pin("description").await?;
        let priority: String = context.evaluate_pin("priority").await?;
        let assignee_id: String = context.evaluate_pin("assignee_id").await?;
        let labels: Vec<String> = context.evaluate_pin("labels").await.unwrap_or_default();
        let comment: String = context.evaluate_pin("comment").await?;

        if issue_key.is_empty() {
            context.log_message("Issue key is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        // Build update fields
        let mut fields = json!({});
        let mut has_updates = false;

        if !summary.is_empty() {
            fields["summary"] = json!(summary);
            has_updates = true;
        }

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
            has_updates = true;
        }

        if !priority.is_empty() {
            fields["priority"] = json!({ "name": priority });
            has_updates = true;
        }

        if !assignee_id.is_empty() {
            if assignee_id.to_lowercase() == "unassigned" {
                fields["assignee"] = json!(null);
            } else if provider.is_cloud {
                fields["assignee"] = json!({ "accountId": assignee_id });
            } else {
                fields["assignee"] = json!({ "name": assignee_id });
            }
            has_updates = true;
        }

        if !labels.is_empty() {
            fields["labels"] = json!(labels);
            has_updates = true;
        }

        if !has_updates && comment.is_empty() {
            context.log_message("No fields to update", LogLevel::Warn);
            // Still fetch and return the issue
            let issue = fetch_issue(&provider, &issue_key, context).await?;
            context.set_pin_value("issue", json!(issue)).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();

        // Update fields if any
        if has_updates {
            let url = provider.jira_api_url(&format!("/issue/{}", issue_key));
            let body = json!({ "fields": fields });

            context.log_message(
                &format!("Updating Jira issue: {}", issue_key),
                LogLevel::Debug,
            );

            let response = client
                .put(&url)
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
        }

        // Add comment if provided
        if !comment.is_empty() {
            add_comment(&provider, &issue_key, &comment, context).await?;
        }

        // Fetch updated issue
        let issue = fetch_issue(&provider, &issue_key, context).await?;
        context.set_pin_value("issue", json!(issue)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

async fn fetch_issue(
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
                &format!("Failed to fetch updated issue: {}", e),
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

async fn add_comment(
    provider: &AtlassianProvider,
    issue_key: &str,
    comment: &str,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<()> {
    let client = reqwest::Client::new();
    let url = provider.jira_api_url(&format!("/issue/{}/comment", issue_key));

    let body = if provider.is_cloud {
        json!({
            "body": {
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
            }
        })
    } else {
        json!({ "body": comment })
    };

    let response = client
        .post(&url)
        .header("Authorization", provider.auth_header())
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await;

    match response {
        Ok(r) if r.status().is_success() => {
            context.log_message("Comment added successfully", LogLevel::Debug);
        }
        Ok(r) => {
            let error_text = r.text().await.unwrap_or_default();
            context.log_message(
                &format!("Failed to add comment: {}", error_text),
                LogLevel::Warn,
            );
        }
        Err(e) => {
            context.log_message(&format!("Failed to add comment: {}", e), LogLevel::Warn);
        }
    }

    Ok(())
}
