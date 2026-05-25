use super::{
    graph::{
        graph_error_message, graph_get_json, graph_get_paginated_values, normalize_graph_path,
    },
    provider::{MICROSOFT_PROVIDER_ID, MicrosoftGraphProvider},
};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{
    JsonSchema, Value, async_trait,
    json::{self, json},
    reqwest,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ExcelWorksheet {
    pub id: String,
    pub name: String,
    pub position: i64,
    pub visibility: String,
}

fn workbook_api_path(file_path: &str, suffix: &str) -> String {
    format!(
        "/me/drive/root:{}:/workbook/{}",
        normalize_graph_path(file_path),
        suffix
    )
}

#[crate::register_node]
#[derive(Default)]
pub struct ListExcelWorksheetsNode {}

impl ListExcelWorksheetsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ListExcelWorksheetsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_excel_list_worksheets",
            "List Excel Worksheets",
            "List worksheets in an Excel workbook stored in OneDrive",
            "Data/Microsoft/Excel",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to Excel file in OneDrive",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("worksheets", "Worksheets", "", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<ExcelWorksheet>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Files.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;

        let client = reqwest::Client::new();
        match graph_get_paginated_values(
            &client,
            &provider,
            provider.api_url(&workbook_api_path(&file_path, "worksheets")),
        )
        .await
        {
            Ok(values) => {
                let worksheets: Vec<ExcelWorksheet> = values
                    .iter()
                    .filter_map(|w| {
                        Some(ExcelWorksheet {
                            id: w["id"].as_str()?.to_string(),
                            name: w["name"].as_str()?.to_string(),
                            position: w["position"].as_i64().unwrap_or(0),
                            visibility: w["visibility"].as_str().unwrap_or("Visible").to_string(),
                        })
                    })
                    .collect();
                context
                    .set_pin_value("worksheets", json!(worksheets))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ReadExcelRangeNode {}

impl ReadExcelRangeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReadExcelRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_excel_read_range",
            "Read Excel Range",
            "Read data from a range in an Excel worksheet",
            "Data/Microsoft/Excel",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to Excel file in OneDrive",
            VariableType::String,
        );
        node.add_input_pin(
            "worksheet",
            "Worksheet",
            "Worksheet name",
            VariableType::String,
        );
        node.add_input_pin("range", "Range", "A1 notation range", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "values",
            "Values",
            "2D array of cell values",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin("row_count", "Row Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Files.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;
        let worksheet: String = context.evaluate_pin("worksheet").await?;
        let range: String = context.evaluate_pin("range").await?;

        let client = reqwest::Client::new();
        let url = provider.api_url(&workbook_api_path(
            &file_path,
            &format!(
                "worksheets/{}/range(address='{}')",
                urlencoding::encode(&worksheet),
                urlencoding::encode(&range)
            ),
        ));

        match graph_get_json(&client, &provider, url).await {
            Ok(body) => {
                let values = body["values"].clone();
                let row_count = values.as_array().map(|arr| arr.len()).unwrap_or(0) as i64;
                context.set_pin_value("values", values).await?;
                context.set_pin_value("row_count", json!(row_count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct WriteExcelRangeNode {}

impl WriteExcelRangeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WriteExcelRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_excel_write_range",
            "Write Excel Range",
            "Write data to a range in an Excel worksheet",
            "Data/Microsoft/Excel",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to Excel file in OneDrive",
            VariableType::String,
        );
        node.add_input_pin(
            "worksheet",
            "Worksheet",
            "Worksheet name",
            VariableType::String,
        );
        node.add_input_pin("range", "Range", "A1 notation range", VariableType::String);
        node.add_input_pin(
            "values",
            "Values",
            "2D array of values to write",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Files.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;
        let worksheet: String = context.evaluate_pin("worksheet").await?;
        let range: String = context.evaluate_pin("range").await?;
        let values: Value = context.evaluate_pin("values").await?;

        let body = json!({ "values": values });

        let client = reqwest::Client::new();
        let url = provider.api_url(&workbook_api_path(
            &file_path,
            &format!(
                "worksheets/{}/range(address='{}')",
                urlencoding::encode(&worksheet),
                urlencoding::encode(&range)
            ),
        ));
        let response = client
            .patch(url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetExcelUsedRangeNode {}

impl GetExcelUsedRangeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetExcelUsedRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_excel_used_range",
            "Get Excel Used Range",
            "Get the used range of a worksheet",
            "Data/Microsoft/Excel",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to Excel file",
            VariableType::String,
        );
        node.add_input_pin(
            "worksheet",
            "Worksheet",
            "Worksheet name",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "values",
            "Values",
            "2D array of data",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "address",
            "Address",
            "A1 notation address",
            VariableType::String,
        );
        node.add_output_pin("row_count", "Row Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Files.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;
        let worksheet: String = context.evaluate_pin("worksheet").await?;

        let client = reqwest::Client::new();
        let url = provider.api_url(&workbook_api_path(
            &file_path,
            &format!("worksheets/{}/usedRange", urlencoding::encode(&worksheet)),
        ));

        match graph_get_json(&client, &provider, url).await {
            Ok(body) => {
                let values = body["values"].clone();
                let address = body["address"].as_str().unwrap_or("").to_string();
                let row_count = body["rowCount"].as_i64().unwrap_or(0);
                context.set_pin_value("values", values).await?;
                context.set_pin_value("address", json!(address)).await?;
                context.set_pin_value("row_count", json!(row_count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GetExcelTableNode {}

impl GetExcelTableNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetExcelTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_excel_get_table",
            "Get Excel Table",
            "Get data from an Excel table by name",
            "Data/Microsoft/Excel",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to Excel file",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name of the table",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("rows", "Rows", "Table rows as array", VariableType::Generic)
            .set_value_type(ValueType::Array);
        node.add_output_pin("headers", "Headers", "Column headers", VariableType::String)
            .set_value_type(ValueType::Array);
        node.add_output_pin("row_count", "Row Count", "", VariableType::Integer);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Files.Read.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;

        let client = reqwest::Client::new();

        let headers_url = provider.api_url(&workbook_api_path(
            &file_path,
            &format!("tables/{}/headerRowRange", urlencoding::encode(&table_name)),
        ));
        let headers: Vec<String> = match graph_get_json(&client, &provider, headers_url).await {
            Ok(body) => body["values"][0]
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };

        let body_url = provider.api_url(&workbook_api_path(
            &file_path,
            &format!("tables/{}/dataBodyRange", urlencoding::encode(&table_name)),
        ));

        match graph_get_json(&client, &provider, body_url).await {
            Ok(body) => {
                let rows: Vec<Value> = body["values"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .map(|row| {
                                let mut obj = json::Map::new();
                                if let Some(row_arr) = row.as_array() {
                                    for (i, val) in row_arr.iter().enumerate() {
                                        let key = headers
                                            .get(i)
                                            .cloned()
                                            .unwrap_or_else(|| format!("column_{}", i));
                                        obj.insert(key, val.clone());
                                    }
                                }
                                Value::Object(obj)
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let row_count = rows.len() as i64;
                context.set_pin_value("rows", json!(rows)).await?;
                context.set_pin_value("headers", json!(headers)).await?;
                context.set_pin_value("row_count", json!(row_count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct AddExcelTableRowNode {}

impl AddExcelTableRowNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddExcelTableRowNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_microsoft_excel_add_table_row",
            "Add Excel Table Row",
            "Add a row to an Excel table",
            "Data/Microsoft/Excel",
        );
        node.set_version(1);
        node.add_icon("/flow/icons/microsoft.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Microsoft Graph provider",
            VariableType::Struct,
        )
        .set_schema::<MicrosoftGraphProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "file_path",
            "File Path",
            "Path to Excel file",
            VariableType::String,
        );
        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name of the table",
            VariableType::String,
        );
        node.add_input_pin(
            "values",
            "Values",
            "Array of values for the row",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "row_index",
            "Row Index",
            "Index of added row",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(MICROSOFT_PROVIDER_ID, vec!["Files.ReadWrite.All"]);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: MicrosoftGraphProvider = context.evaluate_pin("provider").await?;
        let file_path: String = context.evaluate_pin("file_path").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;
        let values: Value = context.evaluate_pin("values").await?;

        let body = json!({ "values": [values] });

        let client = reqwest::Client::new();
        let url = provider.api_url(&workbook_api_path(
            &file_path,
            &format!("tables/{}/rows/add", urlencoding::encode(&table_name)),
        ));
        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let row_index = body["index"].as_i64().unwrap_or(0);
                context.set_pin_value("row_index", json!(row_index)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = graph_error_message(resp).await;
                context.set_pin_value("error_message", json!(error)).await?;
                context.activate_exec_pin("error").await?;
            }
            Err(e) => {
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }
        Ok(())
    }
}
