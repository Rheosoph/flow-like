//! AI-assisted table extraction.
//!
//! The LLM only ever decides *geometry and roles* (table ranges, header depth,
//! rows to skip) — never data. It receives a compact, address-annotated sheet
//! encoding (SpreadsheetLLM-style structural anchors) primed with heuristic
//! candidates, and can interrogate the sheet through narrow tools before
//! committing: `inspect_range` (verbatim cells of a small range),
//! `find_styled_cells` (bold/colored cells without style dumps), and
//! `query_data` (SQL over candidate tables via DataFusion, row-capped).
//! The submitted ranges are validated and extracted deterministically by the
//! shared detection core, so values are never hallucinated.

use crate::data::excel::CSVTable;
use crate::data::excel::table_detect::ExtractConfig;
use crate::data::path::FlowPath;
use flow_like::bit::Bit;
use flow_like::flow::node::NodeLogic;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::Node,
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{JsonSchema, async_trait, json::json};
use serde::{Deserialize, Serialize};

#[cfg(feature = "execute")]
use crate::data::datafusion::query::batches_to_csv_table;
#[cfg(feature = "execute")]
use crate::data::excel::grid::{
    SheetGrid, Workbook, normalize_table_name, parse_a1_range, truncate_chars, unique_table_name,
};
#[cfg(feature = "execute")]
use crate::data::excel::sheet_compressor::{
    EncodeOptions, encode_inverted_index, encode_sheet_compact, render_range,
};
#[cfg(feature = "execute")]
use crate::data::excel::styles::{SheetStyles, color_matches, load_workbook_styles};
#[cfg(feature = "execute")]
use crate::data::excel::table_detect::{
    BuildOverrides, DetectedTable, Rect, build_table_from_rect, detect_table_regions,
    detected_to_csv, extract_tables_from_grid, tighten,
};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
#[cfg(feature = "execute")]
use flow_like_storage::datafusion::prelude::SessionContext;
#[cfg(feature = "execute")]
use flow_like_types::tokio;
#[cfg(feature = "execute")]
use rig::OneOrMany;
#[cfg(feature = "execute")]
use rig::completion::{Completion, Message, ToolDefinition};
#[cfg(feature = "execute")]
use rig::message::{
    AssistantContent, Text, ToolCall, ToolChoice, ToolFunction, ToolResult as RigToolResult,
    ToolResultContent, UserContent,
};
#[cfg(feature = "execute")]
use rig::tool::Tool;
#[cfg(feature = "execute")]
use std::collections::HashSet;
#[cfg(feature = "execute")]
use std::fmt;
#[cfg(feature = "execute")]
use std::sync::Arc;

#[cfg(feature = "execute")]
const MAX_HEADER_ROWS_AI: usize = 5;
#[cfg(feature = "execute")]
const MAX_INSPECT_ROWS: usize = 60;
#[cfg(feature = "execute")]
const MAX_QUERY_RESULT_ROWS: usize = 50;
#[cfg(feature = "execute")]
const MAX_CANDIDATE_SQL_TABLES: usize = 12;
#[cfg(feature = "execute")]
const STYLE_PRELOAD_MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AIExtractionConfig {
    /// Maximum rows rendered verbatim in the prompt (boundary rows are prioritized)
    pub sample_rows: usize,
    /// Maximum columns rendered per row
    pub sample_cols: usize,
    /// Include per-column type profiles in the prompt
    pub include_statistics: bool,
    /// Include the merged-region list in the prompt
    pub detect_merged_regions: bool,
    /// Maximum tool-use turns per sheet before falling back to heuristics
    pub max_tool_turns: usize,
}

impl Default for AIExtractionConfig {
    fn default() -> Self {
        Self {
            sample_rows: 80,
            sample_cols: 30,
            include_statistics: true,
            detect_merged_regions: true,
            max_tool_turns: 6,
        }
    }
}

// ============================ Strategy types ============================

#[cfg(feature = "execute")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct TableSpec {
    table_name: Option<String>,
    /// A1 range including the header rows, e.g. "A3:F42"
    range: String,
    /// Number of header rows at the top of the range (0 = no header)
    header_rows: Option<usize>,
    /// Absolute 1-based row numbers to exclude (subtotals, section breaks)
    #[serde(default)]
    skip_rows: Vec<usize>,
    column_names: Option<Vec<String>>,
}

