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
pub struct HybridSearchLocalDatabaseNode {}

impl HybridSearchLocalDatabaseNode {
    pub fn new() -> Self {
        HybridSearchLocalDatabaseNode {}
    }
}

#[async_trait]
impl NodeLogic for HybridSearchLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "hybrid_search_local_db",
            "Hybrid Search",
            "Searches the Database using both Vector and Full-Text Search",
            "Data/Database/Search",
        );
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
            "search",
            "Search Term",
            "Full Text Search Term",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin("vector", "Vector", "Vector to Search", VariableType::Float)
            .set_value_type(ValueType::Array);
        node.add_input_pin(
            "fields",
            "Fields",
            "Column names for both vector (first) and FTS search",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(json!([])));
        node.add_input_pin(
            "filter",
            "SQL Filter",
            "Optional SQL filter on the table's columns. Use $name for a value that comes from a wire — `id = $id` mints a `$id` pin, and the value is bound as a literal instead of being pasted into the predicate.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        params::add_params_pin(&mut node, params::SqlFlavor::LanceFilter);

        node.add_input_pin(
            "rerank",
            "Re-Rank",
            "Should the items be reranked using RRF?",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));

        node.add_input_pin("limit", "Limit", "Limit", VariableType::Integer)
            .set_default_value(Some(json!(10)));

        node.add_input_pin("offset", "Offset", "Offset", VariableType::Integer)
            .set_default_value(Some(json!(0)));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Done Searching Database",
            VariableType::Execution,
        );

        node.add_output_pin("values", "Values", "Found Items", VariableType::Struct)
            .set_value_type(ValueType::Array)
            .set_open_schema();

        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "filter", board, params::SqlFlavor::LanceFilter);
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let vector: Vec<f64> = context.evaluate_pin("vector").await?;
        let search: String = context.evaluate_pin("search").await?;
        let fields: Vec<String> = context.evaluate_pin("fields").await.unwrap_or(vec![]);
        let fields = if fields.is_empty() {
            None
        } else {
            Some(fields)
        };
        let filter: String = context.evaluate_pin("filter").await?;
        let filter = params::bind_lance_filter(context, &filter).await?;
        let filter: Option<&str> = if filter.is_empty() {
            None
        } else {
            Some(&filter)
        };
        let limit: i64 = context.evaluate_pin("limit").await?;
        let offset: i64 = context.evaluate_pin("offset").await?;
        let rerank: bool = context.evaluate_pin("rerank").await?;
        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let database = cached_db.db.read().await;
        let results = database
            .hybrid_search(
                vector,
                &search,
                filter,
                None,
                fields,
                limit as usize,
                offset as usize,
                rerank,
            )
            .await?;
        context.set_pin_value("values", json!(results)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
