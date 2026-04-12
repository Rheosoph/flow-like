use crate::data::excel::CSVTable;
use crate::data::path::FlowPath;
#[cfg(feature = "execute")]
use bincode;
#[cfg(feature = "execute")]
use calamine::{Data, Range, Reader, open_workbook_auto_from_rs};
use flow_like::bit::Bit;
use flow_like::flow::node::NodeLogic;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::Node,
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::Value;
#[cfg(feature = "execute")]
use flow_like_types::json::{self, Deserialize, Serialize};
use flow_like_types::{JsonSchema, async_trait, json::json, tokio};
#[cfg(feature = "execute")]
use rig::completion::{Completion, Message, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{AssistantContent, ToolCall, ToolChoice, ToolFunction};
#[cfg(feature = "execute")]
use rig::tool::Tool;
use serde::{Deserialize as SerdeDeserialize, Serialize as SerdeSerialize};
#[cfg(feature = "execute")]
use std::fmt;
#[cfg(feature = "execute")]
use std::io::Cursor;

#[cfg(feature = "execute")]
const MAX_CELL_PREVIEW_LEN: usize = 50;

#[derive(Clone, Debug, SerdeSerialize, SerdeDeserialize, JsonSchema)]
pub struct AIExtractionConfig {
    pub sample_rows: usize,
    pub sample_cols: usize,
    pub include_statistics: bool,
    pub detect_merged_regions: bool,
}

impl Default for AIExtractionConfig {
    fn default() -> Self {
        Self {
            sample_rows: 15,
            sample_cols: 20,
            include_statistics: true,
            detect_merged_regions: true,
        }
    }
}

#[cfg(feature = "execute")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TableExtractionSpec {
    /// Human-readable name/identifier for this table (e.g., "Main Data", "Summary Table")
    table_name: Option<String>,
    header_row: usize,
    data_start_row: usize,
    data_end_row: Option<usize>,
    start_column: usize,
    end_column: Option<usize>,
    skip_rows: Vec<usize>,
    header_names: Option<Vec<String>>,
}

#[cfg(feature = "execute")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractionStrategy {
    /// List of tables to extract from this sheet
    tables: Vec<TableExtractionSpec>,
    /// Brief explanation of why you chose this extraction strategy
    reasoning: String,
}

#[cfg(feature = "execute")]
#[derive(Debug, Serialize, Deserialize)]
struct SubmitExtractionTool {
    schema: Value,
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct ExtractionError(String);

#[cfg(feature = "execute")]
impl fmt::Display for ExtractionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Extraction strategy error: {}", self.0)
    }
}

#[cfg(feature = "execute")]
impl std::error::Error for ExtractionError {}

#[cfg(feature = "execute")]
impl Tool for SubmitExtractionTool {
    const NAME: &'static str = "submit_extraction_strategy";
    type Error = ExtractionError;
    type Args = ExtractionStrategy;
    type Output = ExtractionStrategy;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit your table extraction strategy. Analyze the sheet sample and determine which tables to extract. You can extract multiple tables from a single sheet.".to_string(),
            parameters: self.schema.clone(),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        if args.tables.is_empty() {
            return Err(ExtractionError(
                "At least one table must be specified".to_string(),
            ));
        }
        for (i, table) in args.tables.iter().enumerate() {
            if table.header_row >= table.data_start_row && table.data_start_row > 0 {
                return Err(ExtractionError(format!(
                    "Table {}: Header row must be before data start row",
                    i + 1
                )));
            }
        }
        Ok(args)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(feature = "execute")]
