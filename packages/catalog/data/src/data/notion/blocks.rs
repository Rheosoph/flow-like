use super::get_page::{NotionBlock, parse_block};
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
use flow_like_types::{Value, async_trait, json::json, reqwest};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct BlocksResponse {
    results: Vec<Value>,
    next_cursor: Option<String>,
    has_more: bool,
}

fn add_provider_pin(node: &mut Node) {
    node.add_input_pin(
        "provider",
        "Provider",
        "Notion provider (from Notion node)",
        VariableType::Struct,
    )
    .set_schema::<NotionProvider>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_standard_scores(node: &mut Node) {
    node.add_required_oauth_scopes(NOTION_PROVIDER_ID, vec![]);
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
}

async fn parse_block_response(
    context: &mut ExecutionContext,
    response: Result<reqwest::Response, reqwest::Error>,
) -> flow_like_types::Result<Option<Value>> {
    match response {
        Ok(resp) => {
            if !resp.status().is_success() {
                context.log_message(&notion_error(resp).await, LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(None);
            }

            Ok(Some(resp.json::<Value>().await.map_err(|err| {
                flow_like_types::anyhow!("Failed to parse response: {}", err)
            })?))
        }
        Err(err) => {
            log_and_error(context, format!("Network error: {}", err)).await?;
            Ok(None)
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ListNotionBlockChildrenNode {}

impl ListNotionBlockChildrenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListNotionBlockChildrenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_list_block_children",
            "List Notion Block Children",
            "Lists child blocks for a Notion block or page",
            "Data/Notion",
        );
        node.set_flowscript_name("notion", "listBlockChildren");
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "block_id",
            "Block ID",
            "The block or page ID whose children should be listed",
            VariableType::String,
        );
        node.add_input_pin(
            "page_size",
            "Page Size",
            "Maximum number of blocks per page (1-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));
        node.add_input_pin(
            "start_cursor",
            "Start Cursor",
            "Pagination cursor from a previous list call",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "fetch_all",
            "Fetch All",
            "Fetch every available page of child blocks",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );
        node.add_output_pin(
            "blocks",
            "Blocks",
            "Array of child blocks",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<NotionBlock>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin("count", "Count", "Number of blocks", VariableType::Integer);
        node.add_output_pin(
            "has_more",
            "Has More",
            "Whether there are more child blocks",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "next_cursor",
            "Next Cursor",
            "Cursor to request the next page",
            VariableType::String,
        );

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let block_id: String = context.evaluate_pin("block_id").await?;
        let page_size: i64 = context.evaluate_pin("page_size").await.unwrap_or(100);
        let start_cursor: String = context
            .evaluate_pin("start_cursor")
            .await
            .unwrap_or_default();
        let fetch_all: bool = context.evaluate_pin("fetch_all").await.unwrap_or(false);

        if block_id.is_empty() {
            log_and_error(context, "Block ID cannot be empty").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let mut cursor = if start_cursor.is_empty() {
            None
        } else {
            Some(start_cursor)
        };
        let mut raw_blocks = Vec::new();
        let mut has_more: bool;
        let mut next_cursor: Option<String>;

        loop {
            let mut url = format!(
                "https://api.notion.com/v1/blocks/{}/children?page_size={}",
                block_id,
                page_size.clamp(1, 100)
            );
            if let Some(cursor) = &cursor {
                url.push_str("&start_cursor=");
                url.push_str(&urlencoding::encode(cursor));
            }

            let response = client
                .get(&url)
                .header("Authorization", auth_header(&provider.access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .send()
                .await;
            let response = match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        context.log_message(&notion_error(resp).await, LogLevel::Error);
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }

                    resp.json::<BlocksResponse>().await.map_err(|err| {
                        flow_like_types::anyhow!("Failed to parse response: {}", err)
                    })?
                }
                Err(err) => {
                    log_and_error(context, format!("Network error: {}", err)).await?;
                    return Ok(());
                }
            };

            has_more = response.has_more;
            next_cursor = response.next_cursor;
            raw_blocks.extend(response.results);

            if !fetch_all || !has_more {
                break;
            }

            cursor = next_cursor.clone();
            if cursor.is_none() {
                break;
            }
        }

        let blocks = raw_blocks.iter().map(parse_block).collect::<Vec<_>>();
        let count = blocks.len() as i64;

        context.set_pin_value("blocks", json!(blocks)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.set_pin_value("has_more", json!(has_more)).await?;
        context
            .set_pin_value("next_cursor", json!(next_cursor.unwrap_or_default()))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AppendNotionBlockChildrenNode {}

impl AppendNotionBlockChildrenNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AppendNotionBlockChildrenNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_append_block_children",
            "Append Notion Block Children",
            "Appends child blocks to a Notion block or page",
            "Data/Notion",
        );
        node.set_flowscript_name("notion", "appendBlockChildren");
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "block_id",
            "Block ID",
            "The block or page ID to append children to",
            VariableType::String,
        );
        node.add_input_pin(
            "children",
            "Children",
            "Array of Notion block objects to append",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "after_block_id",
            "After Block ID",
            "Optional sibling block ID to insert after",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );
        node.add_output_pin("blocks", "Blocks", "Appended blocks", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<NotionBlock>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "count",
            "Count",
            "Number of appended blocks",
            VariableType::Integer,
        );

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let block_id: String = context.evaluate_pin("block_id").await?;
        let children_raw: Value = context.evaluate_pin("children").await.unwrap_or(json!([]));
        let after_block_id: String = context
            .evaluate_pin("after_block_id")
            .await
            .unwrap_or_default();

        if block_id.is_empty() {
            log_and_error(context, "Block ID cannot be empty").await?;
            return Ok(());
        }

        let children = match optional_json_value(children_raw, "children") {
            Ok(Some(value)) if value.is_array() => value,
            Ok(Some(_)) => {
                log_and_error(context, "Children must be an array of Notion block objects").await?;
                return Ok(());
            }
            Ok(None) => {
                log_and_error(context, "Children cannot be empty").await?;
                return Ok(());
            }
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        };

        let mut body = json!({ "children": children });
        if !after_block_id.is_empty() {
            body["position"] = json!({
                "type": "after",
                "after": after_block_id
            });
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/blocks/{}/children", block_id);
        let data = parse_block_response(
            context,
            client
                .patch(&url)
                .header("Authorization", auth_header(&provider.access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await,
        )
        .await?;
        let Some(data) = data else {
            return Ok(());
        };

        let blocks = data["results"]
            .as_array()
            .map(|blocks| blocks.iter().map(parse_block).collect::<Vec<_>>())
            .unwrap_or_default();
        let count = blocks.len() as i64;

        context.set_pin_value("blocks", json!(blocks)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct UpdateNotionBlockNode {}

impl UpdateNotionBlockNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateNotionBlockNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_update_block",
            "Update Notion Block",
            "Updates a Notion block with a raw Notion block update object",
            "Data/Notion",
        );
        node.set_flowscript_name("notion", "updateBlock");
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "block_id",
            "Block ID",
            "The Notion block ID to update",
            VariableType::String,
        );
        node.add_input_pin(
            "block_update",
            "Block Update",
            "Notion block update object",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!({})));
        node.add_input_pin(
            "in_trash",
            "In Trash",
            "Target trash state. True moves the block to trash; false restores when Change Trash State is enabled.",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        node.add_input_pin(
            "change_trash_state",
            "Change Trash State",
            "Enable to apply the In Trash value",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );
        node.add_output_pin("block", "Block", "Updated block", VariableType::Struct)
            .set_schema::<NotionBlock>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let block_id: String = context.evaluate_pin("block_id").await?;
        let update_raw: Value = context
            .evaluate_pin("block_update")
            .await
            .unwrap_or(json!({}));
        let in_trash: bool = context.evaluate_pin("in_trash").await.unwrap_or(false);
        let change_trash_state: bool = context
            .evaluate_pin("change_trash_state")
            .await
            .unwrap_or(false);

        if block_id.is_empty() {
            log_and_error(context, "Block ID cannot be empty").await?;
            return Ok(());
        }

        let mut body = match optional_json_value(update_raw, "block_update") {
            Ok(Some(value)) if value.is_object() => value,
            Ok(Some(_)) => {
                log_and_error(context, "Block Update must be an object").await?;
                return Ok(());
            }
            Ok(None) => json!({}),
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        };
        if in_trash || change_trash_state {
            body["in_trash"] = json!(in_trash);
        }
        if body.as_object().map(|body| body.is_empty()).unwrap_or(true) {
            log_and_error(context, "No block updates were provided").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/blocks/{}", block_id);
        let data = parse_block_response(
            context,
            client
                .patch(&url)
                .header("Authorization", auth_header(&provider.access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await,
        )
        .await?;
        let Some(data) = data else {
            return Ok(());
        };

        context
            .set_pin_value("block", json!(parse_block(&data)))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct DeleteNotionBlockNode {}

impl DeleteNotionBlockNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteNotionBlockNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_delete_block",
            "Delete Notion Block",
            "Moves a Notion block to trash",
            "Data/Notion",
        );
        node.set_flowscript_name("notion", "deleteBlock");
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "block_id",
            "Block ID",
            "The Notion block ID to delete",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
        );
        node.add_output_pin("block", "Block", "Deleted block", VariableType::Struct)
            .set_schema::<NotionBlock>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let block_id: String = context.evaluate_pin("block_id").await?;

        if block_id.is_empty() {
            log_and_error(context, "Block ID cannot be empty").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/blocks/{}", block_id);
        let data = parse_block_response(
            context,
            client
                .delete(&url)
                .header("Authorization", auth_header(&provider.access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .send()
                .await,
        )
        .await?;
        let Some(data) = data else {
            return Ok(());
        };

        context
            .set_pin_value("block", json!(parse_block(&data)))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
