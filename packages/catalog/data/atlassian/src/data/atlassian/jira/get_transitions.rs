use crate::data::atlassian::provider::AtlassianProvider;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

use super::{JiraTransition, parse_jira_transitions};

/// Get available transitions for a Jira issue
#[crate::register_node]
#[derive(Default)]
pub struct GetJiraTransitionsNode {}

impl GetJiraTransitionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetJiraTransitionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_jira_get_transitions",
            "Get Transitions",
            "Get available workflow transitions for a Jira issue",
            "Data/Atlassian/Jira",
        );
        node.set_flowscript_name("jira", "getTransitions");
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

        node.add_output_pin(
            "transitions",
            "Transitions",
            "Available transitions for the issue",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<JiraTransition>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let issue_key: String = context.evaluate_pin("issue_key").await?;

        if issue_key.is_empty() {
            return Err(flow_like_types::anyhow!("Issue key is required"));
        }

        let client = reqwest::Client::new();
        let url = provider.jira_api_url(&format!("/issue/{}/transitions", issue_key));

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get transitions: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;
        let transitions = parse_jira_transitions(&data);

        context
            .set_pin_value("transitions", json!(transitions))
            .await?;

        Ok(())
    }
}
