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
pub struct DatabricksFileInfo {
    pub path: String,
    pub is_dir: bool,
    pub file_size: i64,
    pub modification_time: i64,
}

fn parse_file_info(file: &Value) -> Option<DatabricksFileInfo> {
    Some(DatabricksFileInfo {
        path: file["path"].as_str()?.to_string(),
        is_dir: file["is_dir"].as_bool().unwrap_or(false),
        file_size: file["file_size"].as_i64().unwrap_or(0),
        modification_time: file["modification_time"].as_i64().unwrap_or(0),
    })
}

// =============================================================================
// List DBFS Files Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ListDatabricksDbfsNode {}

impl ListDatabricksDbfsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListDatabricksDbfsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_list_dbfs",
            "List DBFS Files",
            "List files and directories in the Databricks File System (DBFS)",
            "Data/Databricks/DBFS",
        );
        node.set_flowscript_name("databricks", "listDbfs");
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
            "path",
            "Path",
            "DBFS path to list (e.g., /FileStore, /mnt)",
            VariableType::String,
        )
        .set_default_value(Some(json!("/")));

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
            "files",
            "Files",
            "Array of files and directories",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<DatabricksFileInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin("count", "Count", "Number of items", VariableType::Integer);

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
        let path: String = context
            .evaluate_pin("path")
            .await
            .unwrap_or_else(|_| "/".to_string());

        let url = provider.api_url("/dbfs/list");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("path", &path)])
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let files_array = data["files"].as_array();
                    let files: Vec<DatabricksFileInfo> = files_array
                        .map(|arr| arr.iter().filter_map(parse_file_info).collect())
                        .unwrap_or_default();

                    let count = files.len();

                    context.set_pin_value("files", json!(files)).await?;
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
// Read DBFS File Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ReadDatabricksDbfsNode {}

impl ReadDatabricksDbfsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReadDatabricksDbfsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_read_dbfs",
            "Read DBFS File",
            "Read the contents of a file from DBFS. Returns base64 encoded content for binary files.",
            "Data/Databricks/DBFS",
        );
        node.set_flowscript_name("databricks", "readDbfs");
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
            "path",
            "Path",
            "DBFS path of the file to read",
            VariableType::String,
        );

        node.add_input_pin(
            "offset",
            "Offset",
            "Byte offset to start reading from",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));

        node.add_input_pin(
            "length",
            "Length",
            "Number of bytes to read (max 1MB)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1048576))); // 1MB default

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
            "content",
            "Content",
            "File content (base64 encoded for binary files)",
            VariableType::String,
        );

        node.add_output_pin(
            "bytes_read",
            "Bytes Read",
            "Number of bytes read",
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
                .set_performance(6)
                .set_governance(6)
                .set_reliability(8)
                .set_cost(7)
                .build(),
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: DatabricksProvider = context.evaluate_pin("provider").await?;
        let path: String = context.evaluate_pin("path").await?;
        let offset: i64 = context.evaluate_pin("offset").await.unwrap_or(0);
        let length: i64 = context.evaluate_pin("length").await.unwrap_or(1048576);

        if path.is_empty() {
            context
                .set_pin_value("error_message", json!("File path is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url("/dbfs/read");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[
                ("path", path.as_str()),
                ("offset", &offset.to_string()),
                ("length", &length.clamp(1, 1048576).to_string()),
            ])
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    let content = data["data"].as_str().unwrap_or_default();
                    let bytes_read = data["bytes_read"].as_i64().unwrap_or(0);

                    context.set_pin_value("content", json!(content)).await?;
                    context
                        .set_pin_value("bytes_read", json!(bytes_read))
                        .await?;
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
// Get DBFS File Status Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetDatabricksDbfsStatusNode {}

impl GetDatabricksDbfsStatusNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetDatabricksDbfsStatusNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_databricks_get_dbfs_status",
            "Get DBFS Status",
            "Get the status (metadata) of a file or directory in DBFS",
            "Data/Databricks/DBFS",
        );
        node.set_flowscript_name("databricks", "getDbfsStatus");
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

        node.add_input_pin("path", "Path", "DBFS path to check", VariableType::String);

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
            "file_info",
            "File Info",
            "File or directory information",
            VariableType::Struct,
        )
        .set_schema::<DatabricksFileInfo>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exists",
            "Exists",
            "Whether the path exists",
            VariableType::Boolean,
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
                .set_performance(9)
                .set_governance(7)
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
        let path: String = context.evaluate_pin("path").await?;

        if path.is_empty() {
            context
                .set_pin_value("error_message", json!("Path is required"))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        let url = provider.api_url("/dbfs/get-status");

        let client = reqwest::Client::new();
        let response = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("path", &path)])
            .send()
            .await;

        match response {
            Ok(resp) => {
                if resp.status().is_success() {
                    let data: Value = resp
                        .json()
                        .await
                        .map_err(|e| flow_like_types::anyhow!("Failed to parse response: {}", e))?;

                    if let Some(file_info) = parse_file_info(&data) {
                        context.set_pin_value("file_info", json!(file_info)).await?;
                        context.set_pin_value("exists", json!(true)).await?;
                        context.set_pin_value("error_message", json!("")).await?;
                        context.activate_exec_pin("exec_out").await?;
                    } else {
                        context.set_pin_value("exists", json!(false)).await?;
                        context
                            .set_pin_value("error_message", json!("Path not found"))
                            .await?;
                        context.activate_exec_pin("error").await?;
                    }
                } else if resp.status().as_u16() == 404 {
                    context.set_pin_value("exists", json!(false)).await?;
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
