use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, reqwest};

/// Delete a Confluence page
#[crate::register_node]
#[derive(Default)]
pub struct DeleteConfluencePageNode {}

impl DeleteConfluencePageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteConfluencePageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_delete_page",
            "Delete Page",
            "Delete a Confluence page. Use with caution - this action cannot be undone.",
            "Data/Atlassian/Confluence",
        );
        node.set_flowscript_name("confluence", "deletePage");
        node.add_icon("/flow/icons/confluence.svg");
        node.set_version(1);

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
            "page_id",
            "Page ID",
            "The ID of the page to delete",
            VariableType::String,
        );

        node.add_output_pin(
            "success",
            "Success",
            "Whether the deletion was successful",
            VariableType::Boolean,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:confluence-content"]);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }

        let client = reqwest::Client::new();

        let url = if provider.is_cloud {
            provider.confluence_api_url(&format!("/pages/{}", page_id))
        } else {
            provider.confluence_rest_api_url(&format!("/content/{}", page_id))
        };

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
                "Failed to delete page: {} - {}",
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
