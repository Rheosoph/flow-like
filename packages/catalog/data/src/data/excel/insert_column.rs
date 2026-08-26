use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::anyhow;
use flow_like_types::{async_trait, json::json};

use crate::data::path::FlowPath;

/// InsertColumnNode
/// -----------------
/// Inserts one or more columns **before/after** a target column in a worksheet
/// of an existing `.xlsx` workbook.
///
/// This is an **impure** node because it mutates a file.
///
/// Inputs
/// - `exec_in` (Execution): trigger.
/// - `file` (Struct<FlowPath>): the XLSX file to modify.
/// - `sheet_name` (String): target worksheet, defaults to `Sheet1`.
/// - `column` (String): column reference where to insert. Accepts an Excel letter
///   (e.g. `B`, `AA`) **or** a 1-based index as string (e.g. `2`).
/// - `position` (Enum String): `before` (default) or `after` the target column.
/// - `num_columns` (Integer): how many columns to insert, default `1`, min `1`.
/// - `adjust_references` (Boolean): if `true` (default), adjust formulas/refs across the workbook
///   using the Spreadsheet-level API. If `false`, only the current sheet is adjusted.
///
/// Outputs
/// - `exec_out` (Execution): fired when operation completes successfully.
/// - `inserted` (Boolean): true when columns were inserted.
/// - `final_column_index` (Integer): the actual 1-based column index used.
/// - `final_column_letter` (String): the Excel letter for the final index.
/// - `total_columns_inserted` (Integer): echo of the `num_columns` actually applied.
#[crate::register_node]
#[derive(Default)]
pub struct InsertColumnNode {}

