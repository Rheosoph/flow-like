use super::provider::{NOTION_PROVIDER_ID, NotionProvider};
use super::utils::{NOTION_API_VERSION, auth_header, notion_error, optional_json_value};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionPage {
    pub id: String,
    pub url: String,
    pub created_time: String,
    pub last_edited_time: String,
    pub archived: bool,
    pub in_trash: bool,
    pub icon_emoji: Option<String>,
    pub properties: Value,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    results: Vec<Value>,
    next_cursor: Option<String>,
    has_more: bool,
}

#[crate::register_node]
#[derive(Default)]
pub struct QueryNotionDatabaseNode {}

impl QueryNotionDatabaseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for QueryNotionDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_query_database",
            "Query Notion Database",
            "Queries a Notion database and returns matching pages",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the database query",
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
            "Database ID",
            "The ID of the Notion database to query",
            VariableType::String,
        );

        node.add_input_pin(
            "filter",
            "Filter",
            "Optional filter object in Notion filter format",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!({})));

        node.add_input_pin(
            "sorts",
            "Sorts",
            "Optional Notion sorts array. Overrides Sort Property when provided.",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!([])));

        node.add_input_pin(
            "sort_property",
            "Sort Property",
            "Property name to sort by",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "sort_direction",
            "Sort Direction",
            "Sort direction (ascending or descending)",
            VariableType::String,
        )
        .set_default_value(Some(json!("descending")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["ascending".to_string(), "descending".to_string()])
                .build(),
        );

        node.add_input_pin(
            "page_size",
            "Page Size",
            "Maximum number of results to return (1-100)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(100)));

        node.add_input_pin(
            "start_cursor",
            "Start Cursor",
            "Pagination cursor from a previous query",
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
            "Triggered when query completes successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "pages",
            "Pages",
            "Array of Notion pages matching the query",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<NotionPage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of pages returned",
            VariableType::Integer,
        );

        node.add_output_pin(
            "has_more",
            "Has More",
            "Whether there are more results available",
            VariableType::Boolean,
        );

        node.add_output_pin(
            "next_cursor",
            "Next Cursor",
            "Cursor to request the next page of pages",
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

        let database_id: String = context.evaluate_pin("database_id").await?;
        let filter_raw: Value = context.evaluate_pin("filter").await.unwrap_or(json!({}));
        let sorts_raw: Value = context.evaluate_pin("sorts").await.unwrap_or(json!([]));
        let sort_property: String = context.evaluate_pin("sort_property").await?;
        let sort_direction: String = context.evaluate_pin("sort_direction").await?;
        let page_size: i64 = context.evaluate_pin("page_size").await?;
        let start_cursor: String = context
            .evaluate_pin("start_cursor")
            .await
            .unwrap_or_default();
        let fetch_all: bool = context.evaluate_pin("fetch_all").await.unwrap_or(false);

        if database_id.is_empty() {
            context.log_message("Database ID cannot be empty", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/databases/{}/query", database_id);

        let mut body = json!({
            "page_size": page_size.clamp(1, 100)
        });

        let filter = match optional_json_value(filter_raw, "filter") {
            Ok(value) => value,
            Err(err) => {
                context.log_message(&err.to_string(), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };
        if let Some(filter) = filter {
            body["filter"] = filter;
        }

        let sorts = match optional_json_value(sorts_raw, "sorts") {
            Ok(value) => value,
            Err(err) => {
                context.log_message(&err.to_string(), LogLevel::Error);
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        };

        if let Some(sorts) = sorts {
            body["sorts"] = sorts;
        } else if !sort_property.is_empty() {
            body["sorts"] = json!([{
                "property": sort_property,
                "direction": sort_direction
            }]);
        }

        context.log_message(
            &format!("Querying Notion database: {}", database_id),
            LogLevel::Debug,
        );

        let mut cursor = if start_cursor.is_empty() {
            None
        } else {
            Some(start_cursor)
        };
        let mut all_pages = Vec::new();
        let mut has_more: bool;
        let mut next_cursor: Option<String>;

        loop {
            let mut request_body = body.clone();
            if let Some(cursor) = &cursor {
                request_body["start_cursor"] = json!(cursor);
            }

            let response = client
                .post(&url)
                .header("Authorization", auth_header(&access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await;

            let query_response = match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        context.log_message(&notion_error(resp).await, LogLevel::Error);
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }

                    resp.json::<QueryResponse>().await.map_err(|e| {
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

            has_more = query_response.has_more;
            next_cursor = query_response.next_cursor;
            all_pages.extend(query_response.results);

            if !fetch_all || !has_more {
                break;
            }

            cursor = next_cursor.clone();
            if cursor.is_none() {
                break;
            }
        }

        let pages: Vec<NotionPage> = all_pages
            .into_iter()
            .filter_map(|page| {
                let id = page["id"].as_str()?.to_string();
                let url = page["url"].as_str()?.to_string();
                let created_time = page["created_time"].as_str()?.to_string();
                let last_edited_time = page["last_edited_time"].as_str()?.to_string();
                let in_trash = page["in_trash"]
                    .as_bool()
                    .or_else(|| page["archived"].as_bool())
                    .unwrap_or(false);
                let icon_emoji = page["icon"]["emoji"].as_str().map(String::from);
                let properties = page["properties"].clone();

                Some(NotionPage {
                    id,
                    url,
                    created_time,
                    last_edited_time,
                    archived: in_trash,
                    in_trash,
                    icon_emoji,
                    properties,
                })
            })
            .collect();

        let count = pages.len() as i64;

        context.log_message(
            &format!("Found {} pages in database", count),
            LogLevel::Info,
        );

        context.set_pin_value("pages", json!(pages)).await?;
        context.set_pin_value("count", json!(count)).await?;
        context.set_pin_value("has_more", json!(has_more)).await?;
        context
            .set_pin_value("next_cursor", json!(next_cursor.unwrap_or_default()))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
