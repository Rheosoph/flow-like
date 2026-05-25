use super::provider::{NOTION_PROVIDER_ID, NotionProvider};
use super::utils::{
    NOTION_API_VERSION, auth_header, log_and_error, notion_error, optional_json_value,
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatedNotionPage {
    pub id: String,
    pub url: String,
    pub created_time: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateNotionPageNode {}

impl CreateNotionPageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateNotionPageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_create_page",
            "Create Notion Page",
            "Creates a new page under a Notion data source, database, or page",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger page creation",
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
            "database_id",
            "Parent ID",
            "The data source, database, or page ID to create the page under",
            VariableType::String,
        );

        node.add_input_pin(
            "parent_type",
            "Parent Type",
            "Parent type for the page",
            VariableType::String,
        )
        .set_default_value(Some(json!("data_source_id")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "data_source_id".to_string(),
                    "database_id".to_string(),
                    "page_id".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "properties",
            "Properties",
            "Page properties in Notion API format",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!({})));

        node.add_input_pin(
            "content",
            "Content",
            "Optional page content as an array of Notion block objects",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "icon_emoji",
            "Icon Emoji",
            "Optional: Emoji to use as the page icon",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when page is successfully created",
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
            "Created Page",
            "The created page info",
            VariableType::Struct,
        )
        .set_schema::<CreatedNotionPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "page_id",
            "Page ID",
            "The ID of the created page",
            VariableType::String,
        );

        node.add_output_pin(
            "page_url",
            "Page URL",
            "The URL of the created page",
            VariableType::String,
        );

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

        let parent_id: String = context.evaluate_pin("database_id").await?;
        let parent_type: String = context
            .evaluate_pin("parent_type")
            .await
            .unwrap_or_else(|_| "data_source_id".to_string());
        let properties_raw: Value = context
            .evaluate_pin("properties")
            .await
            .unwrap_or(json!({}));
        let content_raw: Value = context.evaluate_pin("content").await.unwrap_or(json!([]));
        let icon_emoji: String = context.evaluate_pin("icon_emoji").await?;

        if parent_id.is_empty() {
            log_and_error(context, "Parent ID cannot be empty").await?;
            return Ok(());
        }

        if !matches!(
            parent_type.as_str(),
            "data_source_id" | "database_id" | "page_id"
        ) {
            log_and_error(
                context,
                "Parent Type must be data_source_id, database_id, or page_id",
            )
            .await?;
            return Ok(());
        }

        let mut parent = json!({
            "type": parent_type.clone()
        });
        parent[parent_type.as_str()] = json!(parent_id);

        let properties = match optional_json_value(properties_raw, "properties") {
            Ok(Some(value)) => value,
            Ok(None) => json!({}),
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        };

        let content = match optional_json_value(content_raw, "content") {
            Ok(value) => value,
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        };

        let mut body = json!({
            "parent": parent,
            "properties": properties
        });

        if let Some(children) = content {
            body["children"] = children;
        }

        if !icon_emoji.is_empty() {
            body["icon"] = json!({
                "type": "emoji",
                "emoji": icon_emoji
            });
        }

        let client = reqwest::Client::new();

        context.log_message("Creating Notion page...", LogLevel::Debug);

        let response = client
            .post("https://api.notion.com/v1/pages")
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

                let page_id = page_data["id"].as_str().unwrap_or("").to_string();
                let page_url = page_data["url"].as_str().unwrap_or("").to_string();
                let created_time = page_data["created_time"].as_str().unwrap_or("").to_string();

                let created_page = CreatedNotionPage {
                    id: page_id.clone(),
                    url: page_url.clone(),
                    created_time,
                };

                context.log_message(
                    &format!("Successfully created page: {}", page_id),
                    LogLevel::Info,
                );

                context.set_pin_value("page", json!(created_page)).await?;
                context.set_pin_value("page_id", json!(page_id)).await?;
                context.set_pin_value("page_url", json!(page_url)).await?;
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
