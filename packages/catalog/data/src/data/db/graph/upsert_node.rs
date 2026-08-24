use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_catalog_core::NodeGraphConnection;
use flow_like_types::{async_trait, json::json};

/// # Upsert Graph Node
/// Inserts or updates a node in a graph overlay's underlying table.
#[crate::register_node]
#[derive(Default)]
pub struct UpsertGraphNodeNode {}

impl UpsertGraphNodeNode {
    pub fn new() -> Self {
        UpsertGraphNodeNode {}
    }
}

#[async_trait]
impl NodeLogic for UpsertGraphNodeNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "upsert_graph_node",
            "Upsert Graph Node",
            "Inserts or updates a node in the graph overlay's underlying table",
            "Data/Database/Graph/Write",
        );
        node.set_flowscript_name("db.graph", "upsertNode");
        node.set_receiver("graph");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "graph",
            "Graph Connection",
            "Graph connection reference",
            VariableType::Struct,
        )
        .set_schema::<NodeGraphConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "label",
            "Node Label",
            "Label of the node type to upsert into",
            VariableType::String,
        );
        node.add_input_pin(
            "value",
            "Value",
            "Node data as JSON object",
            VariableType::Struct,
        )
        .set_open_schema();

        node.add_output_pin(
            "exec_out",
            "Success",
            "Upsert succeeded",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Upsert failed", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let conn: NodeGraphConnection = context.evaluate_pin("graph").await?;
        let label: String = context.evaluate_pin("label").await?;
        let value: flow_like_types::Value = context.evaluate_pin("value").await?;

        let store = super::load_graph_store(context, &conn.cache_key).await?;
        let overlay = store.overlay();

        let node_def = overlay
            .nodes
            .iter()
            .find(|n| n.label == label)
            .ok_or_else(|| {
                flow_like_types::anyhow!("Node label '{}' not found in overlay", label)
            })?;

        let table_name = node_def.table.clone();
        let id_column = node_def.id_column.clone();

        // Open underlying table and upsert via VectorStore pattern
        let connection = store.connection();
        let table = connection
            .open_table(&table_name)
            .execute()
            .await
            .map_err(|e| {
                flow_like_types::anyhow!("Failed to open table '{}': {}", table_name, e)
            })?;

        let rows = vec![value];
        let batch = flow_like_storage::arrow_utils::value_to_record_batch(rows)?;

        let schema = batch.schema();
        let reader: Box<dyn flow_like_storage::arrow::record_batch::RecordBatchReader + Send> =
            Box::new(
                flow_like_storage::arrow::record_batch::RecordBatchIterator::new(
                    vec![Ok(batch)],
                    schema,
                ),
            );
        let mut merger = table.merge_insert(&[&id_column]);
        merger
            .when_matched_update_all(None)
            .when_not_matched_insert_all();
        match merger.execute(reader).await {
            Ok(_) => {}
            Err(e) => {
                context.log_message(
                    &format!("Database graph-node upsert failed: {e:#}"),
                    LogLevel::Error,
                );
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
                return Ok(());
            }
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the 'execute' feature flag."
        ))
    }
}
