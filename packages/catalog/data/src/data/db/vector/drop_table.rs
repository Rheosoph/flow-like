use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::async_trait;

use super::NodeDBConnection;

/// # Drop Table
/// Permanently deletes a whole table, data and schema, and prunes graph
/// overlays that referenced it.
#[crate::register_node]
#[derive(Default)]
pub struct DropTableLocalDatabaseNode {}

impl DropTableLocalDatabaseNode {
    pub fn new() -> Self {
        DropTableLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for DropTableLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "drop_table_local_db",
            "Drop Table",
            "Permanently deletes the entire table, both its rows and its schema, so it can be recreated later with a different schema. This is irreversible and cannot be undone. Buffered writes that have not been flushed yet are discarded instead of written back. Graph overlays referencing the table are pruned and reported on References; saved queries are never modified. Known limitation: a DataFusion table provider registered from this table earlier in the same run keeps pointing at the deleted dataset, because mounts are only refreshed when the credential generation changes.",
            "Data/Database/Delete",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );
        node.add_input_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done dropping the table",
            VariableType::Execution,
        );
        node.add_output_pin(
            "dropped",
            "Dropped",
            "True when the table existed and was removed",
            VariableType::Boolean,
        );
        node.add_output_pin(
            "references",
            "References",
            "Names of the graph overlays that referenced the table and were pruned",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

        node.set_scores(
            NodeScores::new()
                .set_privacy(8)
                .set_security(4)
                .set_performance(8)
                .set_governance(2)
                .set_reliability(3)
                .set_cost(9)
                .build(),
        );

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        use flow_like::flow::execution::LogLevel;
        use flow_like_storage::databases::{
            table_cascade::prune_table_references, vector::VectorStore,
        };
        use flow_like_types::json::json;

        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;

        let mut db = cached_db.db.write().await;
        let table_name = db.inner().table_name().to_string();
        if flow_like_catalog_core::is_reserved_table(&table_name) {
            return Err(flow_like_types::anyhow!(
                "Table '{table_name}' is reserved for internal use and cannot be dropped"
            ));
        }

        let discarded_writes = db.is_dirty();
        if discarded_writes {
            db.discard_buffer();
        }

        let existed = db
            .inner()
            .list_tables()
            .await?
            .iter()
            .any(|name| name == &table_name);

        let connection = db.inner().connection().clone();
        let report = prune_table_references(&connection, &table_name).await;

        db.inner_mut().drop_table().await?;
        drop(db);

        if discarded_writes {
            context.log_message(
                &format!("Discarded buffered writes for dropped table '{table_name}'"),
                LogLevel::Warn,
            );
        }
        for warning in &report.warnings {
            context.log_message(warning, LogLevel::Warn);
        }
        if !report.saved_queries.is_empty() {
            context.log_message(
                &format!(
                    "Saved queries still referencing '{table_name}': {}",
                    report.saved_queries.join(", ")
                ),
                LogLevel::Warn,
            );
        }

        context.set_pin_value("dropped", json!(existed)).await?;
        context
            .set_pin_value("references", json!(report.ontologies))
            .await?;
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