fn build_sheet_sample(
    range: &Range<Data>,
    config: &AIExtractionConfig,
) -> (String, usize, usize, SheetStatistics) {
    let height = range.get_size().0;
    let width = range.get_size().1;

    let mut stats = SheetStatistics {
        total_rows: height,
        total_cols: width,
        non_empty_cells: 0,
        empty_rows: Vec::new(),
        potential_header_rows: Vec::new(),
        numeric_columns: Vec::new(),
        text_columns: Vec::new(),
    };

    let sample_rows = config.sample_rows.min(height);
    let sample_cols = config.sample_cols.min(width);

    let mut sample_grid: Vec<Vec<String>> = Vec::with_capacity(sample_rows);
    let mut col_stats: Vec<ColumnStats> = vec![
        ColumnStats {
            numeric_count: 0,
            text_count: 0,
            empty_count: 0,
        };
        sample_cols
    ];

    for (r_idx, row) in range.rows().take(sample_rows).enumerate() {
        let mut row_cells: Vec<String> = Vec::with_capacity(sample_cols);
        let mut row_empty = true;
        let mut row_text_ratio = 0usize;

        for (c_idx, cell) in row.iter().take(sample_cols).enumerate() {
            let cell_str = data_to_preview_string(cell, MAX_CELL_PREVIEW_LEN);
            let trimmed = cell_str.trim();

            if !trimmed.is_empty() {
                stats.non_empty_cells += 1;
                row_empty = false;

                if is_likely_numeric(trimmed) {
                    col_stats[c_idx].numeric_count += 1;
                } else {
                    col_stats[c_idx].text_count += 1;
                    row_text_ratio += 1;
                }
            } else {
                col_stats[c_idx].empty_count += 1;
            }

            row_cells.push(cell_str);
        }

        if row_empty {
            stats.empty_rows.push(r_idx);
        } else if row_text_ratio > sample_cols / 2 {
            stats.potential_header_rows.push(r_idx);
        }

        sample_grid.push(row_cells);
    }

    for (c_idx, cs) in col_stats.iter().enumerate() {
        if cs.numeric_count > cs.text_count && cs.numeric_count > 0 {
            stats.numeric_columns.push(c_idx);
        } else if cs.text_count > 0 {
            stats.text_columns.push(c_idx);
        }
    }

    let mut output = String::with_capacity(sample_rows * sample_cols * 20);
    output.push_str(&format!(
        "Sheet dimensions: {} rows × {} columns\n",
        height, width
    ));
    output.push_str(&format!(
        "Showing sample: {} rows × {} columns\n\n",
        sample_rows, sample_cols
    ));

    output.push_str("Sample data (row numbers are 0-indexed):\n");
    output.push_str("─".repeat(80).as_str());
    output.push('\n');

    for (r_idx, row) in sample_grid.iter().enumerate() {
        output.push_str(&format!("Row {:>3}: ", r_idx));
        for (c_idx, cell) in row.iter().enumerate() {
            if c_idx > 0 {
                output.push_str(" | ");
            }
            let display = if cell.len() > 15 {
                format!("{}…", &cell[..14])
            } else {
                cell.clone()
            };
            output.push_str(&format!("{:<15}", display));
        }
        output.push('\n');
    }

    if config.include_statistics {
        output.push('\n');
        output.push_str("─".repeat(80).as_str());
        output.push_str("\nStatistics:\n");
        output.push_str(&format!("- Empty rows in sample: {:?}\n", stats.empty_rows));
        output.push_str(&format!(
            "- Potential header rows (text-heavy): {:?}\n",
            stats.potential_header_rows
        ));
        output.push_str(&format!(
            "- Columns with mostly numbers: {:?}\n",
            stats.numeric_columns
        ));
        output.push_str(&format!(
            "- Columns with mostly text: {:?}\n",
            stats.text_columns
        ));
    }

    (output, height, width, stats)
}

#[cfg(feature = "execute")]
#[derive(Debug, Clone)]
struct ColumnStats {
    numeric_count: usize,
    text_count: usize,
    empty_count: usize,
}

#[cfg(feature = "execute")]
#[derive(Debug, Clone)]
struct SheetStatistics {
    total_rows: usize,
    total_cols: usize,
    non_empty_cells: usize,
    empty_rows: Vec<usize>,
    potential_header_rows: Vec<usize>,
    numeric_columns: Vec<usize>,
    text_columns: Vec<usize>,
}

#[cfg(feature = "execute")]
fn data_to_preview_string(v: &Data, max_len: usize) -> String {
    let s = match v {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 && *f >= -9_007_199_254_740_992.0 && *f <= 9_007_199_254_740_992.0 {
                format!("{:.0}", f)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(serial) => format!("DATE:{:.2}", serial),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERR:{:?}", e),
    };

    if s.len() > max_len {
        format!("{}…", &s[..max_len - 1])
    } else {
        s
    }
}

#[cfg(feature = "execute")]
fn is_likely_numeric(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }

    let cleaned: String = trimmed
        .chars()
        .filter(|c| !matches!(c, ',' | ' ' | '$' | '€' | '£' | '%'))
        .collect();

    cleaned.parse::<f64>().is_ok()
}

