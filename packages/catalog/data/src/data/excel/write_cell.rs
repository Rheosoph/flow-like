#[cfg(feature = "execute")]
use crate::data::excel::{parse_col_1_based, parse_row_1_based};
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

/// Write a single cell inside an Excel workbook (XLSX).
/// Works with virtual/object-store files via `FlowPath` (no local filesystem I/O).
/// Creates the file if it does not exist and the sheet if it is missing.
/// Column and Row correspond to the components of an A1 address
/// (e.g. for "B3": col = "B", row = "3").
/// The updated (same) `FlowPath` is returned so downstream nodes can re-use the file.
#[crate::register_node]
#[derive(Default)]
pub struct WriteCellNode {}

impl WriteCellNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for WriteCellNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "excel_write_cell",
            "Excel Write Cell",
            "Write/update a single cell value in an XLSX sheet",
            "Data/Excel",
        );
        node.set_flowscript_name("excel", "writeCell");
        node.add_icon("/flow/icons/file-spreadsheet.svg");

        // Impure node → needs execution pins
        node.add_input_pin("exec_in", "In", "Trigger", VariableType::Execution);

        node.add_input_pin("file", "File", "Target XLSX file", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_input_pin("sheet", "Sheet", "Worksheet name", VariableType::String)
            .set_default_value(Some(json!("Sheet1")));
        node.add_input_pin("row", "Row", "Row number (1-based)", VariableType::String)
            .set_default_value(Some(json!("1")));
        node.add_input_pin(
            "col",
            "Column",
            "Column (letter(s) like A, AA, or 1-based number)",
            VariableType::String,
        )
        .set_default_value(Some(json!("A")));
        node.add_input_pin(
            "value",
            "Value",
            "Value to write (string)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin("exec_out", "Out", "Trigger", VariableType::Execution);
        node.add_output_pin(
            "file_out",
            "File",
            "Updated XLSX path",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, ctx: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::{CachedExcelWorkbook, flush_workbook, get_or_open_workbook};

        ctx.deactivate_exec_pin("exec_out").await?;

        let file: FlowPath = ctx.evaluate_pin("file").await?;
        let sheet: String = ctx.evaluate_pin("sheet").await?;
        let row_str: String = ctx.evaluate_pin("row").await?;
        let col_str: String = ctx.evaluate_pin("col").await?;
        let value: String = ctx.evaluate_pin("value").await?;

        let row = parse_row_1_based(&row_str)?;
        let col = parse_col_1_based(&col_str)?;

        let cached = get_or_open_workbook(ctx, &file, false).await?;
        let wb = cached
            .as_any()
            .downcast_ref::<CachedExcelWorkbook>()
            .ok_or_else(|| flow_like_types::anyhow!("Cache type mismatch"))?;

        {
            let mut book = wb.umya_book_mut()?;
            if book.get_sheet_by_name(&sheet).is_none() {
                book.new_sheet(&sheet)
                    .map_err(|e| flow_like_types::anyhow!("Failed to create sheet: {}", e))?;
            }
            let ws = book.get_sheet_by_name_mut(&sheet).ok_or_else(|| {
                flow_like_types::anyhow!("Failed to access or create sheet: {}", sheet)
            })?;
            ws.get_cell_mut((col, row)).set_value(value);
        }

        flush_workbook(wb, &file, ctx).await?;

        ctx.set_pin_value("file_out", json!(file)).await?;
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
