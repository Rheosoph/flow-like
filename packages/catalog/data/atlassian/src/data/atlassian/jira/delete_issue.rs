use crate::data::atlassian::provider::AtlassianProvider;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, reqwest};

/// Delete a Jira issue
#[crate::register_node]
#[derive(Default)]
pub struct DeleteJiraIssueNode {}

impl DeleteJiraIssueNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteJiraIssueNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_delete_issue",
            "Delete Issue",
            "Delete a Jira issue. Use with caution - this action cannot be undone.",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "deleteIssue");
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
            "The issue key to delete (e.g., PROJ-123)",
            VariableType::String,
        );

        node.add_input_pin(
            "delete_subtasks",
            "Delete Subtasks",
            "Also delete subtasks (required if issue has subtasks)",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the deletion was successful",
            VariableType::Boolean,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;
        let delete_subtasks: bool = context
            .evaluate_pin("delete_subtasks")
            .await
            .unwrap_or(false);

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let client = reqwest::Client::new();
        let mut url = provider.jira_api_url(&format!("/issue/{}", issue_key));

        if delete_subtasks {
            url.push_str("?deleteSubtasks=true");
        }

        let response = client
            .delete(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        let success = response.status().is_success();

        if !success {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to delete issue: {} - {}",
                status,
                error_text
            ));
        }

        context
            .set_pin_value("success", flow_like_types::json::json!(success))
            .await?;

        Ok(())
    }
}
