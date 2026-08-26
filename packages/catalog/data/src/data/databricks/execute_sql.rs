use super::provider::{DATABRICKS_PROVIDER_ID, DatabricksProvider};
use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    JsonSchema, Value, async_trait,
    json::{Map, json},
    reqwest,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksSqlColumn {
    pub name: String,
    pub type_name: String,
    pub type_text: String,
    pub position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DatabricksSqlResult {
    pub statement_id: String,
    pub status: String,
    pub columns: Vec<DatabricksSqlColumn>,
    pub data: Vec<Vec<Value>>,
    pub row_count: i64,
    pub truncated: bool,
}

// =============================================================================
// Execute SQL Statement Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ExecuteDatabricksSqlNode {}

impl ExecuteDatabricksSqlNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExecuteDatabricksSqlNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_execute_sql",
            "Execute SQL",
            "Execute a SQL statement on a Databricks SQL warehouse. Supports SELECT, INSERT, UPDATE, DELETE, and DDL statements.",
            "Data/Databricks",
        );
        node.set_flowscript_name("databricks", "executeSql");
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
            "The SQL warehouse ID to execute the statement on",
            VariableType::String,
        );

        node.add_input_pin(
            "statement",
            "SQL Statement",
            "The SQL statement to execute",
            VariableType::String,
        );

        node.add_input_pin(
            "catalog",
            "Catalog",
            "Optional: The catalog to use (Unity Catalog)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "schema",
            "Schema",
            "Optional: The schema to use",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "row_limit",
            "Row Limit",
            "Maximum number of rows to return (default: 10000)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(10000)));

        node.add_input_pin(
            "wait_timeout",
            "Wait Timeout",
            "Timeout in seconds for synchronous execution (default: 50s, max: 50s)",
            VariableType::String,
        )
        .set_default_value(Some(json!("50s")));

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
            "result",
            "Result",
            "SQL execution result",
            VariableType::Struct,
        )
        .set_schema::<DatabricksSqlResult>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "rows",
            "Rows",
            "Result rows as JSON array",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin(
            "row_count",
            "Row Count",
            "Number of rows returned",
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
                .set_privacy(5)
                .set_security(7)
                .set_performance(7)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(6)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let warehouse_id: String = context.evaluate_pin("warehouse_id").await?;
        let statement: String = context.evaluate_pin("statement").await?;
        let catalog: String = context.evaluate_pin("catalog").await.unwrap_or_default();
        let schema: String = context.evaluate_pin("schema").await.unwrap_or_default();
        let row_limit: i64 = context.evaluate_pin("row_limit").await.unwrap_or(10000);
        let wait_timeout: String = context
            .evaluate_pin("wait_timeout")
            .await
            .unwrap_or_else(|_| "50s".to_string());

        if warehouse_id.is_empty() {
            context
                .set_pin_value("error_message", json!("Warehouse ID is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        if statement.is_empty() {
            context
                .set_pin_value("error_message", json!("SQL statement is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url("/sql/statements");

        let mut body = json!({
            "warehouse_id": warehouse_id,
            "statement": statement,
            "wait_timeout": wait_timeout,
            "on_wait_timeout": "CANCEL",
            "row_limit": row_limit
        });

        if !catalog.is_empty() {
            body["catalog"] = json!(catalog);
        }
        if !schema.is_empty() {
            body["schema"] = json!(schema);
        }

        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let status = data["status"]["state"].as_str().unwrap_or("UNKNOWN");

                    if status != "SUCCEEDED" {
                        let detail = data["status"]["error"]["message"]
                            .as_str()
                            .unwrap_or("Statement did not complete successfully");
                        let error = format!("Databricks SQL statement ended in {status}: {detail}");
                        context.log_message(&error, LogLevel::Error);
                        context.set_pin_value("error_message", json!(error)).await?;
                        context.activate_exec_pin("error").await?;
                        return Ok(());
                    }

                    let columns: Vec<DatabricksSqlColumn> = data["manifest"]["schema"]["columns"]
                        .as_array()
                        .map(|cols| {
                            cols.iter()
                                .enumerate()
                                .filter_map(|(i, col)| {
                                    Some(DatabricksSqlColumn {
                                        name: col["name"].as_str()?.to_string(),
                                        type_name: col["type_name"].as_str()?.to_string(),
                                        type_text: col["type_text"]
                                            .as_str()
                                            .unwrap_or_default()
                                            .to_string(),
                                        position: i as i64,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    let data_array = data["result"]["data_array"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default();

                    let row_count = data_array.len() as i64;
                    let truncated = data["manifest"]["truncated"].as_bool().unwrap_or(false);

                    // Convert data to array of objects for easier use
                    let rows: Vec<Value> = data_array
                        .iter()
                        .map(|row| {
                            let empty = vec![];
                            let row_arr = row.as_array().unwrap_or(&empty);
                            let mut obj = Map::new();
                            for (i, col) in columns.iter().enumerate() {
                                if i < row_arr.len() {
                                    obj.insert(col.name.clone(), row_arr[i].clone());
                                }
                            }
                            Value::Object(obj)
                        })
                        .collect();

                    let result = DatabricksSqlResult {
                        statement_id: data["statement_id"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        status: status.to_string(),
                        columns,
                        data: data_array
                            .iter()
                            .map(|r| r.as_array().cloned().unwrap_or_default())
                            .collect(),
                        row_count,
                        truncated,
                    };

                    context.set_pin_value("result", json!(result)).await?;
                    context.set_pin_value("rows", json!(rows)).await?;
                    context.set_pin_value("row_count", json!(row_count)).await?;
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
