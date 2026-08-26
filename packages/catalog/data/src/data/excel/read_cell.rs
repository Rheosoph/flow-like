#[cfg(feature = "execute")]
use crate::data::excel::{parse_col_1_based, parse_row_1_based};
use crate::data::path::FlowPath;
#[cfg(feature = "execute")]
use calamine::{Data, DataType};
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
fn calamine_data_to_string(cell: &Data) -> String {
    match cell {
        Data::Empty => String::new(),
        Data::String(s) => s.clone(),
        Data::Bool(b) => b.to_string(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => f.to_string(),
        Data::DateTime(_) => cell
            .as_datetime()
            .map(|dt| dt.format("%Y-%m-%dT%H:%M:%S").to_string())
            .or_else(|| cell.as_date().map(|d| d.format("%Y-%m-%d").to_string()))
            .unwrap_or_else(|| cell.as_f64().map(|f| f.to_string()).unwrap_or_default()),
        Data::DateTimeIso(s) => s.clone(),
        Data::DurationIso(s) => s.clone(),
        Data::Error(e) => format!("{:?}", e),
    }
}

/// Read a cell value + optional hyperlink from an already-parsed workbook.
#[cfg(feature = "execute")]
fn read_cell_from_book(
    book: &umya_spreadsheet::Spreadsheet,
    sheet: &str,
    col: u32,
    row: u32,
) -> (String, bool) {
    let Some(ws) = book.get_sheet_by_name(sheet) else {
        return (String::new(), false);
    };
    let Some(cell) = ws.get_cell((col, row)) else {
        return (String::new(), false);
    };
    let value = cell.get_formatted_value();
    let found = !value.is_empty();
    let url = cell
        .get_hyperlink()
        .map(|h| h.get_url().to_owned())
        .unwrap_or_default();
    if !url.is_empty() && found {
        (format!("[{}]({})", value, url), found)
    } else {
        (value, found)
    }
}

/// Read a single cell from an Excel workbook (XLSX) located in object storage via `FlowPath`.
/// - Does **not** touch the local filesystem; reads from bytes and returns the raw string value.
/// - If the file, sheet or cell is missing, `value` is set to "" and `found` is false.
/// - Row/Col are 1-based; column can be letters (A, AA, ...) or a 1-based number.
#[crate::register_node]
#[derive(Default)]
pub struct ReadCellNode {}

impl ReadCellNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ReadCellNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "excel_read_cell",
            "Excel Read Cell",
            "Read a single cell value from an XLSX sheet",
            "Data/Excel",
        );
        node.set_flowscript_name("excel", "readCell");
        node.add_icon("/flow/icons/file-spreadsheet.svg");

        node.add_input_pin("exec_in", "In", "Trigger", VariableType::Execution);

        node.add_input_pin("file", "File", "Source XLSX file", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_input_pin("sheet", "Sheet", "Worksheet name", VariableType::String)
            .set_default_value(Some(json!("Sheet1")));
        node.add_input_pin("row", "Row", "Row number (1-based)", VariableType::String)
            .set_default_value(Some(json!("1")));
        node.add_input_pin(
            "col",
            "Column",
            "Column letters or number (1-based)",
            VariableType::String,
        )
        .set_default_value(Some(json!("A")));

        node.add_output_pin("exec_out", "Out", "Trigger", VariableType::Execution);
        node.add_output_pin(
            "file_out",
            "File",
            "Pass-through XLSX path",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_output_pin(
            "value",
            "Value",
            "Cell value (raw string)",
            VariableType::String,
        );
        node.add_output_pin(
            "found",
            "Found",
            "Cell exists and has a value",
            VariableType::Boolean,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, ctx: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::{CachedExcelWorkbook, get_or_open_workbook};

        ctx.deactivate_exec_pin("exec_out").await?;

        let file: FlowPath = ctx.evaluate_pin("file").await?;
        let sheet: String = ctx.evaluate_pin("sheet").await?;
        let row_str: String = ctx.evaluate_pin("row").await?;
        let col_str: String = ctx.evaluate_pin("col").await?;

        let r1 = parse_row_1_based(&row_str)?;
        let c1 = parse_col_1_based(&col_str)?;

        let cached = get_or_open_workbook(ctx, &file, false).await?;
        let wb = cached
            .as_any()
            .downcast_ref::<CachedExcelWorkbook>()
            .ok_or_else(|| flow_like_types::anyhow!("Cache type mismatch"))?;

        let (value, found) = match wb {
            CachedExcelWorkbook::Umya { book } => {
                let book = book
                    .read()
                    .map_err(|e| flow_like_types::anyhow!("Lock poisoned: {}", e))?;
                read_cell_from_book(&book, &sheet, c1, r1)
            }
            CachedExcelWorkbook::Calamine { sheets } => match sheets.get(&sheet) {
                Some(range) => {
                    let r0 = (r1 - 1) as u32;
                    let c0 = (c1 - 1) as u32;
                    match range.get_value((r0, c0)) {
                        Some(cell) => (calamine_data_to_string(cell), !matches!(cell, Data::Empty)),
                        None => (String::new(), false),
                    }
                }
                None => (String::new(), false),
            },
        };

        ctx.set_pin_value("file_out", json!(file)).await?;
        ctx.set_pin_value("value", json!(value)).await?;
        ctx.set_pin_value("found", json!(found)).await?;
        ctx.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Data processing requires the 'execute' feature"
        ))
    }
}
