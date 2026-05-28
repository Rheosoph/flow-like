use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

use super::ConfluenceUser;

/// Confluence comment
#[derive(Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct ConfluenceComment {
    pub id: String,
    pub body: String,
    pub created_at: Option<String>,
    pub author: Option<ConfluenceUser>,
}

/// Add a comment to a Confluence page
#[crate::register_node]
#[derive(Default)]
pub struct AddConfluenceCommentNode {}

impl AddConfluenceCommentNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddConfluenceCommentNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_add_comment",
            "Add Comment",
            "Add a comment to a Confluence page",
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
            "The ID of the page to comment on",
            VariableType::String,
        );

        node.add_input_pin(
            "body",
            "Comment Body",
            "The comment content (markdown for cloud, storage format for server)",
            VariableType::String,
        );

        node.add_output_pin(
            "comment",
            "Comment",
            "The created comment",
            VariableType::Struct,
        )
        .set_schema::<ConfluenceComment>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:confluence-content"]);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let provider: AtlassianProvider = context.evaluate_pin("provider").await?;
        let page_id: String = context.evaluate_pin("page_id").await?;
        let body: String = context.evaluate_pin("body").await?;

        if page_id.is_empty() {
            return Err(flow_like_types::anyhow!("Page ID is required"));
        }

        if body.is_empty() {
            return Err(flow_like_types::anyhow!("Comment body is required"));
        }

        let client = reqwest::Client::new();

        let (url, request_body) = if provider.is_cloud {
            // Cloud API v2
            let url = provider.confluence_api_url(&format!("/pages/{}/footer-comments", page_id));
            let body_json = json!({
                "body": {
                    "representation": "storage",
                    "value": format!("<p>{}</p>", body)
                }
            });
            (url, body_json)
        } else {
            // Server API
            let url =
                provider.confluence_rest_api_url(&format!("/content/{}/child/comment", page_id));
            let body_json = json!({
                "type": "comment",
                "container": {
                    "id": page_id,
                    "type": "page"
                },
                "body": {
                    "storage": {
                        "value": format!("<p>{}</p>", body),
                        "representation": "storage"
                    }
                }
            });
            (url, body_json)
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

        let data: Value = response.json().await?;

        let comment = ConfluenceComment {
            id: data["id"]
                .as_str()
                .or_else(|| data["id"].as_i64().map(|_| ""))
                .unwrap_or("")
                .to_string(),
            body: body.clone(),
            created_at: data["createdAt"]
                .as_str()
                .or_else(|| data["history"]["createdDate"].as_str())
                .map(|s| s.to_string()),
            author: super::parse_confluence_user(&data["author"])
                .or_else(|| super::parse_confluence_user(&data["history"]["createdBy"])),
        };

        context.set_pin_value("comment", json!(comment)).await?;

        Ok(())
    }
}
