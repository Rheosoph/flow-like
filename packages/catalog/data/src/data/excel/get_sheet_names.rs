#[cfg(feature = "execute")]
use calamine::Reader;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{anyhow, async_trait, json::json};
#[cfg(feature = "execute")]
use std::io::Cursor;

use crate::data::path::FlowPath;

/// GetSheetNamesNode
/// ------------------
/// Returns the list of sheet (worksheet) names from a spreadsheet using **calamine**.
/// This node is **pure** (no side effects).
///
/// Supported formats (via calamine auto-detection): xls, xlsx, xlsm, xlsb, ods, ...
///
/// Inputs
/// - `file` (Struct<FlowPath>): the spreadsheet file to inspect.
///
/// Outputs
/// - `sheet_names` (Array<String>): ordered list of sheet names as they appear.
/// - `count` (Integer): number of sheets.
#[crate::register_node]
#[derive(Default)]
pub struct GetSheetNamesNode {}

impl GetSheetNamesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetSheetNamesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "files_spreadsheet_get_sheet_names",
            "Get Sheet Names",
            "List worksheet names using calamine",
            "Data/Excel",
        );
        node.set_flowscript_name("excel", "getSheetNames");
        node.add_icon("/flow/icons/file-spreadsheet.svg");

        // Pure node: no execution pins

        // File input
        node.add_input_pin(
            "file",
            "File",
            "Spreadsheet file to inspect",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Outputs
        // If Array item typing is supported by PinOptions, set it; otherwise engine accepts JSON array.
        node.add_output_pin(
            "sheet_names",
            "Sheet Names",
            "All worksheet names",
            VariableType::String,
        )
        .set_value_type(flow_like::flow::pin::ValueType::Array)
        .set_options(PinOptions::new().build());
        node.add_output_pin("count", "Count", "Number of sheets", VariableType::Integer);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let file: FlowPath = context.evaluate_pin("file").await?; // throws on failure
        let bytes = file.get(context, false).await?; // throws on failure

        let wb = calamine::open_workbook_auto_from_rs(Cursor::new(bytes))
            .map_err(|e| anyhow!("Failed to read workbook via calamine: {}", e))?;

        let names = wb.sheet_names();

        context.set_pin_value("sheet_names", json!(names)).await?;
        let count: i64 = names.len() as i64;
        context.set_pin_value("count", json!(count)).await?;

        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Data processing requires the 'execute' feature"
        ))
    }
}
