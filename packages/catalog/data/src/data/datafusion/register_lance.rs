use crate::data::datafusion::session::DataFusionSession;
use crate::data::db::vector::NodeDBConnection;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_storage::datafusion::common::TableReference;
use flow_like_types::{async_trait, json::json};
use std::sync::Arc;

#[crate::register_node]
#[derive(Default)]
pub struct RegisterLanceTableNode {}

impl RegisterLanceTableNode {
    pub fn new() -> Self {
        RegisterLanceTableNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterLanceTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_lance",
            "Register Lance Table",
            "Register a LanceDB table into a DataFusion session for SQL queries. Uses the existing to_datafusion() implementation from the vector store.",
            "Data/DataFusion",
        );
        node.add_icon("/flow/icons/database.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger execution",
            VariableType::Execution,
        );

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session to register the table into",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin(
            "database",
            "Database",
            "LanceDB database connection",
            VariableType::Struct,
        )
        .set_schema::<NodeDBConnection>();

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register the table as in the DataFusion catalog. If empty, uses the database's original table name.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered successfully",
            VariableType::Execution,
        );

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 9,
            governance: 9,
            reliability: 9,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let database: NodeDBConnection = context.evaluate_pin("database").await?;
        let mut table_name: String = context.evaluate_pin("table_name").await?;

        let cached_session = session.load(context).await?;
        let cached_db = database.load(context).await?;
        cached_db.ensure_flushed().await?;
        let db_guard = cached_db.db.read().await;
        let inner = db_guard.inner();

        if table_name.is_empty() {
            table_name = inner.table_name().to_string();
        }

        let df_adapter = inner.to_datafusion().await?;

        cached_session
            .ctx
            .register_table(TableReference::bare(table_name), Arc::new(df_adapter))?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
