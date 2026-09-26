use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct SetPrimaryKeyLocalDatabaseNode {}

impl SetPrimaryKeyLocalDatabaseNode {
    pub fn new() -> Self {
        SetPrimaryKeyLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for SetPrimaryKeyLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "set_primary_key_local_db",
            "Set Table Key",
            "Marks a column as the table key so concurrent Upserts on it never create duplicate rows. The column must be required, hold unique values and be a string, 32/64-bit integer or binary column. The key cannot be changed or removed afterwards; setting the current key again succeeds.",
            "Data/Database/Schema",
        );
        node.set_flowscript_name("db", "setPrimaryKey");
        node.set_receiver("database");
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "database",
            "Database",
            "Database Connection Reference",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "column_name",
            "Column Name",
            "Column that uniquely identifies each row, usually the ID Column your Upserts use",
            VariableType::String,
        )
        .set_default_value(Some(json!("id")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "The column is the table key",
            VariableType::Execution,
        );
        node.add_output_pin(
            "schema",
            "Schema",
            "Updated database schema",
            VariableType::Struct,
        )
        .set_schema::<crate::data::db::table_schema::TableSchema>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let column_name: String = context.evaluate_pin("column_name").await?;

        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;

        database.inner().set_primary_key(&column_name).await?;

        let schema = database.schema().await?;
        context.set_pin_value("schema", json!(schema)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}
