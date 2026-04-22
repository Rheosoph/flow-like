use super::element_utils::{extract_element_id, find_element};
use super::update_schemas::{TableCellUpdate, TableColumn};
use flow_like::a2ui::components::TableProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Unified Table update node.
///
/// Manage table data and structure with a single node.
/// The input pins change dynamically based on the selected operation.
///
/// **Operations:**
/// - Set Data: Replace all table data
/// - Set Columns: Define column structure
/// - Add Row: Append a new row
/// - Remove Row: Delete a row by index
/// - Update Cell: Update a specific cell value
/// - Get Data: Retrieve current table data
#[crate::register_node]
#[derive(Default)]
pub struct UpdateTable;

impl UpdateTable {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for UpdateTable {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_update_table",
            "Update Table",
            "Add, remove, or update table data and structure",
            "UI/Elements/Table",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Table",
            "Reference to the table element",
            VariableType::Struct,
        )
        .set_schema::<TableProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "operation",
            "Operation",
            "What operation to perform",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Set Data".to_string(),
                    "Set Columns".to_string(),
                    "Add Row".to_string(),
                    "Remove Row".to_string(),
                    "Update Cell".to_string(),
                    "Get Data".to_string(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("Set Data")));

        // Default: Set Data pins
        node.add_input_pin("data", "Data", "Array of row objects", VariableType::Struct)
            .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid element reference"))?;

        let operation: String = context.evaluate_pin("operation").await?;

        match operation.as_str() {
            "Set Data" => {
                let data: Value = context.evaluate_pin("data").await?;
                let update = json!({
                    "type": "setTableData",
                    "data": data
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Set Columns" => {
                let columns: Vec<TableColumn> = context.evaluate_pin("columns").await?;
                let update = json!({
                    "type": "setTableColumns",
                    "columns": columns
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Add Row" => {
                let row: Value = context.evaluate_pin("row").await?;
                let update = json!({
                    "type": "addTableRow",
                    "row": row
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Remove Row" => {
                let index: i32 = context.evaluate_pin("row_index").await?;
                let update = json!({
                    "type": "removeTableRow",
                    "index": index
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Update Cell" => {
                let cell: TableCellUpdate = context.evaluate_pin("cell").await?;
                let update = json!({
                    "type": "updateTableCell",
                    "rowIndex": cell.row_index,
                    "column": cell.column,
                    "value": cell.value
                });
                context.upsert_element(&element_id, update).await?;
            }
            "Get Data" => {
                let elements = context.get_frontend_elements().await?;
                let element = elements.as_ref().and_then(|e| find_element(e, &element_id));
                let data = element
                    .map(|(_, el)| el)
                    .and_then(|el| el.get("component"))
                    .and_then(|c| c.get("data"))
                    .cloned()
                    .unwrap_or(json!([]));
                let count = data.as_array().map(|a| a.len()).unwrap_or(0);
                context.set_pin_value("data", data).await?;
                context.set_pin_value("row_count", json!(count)).await?;
            }
            _ => return Err(flow_like_types::anyhow!("Unknown operation: {}", operation)),
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let operation = node
            .get_pin_by_name("operation")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "Set Data".to_string());

        // Remove all dynamic pins
        let pins_to_check = ["data", "columns", "row", "row_index", "cell", "row_count"];
        for pin_name in pins_to_check {
            if let Some(pin) = node.get_pin_by_name(pin_name).cloned() {
                remove_pin(node, Some(pin));
            }
        }

        match operation.as_str() {
            "Set Data" => {
                node.add_input_pin("data", "Data", "Array of row objects", VariableType::Struct)
                    .set_value_type(flow_like::flow::pin::ValueType::Array)
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
            }
            "Set Columns" => {
                node.add_input_pin(
                    "columns",
                    "Columns",
                    "Column definitions",
                    VariableType::Struct,
                )
                .set_value_type(flow_like::flow::pin::ValueType::Array)
                .set_schema::<TableColumn>();
            }
            "Add Row" => {
                node.add_input_pin("row", "Row", "Row data object", VariableType::Struct)
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
            }
            "Remove Row" => {
                node.add_input_pin(
                    "row_index",
                    "Row Index",
                    "Index of row to remove (0-based)",
                    VariableType::Integer,
                );
            }
            "Update Cell" => {
                node.add_input_pin(
                    "cell",
                    "Cell",
                    "Cell update parameters",
                    VariableType::Struct,
                )
                .set_schema::<TableCellUpdate>();
            }
            "Get Data" => {
                node.add_output_pin("data", "Data", "Current table data", VariableType::Struct)
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
                node.add_output_pin(
                    "row_count",
                    "Row Count",
                    "Number of rows",
                    VariableType::Integer,
                );
            }
            _ => {}
        }
    }
}