#[cfg(feature = "execute")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ExtractionStrategy {
    tables: Vec<TableSpec>,
    reasoning: String,
}

#[cfg(feature = "execute")]
fn validate_strategy(strategy: &ExtractionStrategy) -> Result<(), String> {
    if strategy.tables.is_empty() {
        return Err("At least one table must be specified".to_string());
    }
    for (i, t) in strategy.tables.iter().enumerate() {
        if parse_a1_range(&t.range).is_none() {
            return Err(format!(
                "Table {}: '{}' is not a valid A1 range (expected e.g. 'A3:F42')",
                i + 1,
                t.range
            ));
        }
        if t.header_rows.unwrap_or(1) > MAX_HEADER_ROWS_AI {
            return Err(format!(
                "Table {}: header_rows must be 0..={MAX_HEADER_ROWS_AI}",
                i + 1
            ));
        }
    }
    Ok(())
}

// ============================ Toolbox ============================

/// Shared state behind the interactive tools; keeps the LLM's view narrow
/// (rendered ranges, style matches, capped SQL results) instead of bulk data.
#[cfg(feature = "execute")]
struct SheetToolbox {
    grid: Arc<SheetGrid>,
    sheet_name: String,
    bytes: Arc<Vec<u8>>,
    candidates: Vec<Rect>,
    detect_cfg: ExtractConfig,
    encode_opts: EncodeOptions,
    styles: flow_like_types::sync::Mutex<Option<Option<Arc<SheetStyles>>>>,
    df: flow_like_types::sync::Mutex<Option<Arc<SessionContext>>>,
}

#[cfg(feature = "execute")]
impl SheetToolbox {
    fn abs_range_to_rect(&self, range: &str) -> Result<Rect, String> {
        let (ar0, ac0, ar1, ac1) =
            parse_a1_range(range).ok_or_else(|| format!("'{range}' is not a valid A1 range"))?;
        let g = &self.grid;
        let r0 = ar0
            .saturating_sub(g.start_row)
            .min(g.height.saturating_sub(1));
        let c0 = ac0
            .saturating_sub(g.start_col)
            .min(g.width.saturating_sub(1));
        let r1 = ar1
            .saturating_sub(g.start_row)
            .min(g.height.saturating_sub(1));
        let c1 = ac1
            .saturating_sub(g.start_col)
            .min(g.width.saturating_sub(1));
        if r1 < r0 || c1 < c0 {
            return Err(format!("Range '{range}' lies outside the used range"));
        }
        Ok(Rect { r0, c0, r1, c1 })
    }

    async fn get_styles(&self) -> Option<Arc<SheetStyles>> {
        let mut guard = self.styles.lock().await;
        if let Some(cached) = &*guard {
            return cached.clone();
        }
        let bytes = self.bytes.clone();
        let sheet = self.sheet_name.clone();
        let loaded = tokio::task::spawn_blocking(move || {
            SheetStyles::load(&bytes, &sheet).ok().map(Arc::new)
        })
        .await
        .ok()
        .flatten();
        *guard = Some(loaded.clone());
        loaded
    }

    fn set_preloaded_styles(&self, styles: Option<Arc<SheetStyles>>) {
        if let Ok(mut guard) = self.styles.try_lock() {
            *guard = Some(styles);
        }
    }

    async fn inspect_range(&self, range: &str, max_rows: Option<usize>) -> String {
        let rect = match self.abs_range_to_rect(range) {
            Ok(r) => r,
            Err(e) => return e,
        };
        let styles = self.get_styles().await;
        let cap = max_rows.unwrap_or(MAX_INSPECT_ROWS).min(MAX_INSPECT_ROWS);
        render_range(&self.grid, styles.as_deref(), &rect, cap, &self.encode_opts)
    }