impl InsertColumnNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for InsertColumnNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "files_spreadsheet_insert_column",
            "Insert Column(s)",
            "Insert one or more columns into a worksheet",
            "Data/Excel",
        );
        node.set_flowscript_name("excel", "insertColumn");
        node.add_icon("/flow/icons/file-spreadsheet.svg");

        // Exec
        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        // File input
        node.add_input_pin(
            "file",
            "File",
            "The .xlsx file to modify",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Worksheet selector
        node.add_input_pin(
            "sheet_name",
            "Sheet Name",
            "Target worksheet name",
            VariableType::String,
        )
        .set_default_value(Some(json!("Sheet1")));

        // Column reference
        node.add_input_pin(
            "column",
            "Column",
            "Target column (letter like 'B' or index like '2')",
            VariableType::String,
        )
        .set_default_value(Some(json!("A")));

        // Before/After
        node.add_input_pin(
            "position",
            "Position",
            "Insert before or after the target column",
            VariableType::String,
        )
        .set_default_value(Some(json!("before")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["before".to_string(), "after".to_string()])
                .build(),
        );

        // Count
        node.add_input_pin(
            "num_columns",
            "Count",
            "How many columns to insert",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        // Reference adjustment
        node.add_input_pin(
            "adjust_references",
            "Adjust References",
            "Adjust formulas across workbook",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        // Outputs
        node.add_output_pin(
            "exec_out",
            "Done",
            "Continue on success",
            VariableType::Execution,
        );
        node.add_output_pin(
            "inserted",
            "Inserted",
            "Whether columns were inserted",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "final_column_index",
            "Final Index",
            "1-based column index used",
            VariableType::Integer,
        );
        node.add_output_pin(
            "final_column_letter",
            "Final Letter",
            "Excel letter for final index",
            VariableType::String,
        );
        node.add_output_pin(
            "total_columns_inserted",
            "Total Inserted",
            "How many columns were inserted",
            VariableType::Integer,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use super::{CachedExcelWorkbook, flush_workbook, get_or_open_workbook};

        context.deactivate_exec_pin("exec_out").await?;

        let file: FlowPath = context.evaluate_pin("file").await?;
        let sheet_name: String = context.evaluate_pin("sheet_name").await?;
        let column_ref: String = context.evaluate_pin("column").await?;
        let position: String = context.evaluate_pin("position").await?;
        let mut num_columns: i64 = context.evaluate_pin("num_columns").await.unwrap_or(1);
        let adjust_refs: bool = context
            .evaluate_pin("adjust_references")
            .await
            .unwrap_or(true);

        if num_columns < 1 {
            num_columns = 1;
        }
        let num_columns_u32: u32 = num_columns as u32;

        let base_index = parse_column_ref(&column_ref)
            .ok_or_else(|| anyhow!("Invalid column reference: '{}'", column_ref))?;
        let insert_index = match position.as_str() {
            "after" => base_index + 1,
            _ => base_index,
        };

        let cached = get_or_open_workbook(context, &file, false).await?;
        let wb = cached
            .as_any()
            .downcast_ref::<CachedExcelWorkbook>()
            .ok_or_else(|| anyhow!("Cache type mismatch"))?;

        {
            let mut book = wb.umya_book_mut()?;

            if book.get_sheet_by_name(&sheet_name).is_none() {
                return Err(anyhow!("Sheet '{}' not found", sheet_name));
            }

            if adjust_refs {
                book.insert_new_column_by_index(&sheet_name, &insert_index, &num_columns_u32);
            } else if let Some(ws) = book.get_sheet_by_name_mut(&sheet_name) {
                ws.insert_new_column_by_index(&insert_index, &num_columns_u32);
            } else {
                return Err(anyhow!("Sheet '{}' not found (mut)", sheet_name));
            }
        }

        flush_workbook(wb, &file, context).await?;

        let letter = index_to_column_letter(insert_index);
        context.set_pin_value("inserted", json!(true)).await?;
        context
            .set_pin_value("final_column_index", json!(insert_index as i64))
            .await?;
        context
            .set_pin_value("final_column_letter", json!(letter))
            .await?;
        context
            .set_pin_value("total_columns_inserted", json!(num_columns as i64))
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Data processing requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
fn parse_column_ref(s: &str) -> Option<u32> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    // Try numeric first
    if t.chars().all(|c| c.is_ascii_digit()) {
        return t.parse::<u32>().ok().filter(|&n| n >= 1);
    }
    // Letters -> index
    let up = t.to_ascii_uppercase();
    if !up.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    let mut acc: u32 = 0;
    for ch in up.chars() {
        let v = (ch as u8 - b'A' + 1) as u32;
        acc = acc.checked_mul(26)?;
        acc = acc.checked_add(v)?;
    }
    Some(acc)
}

#[cfg(feature = "execute")]
fn index_to_column_letter(mut idx: u32) -> String {
    if idx < 1 {
        return "A".to_string();
    }
    let mut buf = Vec::<u8>::new();
    while idx > 0 {
        let rem = ((idx - 1) % 26) as u8;
        buf.push(b'A' + rem);
        idx = (idx - 1) / 26;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap_or_else(|_| "A".to_string())
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::{index_to_column_letter, parse_column_ref};

    #[test]
    fn parse_ref_numeric() {
        assert_eq!(parse_column_ref("1"), Some(1));
        assert_eq!(parse_column_ref("26"), Some(26));
        assert_eq!(parse_column_ref("  3  "), Some(3));
        assert_eq!(parse_column_ref("0"), None);
    }

    #[test]
    fn parse_ref_letters() {
        assert_eq!(parse_column_ref("A"), Some(1));
        assert_eq!(parse_column_ref("Z"), Some(26));
        assert_eq!(parse_column_ref("AA"), Some(27));
        assert_eq!(parse_column_ref("XFD"), Some(16384)); // Excel max
        assert_eq!(parse_column_ref("A1"), None);
    }

    #[test]
    fn index_to_letters() {
        assert_eq!(index_to_column_letter(1), "A");
        assert_eq!(index_to_column_letter(26), "Z");
        assert_eq!(index_to_column_letter(27), "AA");
        assert_eq!(index_to_column_letter(52), "AZ");
    }
}
