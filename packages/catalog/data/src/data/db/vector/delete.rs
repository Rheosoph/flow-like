use crate::data::query_params as params;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
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
            "Delete rows from a database table and return the removed rows",
            "Data/Database/Delete",
        );
        node.add_icon("/flow/icons/database.svg");
        node.set_version(3);

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
            "Optional SQL filter on the table's columns; leave empty to delete all rows. Use $name for a value that comes from a wire — `id = $id` mints a `$id` pin, and the value is bound as a literal instead of being pasted into the predicate.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        params::add_params_pin(&mut node, params::SqlFlavor::LanceFilter);

        node.add_output_pin(
            "exec_out",
            "Created Database",
            "Done Creating Database",
            VariableType::Execution,
        );

        node.add_output_pin(
            "deleted_values",
            "Deleted Values",
            "Rows that were deleted",
            VariableType::Struct,
        )
        .set_value_type(ValueType::Array);

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "filter", board, params::SqlFlavor::LanceFilter);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;
        let filter: String = context.evaluate_pin("filter").await?;
        let filter = params::bind_lance_filter(context, &filter).await?;

        let normalized_filter = filter.trim().to_string();

        let deleted_values = if normalized_filter.is_empty() {
            let count = database.count(None).await?;
            if count == 0 {
                Vec::new()
            } else {
                database.list(None, count, 0).await?
            }
        } else {
            let count = database.count(Some(normalized_filter.clone())).await?;
            if count == 0 {
                Vec::new()
            } else {
                database.filter(&normalized_filter, None, count, 0).await?
            }
        };

        if normalized_filter.is_empty() {
            database.delete("true").await?;
        } else {
            database.delete(&normalized_filter).await?;
        }

        context
            .set_pin_value("deleted_values", json!(deleted_values))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
