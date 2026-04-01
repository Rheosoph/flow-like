use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_storage::databases::vector::lancedb::IndexConfigDto;
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct ListIndicesNode {}

impl ListIndicesNode {
    pub fn new() -> Self {
        ListIndicesNode {}
    }
}

#[async_trait]
impl NodeLogic for ListIndicesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "list_indices_db",
            "List Indices",
            "Lists all indices on a database table",
            "Data/Database/Meta",
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

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done listing indices",
            VariableType::Execution,
        );

        node.add_output_pin(
            "indices",
            "Indices",
            "List of indices on the table",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array)
        .set_schema::<IndexConfigDto>();

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;

        let indices = database.inner().list_indices().await?;

        context.set_pin_value("indices", json!(indices)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