#[cfg(feature = "execute")]
fn apply_table_spec(
    range: &Range<Data>,
    spec: &TableExtractionSpec,
    total_rows: usize,
    total_cols: usize,
) -> flow_like_types::Result<(Vec<String>, Vec<Vec<String>>)> {
    let header_row = spec.header_row.min(total_rows.saturating_sub(1));
    let data_start = spec.data_start_row.min(total_rows);
    let data_end = spec.data_end_row.unwrap_or(total_rows).min(total_rows);
    let start_col = spec.start_column.min(total_cols.saturating_sub(1));
    let end_col = spec.end_column.unwrap_or(total_cols).min(total_cols);

    let skip_set: std::collections::HashSet<usize> = spec.skip_rows.iter().copied().collect();

    let headers: Vec<String> = if let Some(ref custom_headers) = spec.header_names {
        custom_headers.clone()
    } else {
        let mut hdrs = Vec::with_capacity(end_col - start_col);
        if let Some(row) = range.rows().nth(header_row) {
            for cell in row.iter().skip(start_col).take(end_col - start_col) {
                hdrs.push(data_to_string(cell));
            }
        }
        if hdrs.iter().all(|h| h.trim().is_empty()) {
            (start_col..end_col)
                .map(|i| format!("Column_{}", i + 1))
                .collect()
        } else {
            hdrs
        }
    };

    let mut rows: Vec<Vec<String>> = Vec::new();
    for (r_idx, row) in range.rows().enumerate() {
        if r_idx < data_start || r_idx >= data_end {
            continue;
        }
        if skip_set.contains(&r_idx) {
            continue;
        }

        let mut row_data: Vec<String> = Vec::with_capacity(end_col - start_col);
        for cell in row.iter().skip(start_col).take(end_col - start_col) {
            row_data.push(data_to_string(cell));
        }

        if row_data.iter().any(|c| !c.trim().is_empty()) {
            while row_data.len() < headers.len() {
                row_data.push(String::new());
            }
            rows.push(row_data);
        }
    }

    Ok((headers, rows))
}

#[cfg(feature = "execute")]
fn data_to_string(v: &Data) -> String {
    match v {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Float(f) => {
            if f.fract() == 0.0 && *f >= -9_007_199_254_740_992.0 && *f <= 9_007_199_254_740_992.0 {
                format!("{:.0}", f)
            } else {
                f.to_string()
            }
        }
        Data::Int(i) => i.to_string(),
        Data::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Data::DateTime(serial) => serial.to_string(),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("#ERROR:{:?}", e),
    }
}

#[cfg(feature = "execute")]
fn get_extraction_tool_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tables": {
                "type": "array",
                "description": "List of tables to extract from the sheet. Most sheets have one table, but specify multiple if you detect multiple distinct tables.",
                "items": {
                    "type": "object",
                    "properties": {
                        "table_name": {
                            "type": ["string", "null"],
                            "description": "Optional human-readable name for this table (e.g., 'Sales Data', 'Summary')."
                        },
                        "header_row": {
                            "type": "integer",
                            "description": "0-indexed row number containing column headers. Use 0 for the first row."
                        },
                        "data_start_row": {
                            "type": "integer",
                            "description": "0-indexed row number where actual data begins (after headers)."
                        },
                        "data_end_row": {
                            "type": ["integer", "null"],
                            "description": "Optional 0-indexed row number where data ends (exclusive). Null means until next table or end."
                        },
                        "start_column": {
                            "type": "integer",
                            "description": "0-indexed column number where the table starts."
                        },
                        "end_column": {
                            "type": ["integer", "null"],
                            "description": "Optional 0-indexed column number where table ends (exclusive). Null means until the last column."
                        },
                        "skip_rows": {
                            "type": "array",
                            "items": { "type": "integer" },
                            "description": "List of 0-indexed row numbers to skip (e.g., subtotals, section headers within data)."
                        },
                        "header_names": {
                            "type": ["array", "null"],
                            "items": { "type": "string" },
                            "description": "Optional custom header names if the sheet headers are unclear or missing."
                        }
                    },
                    "required": ["header_row", "data_start_row", "start_column", "skip_rows"],
                    "additionalProperties": false
                },
                "minItems": 1
            },
            "reasoning": {
                "type": "string",
                "description": "Brief explanation of why you chose this extraction strategy, including why you identified the number of tables you did."
            }
        },
        "required": ["tables", "reasoning"],
        "additionalProperties": false
    })
}

