use crate::data::query_params as params;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_storage::databases::vector::VectorStore;
use flow_like_types::{async_trait, json::json};

use super::NodeDBConnection;

#[crate::register_node]
#[derive(Default)]
pub struct CountLocalDatabaseNode {}

impl CountLocalDatabaseNode {
    pub fn new() -> Self {
        CountLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for CountLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "count_local_db",
            "Count",
            "Count Items",
            "Data/Database/Meta",
        );
        node.set_flowscript_name("db", "count");
        node.set_receiver("database");
        node.add_icon("/flow/icons/database.svg");
        node.set_version(2);

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
            "Optional SQL filter on the table's columns. Use $name for a value that comes from a wire — `id = $id` mints a `$id` pin, and the value is bound as a literal instead of being pasted into the predicate.",
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

        node.add_output_pin("count", "Count", "Found Items Count", VariableType::Integer);

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "filter", board, params::SqlFlavor::LanceFilter);
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;
        let filter: String = context.evaluate_pin("filter").await?;
        let filter = params::bind_lance_filter(context, &filter).await?;
        let filter: Option<String> = if filter.is_empty() {
            None
        } else {
            Some(filter)
        };
        let result = database.count(filter).await?;
        context.set_pin_value("count", json!(result)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution is not enabled. Rebuild with the execute feature flag."
        ))
    }
}
