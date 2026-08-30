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
pub struct DatabricksSqlWarehouse {
    pub id: String,
    pub name: String,
    pub state: String,
    pub cluster_size: String,
    pub min_num_clusters: i64,
    pub max_num_clusters: i64,
    pub num_clusters: i64,
    pub num_active_sessions: i64,
    pub auto_stop_mins: i64,
    pub creator_name: Option<String>,
    pub enable_serverless_compute: bool,
    pub warehouse_type: String,
    pub channel: Option<String>,
}

fn parse_warehouse(warehouse: &Value) -> Option<DatabricksSqlWarehouse> {
    Some(DatabricksSqlWarehouse {
        id: warehouse["id"].as_str()?.to_string(),
        name: warehouse["name"].as_str()?.to_string(),
        state: warehouse["state"].as_str().unwrap_or("UNKNOWN").to_string(),
        cluster_size: warehouse["cluster_size"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        min_num_clusters: warehouse["min_num_clusters"].as_i64().unwrap_or(1),
        max_num_clusters: warehouse["max_num_clusters"].as_i64().unwrap_or(1),
        num_clusters: warehouse["num_clusters"].as_i64().unwrap_or(0),
        num_active_sessions: warehouse["num_active_sessions"].as_i64().unwrap_or(0),
        auto_stop_mins: warehouse["auto_stop_mins"].as_i64().unwrap_or(0),
        creator_name: warehouse["creator_name"].as_str().map(String::from),
        enable_serverless_compute: warehouse["enable_serverless_compute"]
            .as_bool()
            .unwrap_or(false),
        warehouse_type: warehouse["warehouse_type"]
            .as_str()
            .unwrap_or("CLASSIC")
            .to_string(),
        channel: warehouse["channel"]
            .get("name")
            .and_then(|n| n.as_str())
            .map(String::from),
    })
}

// =============================================================================
// List SQL Warehouses Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabricksSqlWarehousesNode {}

impl ListDatabricksSqlWarehousesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDatabricksSqlWarehousesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_list_sql_warehouses",
            "List SQL Warehouses",
            "List all SQL warehouses in the Databricks workspace",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "listSqlWarehouses");
        node.add_icon("/flow/icons/databricks.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger the warehouse listing",
            VariableType::Execution,
        );

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
            "warehouses",
            "Warehouses",
            "Array of SQL warehouses",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<DatabricksSqlWarehouse>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "count",
            "Count",
            "Number of warehouses returned",
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
                .set_performance(7)
                .set_governance(7)
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

        let url = provider.api_url("/sql/warehouses");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let warehouses_array = data["warehouses"].as_array();
                    let warehouses: Vec<DatabricksSqlWarehouse> = warehouses_array
                        .map(|arr| arr.iter().filter_map(parse_warehouse).collect())
                        .unwrap_or_default();

                    let count = warehouses.len();

                    context
                        .set_pin_value("warehouses", json!(warehouses))
                        .await?;
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
// Start SQL Warehouse Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct StartDatabricksSqlWarehouseNode {}

impl StartDatabricksSqlWarehouseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StartDatabricksSqlWarehouseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_start_sql_warehouse",
            "Start SQL Warehouse",
            "Start a stopped SQL warehouse",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "startSqlWarehouse");
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
            "warehouse_id",
            "Warehouse ID",
            "The ID of the SQL warehouse to start",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when start request is accepted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
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
                .set_security(7)
                .set_performance(6)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(5)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let warehouse_id: String = context.evaluate_pin("warehouse_id").await?;

        if warehouse_id.is_empty() {
            context
                .set_pin_value("error_message", json!("Warehouse ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/sql/warehouses/{}/start", warehouse_id));

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
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
// Stop SQL Warehouse Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct StopDatabricksSqlWarehouseNode {}

impl StopDatabricksSqlWarehouseNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for StopDatabricksSqlWarehouseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_stop_sql_warehouse",
            "Stop SQL Warehouse",
            "Stop a running SQL warehouse",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "stopSqlWarehouse");
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
            "warehouse_id",
            "Warehouse ID",
            "The ID of the SQL warehouse to stop",
            VariableType::String,
        );

        node.add_output_pin(
            "exec_out",
            "Success",
            "Triggered when stop request is accepted",
            VariableType::Execution,
        );

        node.add_output_pin(
            "error",
            "Error",
            "Triggered on error",
            VariableType::Execution,
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
                .set_security(7)
                .set_performance(9)
                .set_governance(7)
                .set_reliability(8)
                .set_cost(10)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let warehouse_id: String = context.evaluate_pin("warehouse_id").await?;

        if warehouse_id.is_empty() {
            context
                .set_pin_value("error_message", json!("Warehouse ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url(&format!("/sql/warehouses/{}/stop", warehouse_id));

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
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
