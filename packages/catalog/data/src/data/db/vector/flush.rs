use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

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
            "Success",
            "Flush complete",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Flush failed", VariableType::Execution);
        node.add_output_pin(
            "error_message",
            "Error Message",
            "Error details",
            VariableType::String,
        );

        node.set_version(2);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        context.deactivate_exec_pin("error").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;

        if let Err(e) = cached_db.ensure_flushed().await {
            context.log_message(&format!("Database flush failed: {e:#}"), LogLevel::Error);
            context
                .set_pin_value("error_message", json!(e.to_string()))
                .await?;
            context.activate_exec_pin("error").await?;
            return Ok(());
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
