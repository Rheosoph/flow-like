use super::provider::{NOTION_PROVIDER_ID, NotionProvider};
use super::utils::{NOTION_API_VERSION, auth_header, notion_error, plain_text_from_rich_text};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionDatabase {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub created_time: String,
    pub last_edited_time: String,
    pub icon_emoji: Option<String>,
    pub data_source_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SearchResponse {
    results: Vec<Value>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[crate::register_node]
#[derive(Default)]
pub struct ListNotionDatabasesNode {}

impl ListNotionDatabasesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListNotionDatabasesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_list_databases",
            "List Notion Databases",
            "Lists all databases the integration has access to",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the database listing",
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
            "query",
            "Search Query",
            "Optional search query to filter databases by title",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "page_size",
            "Page Size",
            "Maximum number of databases to return (1-100)",
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
            "Fetch every available page of database results",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when databases are successfully listed",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "databases",
            "Databases",
            "Array of Notion databases",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<NotionDatabase>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of databases returned",
            VariableType::Integer,
        );

        node.add_output_pin(
            "has_more",
            "Has More",
            "Whether there are more databases available",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "next_cursor",
            "Next Cursor",
            "Cursor to request the next page of databases",
            VariableType::String,
        );

        node.add_required_oauth_scopes(NOTION_PROVIDER_ID, vec![]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(7)
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

        let query: String = context.evaluate_pin("query").await?;
        let page_size: i64 = context.evaluate_pin("page_size").await?;
        let start_cursor: String = context
            .evaluate_pin("start_cursor")
            .await
            .unwrap_or_default();
        let fetch_all: bool = context.evaluate_pin("fetch_all").await.unwrap_or(false);

        let client = reqwest::Client::new();

        let mut body = json!({
            "filter": {
                "value": "database",
                "property": "object"
            },
            "page_size": page_size.clamp(1, 100)
        });

        if !query.is_empty() {
            body["query"] = json!(query);
        }

        context.log_message("Fetching Notion databases...", LogLevel::Debug);

        let mut cursor = if start_cursor.is_empty() {
            None
        } else {
            Some(start_cursor)
        };
        let mut all_databases = Vec::new();
        let mut has_more: bool;
        let mut next_cursor: Option<String>;

        loop {
            let mut request_body = body.clone();
            if let Some(cursor) = &cursor {
                request_body["start_cursor"] = json!(cursor);
            }

            let response = client
                .post("https://api.notion.com/v1/search")
                .header("Authorization", auth_header(&access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await;

            let search_response = match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        context.log_message(&notion_error(resp).await, LogLevel::Error);
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }

                    resp.json::<SearchResponse>().await.map_err(|e| {
                        context.log_message(
                            &format!("Failed to parse response: {}", e),
                            LogLevel::Error,
                        );
                        flow_like_types::anyhow!("Failed to parse Notion response")
                    })?
                }
                Err(e) => {
                    context.log_message(&format!("Network error: {}", e), LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }
            };

            has_more = search_response.has_more;
            next_cursor = search_response.next_cursor;
            all_databases.extend(search_response.results);

            if !fetch_all || !has_more {
                break;
            }

            cursor = next_cursor.clone();
            if cursor.is_none() {
                break;
            }
        }

        let databases: Vec<NotionDatabase> = all_databases
            .into_iter()
            .filter_map(|db| {
                let id = db["id"].as_str()?.to_string();
                let title = {
                    let title = plain_text_from_rich_text(&db["title"]);
                    if title.is_empty() {
                        "Untitled".to_string()
                    } else {
                        title
                    }
                };
                let description_text = plain_text_from_rich_text(&db["description"]);
                let description = if description_text.is_empty() {
                    None
                } else {
                    Some(description_text)
                };
                let url = db["url"].as_str()?.to_string();
                let created_time = db["created_time"].as_str()?.to_string();
                let last_edited_time = db["last_edited_time"].as_str()?.to_string();
                let icon_emoji = db["icon"]["emoji"].as_str().map(String::from);
                let data_source_ids = db["data_sources"]
                    .as_array()
                    .map(|data_sources| {
                        data_sources
                            .iter()
                            .filter_map(|data_source| data_source["id"].as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default();

                Some(NotionDatabase {
                    id,
                    title,
                    description,
                    url,
                    created_time,
                    last_edited_time,
                    icon_emoji,
                    data_source_ids,
                })
            })
            .collect();

        let count = databases.len() as i64;

        context.log_message(&format!("Found {} Notion databases", count), LogLevel::Info);

        context.set_pin_value("databases", json!(databases)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.set_pin_value("has_more", json!(has_more)).await?;
        context
            .set_pin_value("next_cursor", json!(next_cursor.unwrap_or_default()))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
