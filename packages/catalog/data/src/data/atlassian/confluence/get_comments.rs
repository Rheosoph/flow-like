use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

use super::{ConfluenceUser, parse_confluence_user};

/// Confluence comment
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ConfluenceComment {
    pub id: String,
    pub body: String,
    pub body_html: Option<String>,
    pub created_at: Option<String>,
    pub author: Option<ConfluenceUser>,
}

/// Get comments from a Confluence page
#[crate::register_node]
#[derive(Default)]
pub struct GetConfluenceCommentsNode {}

impl GetConfluenceCommentsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetConfluenceCommentsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_get_comments",
            "Get Comments",
            "Get comments from a Confluence page",
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
            "The ID of the page to get comments from",
            VariableType::String,
        );

        node.add_output_pin(
            "comments",
            "Comments",
            "List of comments on the page",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<ConfluenceComment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["read:confluence-content.all"]);

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
            provider.confluence_api_url(&format!("/pages/{}/footer-comments", page_id))
        } else {
            provider.confluence_rest_api_url(&format!(
                "/content/{}/child/comment?expand=body.storage,history",
                page_id
            ))
        };

        let response = client
            .get(&url)
            .header("Authorization", provider.auth_header())
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Failed to get comments: {} - {}",
                status,
                error_text
            ));
        }

        let data: Value = response.json().await?;

        let comments: Vec<ConfluenceComment> = if provider.is_cloud {
            // Cloud API v2 response
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|c| ConfluenceComment {
                    id: c["id"]
                        .as_str()
                        .or_else(|| c["id"].as_i64().map(|_| ""))
                        .unwrap_or("")
                        .to_string(),
                    body: c["body"]["storage"]["value"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    body_html: c["body"]["view"]["value"].as_str().map(|s| s.to_string()),
                    created_at: c["createdAt"].as_str().map(|s| s.to_string()),
                    author: parse_confluence_user(&c["author"]),
                })
                .collect()
        } else {
            // Server API response
            data["results"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .map(|c| ConfluenceComment {
                    id: c["id"].as_str().unwrap_or("").to_string(),
                    body: c["body"]["storage"]["value"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                    body_html: c["body"]["view"]["value"].as_str().map(|s| s.to_string()),
                    created_at: c["history"]["createdDate"].as_str().map(|s| s.to_string()),
                    author: parse_confluence_user(&c["history"]["createdBy"]),
                })
                .collect()
        };

        context.set_pin_value("comments", json!(comments)).await?;

        Ok(())
    }
}
