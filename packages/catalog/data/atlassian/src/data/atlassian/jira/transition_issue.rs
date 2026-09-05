use super::{JiraIssue, JiraTransition, parse_jira_issue, parse_jira_transition};
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
pub struct TransitionJiraIssueNode {}

impl TransitionJiraIssueNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TransitionJiraIssueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_transition_issue",
            "Transition Jira Issue",
            "Change the status of a Jira issue by applying a transition",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "transitionIssue");
        node.add_icon("/flow/icons/jira.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the transition",
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
            "The issue key (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_input_pin(
            "transition_id",
            "Transition ID",
            "The ID of the transition to apply (use 'List Transitions' to get available IDs)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "transition_name",
            "Transition Name",
            "The name of the transition (alternative to ID, e.g., 'Done', 'In Progress')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "comment",
            "Comment",
            "Add a comment while transitioning (optional)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when transition completes successfully",
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
            "The issue after transition",
            VariableType::Struct,
        )
        .set_schema::<JiraIssue>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "available_transitions",
            "Available Transitions",
            "List of available transitions for the issue (populated if transition fails or for reference)",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraTransition>()
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
        let transition_id: String = context.evaluate_pin("transition_id").await?;
        let transition_name: String = context.evaluate_pin("transition_name").await?;
        let comment: String = context.evaluate_pin("comment").await?;

        if issue_key.is_empty() {
            context.log_message("Issue key is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();

        // Get available transitions
        let transitions = get_transitions(&provider, &issue_key, context).await?;
        context
            .set_pin_value("available_transitions", json!(transitions))
            .await?;

        // Determine which transition to use
        let final_transition_id = if !transition_id.is_empty() {
            transition_id
        } else if !transition_name.is_empty() {
            // Find transition by name
            let found = transitions
                .iter()
                .find(|t| t.name.eq_ignore_ascii_case(&transition_name));
            match found {
                Some(t) => t.id.clone(),
                None => {
                    let available_names: Vec<&str> =
                        transitions.iter().map(|t| t.name.as_str()).collect();
                    context.log_message(
                        &format!(
                            "Transition '{}' not found. Available: {:?}",
                            transition_name, available_names
                        ),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            }
        } else {
            context.log_message(
                "Either transition ID or transition name is required",
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        };

        // Apply the transition
        let url = provider.jira_api_url(&format!("/issue/{}/transitions", issue_key));
        let mut body = json!({
            "transition": {
                "id": final_transition_id
            }
        });

        // Add comment if provided
        if !comment.is_empty() {
            if provider.is_cloud {
                body["update"] = json!({
                    "comment": [{
                        "add": {
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
                        }
                    }]
                });
            } else {
                body["update"] = json!({
                    "comment": [{
                        "add": {
                            "body": comment
                        }
                    }]
                });
            }
        }

        context.log_message(
            &format!(
                "Transitioning issue {} with transition ID {}",
                issue_key, final_transition_id
            ),
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

        context.log_message("Transition applied successfully", LogLevel::Debug);

        // Fetch the updated issue
        let issue = fetch_issue(&provider, &issue_key, context).await?;
        context.set_pin_value("issue", json!(issue)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

async fn get_transitions(
    provider: &AtlassianProvider,
    issue_key: &str,
    context: &mut ExecutionContext,
) -> flow_like_types::Result<Vec<JiraTransition>> {
    let client = reqwest::Client::new();
    let url = provider.jira_api_url(&format!("/issue/{}/transitions", issue_key));

    let response = client
        .get(&url)
        .header("Authorization", provider.auth_header())
        .header("Accept", "application/json")
        .send()
        .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            context.log_message(&format!("Failed to get transitions: {}", e), LogLevel::Warn);
            return Ok(Vec::new());
        }
    };

    if !response.status().is_success() {
        return Ok(Vec::new());
    }

    let data: Value = match response.json().await {
        Ok(d) => d,
        Err(_) => return Ok(Vec::new()),
    };

    let transitions = data
        .get("transitions")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_jira_transition).collect())
        .unwrap_or_default();

    Ok(transitions)
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
                &format!("Failed to fetch issue after transition: {}", e),
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
