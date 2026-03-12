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
pub struct DeleteLocalDatabaseNode {}

impl DeleteLocalDatabaseNode {
    pub fn new() -> Self {
        DeleteLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for DeleteLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "filter_delete_local_db",
            "Delete",
            "Filter Database",
            "Data/Database/Delete",
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
            "filter",
            "SQL Filter",
            "Optional SQL Filter",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Created Database",
            "Done Creating Database",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;
        let filter: String = context.evaluate_pin("filter").await?;
        database.delete(&filter).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
