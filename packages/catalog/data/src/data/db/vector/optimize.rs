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
pub struct OptimizeLocalDatabaseNode {}

impl OptimizeLocalDatabaseNode {
    pub fn new() -> Self {
        OptimizeLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for OptimizeLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "optimize_local_db",
            "Optimize and Update",
            "Optimize and Update the Database",
            "Data/Database/Optimization",
        );
        node.set_flowscript_name("db", "optimize");
        node.set_receiver("database");
        node.set_long_running(true);
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
            "keep_versions",
            "Keep Versions?",
            "Otherwise deletes old versions",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

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
        let keep_versions: bool = context.evaluate_pin("keep_versions").await?;
        database.optimize(keep_versions).await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
