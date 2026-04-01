use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::async_trait;

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct FlushLocalDatabaseNode {}

impl FlushLocalDatabaseNode {
    pub fn new() -> Self {
        FlushLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for FlushLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "flush_local_db",
            "Flush Database",
            "Flush any buffered writes to storage immediately",
            "Data/Database/Optimization",
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
            "Output",
            "Flush complete",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;
        if cached_db.db.read().await.is_dirty() {
            cached_db.db.write().await.flush().await?;
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