#[crate::register_node]
#[derive(Default)]
pub struct ExtractExcelTablesAINode {}

impl ExtractExcelTablesAINode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExtractExcelTablesAINode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_excel_extract_tables_ai",
            "Extract Tables AI (Excel)",
            "Uses AI to intelligently extract tables from complex Excel worksheets with unusual layouts",
            "Data/Excel",
        );
        node.add_icon("/flow/icons/file-spreadsheet.svg");
        node.set_version(1);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin(
            "model",
            "Model",
            "AI model for analysis",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("file", "File", "Excel file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "sheet",
            "Sheet",
            "Worksheet name (optional - if empty, extracts from all sheets)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "user_hint",
            "User Hint",
            "Optional guidance for the AI (e.g., 'The table starts at row 5', 'Skip rows with TOTAL')",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "config",
            "Config",
            "AI extraction configuration",
            VariableType::Struct,
        )
        .set_schema::<AIExtractionConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(AIExtractionConfig::default())));

        node.add_output_pin("exec_out", "Output", "Next", VariableType::Execution);
        node.add_output_pin("tables", "Tables", "Extracted tables", VariableType::Struct)
            .set_schema::<CSVTable>()
            .set_value_type(ValueType::Array);
        node.add_output_pin(
            "reasoning",
            "Reasoning",
            "AI's explanation of extraction strategy",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let flow_path: FlowPath = context.evaluate_pin("file").await?;
        let sheet_input: String = context.evaluate_pin("sheet").await.unwrap_or_default();
        let user_hint: String = context.evaluate_pin("user_hint").await.unwrap_or_default();
        let config: AIExtractionConfig = context.evaluate_pin("config").await.unwrap_or_default();

        let file_buffer = flow_path.get(context, false).await?;
        let file_buffer_clone = file_buffer.clone();

        // Determine which sheets to process
        let sheets_to_process: Vec<String> = if sheet_input.trim().is_empty() {
            // Get all sheet names
            tokio::task::spawn_blocking(move || -> flow_like_types::Result<Vec<String>> {
                let cursor = Cursor::new(&file_buffer_clone);
                let wb = open_workbook_auto_from_rs(cursor)?;
                Ok(wb.sheet_names().to_vec())
            })
            .await??
        } else {
            vec![sheet_input]
        };

        let mut all_csv_tables: Vec<CSVTable> = Vec::new();
        let mut all_reasoning: Vec<String> = Vec::new();

        for sheet_name in sheets_to_process {
            let file_buffer_for_sample = file_buffer.clone();
            let sheet_for_sample = sheet_name.clone();
            let config_clone = config.clone();

            // Build sample for this sheet
            let sample_result =
                tokio::task::spawn_blocking(move || -> flow_like_types::Result<_> {
                    let cursor = Cursor::new(&file_buffer_for_sample);
                    let mut wb = open_workbook_auto_from_rs(cursor)?;
                    let range: Range<Data> = wb.worksheet_range(&sheet_for_sample)?;
                    let (sample, rows, cols, stats) = build_sheet_sample(&range, &config_clone);
                    let range_bytes = bincode::serialize(&file_buffer_for_sample)?;
                    Ok((sample, rows, cols, range_bytes, stats))
                })
                .await?;

            let (sample_text, total_rows, total_cols, range_bytes, _stats) = match sample_result {
                Ok(data) => data,
                Err(e) => {
                    all_reasoning.push(format!(
                        "Sheet '{}': Skipped due to error: {}",
                        sheet_name, e
                    ));
                    continue;
                }
            };

            let system_prompt = build_system_prompt(&user_hint);
            let user_prompt = format!(
                "Analyze this Excel sheet '{}' sample and determine the best extraction strategy:\n\n{}",
                sheet_name, sample_text
            );

            let tool_schema = get_extraction_tool_schema();

            let agent = model_bit
                .agent(context, &None)
                .await?
                .preamble(&system_prompt)
                .tool(SubmitExtractionTool {
                    schema: tool_schema,
                })
                .tool_choice(ToolChoice::Required)
                .build();

            let response = agent
                .completion(user_prompt, Vec::<Message>::new())
                .await
                .map_err(|e| {
                    flow_like_types::anyhow!(
                        "AI completion failed for sheet '{}': {}",
                        sheet_name,
                        e
                    )
                })?
                .send()
                .await
                .map_err(|e| {
                    flow_like_types::anyhow!(
                        "Failed to send completion for sheet '{}': {}",
                        sheet_name,
                        e
                    )
                })?;

            let mut strategy: Option<ExtractionStrategy> = None;
            for content in response.choice {
                if let AssistantContent::ToolCall(ToolCall {
                    function:
                        ToolFunction {
                            name, arguments, ..
                        },
                    ..
                }) = content
                    && name == "submit_extraction_strategy"
                {
                    strategy = json::from_value(arguments).ok();
                    break;
                }
            }

            let strategy = match strategy {
                Some(s) => s,
                None => {
                    all_reasoning.push(format!(
                        "Sheet '{}': AI did not return a valid extraction strategy",
                        sheet_name
                    ));
                    continue;
                }
            };

            all_reasoning.push(format!("Sheet '{}': {}", sheet_name, strategy.reasoning));

            // Extract tables using the strategy
            let sheet_for_extract = sheet_name.clone();
            let strategy_clone = strategy.clone();
            let flow_path_clone = flow_path.clone();

            let tables_result =
                tokio::task::spawn_blocking(move || -> flow_like_types::Result<Vec<CSVTable>> {
                    let file_buffer: Vec<u8> = bincode::deserialize(&range_bytes)?;
                    let cursor = Cursor::new(&file_buffer);
                    let mut wb = open_workbook_auto_from_rs(cursor)?;
                    let range: Range<Data> = wb.worksheet_range(&sheet_for_extract)?;

                    let mut tables = Vec::new();
                    for (idx, spec) in strategy_clone.tables.iter().enumerate() {
                        let (headers, rows) =
                            apply_table_spec(&range, spec, total_rows, total_cols)?;

                        let rows_json: Vec<Vec<Value>> = rows
                            .into_iter()
                            .map(|r| r.into_iter().map(|s| json!(s)).collect())
                            .collect();

                        let mut table =
                            CSVTable::new(headers, rows_json, Some(flow_path_clone.clone()));
                        // Add metadata about which sheet and table index
                        if let Some(ref name) = spec.table_name {
                            table.name = Some(format!("{}_{}", sheet_for_extract, name));
                        } else {
                            table.name = Some(format!("{}_{}", sheet_for_extract, idx + 1));
                        }
                        tables.push(table);
                    }
                    Ok(tables)
                })
                .await??;

            all_csv_tables.extend(tables_result);
        }

        let combined_reasoning = all_reasoning.join("\n\n");

        context
            .set_pin_value("tables", json!(all_csv_tables))
            .await?;
        context
            .set_pin_value("reasoning", json!(combined_reasoning))
            .await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "AI extraction requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn build_system_prompt(user_hint: &str) -> String {
    let base = r#"You are an expert data extraction assistant specialized in analyzing Excel spreadsheets with complex, non-standard layouts.

Your task is to analyze a sample of an Excel sheet and determine the optimal extraction strategy. The sample shows the first rows and columns of the sheet.

IMPORTANT: A single sheet may contain MULTIPLE TABLES. Look for:
- Vertical separation: Tables stacked on top of each other with empty rows between
- Horizontal separation: Tables side by side with empty columns between
- Different column structures indicating separate data sets
- Section headers that divide the sheet into multiple logical tables

Common challenges you should handle:
- Headers spanning multiple rows
- Data tables that don't start at row 0 or column 0
- Merged cells creating visual groupings
- Subtotal/total rows interspersed with data
- Section headers within the data area
- Empty rows/columns used as separators
- Multiple tables in a single sheet (EXTRACT EACH AS A SEPARATE TABLE)
- Metadata/title rows before the actual table

When analyzing:
1. First, identify HOW MANY distinct tables exist in the sheet
2. For EACH table, identify where the column headers are located
3. Determine where the data rows begin and end for each table
4. Identify any rows that should be skipped (totals, section breaks)
5. Note the column range containing meaningful data
6. Consider if custom header names would improve clarity
7. Give each table a descriptive name if you can identify its purpose

Always provide your reasoning so users understand your extraction logic, especially explaining why you identified the number of tables you did."#;

    if user_hint.trim().is_empty() {
        base.to_string()
    } else {
        format!("{}\n\nUser guidance: {}", base, user_hint)
    }
}
