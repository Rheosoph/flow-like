use super::provider::{NOTION_PROVIDER_ID, NotionProvider};
use super::utils::{
    NOTION_API_VERSION, auth_header, log_and_error, notion_error, optional_json_value,
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UpdatedNotionPage {
    pub id: String,
    pub url: String,
    pub last_edited_time: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct UpdateNotionPageNode {}

impl UpdateNotionPageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateNotionPageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_update_page",
            "Update Notion Page",
            "Updates properties of an existing Notion page",
            "Data/Notion",
        );
        node.set_flowscript_name("notion", "updatePage");
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger page update",
            VariableType::Execution,
        );

        node.add_input_pin(
            "provider",
            "Provider",
            "Notion provider (from Notion node)",
            VariableType::Struct,
        )
        .set_schema::<NotionProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "page_id",
            "Page ID",
            "The ID of the page to update",
            VariableType::String,
        );

        node.add_input_pin(
            "properties",
            "Properties",
            "Page properties to update in Notion API format",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!({})));

        node.add_input_pin(
            "icon_emoji",
            "Icon Emoji",
            "Optional: New emoji to use as the page icon (empty to keep current)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "archived",
            "In Trash",
            "Target trash state. True moves the page to trash; false restores it when Change Trash State is enabled.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_input_pin(
            "change_archive_state",
            "Change Trash State",
            "Enable to apply the In Trash value. True is still applied automatically for backward compatibility.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when page is successfully updated",
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
            "Updated Page",
            "The updated page info",
            VariableType::Struct,
        )
        .set_schema::<UpdatedNotionPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(NOTION_PROVIDER_ID, vec![]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(7)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let access_token = provider.access_token;

        let page_id: String = context.evaluate_pin("page_id").await?;
        let properties_raw: Value = context
            .evaluate_pin("properties")
            .await
            .unwrap_or(json!({}));
        let icon_emoji: String = context.evaluate_pin("icon_emoji").await?;
        let in_trash: bool = context.evaluate_pin("archived").await?;
        let change_archive_state: bool = context
            .evaluate_pin("change_archive_state")
            .await
            .unwrap_or(false);

        if page_id.is_empty() {
            log_and_error(context, "Page ID cannot be empty").await?;
            return Ok(());
        }

        let mut body = json!({});

        let properties = match optional_json_value(properties_raw, "properties") {
            Ok(value) => value,
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        };

        if let Some(properties) = properties {
            body["properties"] = properties;
        }

        if !icon_emoji.is_empty() {
            body["icon"] = json!({
                "type": "emoji",
                "emoji": icon_emoji
            });
        }

        if in_trash || change_archive_state {
            body["in_trash"] = json!(in_trash);
        }

        if body.as_object().map(|obj| obj.is_empty()).unwrap_or(true) {
            log_and_error(context, "No page updates were provided").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/pages/{}", page_id);

        context.log_message(
            &format!("Updating Notion page: {}", page_id),
            LogLevel::Debug,
        );

        let response = client
            .patch(&url)
            .header("Authorization", auth_header(&access_token))
            .header("Notion-Version", NOTION_API_VERSION)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    context.log_message(&notion_error(resp).await, LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let page_data: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let updated_page = UpdatedNotionPage {
                    id: page_data["id"].as_str().unwrap_or("").to_string(),
                    url: page_data["url"].as_str().unwrap_or("").to_string(),
                    last_edited_time: page_data["last_edited_time"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                };

                context.log_message(
                    &format!("Successfully updated page: {}", page_id),
                    LogLevel::Info,
                );

                context.set_pin_value("page", json!(updated_page)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
