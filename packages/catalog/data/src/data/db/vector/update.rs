use super::NodeDBConnection;
use crate::data::query_params as params;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[crate::register_node]
#[derive(Default)]
pub struct UpdateLocalDatabaseNode {}

#[async_trait]
impl NodeLogic for UpdateLocalDatabaseNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "update_local_db",
            "Update",
            "Set fields on rows matching a filter. With offline buffering enabled, success means durable local acceptance; Write State reports pending cloud replay.",
            "Data/Database/Update",
        );
        node.set_flowscript_name("db", "update");
        node.set_receiver("database");
        node.set_version(1);
        node.add_icon("/flow/icons/database.svg");
        node.add_input_pin("exec_in", "Input", "", VariableType::Execution);
        node.add_input_pin(
            "database",
            "Database",
            "Database connection",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("filter", "SQL Filter", "Rows to update. Use true to update every row. Values from $name pins are bound as literals.", VariableType::String)
            .set_default_value(Some(json!("")));
        params::add_params_pin(&mut node, params::SqlFlavor::LanceFilter);
        node.add_input_pin(
            "updates",
            "Updates",
            "Object mapping column names to replacement values",
            VariableType::Struct,
        )
        .set_open_schema();
        node.add_output_pin(
            "exec_out",
            "Success",
            "Accepted by the configured store",
            VariableType::Execution,
        );
        super::add_write_receipt_outputs(&mut node);
        node
    }

    async fn on_update(&self, node: &mut Node, board: &Board) {
        node.error = None;
        params::sync_param_pins(node, "filter", board, params::SqlFlavor::LanceFilter);
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        super::publish_write_receipt(context, None, "").await?;
        let connection: NodeDBConnection = context.evaluate_pin("database").await?;
        let database = connection.load(context).await?;
        database.ensure_flushed().await?;
        let filter: String = context.evaluate_pin("filter").await?;
        let filter = params::bind_lance_filter(context, &filter).await?;
        if filter.trim().is_empty() {
            return Err(flow_like_types::anyhow!(
                "Provide an update filter; use true to update every row"
            ));
        }
        let updates: std::collections::HashMap<String, flow_like_types::Value> =
            context.evaluate_pin("updates").await?;
        if updates.is_empty() {
            return Err(flow_like_types::anyhow!(
                "Provide at least one column to update"
            ));
        }
        let guard = database.db.write().await;
        guard.inner().update(&filter, updates).await?;
        let receipt = guard.inner().last_write_receipt();
        drop(guard);
        super::publish_write_receipt(context, receipt, "applied").await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Node execution requires the execute feature"
        ))
    }
}
