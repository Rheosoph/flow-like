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
pub struct NotionDatabaseProperty {
    pub id: String,
    pub name: String,
    pub property_type: String,
    pub config: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionDatabaseDataSource {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NotionDatabaseSchema {
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    pub url: String,
    pub properties: Vec<NotionDatabaseProperty>,
    pub data_sources: Vec<NotionDatabaseDataSource>,
    pub is_inline: bool,
    pub created_time: String,
    pub last_edited_time: String,
}

#[crate::register_node]
#[derive(Default)]
pub struct GetNotionDatabaseNode {}

impl GetNotionDatabaseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetNotionDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_notion_get_database",
            "Get Notion Database",
            "Retrieves a Notion database schema with its properties",
            "Data/Notion",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/notion.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger database retrieval",
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
            "The ID of the Notion database to retrieve",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when database is successfully retrieved",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered when an error occurs",
            VariableType::Execution,
        );

        node.add_output_pin(
            "database",
            "Database",
            "The Notion database schema",
            VariableType::Struct,
        )
        .set_schema::<NotionDatabaseSchema>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("title", "Title", "The database title", VariableType::String);

        node.add_output_pin(
            "property_names",
            "Property Names",
            "List of property names in the database",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin(
            "data_source_ids",
            "Data Source IDs",
            "List of data source IDs belonging to this database",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

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

        let database_id: String = context.evaluate_pin("database_id").await?;

        if database_id.is_empty() {
            context.log_message("Database ID cannot be empty", LogLevel::Error);
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let client = reqwest::Client::new();
        let url = format!("https://api.notion.com/v1/databases/{}", database_id);

        context.log_message(
            &format!("Fetching Notion database: {}", database_id),
            LogLevel::Debug,
        );

        let response = client
            .get(&url)
            .header("Authorization", auth_header(&access_token))
            .header("Notion-Version", NOTION_API_VERSION)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if !resp.status().is_success() {
                    context.log_message(&notion_error(resp).await, LogLevel::Error);
                    context.activate_exec_pin("error").await?;
                    return Ok(());
                }

                let db_data: Value = resp
                    .json()
                    .await
                    .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                let title_text = plain_text_from_rich_text(&db_data["title"]);
                let title = if title_text.is_empty() {
                    "Untitled".to_string()
                } else {
                    title_text
                };

                let description_text = plain_text_from_rich_text(&db_data["description"]);
                let description = if description_text.is_empty() {
                    None
                } else {
                    Some(description_text)
                };

                let mut properties: Vec<NotionDatabaseProperty> = Vec::new();
                let mut property_names: Vec<String> = Vec::new();

                if let Some(props) = db_data["properties"].as_object() {
                    for (name, prop) in props {
                        let prop_type = prop["type"].as_str().unwrap_or("unknown").to_string();
                        let prop_id = prop["id"].as_str().unwrap_or("").to_string();

                        property_names.push(name.clone());
                        properties.push(NotionDatabaseProperty {
                            id: prop_id,
                            name: name.clone(),
                            property_type: prop_type.clone(),
                            config: prop[&prop_type].clone(),
                        });
                    }
                }

                let data_sources: Vec<NotionDatabaseDataSource> = db_data["data_sources"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|data_source| {
                                Some(NotionDatabaseDataSource {
                                    id: data_source["id"].as_str()?.to_string(),
                                    name: data_source["name"].as_str().unwrap_or("").to_string(),
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let data_source_ids: Vec<String> = data_sources
                    .iter()
                    .map(|data_source| data_source.id.clone())
                    .collect();

                let schema = NotionDatabaseSchema {
                    id: db_data["id"].as_str().unwrap_or("").to_string(),
                    title: title.clone(),
                    description,
                    url: db_data["url"].as_str().unwrap_or("").to_string(),
                    properties,
                    data_sources,
                    is_inline: db_data["is_inline"].as_bool().unwrap_or(false),
                    created_time: db_data["created_time"].as_str().unwrap_or("").to_string(),
                    last_edited_time: db_data["last_edited_time"]
                        .as_str()
                        .unwrap_or("")
                        .to_string(),
                };

                context.log_message(
                    &format!("Successfully retrieved database: {}", title),
                    LogLevel::Info,
                );

                context.set_pin_value("database", json!(schema)).await?;
                context.set_pin_value("title", json!(title)).await?;
                context
                    .set_pin_value("property_names", json!(property_names))
                    .await?;
                context
                    .set_pin_value("data_source_ids", json!(data_source_ids))
                    .await?;
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