    async fn find_styled_cells(
        &self,
        bold: Option<bool>,
        italic: Option<bool>,
        fill_color: Option<String>,
        font_color: Option<String>,
    ) -> String {
        let Some(styles) = self.get_styles().await else {
            return "No style information available (styles require xlsx format).".to_string();
        };
        if styles.is_empty() {
            return "The sheet has no styled cells.".to_string();
        }
        if bold.is_none() && italic.is_none() && fill_color.is_none() && font_color.is_none() {
            return format!("Styling on this sheet: {}", styles.summarize(12));
        }
        let matches = styles.find(|s| {
            bold.is_none_or(|b| s.bold == b)
                && italic.is_none_or(|i| s.italic == i)
                && fill_color
                    .as_deref()
                    .is_none_or(|q| color_matches(q, s.fill_color_name()))
                && font_color
                    .as_deref()
                    .is_none_or(|q| color_matches(q, s.font_color_name()))
        });
        if matches.is_empty() {
            return format!(
                "No cells match. Styling present on this sheet: {}",
                styles.summarize(12)
            );
        }
        let addrs: Vec<String> = matches
            .iter()
            .take(100)
            .map(|&(r, c, _)| format!("{}{}", crate::data::excel::grid::col_to_letters(c), r + 1))
            .collect();
        let suffix = if matches.len() > 100 { ", …" } else { "" };
        format!(
            "{} matching cells: {}{}",
            matches.len(),
            addrs.join(", "),
            suffix
        )
    }

    async fn query_data(&self, sql: &str) -> String {
        let ctx = {
            let mut guard = self.df.lock().await;
            if let Some(ctx) = &*guard {
                ctx.clone()
            } else {
                let ctx = Arc::new(SessionContext::new());
                let mut registered: Vec<String> = Vec::new();
                let tables: Vec<(String, DetectedTable)> = if self.candidates.is_empty() {
                    crate::data::excel::table_detect::whole_sheet_table(
                        &self.grid,
                        &self.detect_cfg,
                    )
                    .map(|t| ("sheet_data".to_string(), t))
                    .into_iter()
                    .collect()
                } else {
                    self.candidates
                        .iter()
                        .take(MAX_CANDIDATE_SQL_TABLES)
                        .enumerate()
                        .filter_map(|(i, rect)| {
                            build_table_from_rect(
                                &self.grid,
                                rect,
                                &self.detect_cfg,
                                &BuildOverrides::default(),
                            )
                            .map(|t| (format!("c{}", i + 1), t))
                        })
                        .collect()
                };
                for (name, table) in tables {
                    let csv = detected_to_csv(table, name.clone(), None);
                    if csv.register_with_datafusion(&ctx, &name).is_ok() {
                        registered.push(name);
                    }
                }
                if registered.is_empty() {
                    return "No queryable tables could be built from this sheet.".to_string();
                }
                *guard = Some(ctx.clone());
                ctx
            }
        };

        let df = match ctx.sql(sql).await {
            Ok(df) => df,
            Err(e) => return format!("SQL error: {e}"),
        };
        let df = match df.limit(0, Some(MAX_QUERY_RESULT_ROWS)) {
            Ok(df) => df,
            Err(e) => return format!("SQL error: {e}"),
        };
        let batches = match df.collect().await {
            Ok(b) => b,
            Err(e) => return format!("SQL execution error: {e}"),
        };
        let table = match batches_to_csv_table(&batches) {
            Ok(t) => t,
            Err(e) => return format!("Result conversion error: {e}"),
        };
        let headers = table.headers();
        let rows = table.rows_as_strings();
        let mut out = String::new();
        out.push_str(&headers.join(" | "));
        out.push('\n');
        for row in rows.iter().take(MAX_QUERY_RESULT_ROWS) {
            let cells: Vec<String> = row.iter().map(|c| truncate_chars(c, 40)).collect();
            out.push_str(&cells.join(" | "));
            out.push('\n');
        }
        if rows.is_empty() {
            out.push_str("(no rows)\n");
        }
        out
    }
}

// ============================ rig tool definitions ============================

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
struct SubmitExtractionTool;

