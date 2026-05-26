use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

use super::ConfluencePage;

/// Get child pages of a page
#[crate::register_node]
#[derive(Default)]
pub struct GetPageChildrenNode {}

impl GetPageChildrenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetPageChildrenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_get_page_children",
            "Get Page Children",
            "Get all child pages of a Confluence page",
            "Data/Atlassian/Confluence",
        );
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
            "The ID of the parent page",
            VariableType::String,
        );

        node.add_input_pin(
            "expand",
            "Expand",
            "Properties to expand (comma-separated, e.g., 'body.storage,version')",
            VariableType::String,
        );

        node.add_input_pin(
            "limit",
            "Limit",
            "Maximum number of children to return (default: 25)",
            VariableType::Integer,
        );

        node.add_output_pin(
            "children",
            "Children",
            "List of child pages",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluencePage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of children",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-content.all"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;
        let expand: String = context.evaluate_pin("expand").await.unwrap_or_default();
        let limit: i64 = context.evaluate_pin("limit").await.unwrap_or(25);

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }

        let client = reqwest::Client::new();

        let children = if provider.is_cloud {
            // Cloud v2 API
            let url = provider
                .confluence_api_url(&format!("/pages/{}/children?limit={}", page_id, limit));

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to get page children: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|p| super::parse_confluence_page(p, &provider.base_url))
                .collect::<Vec<_>>()
        } else {
            // Server v1 API
            let mut url = provider.confluence_rest_api_url(&format!(
                "/content/{}/child/page?limit={}",
                page_id, limit
            ));
            if !expand.is_empty() {
                url.push_str(&format!("&expand={}", urlencoding::encode(&expand)));
            }

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to get page children: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|p| super::parse_confluence_page(p, &provider.base_url))
                .collect::<Vec<_>>()
        };

        let count = children.len() as i64;

        context.set_pin_value("children", json!(children)).await?;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }
}

/// Get ancestor pages (parent hierarchy)
#[crate::register_node]
#[derive(Default)]
pub struct GetPageAncestorsNode {}

impl GetPageAncestorsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetPageAncestorsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_get_page_ancestors",
            "Get Page Ancestors",
            "Get the ancestor pages (parent hierarchy) of a page",
            "Data/Atlassian/Confluence",
        );
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
            "The ID of the page to get ancestors for",
            VariableType::String,
        );

        node.add_output_pin(
            "ancestors",
            "Ancestors",
            "List of ancestor pages (from root to immediate parent)",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluencePage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "depth",
            "Depth",
            "Depth in page hierarchy",
            VariableType::Integer,
        );

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-content.all"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(7)
                .set_security(8)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }

        let client = reqwest::Client::new();

        let ancestors = if provider.is_cloud {
            // Cloud v2 API
            let url = provider.confluence_api_url(&format!("/pages/{}/ancestors", page_id));

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to get page ancestors: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|p| super::parse_confluence_page(p, &provider.base_url))
                .collect::<Vec<_>>()
        } else {
            // Server v1 API - get ancestors via content expand
            let url =
                provider.confluence_rest_api_url(&format!("/content/{}?expand=ancestors", page_id));

            let response = client
                .get(&url)
                .header("Authorization", provider.auth_header())
                .send()
                .await?;

            if !response.status().is_success() {
                let status = response.status();
                let error_text = response.text().await.unwrap_or_default();
                return Err(flow_like_types::anyhow!(
                    "Failed to get page ancestors: {} - {}",
                    status,
                    error_text
                ));
            }

            let data: Value = response.json().await?;
            data["ancestors"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|p| super::parse_confluence_page(p, &provider.base_url))
                .collect::<Vec<_>>()
        };

        let depth = ancestors.len() as i64;

        context.set_pin_value("ancestors", json!(ancestors)).await?;
        context.set_pin_value("depth", json!(depth)).await?;

        Ok(())
    }
}
