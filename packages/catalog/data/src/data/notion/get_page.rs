use super::provider::{NOTION_PROVIDER_ID, NotionProvider};
use super::utils::{
    NOTION_API_VERSION, auth_header, block_plain_text, log_and_error, notion_error,
    title_from_page_properties,
};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionBlock {
    pub id: String,
    pub block_type: String,
    pub has_children: bool,
    pub in_trash: bool,
    pub content: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionPageContent {
    pub id: String,
    pub url: String,
    pub title: String,
    pub created_time: String,
    pub last_edited_time: String,
    pub archived: bool,
    pub in_trash: bool,
    pub icon_emoji: Option<String>,
    pub properties: Value,
    pub blocks: Vec<NotionBlock>,
    pub plain_text: String,
}

#[derive(Debug, Deserialize)]
struct BlocksResponse {
    results: Vec<Value>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[crate::register_node]
#[derive(Default)]
pub struct GetNotionPageNode {}

impl GetNotionPageNode {
    pub fn new() -> Self {
        Self {}
    }
}

pub fn parse_block(block: &Value) -> NotionBlock {
    let block_type = block["type"].as_str().unwrap_or("unknown").to_string();
    let in_trash = block["in_trash"]
        .as_bool()
        .or_else(|| block["archived"].as_bool())
        .unwrap_or(false);

    NotionBlock {
        id: block["id"].as_str().unwrap_or("").to_string(),
        block_type: block_type.clone(),
        has_children: block["has_children"].as_bool().unwrap_or(false),
        in_trash,
        content: block[&block_type].clone(),
    }
}

async fn fetch_block_children(
    client: &reqwest::Client,
    access_token: &str,
    block_id: &str,
) -> flow_like_types::Result<Vec<Value>> {
    let mut blocks = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let mut url = format!(
            "https://api.notion.com/v1/blocks/{}/children?page_size=100",
            block_id
        );
        if let Some(cursor) = &cursor {
            url.push_str("&start_cursor=");
            url.push_str(&urlencoding::encode(cursor));
        }

        let response = client
            .get(&url)
            .header("Authorization", auth_header(access_token))
            .header("Notion-Version", NOTION_API_VERSION)
            .send()
            .await
            .map_err(|err| flow_like_types::anyhow!("Network error: {}", err))?;

        if !response.status().is_success() {
            return Err(flow_like_types::anyhow!("{}", notion_error(response).await));
        }

        let blocks_response: BlocksResponse = response.json().await.map_err(|err| {
            flow_like_types::anyhow!("Failed to parse block children response: {}", err)
        })?;

        blocks.extend(blocks_response.results);
        if !blocks_response.has_more {
            break;
        }

        cursor = blocks_response.next_cursor;
        if cursor.is_none() {
            break;
        }
    }

    Ok(blocks)
}

#[async_trait]
impl NodeLogic for GetNotionPageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_get_page",
            "Get Notion Page",
            "Retrieves a Notion page with its content and blocks",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger page retrieval",
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
            "The ID of the Notion page to retrieve",
            VariableType::String,
        );

        node.add_input_pin(
            "include_content",
            "Include Content",
            "Whether to fetch the page content blocks",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin(
            "include_nested_content",
            "Include Nested Content",
            "Whether to fetch child blocks recursively",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when page is successfully retrieved",
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
            "The Notion page with content",
            VariableType::Struct,
        )
        .set_schema::<NotionPageContent>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("title", "Title", "The page title", VariableType::String);

        node.add_output_pin(
            "plain_text",
            "Plain Text",
            "The page content as plain text",
            VariableType::String,
        );

        node.add_output_pin(
            "blocks",
            "Blocks",
            "Array of content blocks",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<NotionBlock>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_required_oauth_scopes(NOTION_PROVIDER_ID, vec![]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(9)
                .set_cost(7)
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
        let include_content: bool = context.evaluate_pin("include_content").await?;
        let include_nested_content: bool = context
            .evaluate_pin("include_nested_content")
            .await
            .unwrap_or(false);

        if page_id.is_empty() {
            log_and_error(context, "Page ID cannot be empty").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();

        context.log_message(
            &format!("Fetching Notion page: {}", page_id),
            LogLevel::Debug,
        );

        let page_response = client
            .get(format!("https://api.notion.com/v1/pages/{}", page_id))
            .header("Authorization", auth_header(&access_token))
            .header("Notion-Version", NOTION_API_VERSION)
            .send()
            .await;

        let page_data: Value = match page_response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    context.log_message(&notion_error(resp).await, LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
                resp.json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse page response: {}", e))?
            }
            Err(e) => {
                context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        let title = title_from_page_properties(&page_data["properties"]);

        let mut blocks: Vec<NotionBlock> = Vec::new();
        let mut plain_text_parts: Vec<String> = Vec::new();

        if include_content {
            let mut raw_blocks = match fetch_block_children(&client, &access_token, &page_id).await
            {
                Ok(blocks) => blocks,
                Err(err) => {
                    log_and_error(context, err.to_string()).await?;
                    return Ok(());
                }
            };

            if include_nested_content {
                let mut visited = HashSet::new();
                let mut queue: VecDeque<String> = raw_blocks
                    .iter()
                    .filter(|block| block["has_children"].as_bool().unwrap_or(false))
                    .filter_map(|block| block["id"].as_str().map(String::from))
                    .collect();

                while let Some(block_id) = queue.pop_front() {
                    if !visited.insert(block_id.clone()) {
                        continue;
                    }

                    let child_blocks =
                        match fetch_block_children(&client, &access_token, &block_id).await {
                            Ok(blocks) => blocks,
                            Err(err) => {
                                log_and_error(context, err.to_string()).await?;
                                return Ok(());
                            }
                        };

                    for block in &child_blocks {
                        if block["has_children"].as_bool().unwrap_or(false)
                            && let Some(id) = block["id"].as_str()
                        {
                            queue.push_back(id.to_string());
                        }
                    }

                    raw_blocks.extend(child_blocks);
                }
            }

            for block in raw_blocks {
                let text = block_plain_text(&block);
                if !text.is_empty() {
                    plain_text_parts.push(text);
                }

                blocks.push(parse_block(&block));
            }
        }

        let plain_text = plain_text_parts.join("\n");

        let in_trash = page_data["in_trash"]
            .as_bool()
            .or_else(|| page_data["archived"].as_bool())
            .unwrap_or(false);

        let page_content = NotionPageContent {
            id: page_data["id"].as_str().unwrap_or("").to_string(),
            url: page_data["url"].as_str().unwrap_or("").to_string(),
            title: title.clone(),
            created_time: page_data["created_time"].as_str().unwrap_or("").to_string(),
            last_edited_time: page_data["last_edited_time"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            archived: in_trash,
            in_trash,
            icon_emoji: page_data["icon"]["emoji"].as_str().map(String::from),
            properties: page_data["properties"].clone(),
            blocks: blocks.clone(),
            plain_text: plain_text.clone(),
        };

        context.log_message(
            &format!("Successfully retrieved page: {}", title),
            LogLevel::Info,
        );

        context.set_pin_value("page", json!(page_content)).await?;
        context.set_pin_value("title", json!(title)).await?;
        context
            .set_pin_value("plain_text", json!(plain_text))
            .await?;
        context.set_pin_value("blocks", json!(blocks)).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