#[cfg(feature = "execute")]
impl Tool for SubmitExtractionTool {
    const NAME: &'static str = "submit_extraction_strategy";
    type Error = ExtractionError;
    type Args = ExtractionStrategy;
    type Output = ExtractionStrategy;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Submit the final extraction strategy: the A1 range of every table (including header rows), header row count, and rows to skip. Call this once you are confident.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "tables": {
                        "type": "array",
                        "minItems": 1,
                        "description": "Every distinct table found on the sheet.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "table_name": {
                                    "type": ["string", "null"],
                                    "description": "Short snake_case name describing the table's content (e.g. 'sales_by_region')."
                                },
                                "range": {
                                    "type": "string",
                                    "description": "A1 range of the table INCLUDING its header rows but EXCLUDING titles above and notes below, e.g. 'A3:F42'. Row numbers are the absolute numbers shown in the sample."
                                },
                                "header_rows": {
                                    "type": "integer",
                                    "minimum": 0,
                                    "maximum": 5,
                                    "description": "How many rows at the top of the range are header rows. 0 if the table has no header."
                                },
                                "skip_rows": {
                                    "type": "array",
                                    "items": { "type": "integer" },
                                    "description": "Absolute 1-based row numbers inside the range to exclude (subtotal rows, section headers, repeated headers)."
                                },
                                "column_names": {
                                    "type": ["array", "null"],
                                    "items": { "type": "string" },
                                    "description": "Optional better column names when sheet headers are missing or unclear."
                                }
                            },
                            "required": ["range", "header_rows"],
                            "additionalProperties": false
                        }
                    },
                    "reasoning": {
                        "type": "string",
                        "description": "Brief explanation: how many tables and why these ranges."
                    }
                },
                "required": ["tables", "reasoning"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        validate_strategy(&args).map_err(ExtractionError)?;
        Ok(args)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct InspectRangeArgs {
    range: String,
    max_rows: Option<usize>,
}

#[cfg(feature = "execute")]
struct InspectRangeTool {
    toolbox: Arc<SheetToolbox>,
}

#[cfg(feature = "execute")]
impl Tool for InspectRangeTool {
    const NAME: &'static str = "inspect_range";
    type Error = ExtractionError;
    type Args = InspectRangeArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Look at the verbatim cells of a small A1 range (max {MAX_INSPECT_ROWS} rows per call). Use this to check boundary rows, header structure, or rows hidden behind an 'omitted' marker before submitting."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "range": { "type": "string", "description": "A1 range to inspect, e.g. 'A40:F60'." },
                    "max_rows": { "type": "integer", "description": "Optional row cap (default 60)." }
                },
                "required": ["range"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        Ok(self.toolbox.inspect_range(&args.range, args.max_rows).await)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct FindStyledCellsArgs {
    bold: Option<bool>,
    italic: Option<bool>,
    fill_color: Option<String>,
    font_color: Option<String>,
}

#[cfg(feature = "execute")]
struct FindStyledCellsTool {
    toolbox: Arc<SheetToolbox>,
}

#[cfg(feature = "execute")]
impl Tool for FindStyledCellsTool {
    const NAME: &'static str = "find_styled_cells";
    type Error = ExtractionError;
    type Args = FindStyledCellsArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Find cells by styling without a style dump: filter by bold, italic, fill_color or font_color (color names like 'green', 'red', 'yellow'). With no filters it returns a summary of all styling on the sheet. xlsx only.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "bold": { "type": ["boolean", "null"] },
                    "italic": { "type": ["boolean", "null"] },
                    "fill_color": { "type": ["string", "null"], "description": "Fill color name: red, orange, yellow, green, cyan, blue, purple, pink, gray, black." },
                    "font_color": { "type": ["string", "null"], "description": "Font color name (same palette)." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        Ok(self
            .toolbox
            .find_styled_cells(args.bold, args.italic, args.fill_color, args.font_color)
            .await)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct ValueIndexArgs {
    contains: Option<String>,
}

#[cfg(feature = "execute")]
struct ValueIndexTool {
    toolbox: Arc<SheetToolbox>,
}

#[cfg(feature = "execute")]
impl Tool for ValueIndexTool {
    const NAME: &'static str = "value_index";
    type Error = ExtractionError;
    type Args = ValueIndexArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "Inverted index of the sheet: text values → the cell addresses/ranges holding them, plus homogeneous numeric runs. Pass `contains` to find where a specific value appears (case-insensitive substring). Use to locate labels, repeated headers, or section markers without scanning ranges.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "contains": { "type": ["string", "null"], "description": "Optional substring filter, e.g. 'total'." }
                },
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        Ok(encode_inverted_index(
            &self.toolbox.grid,
            300,
            args.contains.as_deref(),
        ))
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

#[cfg(feature = "execute")]
#[derive(Debug, Deserialize)]
struct QueryDataArgs {
    sql: String,
}

#[cfg(feature = "execute")]
struct QueryDataTool {
    toolbox: Arc<SheetToolbox>,
}

#[cfg(feature = "execute")]
impl Tool for QueryDataTool {
    const NAME: &'static str = "query_data";
    type Error = ExtractionError;
    type Args = QueryDataArgs;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: format!(
                "Run a SQL query (DataFusion dialect) against the heuristic candidate tables, registered as c1, c2, … in candidate order ('sheet_data' when there are no candidates). Results are capped at {MAX_QUERY_RESULT_ROWS} rows. Use for questions the sample can't answer: row counts, value ranges, whether two candidates share a schema, distinct values of a column."
            ),
            parameters: json!({
                "type": "object",
                "properties": {
                    "sql": { "type": "string", "description": "e.g. SELECT COUNT(*) FROM c1, or SELECT * FROM c2 LIMIT 5" }
                },
                "required": ["sql"],
                "additionalProperties": false
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> std::result::Result<Self::Output, Self::Error> {
        Ok(self.toolbox.query_data(&args.sql).await)
    }

    fn name(&self) -> String {
        Self::NAME.to_string()
    }
}

// ============================ Prompting ============================

#[cfg(feature = "execute")]
fn build_system_prompt(user_hint: &str, candidates: &[Rect], grid: &SheetGrid) -> String {
    let sql_tables = if candidates.is_empty() {
        "sheet_data (whole sheet)".to_string()
    } else {
        candidates
            .iter()
            .take(MAX_CANDIDATE_SQL_TABLES)
            .enumerate()
            .map(|(i, r)| format!("c{} = {}", i + 1, grid.a1_range(r.r0, r.c0, r.r1, r.c1)))
            .collect::<Vec<_>>()
            .join(", ")
    };

    let base = format!(
        r#"You are an expert at analyzing Excel spreadsheets with complex, non-standard layouts.

You receive a compact encoding of one worksheet: its used range, merged regions, styling summary, heuristic table candidates, column type profiles, and the contents of structurally interesting rows (with absolute A1 addresses). Omitted row runs are structurally similar to the rows shown around them.

Your job is to identify EVERY distinct table and submit its geometry via `submit_extraction_strategy`. You never output table data yourself — extraction is done deterministically from your ranges.

Investigation tools (use them when the sample leaves doubt, then submit):
- `inspect_range`: see verbatim cells of a small range (check boundaries, headers, omitted regions).
- `find_styled_cells`: locate bold/colored cells (e.g. the user asks about 'green' rows) without a style dump.
- `value_index`: find where a text value appears (e.g. every 'Total' row or repeated section marker).
- `query_data`: SQL over the candidate tables ({sql_tables}); use for row counts, schema comparisons, distinct values.

Decision rules:
- Tables can be stacked vertically, side by side, or separated by section headers.
- The `range` must INCLUDE header rows and all data rows, but EXCLUDE titles above and footnotes below.
- `header_rows` counts the header rows at the TOP of your range (multi-row headers with merged spans are common; 0 = headerless).
- Use `skip_rows` for subtotal/total rows, repeated headers, or section dividers inside the range.
- Two stacked candidates with the same column structure separated only by a spacer are usually ONE table (verify with `query_data` schema comparison or `inspect_range` at the boundary); a candidate whose first row re-scores as a new header over different columns is a SEPARATE table.
- Prefer precise ranges; when headers are cryptic, provide `column_names`.
- Do not request large amounts of data — investigate narrowly, then submit."#
    );

    if user_hint.trim().is_empty() {
        base
    } else {
        format!("{base}\n\nUser guidance: {user_hint}")
    }
}

// ============================ Spec application ============================

#[cfg(feature = "execute")]
fn apply_spec(
    grid: &SheetGrid,
    spec: &TableSpec,
    cfg: &ExtractConfig,
    warnings: &mut Vec<String>,
) -> Option<DetectedTable> {
    let (ar0, ac0, ar1, ac1) = parse_a1_range(&spec.range)?;
    let r0 = ar0
        .saturating_sub(grid.start_row)
        .min(grid.height.saturating_sub(1));
    let c0 = ac0
        .saturating_sub(grid.start_col)
        .min(grid.width.saturating_sub(1));
    let r1 = ar1
        .saturating_sub(grid.start_row)
        .min(grid.height.saturating_sub(1));
    let c1 = ac1
        .saturating_sub(grid.start_col)
        .min(grid.width.saturating_sub(1));
    if r1 < r0 || c1 < c0 {
        warnings.push(format!("Range '{}' is outside the used range", spec.range));
        return None;
    }
    let rect = Rect { r0, c0, r1, c1 };
    let Some(rect) = tighten(grid, &rect) else {
        warnings.push(format!("Range '{}' contains no data", spec.range));
        return None;
    };
    let overrides = BuildOverrides {
        header_rows: Some(spec.header_rows.unwrap_or(1)),
        skip_rows: spec.skip_rows.iter().copied().collect(),
        column_names: spec.column_names.clone(),
    };
    build_table_from_rect(grid, &rect, cfg, &overrides)
}

// ============================ Node ============================

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
            "Uses AI to locate tables in complex Excel worksheets (unusual layouts, multiple tables, multi-row headers, styling-based hints); extraction itself stays deterministic",
            "Data/Excel",
        );
        node.add_icon("/flow/icons/file-spreadsheet.svg");
        node.set_version(4);

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
            "Optional guidance for the AI (e.g., 'The table starts at row 5', 'Only rows highlighted green matter')",
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
        let ai_cfg: AIExtractionConfig = context.evaluate_pin("config").await.unwrap_or_default();
        let detect_cfg = ExtractConfig::default();

        let bytes: Arc<Vec<u8>> = Arc::new(flow_path.get(context, false).await?);

        struct SheetPrep {
            name: String,
            grid: Arc<SheetGrid>,
            candidates: Vec<Rect>,
            encoding: String,
            styles: Option<Arc<SheetStyles>>,
        }

        let encode_opts = EncodeOptions {
            max_rows: ai_cfg.sample_rows,
            max_cols: ai_cfg.sample_cols,
            include_merges: ai_cfg.detect_merged_regions,
            include_column_profiles: ai_cfg.include_statistics,
            ..EncodeOptions::default()
        };

        let bytes_prep = bytes.clone();
        let detect_cfg_prep = detect_cfg.clone();
        let opts_prep = encode_opts.clone();
        let sheet_filter = sheet_input.trim().to_string();
        let (preps, prep_warnings) = tokio::task::spawn_blocking(
            move || -> flow_like_types::Result<(Vec<SheetPrep>, Vec<String>)> {
                let mut wb = Workbook::open(bytes_prep.as_ref().clone())?;
                let sheet_names: Vec<String> = if sheet_filter.is_empty() {
                    wb.sheet_names()
                } else {
                    if !wb.sheet_names().iter().any(|n| n == &sheet_filter) {
                        return Err(flow_like_types::anyhow!(
                            "Sheet '{sheet_filter}' not found in workbook"
                        ));
                    }
                    vec![sheet_filter]
                };
                let mut all_styles = if bytes_prep.len() <= STYLE_PRELOAD_MAX_BYTES {
                    load_workbook_styles(&bytes_prep).unwrap_or_default()
                } else {
                    Default::default()
                };
                let mut preps = Vec::new();
                let mut warnings = Vec::new();
                for name in sheet_names {
                    match wb.read_grid(&name) {
                        Ok(grid) if grid.height > 0 && grid.width > 0 => {
                            let candidates = detect_table_regions(&grid, &detect_cfg_prep);
                            let styles = all_styles.remove(&name).map(Arc::new);
                            let encoding = encode_sheet_compact(
                                &grid,
                                &candidates,
                                styles.as_deref(),
                                &opts_prep,
                                &name,
                            );
                            preps.push(SheetPrep {
                                name,
                                grid: Arc::new(grid),
                                candidates,
                                encoding,
                                styles,
                            });
                        }
                        Ok(_) => warnings.push(format!("Sheet '{name}': empty, skipped")),
                        Err(e) => warnings.push(format!("Sheet '{name}': {e}")),
                    }
                }
                Ok((preps, warnings))
            },
        )
        .await??;

        for w in prep_warnings {
            context.log_message(&w, LogLevel::Warn);
        }

        let mut all_tables: Vec<CSVTable> = Vec::new();
        let mut all_reasoning: Vec<String> = Vec::new();
        let mut used_names: HashSet<String> = HashSet::new();
        let max_turns = ai_cfg.max_tool_turns.clamp(1, 16);

        for prep in preps {
            let toolbox = Arc::new(SheetToolbox {
                grid: prep.grid.clone(),
                sheet_name: prep.name.clone(),
                bytes: bytes.clone(),
                candidates: prep.candidates.clone(),
                detect_cfg: detect_cfg.clone(),
                encode_opts: encode_opts.clone(),
                styles: flow_like_types::sync::Mutex::new(None),
                df: flow_like_types::sync::Mutex::new(None),
            });
            toolbox.set_preloaded_styles(prep.styles.clone());

            let system_prompt = build_system_prompt(&user_hint, &prep.candidates, &prep.grid);
            let agent = model_bit
                .agent(context, &None)
                .await?
                .preamble(&system_prompt)
                .tool(SubmitExtractionTool)
                .tool(InspectRangeTool {
                    toolbox: toolbox.clone(),
                })
                .tool(FindStyledCellsTool {
                    toolbox: toolbox.clone(),
                })
                .tool(ValueIndexTool {
                    toolbox: toolbox.clone(),
                })
                .tool(QueryDataTool {
                    toolbox: toolbox.clone(),
                })
                .tool_choice(ToolChoice::Required)
                .build();

            let mut history: Vec<Message> = Vec::new();
            let mut next: Message = Message::user(format!(
                "Analyze this Excel sheet and submit the extraction strategy:\n\n{}",
                prep.encoding
            ));
            let mut strategy: Option<ExtractionStrategy> = None;

            for _turn in 0..max_turns {
                let response = agent
                    .completion(next.clone(), history.clone())
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!(
                            "AI completion failed for sheet '{}': {e}",
                            prep.name
                        )
                    })?
                    .send()
                    .await
                    .map_err(|e| {
                        flow_like_types::anyhow!(
                            "Failed to send completion for sheet '{}': {e}",
                            prep.name
                        )
                    })?;

                history.push(next.clone());
                let contents: Vec<AssistantContent> = response.choice.into_iter().collect();
                history.push(Message::Assistant {
                    id: None,
                    content: OneOrMany::many(contents.clone()).unwrap_or_else(|_| {
                        OneOrMany::one(AssistantContent::Text(Text {
                            text: String::new(),
                            additional_params: None,
                        }))
                    }),
                });

                let Some((id, call_id, name, args)) = contents.iter().find_map(|c| {
                    if let AssistantContent::ToolCall(ToolCall {
                        id,
                        call_id,
                        function: ToolFunction { name, arguments },
                        ..
                    }) = c
                    {
                        Some((id.clone(), call_id.clone(), name.clone(), arguments.clone()))
                    } else {
                        None
                    }
                }) else {
                    next = Message::user(
                        "Please respond by calling one of the provided tools.".to_string(),
                    );
                    continue;
                };

                let tool_result = |text: String| Message::User {
                    content: OneOrMany::one(UserContent::ToolResult(RigToolResult {
                        id: id.clone(),
                        call_id: call_id.clone().or_else(|| Some(id.clone())),
                        content: OneOrMany::one(ToolResultContent::text(text)),
                    })),
                };

                match name.as_str() {
                    SubmitExtractionTool::NAME => {
                        match flow_like_types::json::from_value::<ExtractionStrategy>(args) {
                            Ok(s) => match validate_strategy(&s) {
                                Ok(()) => {
                                    strategy = Some(s);
                                    break;
                                }
                                Err(e) => next = tool_result(format!("Invalid strategy: {e}")),
                            },
                            Err(e) => next = tool_result(format!("Could not parse strategy: {e}")),
                        }
                    }
                    InspectRangeTool::NAME => {
                        let result =
                            match flow_like_types::json::from_value::<InspectRangeArgs>(args) {
                                Ok(a) => toolbox.inspect_range(&a.range, a.max_rows).await,
                                Err(e) => format!("Invalid arguments: {e}"),
                            };
                        next = tool_result(result);
                    }
                    FindStyledCellsTool::NAME => {
                        let result =
                            match flow_like_types::json::from_value::<FindStyledCellsArgs>(args) {
                                Ok(a) => {
                                    toolbox
                                        .find_styled_cells(
                                            a.bold,
                                            a.italic,
                                            a.fill_color,
                                            a.font_color,
                                        )
                                        .await
                                }
                                Err(e) => format!("Invalid arguments: {e}"),
                            };
                        next = tool_result(result);
                    }
                    ValueIndexTool::NAME => {
                        let result = match flow_like_types::json::from_value::<ValueIndexArgs>(args)
                        {
                            Ok(a) => {
                                encode_inverted_index(&toolbox.grid, 300, a.contains.as_deref())
                            }
                            Err(e) => format!("Invalid arguments: {e}"),
                        };
                        next = tool_result(result);
                    }
                    QueryDataTool::NAME => {
                        let result = match flow_like_types::json::from_value::<QueryDataArgs>(args)
                        {
                            Ok(a) => toolbox.query_data(&a.sql).await,
                            Err(e) => format!("Invalid arguments: {e}"),
                        };
                        next = tool_result(result);
                    }
                    other => {
                        next = tool_result(format!("Unknown tool '{other}'."));
                    }
                }
            }

            let (specs, reasoning): (Vec<TableSpec>, String) = match strategy {
                Some(s) => (s.tables, s.reasoning),
                None => {
                    context.log_message(
                        &format!(
                            "Sheet '{}': no strategy after {max_turns} turns, falling back to heuristic extraction",
                            prep.name
                        ),
                        LogLevel::Warn,
                    );
                    all_reasoning.push(format!(
                        "Sheet '{}': AI returned no strategy; heuristic extraction used instead",
                        prep.name
                    ));
                    let grid = prep.grid.clone();
                    let cfg = detect_cfg.clone();
                    let tables =
                        tokio::task::spawn_blocking(move || extract_tables_from_grid(&grid, &cfg))
                            .await?;
                    let sheet_base = normalize_table_name(&prep.name);
                    for table in tables {
                        let name = unique_table_name(&used_names, &sheet_base);
                        used_names.insert(name.clone());
                        all_tables.push(detected_to_csv(table, name, Some(flow_path.clone())));
                    }
                    continue;
                }
            };

            all_reasoning.push(format!("Sheet '{}': {}", prep.name, reasoning));

            let detect_cfg_apply = detect_cfg.clone();
            let grid = prep.grid.clone();
            let (tables, warnings) = tokio::task::spawn_blocking(move || {
                let mut warnings = Vec::new();
                let tables: Vec<_> = specs
                    .iter()
                    .filter_map(|spec| {
                        apply_spec(&grid, spec, &detect_cfg_apply, &mut warnings)
                            .map(|t| (spec.table_name.clone(), t))
                    })
                    .collect();
                (tables, warnings)
            })
            .await?;

            for w in warnings {
                context.log_message(&format!("Sheet '{}': {w}", prep.name), LogLevel::Warn);
            }

            let sheet_base = normalize_table_name(&prep.name);
            for (spec_name, table) in tables {
                let base = spec_name
                    .as_deref()
                    .map(normalize_table_name)
                    .filter(|n| n != "table")
                    .unwrap_or_else(|| sheet_base.clone());
                let name = unique_table_name(&used_names, &base);
                used_names.insert(name.clone());
                all_tables.push(detected_to_csv(table, name, Some(flow_path.clone())));
            }
        }

        context.set_pin_value("tables", json!(all_tables)).await?;
        context
            .set_pin_value("reasoning", json!(all_reasoning.join("\n\n")))
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
