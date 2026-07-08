use super::chart_data_utils::{extract_from_csv_table, has_csv_table_data, parse_csv_text};
use super::element_utils::extract_element_id;
use flow_like::a2ui::components::TableProps;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::Map, json::json};

/// Push CSV or Table data directly to a table element.
///
/// Parses CSV text or Table data, automatically creates column definitions,
/// and updates both columns and data on the table in a single operation.
#[crate::register_node]
#[derive(Default)]
pub struct WriteCsvToTable;

impl WriteCsvToTable {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for WriteCsvToTable {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_write_csv_to_table",
            "Push CSV to Table",
            "Push CSV or Table data directly to a table element",
            "UI/Elements/Table",
        );
        node.add_icon("/flow/icons/a2ui.svg");

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Element",
            "Reference to the table element",
            VariableType::Struct,
        )
        .set_schema::<TableProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin("csv", "CSV", "CSV text with headers", VariableType::String)
            .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "table",
            "Table",
            "Table data from DataFusion query",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "delimiter",
            "Delimiter",
            "CSV delimiter (default: comma)",
            VariableType::String,
        )
        .set_default_value(Some(json!(",")));

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

        let delimiter: String = context.evaluate_pin("delimiter").await?;
        let delimiter_char = delimiter.chars().next().unwrap_or(',');

        // Get data from either table or CSV
        let table_result = context.evaluate_pin::<Value>("table").await;
        let (headers, rows) = if let Ok(table_value) = table_result {
            if has_csv_table_data(&table_value) {
                extract_from_csv_table(&table_value)?
            } else {
                let csv_text: String = context.evaluate_pin("csv").await?;
                parse_csv_text(&csv_text, delimiter_char)?
            }
        } else {
            let csv_text: String = context.evaluate_pin("csv").await?;
            parse_csv_text(&csv_text, delimiter_char)?
        };

        if headers.is_empty() {
            let update_value = json!({
                "type": "setProps",
                "props": {
                    "columns": { "literalJson": "[]" },
                    "data": { "literalJson": "[]" }
                }
            });
            context.upsert_element(&element_id, update_value).await?;
            context.activate_exec_pin("exec_out").await?;
            return Ok(());
        }

        let columns: Vec<Value> = headers
            .iter()
            .map(|h| {
                let id = h.to_lowercase().replace(' ', "_");
                json!({
                    "id": id,
                    "header": { "literalString": h },
                    "accessor": { "literalString": id }
                })
            })
            .collect();

        let data: Vec<Value> = rows
            .iter()
            .map(|row| {
                let mut obj = Map::new();
                for (i, header) in headers.iter().enumerate() {
                    let key = header.to_lowercase().replace(' ', "_");
                    let value = row.get(i).cloned().unwrap_or_default();
                    obj.insert(key, json!(value));
                }
                Value::Object(obj)
            })
            .collect();

        let columns_json = flow_like_types::json::to_string(&columns)?;
        let data_json = flow_like_types::json::to_string(&data)?;
        let update_value = json!({
            "type": "setProps",
            "props": {
                "columns": { "literalJson": columns_json },
                "data": { "literalJson": data_json }
            }
        });

        context.upsert_element(&element_id, update_value).await?;
        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
