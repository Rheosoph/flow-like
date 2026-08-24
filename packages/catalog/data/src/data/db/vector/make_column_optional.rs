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
pub struct MakeColumnOptionalLocalDatabaseNode {}

impl MakeColumnOptionalLocalDatabaseNode {
    pub fn new() -> Self {
        MakeColumnOptionalLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for MakeColumnOptionalLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "make_column_optional_local_db",
            "Make Column Optional",
            "Marks a column as optional (nullable).",
            "Data/Database/Schema",
        );
        node.set_flowscript_name("db", "makeColumnOptional");
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
            "Name of the column",
            VariableType::String,
        )
        .set_default_value(Some(json!("new_column")));

        node.add_input_pin(
            "optional",
            "Optional",
            "True = nullable, false = required",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done altering schema",
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

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let column_name: String = context.evaluate_pin("column_name").await?;
        let optional: bool = context.evaluate_pin("optional").await.unwrap_or(true);

        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;

        database
            .inner()
            .make_column_nullable(&column_name, optional)
            .await?;

        let schema = database.schema().await?;
        context.set_pin_value("schema", json!(schema)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
