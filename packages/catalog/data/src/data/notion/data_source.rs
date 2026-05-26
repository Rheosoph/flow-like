use super::provider::{NOTION_PROVIDER_ID, NotionProvider};
use super::utils::{
    NOTION_API_VERSION, auth_header, log_and_error, notion_error, optional_json_value,
    plain_text_from_rich_text, rich_text_from_plain, title_from_object,
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
pub struct NotionDataSourceProperty {
    pub id: String,
    pub name: String,
    pub property_type: String,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionDataSource {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub properties: Vec<NotionDataSourceProperty>,
    pub parent: Value,
    pub created_time: String,
    pub last_edited_time: String,
    pub archived: bool,
    pub in_trash: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionDataSourceQueryResult {
    pub id: String,
    pub object_type: String,
    pub title: String,
    pub url: String,
    pub created_time: String,
    pub last_edited_time: String,
    pub archived: bool,
    pub in_trash: bool,
    pub properties: Value,
}

#[derive(Debug, Deserialize)]
struct QueryResponse {
    results: Vec<Value>,
    next_cursor: Option<String>,
    has_more: bool,
}

fn parse_property(name: &str, property: &Value) -> NotionDataSourceProperty {
    let property_type = property["type"].as_str().unwrap_or("unknown").to_string();
    NotionDataSourceProperty {
        id: property["id"].as_str().unwrap_or("").to_string(),
        name: name.to_string(),
        property_type: property_type.clone(),
        config: property[&property_type].clone(),
    }
}

pub fn parse_data_source(value: &Value) -> Option<NotionDataSource> {
    let mut properties = Vec::new();
    if let Some(props) = value["properties"].as_object() {
        for (name, property) in props {
            properties.push(parse_property(name, property));
        }
    }

    let description_text = plain_text_from_rich_text(&value["description"]);
    let description = if description_text.is_empty() {
        None
    } else {
        Some(description_text)
    };
    let in_trash = value["in_trash"]
        .as_bool()
        .or_else(|| value["archived"].as_bool())
        .unwrap_or(false);

    Some(NotionDataSource {
        id: value["id"].as_str()?.to_string(),
        title: title_from_object(value),
        description,
        url: value["url"].as_str().unwrap_or("").to_string(),
        properties,
        parent: value["parent"].clone(),
        created_time: value["created_time"].as_str().unwrap_or("").to_string(),
        last_edited_time: value["last_edited_time"].as_str().unwrap_or("").to_string(),
        archived: in_trash,
        in_trash,
    })
}

fn parse_query_result(value: &Value) -> Option<NotionDataSourceQueryResult> {
    let in_trash = value["in_trash"]
        .as_bool()
        .or_else(|| value["archived"].as_bool())
        .unwrap_or(false);

    Some(NotionDataSourceQueryResult {
        id: value["id"].as_str()?.to_string(),
        object_type: value["object"].as_str().unwrap_or("").to_string(),
        title: title_from_object(value),
        url: value["url"].as_str().unwrap_or("").to_string(),
        created_time: value["created_time"].as_str().unwrap_or("").to_string(),
        last_edited_time: value["last_edited_time"].as_str().unwrap_or("").to_string(),
        archived: in_trash,
        in_trash,
        properties: value["properties"].clone(),
    })
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

async fn parse_notion_response(
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
            context.log_message(&format!("Network error: {}", err), LogLevel::Error);
            context.activate_exec_pin("error").await?;
            Ok(None)
        }
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetNotionDataSourceNode {}

impl GetNotionDataSourceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetNotionDataSourceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_get_data_source",
            "Get Notion Data Source",
            "Retrieves a Notion data source schema with its properties",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "data_source_id",
            "Data Source ID",
            "The ID of the Notion data source to retrieve",
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
        node.add_output_pin(
            "data_source",
            "Data Source",
            "The Notion data source",
            VariableType::Struct,
        )
        .set_schema::<NotionDataSource>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin("title", "Title", "Data source title", VariableType::String);
        node.add_output_pin(
            "property_names",
            "Property Names",
            "List of property names in the data source",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let data_source_id: String = context.evaluate_pin("data_source_id").await?;

        if data_source_id.is_empty() {
            log_and_error(context, "Data Source ID cannot be empty").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/data_sources/{}", data_source_id);
        let data = parse_notion_response(
            context,
            client
                .get(&url)
                .header("Authorization", auth_header(&provider.access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .send()
                .await,
        )
        .await?;
        let Some(data) = data else {
            return Ok(());
        };

        let Some(data_source) = parse_data_source(&data) else {
            log_and_error(context, "Failed to parse Notion data source").await?;
            return Ok(());
        };
        let title = data_source.title.clone();
        let property_names = data_source
            .properties
            .iter()
            .map(|property| property.name.clone())
            .collect::<Vec<_>>();

        context
            .set_pin_value("data_source", json!(data_source))
            .await?;
        context.set_pin_value("title", json!(title)).await?;
        context
            .set_pin_value("property_names", json!(property_names))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct QueryNotionDataSourceNode {}

impl QueryNotionDataSourceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for QueryNotionDataSourceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_query_data_source",
            "Query Notion Data Source",
            "Queries a Notion data source and returns matching pages or child data sources",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "data_source_id",
            "Data Source ID",
            "The ID of the Notion data source to query",
            VariableType::String,
        );
        node.add_input_pin(
            "filter",
            "Filter",
            "Optional Notion filter object",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!({})));
        node.add_input_pin(
            "sorts",
            "Sorts",
            "Optional Notion sorts array",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "page_size",
            "Page Size",
            "Maximum number of results per page (1-100)",
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
            "Fetch every available page of query results",
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
            "results",
            "Results",
            "Array of query results",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<NotionDataSourceQueryResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin("count", "Count", "Number of results", VariableType::Integer);
        node.add_output_pin(
            "has_more",
            "Has More",
            "Whether there are more results",
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
        let data_source_id: String = context.evaluate_pin("data_source_id").await?;
        let filter_raw: Value = context.evaluate_pin("filter").await.unwrap_or(json!({}));
        let sorts_raw: Value = context.evaluate_pin("sorts").await.unwrap_or(json!([]));
        let page_size: i64 = context.evaluate_pin("page_size").await.unwrap_or(100);
        let start_cursor: String = context
            .evaluate_pin("start_cursor")
            .await
            .unwrap_or_default();
        let fetch_all: bool = context.evaluate_pin("fetch_all").await.unwrap_or(false);

        if data_source_id.is_empty() {
            log_and_error(context, "Data Source ID cannot be empty").await?;
            return Ok(());
        }

        let mut body = json!({ "page_size": page_size.clamp(1, 100) });
        match optional_json_value(filter_raw, "filter") {
            Ok(Some(filter)) => body["filter"] = filter,
            Ok(None) => {}
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        }
        match optional_json_value(sorts_raw, "sorts") {
            Ok(Some(sorts)) => body["sorts"] = sorts,
            Ok(None) => {}
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        }

        let client = reqwest::Client::new();
        let url = format!(
            "https://api.notion.com/v1/data_sources/{}/query",
            data_source_id
        );
        let mut cursor = if start_cursor.is_empty() {
            None
        } else {
            Some(start_cursor)
        };
        let mut all_results = Vec::new();
        let mut has_more: bool;
        let mut next_cursor: Option<String>;

        loop {
            let mut request_body = body.clone();
            if let Some(cursor) = &cursor {
                request_body["start_cursor"] = json!(cursor);
            }

            let response = client
                .post(&url)
                .header("Authorization", auth_header(&provider.access_token))
                .header("Notion-Version", NOTION_API_VERSION)
                .header("Content-Type", "application/json")
                .json(&request_body)
                .send()
                .await;

            let response = match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        context.log_message(&notion_error(resp).await, LogLevel::Error);
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }

                    resp.json::<QueryResponse>().await.map_err(|err| {
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
            all_results.extend(response.results);

            if !fetch_all || !has_more {
                break;
            }

            cursor = next_cursor.clone();
            if cursor.is_none() {
                break;
            }
        }

        let results = all_results
            .iter()
            .filter_map(parse_query_result)
            .collect::<Vec<_>>();
        let count = results.len() as i64;

        context.set_pin_value("results", json!(results)).await?;
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
pub struct CreateNotionDataSourceNode {}

impl CreateNotionDataSourceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateNotionDataSourceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_create_data_source",
            "Create Notion Data Source",
            "Creates a Notion data source inside an existing database",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "database_id",
            "Database ID",
            "Parent Notion database ID",
            VariableType::String,
        );
        node.add_input_pin("title", "Title", "Data source title", VariableType::String);
        node.add_input_pin(
            "properties",
            "Properties",
            "Data source property schema in Notion API format",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!({})));
        node.add_input_pin(
            "icon_emoji",
            "Icon Emoji",
            "Optional emoji icon",
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
        node.add_output_pin(
            "data_source",
            "Data Source",
            "Created data source",
            VariableType::Struct,
        )
        .set_schema::<NotionDataSource>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_output_pin(
            "data_source_id",
            "Data Source ID",
            "Created data source ID",
            VariableType::String,
        );

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let database_id: String = context.evaluate_pin("database_id").await?;
        let title: String = context.evaluate_pin("title").await?;
        let properties_raw: Value = context
            .evaluate_pin("properties")
            .await
            .unwrap_or(json!({}));
        let icon_emoji: String = context.evaluate_pin("icon_emoji").await.unwrap_or_default();

        if database_id.is_empty() {
            log_and_error(context, "Database ID cannot be empty").await?;
            return Ok(());
        }
        if title.is_empty() {
            log_and_error(context, "Title cannot be empty").await?;
            return Ok(());
        }

        let properties = match optional_json_value(properties_raw, "properties") {
            Ok(Some(properties)) => properties,
            Ok(None) => {
                log_and_error(context, "Properties cannot be empty").await?;
                return Ok(());
            }
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        };

        let mut body = json!({
            "parent": {
                "type": "database_id",
                "database_id": database_id
            },
            "title": rich_text_from_plain(&title),
            "properties": properties
        });
        if !icon_emoji.is_empty() {
            body["icon"] = json!({ "type": "emoji", "emoji": icon_emoji });
        }

        let client = reqwest::Client::new();
        let data = parse_notion_response(
            context,
            client
                .post("https://api.notion.com/v1/data_sources")
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
        let Some(data_source) = parse_data_source(&data) else {
            log_and_error(context, "Failed to parse created data source").await?;
            return Ok(());
        };

        context
            .set_pin_value("data_source_id", json!(data_source.id.clone()))
            .await?;
        context
            .set_pin_value("data_source", json!(data_source))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct UpdateNotionDataSourceNode {}

impl UpdateNotionDataSourceNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for UpdateNotionDataSourceNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_update_data_source",
            "Update Notion Data Source",
            "Updates a Notion data source title, description, icon, or property schema",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        add_provider_pin(&mut node);
        node.add_input_pin(
            "data_source_id",
            "Data Source ID",
            "The Notion data source ID to update",
            VariableType::String,
        );
        node.add_input_pin("title", "Title", "New title", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "description",
            "Description",
            "New description",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "properties",
            "Properties",
            "Property schema updates in Notion API format",
            VariableType::Struct,
        )
        .set_schema::<Value>()
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_default_value(Some(json!({})));
        node.add_input_pin(
            "icon_emoji",
            "Icon Emoji",
            "Optional new emoji icon",
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
        node.add_output_pin(
            "data_source",
            "Data Source",
            "Updated data source",
            VariableType::Struct,
        )
        .set_schema::<NotionDataSource>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        add_standard_scores(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: NotionProvider = context.evaluate_pin("provider").await?;
        let data_source_id: String = context.evaluate_pin("data_source_id").await?;
        let title: String = context.evaluate_pin("title").await.unwrap_or_default();
        let description: String = context
            .evaluate_pin("description")
            .await
            .unwrap_or_default();
        let properties_raw: Value = context
            .evaluate_pin("properties")
            .await
            .unwrap_or(json!({}));
        let icon_emoji: String = context.evaluate_pin("icon_emoji").await.unwrap_or_default();

        if data_source_id.is_empty() {
            log_and_error(context, "Data Source ID cannot be empty").await?;
            return Ok(());
        }

        let mut body = json!({});
        if !title.is_empty() {
            body["title"] = rich_text_from_plain(&title);
        }
        if !description.is_empty() {
            body["description"] = rich_text_from_plain(&description);
        }
        match optional_json_value(properties_raw, "properties") {
            Ok(Some(properties)) => body["properties"] = properties,
            Ok(None) => {}
            Err(err) => {
                log_and_error(context, err.to_string()).await?;
                return Ok(());
            }
        }
        if !icon_emoji.is_empty() {
            body["icon"] = json!({ "type": "emoji", "emoji": icon_emoji });
        }

        if body.as_object().map(|body| body.is_empty()).unwrap_or(true) {
            log_and_error(context, "No data source updates were provided").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/data_sources/{}", data_source_id);
        let data = parse_notion_response(
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
        let Some(data_source) = parse_data_source(&data) else {
            log_and_error(context, "Failed to parse updated data source").await?;
            return Ok(());
        };

        context
            .set_pin_value("data_source", json!(data_source))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
