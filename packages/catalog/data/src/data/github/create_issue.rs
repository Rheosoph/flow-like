use super::{
    list_issues::{GitHubIssue, parse_issue},
    provider::{GITHUB_API_VERSION, GITHUB_PROVIDER_ID, GitHubProvider},
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

fn string_list_from_value(value: Value) -> Vec<String> {
    if let Some(values) = value.as_array() {
        return values
            .iter()
            .filter_map(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(String::from)
            .collect();
    }

    value
        .as_str()
        .unwrap_or_default()
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .collect()
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateGitHubIssueNode {}

impl CreateGitHubIssueNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGitHubIssueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_github_create_issue",
            "Create Issue",
            "Create a new issue in a repository",
            "Data/GitHub",
        );
        node.add_icon("/flow/icons/github.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "GitHub provider",
            VariableType::Struct,
        )
        .set_schema::<GitHubProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("owner", "Owner", "Repository owner", VariableType::String);
        node.add_input_pin(
            "repo",
            "Repository",
            "Repository name",
            VariableType::String,
        );
        node.add_input_pin("title", "Title", "Issue title", VariableType::String);

        node.add_input_pin(
            "body",
            "Body",
            "Issue body (Markdown supported)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "labels",
            "Labels",
            "Label names to apply",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "assignees",
            "Assignees",
            "Usernames to assign",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_input_pin(
            "milestone",
            "Milestone",
            "Milestone number to associate with the issue",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "issue_type",
            "Issue Type",
            "Issue type name or ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "issue_field_values",
            "Issue Field Values",
            "Issue form field values accepted by the GitHub API",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );

        node.add_output_pin("issue", "Issue", "Created issue", VariableType::Struct)
            .set_schema::<GitHubIssue>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "issue_number",
            "Issue Number",
            "The number of the created issue",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(GITHUB_PROVIDER_ID, vec!["repo"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(8)
                .set_governance(6)
                .set_reliability(9)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GitHubProvider = context.evaluate_pin("provider").await?;
        let owner: String = context.evaluate_pin("owner").await?;
        let repo: String = context.evaluate_pin("repo").await?;
        let title: String = context.evaluate_pin("title").await?;
        let body: String = context.evaluate_pin("body").await.unwrap_or_default();
        let milestone: i64 = context.evaluate_pin("milestone").await.unwrap_or(0);
        let issue_type: String = context.evaluate_pin("issue_type").await.unwrap_or_default();

        if owner.is_empty() || repo.is_empty() || title.is_empty() {
            context.log_message("Owner, repository, and title are required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/repos/{}/{}/issues", owner, repo));

        let mut request_body = json!({
            "title": title
        });

        if !body.is_empty() {
            request_body["body"] = json!(body);
        }

        if let Ok(labels_value) = context.evaluate_pin::<Value>("labels").await {
            let labels = string_list_from_value(labels_value);
            if !labels.is_empty() {
                request_body["labels"] = json!(labels);
            }
        }

        if let Ok(assignees_value) = context.evaluate_pin::<Value>("assignees").await {
            let assignees = string_list_from_value(assignees_value);
            if !assignees.is_empty() {
                request_body["assignees"] = json!(assignees);
            }
        }

        if milestone > 0 {
            request_body["milestone"] = json!(milestone);
        }

        if !issue_type.is_empty() {
            request_body["type"] = json!(issue_type);
        }

        if let Ok(issue_field_values) = context.evaluate_pin::<Value>("issue_field_values").await {
            if issue_field_values
                .as_array()
                .map(|values| !values.is_empty())
                .unwrap_or(false)
            {
                request_body["issue_field_values"] = issue_field_values;
            }
        }

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header("User-Agent", "flow-like")
            .json(&request_body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    let status = resp.status();
                    let error_text = resp.text().await.unwrap_or_default();
                    context.log_message(
                        &format!("GitHub API error {}: {}", status, error_text),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let issue_json: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                if let Some(issue) = parse_issue(&issue_json) {
                    context.log_message(
                        &format!("Created issue #{}: {}", issue.number, issue.title),
                        LogLevel::Info,
                    );
                    context
                        .set_pin_value("issue_number", json!(issue.number))
                        .await?;
                    context.set_pin_value("issue", json!(issue)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context.log_message("Failed to parse created issue", LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
