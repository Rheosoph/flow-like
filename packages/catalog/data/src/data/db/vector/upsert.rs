use flow_like::flow::{
    execution::{LogLevel, context::ExecutionContext},
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct UpsertLocalDatabaseNode {}

impl UpsertLocalDatabaseNode {
    pub fn new() -> Self {
        UpsertLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for UpsertLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "upsert_local_db",
            "Upsert",
            "Inserts if the Item does not exist, Updates if it does",
            "Data/Database/Insert",
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
        node.add_input_pin("id_row", "ID Column", "The ID Column", VariableType::String);

        node.add_input_pin("value", "Value", "Value to Insert", VariableType::Struct)
        .set_open_schema();

        node.add_output_pin(
            "exec_out",
            "Success",
            "Upsert succeeded",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Upsert failed", VariableType::Execution);
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
        let database = database.load(context).await?;
        let id_row: String = context.evaluate_pin("id_row").await?;
        let value: Value = context.evaluate_pin("value").await?;
        let value = vec![value];

        match database.upsert_from(context, value, id_row).await {
            Ok(()) => {
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(&format!("Database upsert failed: {e:#}"), LogLevel::Error);
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BatchUpsertLocalDatabaseNode {}

impl BatchUpsertLocalDatabaseNode {
    pub fn new() -> Self {
        BatchUpsertLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for BatchUpsertLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "batch_upsert_local_db",
            "Batch Upsert",
            "Inserts if the Item does not exist, Updates if it does",
            "Data/Database/Insert",
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
        node.add_input_pin("id_row", "ID Column", "The ID Column", VariableType::String);

        node.add_input_pin("value", "Value", "Value to Insert", VariableType::Struct)
            .set_value_type(ValueType::Array)
        .set_open_schema();

        node.add_output_pin(
            "exec_out",
            "Success",
            "Upsert succeeded",
            VariableType::Execution,
        );
        node.add_output_pin("error", "Error", "Upsert failed", VariableType::Execution);
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
        let database = database.load(context).await?;
        let value: Vec<Value> = context.evaluate_pin("value").await?;
        let id_row: String = context.evaluate_pin("id_row").await?;

        match database.upsert_from(context, value, id_row).await {
            Ok(()) => {
                context.activate_exec_pin("exec_out").await?;
            }
            Err(e) => {
                context.log_message(
                    &format!("Database batch upsert failed: {e:#}"),
                    LogLevel::Error,
                );
                context
                    .set_pin_value("error_message", json!(e.to_string()))
                    .await?;
                context.activate_exec_pin("error").await?;
            }
        }

        Ok(())
    }
}
