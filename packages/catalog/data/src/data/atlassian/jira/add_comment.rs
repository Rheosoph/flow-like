use crate::data::atlassian::provider::AtlassianProvider;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

use super::JiraComment;

/// Add a comment to a Jira issue
#[crate::register_node]
#[derive(Default)]
pub struct AddJiraCommentNode {}

impl AddJiraCommentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddJiraCommentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_add_comment",
            "Add Comment",
            "Add a comment to a Jira issue",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "addComment");
        node.add_icon("/flow/icons/jira.svg");

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
            "issue_key",
            "Issue Key",
            "The issue key (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_input_pin(
            "body",
            "Comment Body",
            "The comment text (supports markdown for cloud, wiki markup for server)",
            VariableType::String,
        );

        node.add_output_pin(
            "comment",
            "Comment",
            "The created comment",
            VariableType::Struct,
        )
        .set_schema::<JiraComment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;
        let body: String = context.evaluate_pin("body").await?;

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        if body.is_empty() {
            return Err(flow_like_types::anyhow!("Comment body is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}/comment", issue_key));

        // Build request body based on cloud vs server
        let request_body = if provider.is_cloud {
            // Cloud uses ADF (Atlassian Document Format)
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
                                    "text": body
                                }
                            ]
                        }
                    ]
                }
            })
        } else {
            // Server uses plain text or wiki markup
            json!({
                "body": body
            })
        };

        let response = client
            .post(&url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to add comment: {} - {}",
                status,
                error_text
            ));
        }

        let comment_data: Value = response.json().await?;
        let comment = super::parse_jira_comment(&comment_data);

        context.set_pin_value("comment", json!(comment)).await?;

        Ok(())
    }
}
