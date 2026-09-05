use super::provider::{DATABRICKS_PROVIDER_ID, DatabricksProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksCatalog {
    pub name: String,
    pub owner: Option<String>,
    pub comment: Option<String>,
    pub catalog_type: String,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub isolation_mode: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksSchema {
    pub name: String,
    pub catalog_name: String,
    pub owner: Option<String>,
    pub comment: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksTable {
    pub name: String,
    pub catalog_name: String,
    pub schema_name: String,
    pub table_type: String,
    pub data_source_format: Option<String>,
    pub owner: Option<String>,
    pub comment: Option<String>,
    pub created_at: Option<i64>,
    pub updated_at: Option<i64>,
    pub storage_location: Option<String>,
}

fn parse_catalog(catalog: &Value) -> Option<DatabricksCatalog> {
    Some(DatabricksCatalog {
        name: catalog["name"].as_str()?.to_string(),
        owner: catalog["owner"].as_str().map(String::from),
        comment: catalog["comment"].as_str().map(String::from),
        catalog_type: catalog["catalog_type"]
            .as_str()
            .unwrap_or("MANAGED_CATALOG")
            .to_string(),
        created_at: catalog["created_at"].as_i64(),
        updated_at: catalog["updated_at"].as_i64(),
        isolation_mode: catalog["isolation_mode"].as_str().map(String::from),
    })
}

fn parse_schema(schema: &Value) -> Option<DatabricksSchema> {
    Some(DatabricksSchema {
        name: schema["name"].as_str()?.to_string(),
        catalog_name: schema["catalog_name"].as_str()?.to_string(),
        owner: schema["owner"].as_str().map(String::from),
        comment: schema["comment"].as_str().map(String::from),
        created_at: schema["created_at"].as_i64(),
        updated_at: schema["updated_at"].as_i64(),
    })
}

fn parse_table(table: &Value) -> Option<DatabricksTable> {
    Some(DatabricksTable {
        name: table["name"].as_str()?.to_string(),
        catalog_name: table["catalog_name"].as_str()?.to_string(),
        schema_name: table["schema_name"].as_str()?.to_string(),
        table_type: table["table_type"]
            .as_str()
            .unwrap_or("MANAGED")
            .to_string(),
        data_source_format: table["data_source_format"].as_str().map(String::from),
        owner: table["owner"].as_str().map(String::from),
        comment: table["comment"].as_str().map(String::from),
        created_at: table["created_at"].as_i64(),
        updated_at: table["updated_at"].as_i64(),
        storage_location: table["storage_location"].as_str().map(String::from),
    })
}

// =============================================================================
// List Catalogs Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabricksCatalogsNode {}

impl ListDatabricksCatalogsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDatabricksCatalogsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_list_catalogs",
            "List Catalogs",
            "List all catalogs in Unity Catalog",
            "Data/Databricks/Unity Catalog",
        );
        node.set_flowscript_name("databricks", "listCatalogs");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "Databricks provider",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

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
            "catalogs",
            "Catalogs",
            "Array of catalogs",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<DatabricksCatalog>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of catalogs",
            VariableType::Integer,
        );

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details if the request fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;

        let url = provider.api_url_v21("/unity-catalog/catalogs");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let catalogs_array = data["catalogs"].as_array();
                    let catalogs: Vec<DatabricksCatalog> = catalogs_array
                        .map(|arr| arr.iter().filter_map(parse_catalog).collect())
                        .unwrap_or_default();

                    let count = catalogs.len();

                    context.set_pin_value("catalogs", json!(catalogs)).await?;
                    context.set_pin_value("count", json!(count)).await?;
                    context.set_pin_value("error_message", json!("")).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context.log_message(
                        &format!("Request failed ({}): {}", status, error_text),
                        LogLevel::Error,
                    );
                    context
                        .set_pin_value("error_message", json!(error_text))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Request error: {}", e), LogLevel::Error);
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Schemas Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabricksSchemasNode {}

impl ListDatabricksSchemasNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDatabricksSchemasNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_list_schemas",
            "List Schemas",
            "List all schemas in a catalog",
            "Data/Databricks/Unity Catalog",
        );
        node.set_flowscript_name("databricks", "listSchemas");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "Databricks provider",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "catalog_name",
            "Catalog Name",
            "The name of the catalog",
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
            "schemas",
            "Schemas",
            "Array of schemas",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<DatabricksSchema>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of schemas", VariableType::Integer);

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details if the request fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(8)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(9)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let catalog_name: String = context.evaluate_pin("catalog_name").await?;

        if catalog_name.is_empty() {
            context
                .set_pin_value("error_message", json!("Catalog name is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url_v21(&format!(
            "/unity-catalog/schemas?catalog_name={}",
            urlencoding::encode(&catalog_name)
        ));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let schemas_array = data["schemas"].as_array();
                    let schemas: Vec<DatabricksSchema> = schemas_array
                        .map(|arr| arr.iter().filter_map(parse_schema).collect())
                        .unwrap_or_default();

                    let count = schemas.len();

                    context.set_pin_value("schemas", json!(schemas)).await?;
                    context.set_pin_value("count", json!(count)).await?;
                    context.set_pin_value("error_message", json!("")).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context.log_message(
                        &format!("Request failed ({}): {}", status, error_text),
                        LogLevel::Error,
                    );
                    context
                        .set_pin_value("error_message", json!(error_text))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Request error: {}", e), LogLevel::Error);
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

// =============================================================================
// List Tables Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabricksTablesNode {}

impl ListDatabricksTablesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDatabricksTablesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_list_tables",
            "List Tables",
            "List all tables in a schema",
            "Data/Databricks/Unity Catalog",
        );
        node.set_flowscript_name("databricks", "listTables");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "provider",
            "Provider",
            "Databricks provider",
            VariableType::Struct,
        )
        .set_schema::<DatabricksProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "catalog_name",
            "Catalog Name",
            "The name of the catalog",
            VariableType::String,
        );

        node.add_input_pin(
            "schema_name",
            "Schema Name",
            "The name of the schema",
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

        node.add_output_pin("tables", "Tables", "Array of tables", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<DatabricksTable>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of tables", VariableType::Integer);

        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details if the request fails",
            VariableType::String,
        );

        node.add_required_oauth_scopes(DATABRICKS_PROVIDER_ID, vec!["all-apis"]);
        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(8)
                .set_performance(7)
                .set_governance(8)
                .set_reliability(9)
                .set_cost(8)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let catalog_name: String = context.evaluate_pin("catalog_name").await?;
        let schema_name: String = context.evaluate_pin("schema_name").await?;

        if catalog_name.is_empty() {
            context
                .set_pin_value("error_message", json!("Catalog name is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        if schema_name.is_empty() {
            context
                .set_pin_value("error_message", json!("Schema name is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url_v21(&format!(
            "/unity-catalog/tables?catalog_name={}&schema_name={}",
            urlencoding::encode(&catalog_name),
            urlencoding::encode(&schema_name)
        ));

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let tables_array = data["tables"].as_array();
                    let tables: Vec<DatabricksTable> = tables_array
                        .map(|arr| arr.iter().filter_map(parse_table).collect())
                        .unwrap_or_default();

                    let count = tables.len();

                    context.set_pin_value("tables", json!(tables)).await?;
                    context.set_pin_value("count", json!(count)).await?;
                    context.set_pin_value("error_message", json!("")).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    let status = resp.status();
                    let error_text = resp
                        .text()
                        .await
                        .unwrap_or_else(|_| "Unknown error".to_string());
                    context.log_message(
                        &format!("Request failed ({}): {}", status, error_text),
                        LogLevel::Error,
                    );
                    context
                        .set_pin_value("error_message", json!(error_text))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Err(e) => {
                context.log_message(&format!("Request error: {}", e), LogLevel::Error);
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
