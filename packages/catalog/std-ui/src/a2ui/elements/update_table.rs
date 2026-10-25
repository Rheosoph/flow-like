use super::element_utils::extract_element_id;
use super::update_schemas::{TableCellUpdate, TableColumn};
use flow_like::a2ui::components::TableProps;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, PinType, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// Pins only some operations show.
const OPERATION_PINS: [&str; 6] = ["data", "columns", "row", "row_index", "cell", "row_count"];

/// The operation pins an operation shows: (name, direction, container).
fn operation_pins(operation: &str) -> &'static [(&'static str, PinType, ValueType)] {
    match operation {
        "Set Data" => &[("data", PinType::Input, ValueType::Array)],
        "Set Columns" => &[("columns", PinType::Input, ValueType::Array)],
        "Add Row" => &[("row", PinType::Input, ValueType::Normal)],
        "Remove Row" => &[("row_index", PinType::Input, ValueType::Normal)],
        "Update Cell" => &[("cell", PinType::Input, ValueType::Normal)],
        "Get Data" => &[
            ("data", PinType::Output, ValueType::Array),
            ("row_count", PinType::Output, ValueType::Normal),
        ],
        _ => &[],
    }
}

/// Set Data's `data`: the rows to show, one object per row, as a query returns them.
fn add_set_data_pin(node: &mut Node) {
    node.add_input_pin("data", "Data", "Array of row objects", VariableType::Struct)
        .set_value_type(ValueType::Array)
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_open_schema();
}

/// Add the operation pin `name` in the direction [`operation_pins`] gives it.
fn add_operation_pin(node: &mut Node, name: &str, pin_type: &PinType) {
    match (name, pin_type) {
        ("data", PinType::Input) => add_set_data_pin(node),
        ("data", PinType::Output) => {
            node.add_output_pin("data", "Data", "Current table data", VariableType::Struct)
                .set_value_type(ValueType::Array)
                .set_options(PinOptions::new().set_enforce_schema(false).build());
        }
        ("columns", _) => {
            node.add_input_pin(
                "columns",
                "Columns",
                "Column definitions",
                VariableType::Struct,
            )
            .set_value_type(ValueType::Array)
            .set_schema::<TableColumn>();
        }
        ("row", _) => {
            node.add_input_pin("row", "Row", "Row data object", VariableType::Struct)
                .set_options(PinOptions::new().set_enforce_schema(false).build());
        }
        ("row_index", _) => {
            node.add_input_pin(
                "row_index",
                "Row Index",
                "Index of row to remove (0-based)",
                VariableType::Integer,
            );
        }
        ("cell", _) => {
            node.add_input_pin(
                "cell",
                "Cell",
                "Cell update parameters",
                VariableType::Struct,
            )
            .set_schema::<TableCellUpdate>();
        }
        ("row_count", _) => {
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
        node.set_flowscript_name("ui", "updateTable");
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

        add_set_data_pin(&mut node);

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);
        node.set_version(1);

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
                let element = context.read_element(&element_id).await?;
                let data = element
                    .as_ref()
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

        // A pin the operation still shows is kept, not re-minted: a fresh pin id drops its wires
        // and keeps the board update loop dirty.
        let shown = operation_pins(&operation);
        node.pins.retain(|_, pin| {
            !OPERATION_PINS.contains(&pin.name.as_str())
                || shown.iter().any(|(name, pin_type, value_type)| {
                    pin.name == *name && pin.pin_type == *pin_type && pin.value_type == *value_type
                })
        });
        for (name, pin_type, _) in shown {
            if !node.pins.values().any(|pin| pin.name == *name) {
                add_operation_pin(node, name, pin_type);
            }
        }
    }
}
