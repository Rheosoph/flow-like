use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct DropColumnLocalDatabaseNode {}

impl DropColumnLocalDatabaseNode {
    pub fn new() -> Self {
        DropColumnLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for DropColumnLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "drop_column_local_db",
            "Drop Column",
            "Drops a column from the database table.",
            "Data/Database/Schema",
        );
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
            "Name of the column to drop",
            VariableType::String,
        )
        .set_default_value(Some(json!("new_column")));

        node.add_output_pin("exec_out", "Done", "Done altering schema", VariableType::Execution);
        node.add_output_pin("schema", "Schema", "Updated database schema", VariableType::Struct);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let column_name: String = context.evaluate_pin("column_name").await?;

        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;

        database.inner().drop_columns(&[column_name.as_str()]).await?;

        let schema = database.schema().await?;
        context.set_pin_value("schema", json!(schema)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}