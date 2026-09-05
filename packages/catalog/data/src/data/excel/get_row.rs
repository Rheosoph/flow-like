use std::collections::BTreeMap;

use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, anyhow, async_trait, json::json};

use crate::data::excel::{CSVTable, Cell};

/// GetRowByIndexNode
/// ------------------
/// Returns a single row (as a struct) from a `CSVTable` by **1-based** index.
/// Pure node – no exec pins; errors are propagated.
///
/// Inputs
/// - `table` (Struct<CSVTable>): table to read from.
/// - `row_index` (Integer): 1-based row index (>=1).
///
/// Outputs
/// - `row` (Struct): the row as `{ header: value, ... }`.
/// - `row_index` (Integer): echo of the requested index.
#[crate::register_node]
#[derive(Default)]
pub struct GetRowByIndexNode {}

impl GetRowByIndexNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GetRowByIndexNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "tables_get_row_by_index",
            "Get Row By Index",
            "Return a single row as a struct (1-based index)",
            "Data/Excel/Rows",
        );
        node.set_flowscript_name("excel", "getRowByIndex");
        node.add_icon("/flow/icons/file-spreadsheet.svg");

        // Table input
        node.add_input_pin("table", "Table", "CSVTable to read", VariableType::Struct)
            .set_schema::<CSVTable>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "row_index",
            "Row Index",
            "1-based row index (>=1)",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(1)));

        // Outputs
        node.add_output_pin("row", "Row", "Row as struct", VariableType::Struct)
            .set_open_schema();
        node.add_output_pin(
            "actual_row_index",
            "Row Index",
            "Echo of requested index",
            VariableType::Integer,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let table: CSVTable = context.evaluate_pin("table").await?;
        let row_index_in: i64 = context.evaluate_pin("row_index").await.unwrap_or(1);
        if row_index_in < 1 {
            return Err(anyhow!("row_index must be >= 1 (got {row_index_in})"));
        }

        let total_rows = table.row_count();
        let idx0 = (row_index_in as usize).saturating_sub(1);
        if idx0 >= total_rows {
            return Err(anyhow!(
                "row_index {} out of range (rows: {})",
                row_index_in,
                total_rows
            ));
        }

        let row = &table.rows()[idx0];
        let mut obj: BTreeMap<String, Value> = BTreeMap::new();
        for col_idx in 0..table.headers_ref().len() {
            let key = table.headers_ref()[col_idx].as_ref();
            let val = row.get(col_idx).unwrap_or(&Cell::Null);
            obj.insert(key.to_string(), cell_to_json(val));
        }

        context.set_pin_value("row", json!(obj)).await?;
        context
            .set_pin_value("actual_row_index", json!(row_index_in))
            .await?;
        Ok(())
    }
}

#[inline]
fn cell_to_json(c: &Cell) -> Value {
    match c {
        Cell::Null => Value::Null,
        Cell::Bool(b) => json!(*b),
        Cell::Int(i) => json!(*i),
        Cell::Float(f) => json!(*f),
        Cell::Str(s) => json!(s.as_ref()),
        Cell::Date { iso, ms } => json!({"iso": iso.as_ref(), "ms": ms}),
    }
}
