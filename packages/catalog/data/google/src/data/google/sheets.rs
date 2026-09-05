use super::provider::{GOOGLE_PROVIDER_ID, GoogleProvider};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, Value, async_trait, json::json, reqwest};
use serde::{Deserialize, Serialize};

// =============================================================================
// Google Sheets Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleSpreadsheet {
    pub spreadsheet_id: String,
    pub title: String,
    pub locale: Option<String>,
    pub time_zone: Option<String>,
    pub sheets: Vec<GoogleSheet>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GoogleSheet {
    pub sheet_id: i64,
    pub title: String,
    pub index: i64,
    pub row_count: Option<i64>,
    pub column_count: Option<i64>,
}

fn parse_spreadsheet(value: &Value) -> Option<GoogleSpreadsheet> {
    let sheets = value["sheets"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|s| {
                    Some(GoogleSheet {
                        sheet_id: s["properties"]["sheetId"].as_i64()?,
                        title: s["properties"]["title"].as_str()?.to_string(),
                        index: s["properties"]["index"].as_i64().unwrap_or(0),
                        row_count: s["properties"]["gridProperties"]["rowCount"].as_i64(),
                        column_count: s["properties"]["gridProperties"]["columnCount"].as_i64(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Some(GoogleSpreadsheet {
        spreadsheet_id: value["spreadsheetId"].as_str()?.to_string(),
        title: value["properties"]["title"].as_str()?.to_string(),
        locale: value["properties"]["locale"].as_str().map(String::from),
        time_zone: value["properties"]["timeZone"].as_str().map(String::from),
        sheets,
    })
}

// =============================================================================
// Create Spreadsheet Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct CreateGoogleSpreadsheetNode {}

impl CreateGoogleSpreadsheetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateGoogleSpreadsheetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_create",
            "Create Spreadsheet",
            "Create a new Google Spreadsheet",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "create");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("title", "Title", "Spreadsheet title", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("spreadsheet_id", "Spreadsheet ID", "", VariableType::String);
        node.add_output_pin("spreadsheet", "Spreadsheet", "", VariableType::Struct)
            .set_schema::<GoogleSpreadsheet>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let title: String = context.evaluate_pin("title").await?;

        let body = json!({
            "properties": {
                "title": title
            }
        });

        let client = reqwest::Client::new();
        let response = client
            .post("https://sheets.googleapis.com/v4/spreadsheets")
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(spreadsheet) = parse_spreadsheet(&body) {
                    let id = spreadsheet.spreadsheet_id.clone();
                    context.set_pin_value("spreadsheet_id", json!(id)).await?;
                    context
                        .set_pin_value("spreadsheet", json!(spreadsheet))
                        .await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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

// =============================================================================
// Get Spreadsheet Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct GetGoogleSpreadsheetNode {}

impl GetGoogleSpreadsheetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetGoogleSpreadsheetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_get",
            "Get Spreadsheet",
            "Get Google Spreadsheet metadata and sheet list",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "get");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "spreadsheet_id",
            "Spreadsheet ID",
            "ID of the spreadsheet",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("spreadsheet", "Spreadsheet", "", VariableType::Struct)
            .set_schema::<GoogleSpreadsheet>();
        node.add_output_pin("sheets", "Sheets", "List of sheets", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_schema::<GoogleSheet>();
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let spreadsheet_id: String = context.evaluate_pin("spreadsheet_id").await?;

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{}",
                spreadsheet_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                if let Some(spreadsheet) = parse_spreadsheet(&body) {
                    let sheets = spreadsheet.sheets.clone();
                    context
                        .set_pin_value("spreadsheet", json!(spreadsheet))
                        .await?;
                    context.set_pin_value("sheets", json!(sheets)).await?;
                    context.activate_exec_pin("exec_out").await?;
                } else {
                    context
                        .set_pin_value("error_message", json!("Failed to parse response"))
                        .await?;
                    context.activate_exec_pin("error").await?;
                }
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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

// =============================================================================
// Read Range Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ReadGoogleSheetsRangeNode {}

impl ReadGoogleSheetsRangeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReadGoogleSheetsRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_read_range",
            "Read Range",
            "Read data from a Google Sheets range",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "readRange");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("spreadsheet_id", "Spreadsheet ID", "", VariableType::String);
        node.add_input_pin(
            "range",
            "Range",
            "A1 notation range (e.g., 'Sheet1!A1:D10')",
            VariableType::String,
        );
        node.add_input_pin(
            "value_render",
            "Value Render",
            "How values should be rendered",
            VariableType::String,
        )
        .set_default_value(Some(json!("FORMATTED_VALUE")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "FORMATTED_VALUE".to_string(),
                    "UNFORMATTED_VALUE".to_string(),
                    "FORMULA".to_string(),
                ])
                .build(),
        );

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

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets.readonly"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let spreadsheet_id: String = context.evaluate_pin("spreadsheet_id").await?;
        let range: String = context.evaluate_pin("range").await?;
        let value_render: String = context
            .evaluate_pin("value_render")
            .await
            .unwrap_or_else(|_| "FORMATTED_VALUE".to_string());

        let client = reqwest::Client::new();
        let response = client
            .get(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
                spreadsheet_id,
                urlencoding::encode(&range)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .query(&[("valueRenderOption", value_render.as_str())])
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let values = body["values"].clone();
                let row_count = values.as_array().map(|arr| arr.len()).unwrap_or(0) as i64;
                context.set_pin_value("values", values).await?;
                context.set_pin_value("row_count", json!(row_count)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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

// =============================================================================
// Write Range Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct WriteGoogleSheetsRangeNode {}

impl WriteGoogleSheetsRangeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WriteGoogleSheetsRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_write_range",
            "Write Range",
            "Write data to a Google Sheets range",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "writeRange");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("spreadsheet_id", "Spreadsheet ID", "", VariableType::String);
        node.add_input_pin("range", "Range", "A1 notation range", VariableType::String);
        node.add_input_pin(
            "values",
            "Values",
            "2D array of values to write",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);
        node.add_input_pin(
            "value_input",
            "Value Input Option",
            "How input should be interpreted",
            VariableType::String,
        )
        .set_default_value(Some(json!("USER_ENTERED")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["RAW".to_string(), "USER_ENTERED".to_string()])
                .build(),
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "updated_cells",
            "Updated Cells",
            "Number of cells updated",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let spreadsheet_id: String = context.evaluate_pin("spreadsheet_id").await?;
        let range: String = context.evaluate_pin("range").await?;
        let values: Value = context.evaluate_pin("values").await?;
        let value_input: String = context
            .evaluate_pin("value_input")
            .await
            .unwrap_or_else(|_| "USER_ENTERED".to_string());

        let body = json!({
            "values": values
        });

        let client = reqwest::Client::new();
        let response = client
            .put(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}",
                spreadsheet_id,
                urlencoding::encode(&range)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[("valueInputOption", value_input.as_str())])
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let updated_cells = body["updatedCells"].as_i64().unwrap_or(0);
                context
                    .set_pin_value("updated_cells", json!(updated_cells))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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

// =============================================================================
// Append Rows Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct AppendGoogleSheetsRowsNode {}

impl AppendGoogleSheetsRowsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AppendGoogleSheetsRowsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_append_rows",
            "Append Rows",
            "Append rows to the end of a Google Sheets range",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "appendRows");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("spreadsheet_id", "Spreadsheet ID", "", VariableType::String);
        node.add_input_pin(
            "range",
            "Range",
            "A1 notation range (e.g., 'Sheet1!A:D')",
            VariableType::String,
        );
        node.add_input_pin(
            "values",
            "Values",
            "2D array of row values to append",
            VariableType::Generic,
        )
        .set_value_type(ValueType::Array);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "updated_range",
            "Updated Range",
            "Range that was updated",
            VariableType::String,
        );
        node.add_output_pin(
            "updated_rows",
            "Updated Rows",
            "Number of rows appended",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let spreadsheet_id: String = context.evaluate_pin("spreadsheet_id").await?;
        let range: String = context.evaluate_pin("range").await?;
        let values: Value = context.evaluate_pin("values").await?;

        let body = json!({
            "values": values
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}:append",
                spreadsheet_id,
                urlencoding::encode(&range)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .query(&[
                ("valueInputOption", "USER_ENTERED"),
                ("insertDataOption", "INSERT_ROWS"),
            ])
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let updated_range = body["updates"]["updatedRange"]
                    .as_str()
                    .unwrap_or("")
                    .to_string();
                let updated_rows = body["updates"]["updatedRows"].as_i64().unwrap_or(0);
                context
                    .set_pin_value("updated_range", json!(updated_range))
                    .await?;
                context
                    .set_pin_value("updated_rows", json!(updated_rows))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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

// =============================================================================
// Clear Range Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct ClearGoogleSheetsRangeNode {}

impl ClearGoogleSheetsRangeNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ClearGoogleSheetsRangeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_clear_range",
            "Clear Range",
            "Clear values from a Google Sheets range",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "clearRange");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("spreadsheet_id", "Spreadsheet ID", "", VariableType::String);
        node.add_input_pin(
            "range",
            "Range",
            "A1 notation range to clear",
            VariableType::String,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("cleared_range", "Cleared Range", "", VariableType::String);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let spreadsheet_id: String = context.evaluate_pin("spreadsheet_id").await?;
        let range: String = context.evaluate_pin("range").await?;

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{}/values/{}:clear",
                spreadsheet_id,
                urlencoding::encode(&range)
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&json!({}))
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let cleared_range = body["clearedRange"].as_str().unwrap_or("").to_string();
                context
                    .set_pin_value("cleared_range", json!(cleared_range))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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

// =============================================================================
// Add Sheet Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct AddGoogleSheetNode {}

impl AddGoogleSheetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for AddGoogleSheetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_add_sheet",
            "Add Sheet",
            "Add a new sheet to a Google Spreadsheet",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "addSheet");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("spreadsheet_id", "Spreadsheet ID", "", VariableType::String);
        node.add_input_pin("title", "Title", "New sheet title", VariableType::String);

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin(
            "sheet_id",
            "Sheet ID",
            "ID of the new sheet",
            VariableType::Integer,
        );
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let spreadsheet_id: String = context.evaluate_pin("spreadsheet_id").await?;
        let title: String = context.evaluate_pin("title").await?;

        let body = json!({
            "requests": [{
                "addSheet": {
                    "properties": {
                        "title": title
                    }
                }
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{}:batchUpdate",
                spreadsheet_id
            ))
            .header("Authorization", format!("Bearer {}", provider.access_token))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => {
                let body: Value = resp.json().await?;
                let sheet_id = body["replies"][0]["addSheet"]["properties"]["sheetId"]
                    .as_i64()
                    .unwrap_or(0);
                context.set_pin_value("sheet_id", json!(sheet_id)).await?;
                context.activate_exec_pin("exec_out").await?;
            }
            Ok(resp) => {
                let error = resp.text().await.unwrap_or_default();
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

// =============================================================================
// Delete Sheet Node
// =============================================================================

#[crate::register_node]
#[derive(Default)]
pub struct DeleteGoogleSheetNode {}

impl DeleteGoogleSheetNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for DeleteGoogleSheetNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_google_sheets_delete_sheet",
            "Delete Sheet",
            "Delete a sheet from a Google Spreadsheet",
            "Data/Google/Sheets",
        );
        node.set_flowscript_name("google.sheets", "deleteSheet");
        node.add_icon("/flow/icons/google.svg");

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);
        node.add_input_pin(
            "provider",
            "Provider",
            "Google Drive provider",
            VariableType::Struct,
        )
        .set_schema::<GoogleProvider>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("spreadsheet_id", "Spreadsheet ID", "", VariableType::String);
        node.add_input_pin(
            "sheet_id",
            "Sheet ID",
            "ID of sheet to delete",
            VariableType::Integer,
        );

        node.add_output_pin("exec_out", "Success", "", VariableType::Execution);
        node.add_output_pin("error", "Error", "", VariableType::Execution);
        node.add_output_pin("error_message", "Error Message", "", VariableType::String);

        node.add_required_oauth_scopes(
            GOOGLE_PROVIDER_ID,
            vec!["https://www.googleapis.com/auth/spreadsheets"],
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let provider: GoogleProvider = context.evaluate_pin("provider").await?;
        let spreadsheet_id: String = context.evaluate_pin("spreadsheet_id").await?;
        let sheet_id: i64 = context.evaluate_pin("sheet_id").await?;

        let body = json!({
            "requests": [{
                "deleteSheet": {
                    "sheetId": sheet_id
                }
            }]
        });

        let client = reqwest::Client::new();
        let response = client
            .post(format!(
                "https://sheets.googleapis.com/v4/spreadsheets/{}:batchUpdate",
                spreadsheet_id
            ))
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
                let error = resp.text().await.unwrap_or_default();
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
