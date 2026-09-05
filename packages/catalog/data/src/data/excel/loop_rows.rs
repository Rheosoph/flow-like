use std::collections::BTreeMap;

use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext, internal_node::InternalNode},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::json::json;
use flow_like_types::{Value as FlowValue, async_trait};

use crate::data::excel::{CSVTable, Cell};

/// RowLoopNode (For Each Row)
/// --------------------------
/// Drives control flow like the generic `For Each` node, but iterates **rows** of a `CSVTable`.
/// Emits one tick per row on `exec_out`, setting `value` (the current row as an object) and `index` (0-based),
/// then finally fires `done`.
#[crate::register_node]
#[derive(Default)]
pub struct RowLoopNode {}

impl RowLoopNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for RowLoopNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "control_for_each_row",
            "For Each Row",
            "Loops over all rows of a table",
            "Control",
        );
        node.set_flowscript_name("control", "forEachRow");
        node.add_icon("/flow/icons/for-each.svg");

        // Exec in
        node.add_input_pin("exec_in", "Input", "Trigger Pin", VariableType::Execution);

        // Table input
        node.add_input_pin("table", "Table", "CSV Table to loop", VariableType::Struct)
            .set_schema::<CSVTable>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        // Per-iteration outputs
        node.add_output_pin(
            "exec_out",
            "For Each Row",
            "Executes the current row",
            VariableType::Execution,
        );
        node.add_output_pin("value", "Row", "Current row object", VariableType::Struct)
            .set_open_schema();
        node.add_output_pin(
            "index",
            "Index",
            "Current row index (0-based)",
            VariableType::Integer,
        );

        // Done output
        node.add_output_pin(
            "done",
            "Done",
            "Executes once all rows are processed",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        // Prepare pins
        let done = context.get_pin_by_name("done").await?;
        context.deactivate_exec_pin_ref(&done).await?;

        let table: CSVTable = context.evaluate_pin("table").await?; // typed read

        let value = context.get_pin_by_name("value").await?;
        let exec_item = context.get_pin_by_name("exec_out").await?;
        let index = context.get_pin_by_name("index").await?;
        let connected = exec_item.get_connected_nodes();

        // Activate the iteration exec pin like the generic For Each does
        context.activate_exec_pin_ref(&exec_item).await?;

        // Iterate rows
        for (i, row) in table.rows().iter().enumerate() {
            // Build { header: value } object
            let mut obj: BTreeMap<String, FlowValue> = BTreeMap::new();
            for col_idx in 0..table.headers_ref().len() {
                let key = table.headers_ref()[col_idx].as_ref();
                let val = row.get(col_idx).unwrap_or(&Cell::Null);
                obj.insert(key.to_string(), cell_to_json(val));
            }

            // Set outputs for this iteration
            let row_value = json!(obj);
            let index_value = FlowValue::from(i as i64);
            context.override_pin_value_if_active(&value.id, &row_value);
            context.override_pin_value_if_active(&index.id, &index_value);
            value.set_value(row_value).await;
            index.set_value(index_value).await;

            // Trigger connected nodes
            for node in connected.iter() {
                let mut sub_context = context.create_sub_context(node).await;
                let run = InternalNode::trigger(&mut sub_context, &mut None, true).await;
                sub_context.end_trace();
                context.push_sub_context(&mut sub_context);

                if let Err(err) = run {
                    context.log_message(
                        &format!("Error: {:?} in iteration {}", err, i),
                        LogLevel::Error,
                    );
                }
            }
        }

        // Finish
        context.deactivate_exec_pin_ref(&exec_item).await?;
        context.activate_exec_pin_ref(&done).await?;
        Ok(())
    }
}

#[inline]
fn cell_to_json(c: &Cell) -> FlowValue {
    match c {
        Cell::Null => FlowValue::Null,
        Cell::Bool(b) => json!(*b),
        Cell::Int(i) => json!(*i),
        Cell::Float(f) => json!(*f),
        Cell::Str(s) => json!(s.as_ref()),
        Cell::Date { iso, ms } => json!({"iso": iso.as_ref(), "ms": ms}),
    }
}
