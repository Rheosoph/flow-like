use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct DropIndexNode {}

impl DropIndexNode {
    pub fn new() -> Self {
        DropIndexNode {}
    }
}

#[async_trait]
impl NodeLogic for DropIndexNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "drop_index_db",
            "Drop Index",
            "Remove an index from a database table",
            "Data/Database/Optimization",
        );
        node.set_flowscript_name("db", "dropIndex");
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
            "index_name",
            "Index Name",
            "Name of the index to drop",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done dropping index",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let index_name: String = context.evaluate_pin("index_name").await?;

        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;

        database.inner().drop_index(&index_name).await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
