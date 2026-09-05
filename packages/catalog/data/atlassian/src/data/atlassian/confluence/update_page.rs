use super::{ConfluencePage, parse_confluence_page};
use crate::data::atlassian::provider::{ATLASSIAN_PROVIDER_ID, AtlassianProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json, reqwest};

#[crate::register_node]
#[derive(Default)]
pub struct UpdateConfluencePageNode {}

impl UpdateConfluencePageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateConfluencePageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_atlassian_confluence_update_page",
            "Update Confluence Page",
            "Update an existing Confluence page's title or body",
            "Data/Atlassian/Confluence",
        );
        node.set_flowscript_name("confluence", "updatePage");
        node.add_icon("/flow/icons/confluence.svg");
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
            "page_id",
            "Page ID",
            "The ID of the page to update",
            VariableType::String,
        );

        node.add_input_pin(
            "title",
            "Title",
            "New page title (leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "body",
            "Body",
            "New page body content (HTML/storage format, leave empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "version_message",
            "Version Message",
            "Optional message for this version (shows in page history)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when page is updated successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "page",
            "Page",
            "The updated Confluence page",
            VariableType::Struct,
        )
        .set_schema::<ConfluencePage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(ATLASSIAN_PROVIDER_ID, vec!["write:confluence-content"]);
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
        let page_id: String = context.evaluate_pin("page_id").await?;
        let title: String = context.evaluate_pin("title").await?;
        let body: String = context.evaluate_pin("body").await?;
        let version_message: String = context.evaluate_pin("version_message").await?;

        if page_id.is_empty() {
            context.log_message("Page ID is required", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();

        // First, get the current page to get version and current values
        let get_url = provider.confluence_rest_api_url(&format!(
            "/content/{}?expand=version,body.storage,space",
            page_id
        ));

        let current_page = client
            .get(&get_url)
            .header("Authorization", provider.auth_header())
            .header("Accept", "application/json")
            .send()
            .await;

        let current_page = match current_page {
            Ok(r) if r.status().is_success() => match r.json::<Value>().await {
                Ok(d) => d,
                Err(e) => {
                    context.log_message(
                        &format!("Failed to parse current page: {}", e),
                        LogLevel::Error,
                    );
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            },
            Ok(r) => {
                let error_text = r.text().await.unwrap_or_default();
                context.log_message(
                    &format!("Failed to get current page: {}", error_text),
                    LogLevel::Error,
                );
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
            Err(e) => {
                context.log_message(&format!("Request failed: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        let current_version = current_page
            .get("version")
            .and_then(|v| v.get("number"))
            .and_then(|n| n.as_i64())
            .unwrap_or(1);

        let current_title = current_page
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let current_body = current_page
            .get("body")
            .and_then(|b| b.get("storage"))
            .and_then(|s| s.get("value"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        let space_key = current_page
            .get("space")
            .and_then(|s| s.get("key"))
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        // Determine final values
        let final_title = if title.is_empty() {
            current_title.to_string()
        } else {
            title
        };

        let final_body = if body.is_empty() {
            current_body.to_string()
        } else {
            body
        };

        // Build update request
        let update_url = provider.confluence_rest_api_url(&format!("/content/{}", page_id));

        let mut request_body = json!({
            "id": page_id,
            "type": "page",
            "title": final_title,
            "space": {
                "key": space_key
            },
            "body": {
                "storage": {
                    "value": final_body,
                    "representation": "storage"
                }
            },
            "version": {
                "number": current_version + 1
            }
        });

        if !version_message.is_empty() {
            request_body["version"]["message"] = json!(version_message);
        }

        context.log_message(
            &format!(
                "Updating Confluence page {} (version {} -> {})",
                page_id,
                current_version,
                current_version + 1
            ),
            LogLevel::Debug,
        );

        let response = client
            .put(&update_url)
            .header("Authorization", provider.auth_header())
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&request_body)
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
                &format!("Confluence API error {}: {}", status, error_text),
                LogLevel::Error,
            );
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let data: Value = match response.json().await {
            Ok(d) => d,
            Err(e) => {
                context.log_message(&format!("Failed to parse response: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        context.log_message("Page updated successfully", LogLevel::Debug);

        let page = parse_confluence_page(&data, &provider.base_url);

        context.set_pin_value("page", json!(page)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
