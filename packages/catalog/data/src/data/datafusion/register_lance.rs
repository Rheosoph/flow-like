use crate::data::datafusion::session::{CachedDataFusionSession, DataFusionSession, DeferredMount};
use crate::data::db::vector::NodeDBConnection;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_storage::datafusion::common::TableReference;
use flow_like_types::{async_trait, json::json};
use std::sync::Arc;

/// Flushing the Lance database and building its DataFusion adapter are deferred to the
/// first query. Data written to the database between this node and the first query is
/// therefore included — the flush happens at materialization time.
struct LanceTableMount {
    database: NodeDBConnection,
    table_name: String,
}

#[async_trait]
impl DeferredMount for LanceTableMount {
    fn describe(&self) -> String {
        if self.table_name.is_empty() {
            "a Lance table (name taken from the database)".to_string()
        } else {
            format!("Lance table '{}'", self.table_name)
        }
    }

    fn dedupe_key(&self) -> Option<String> {
        // An empty name is resolved from the database only at mount time, so there is
        // nothing safe to dedupe on.
        (!self.table_name.is_empty()).then(|| format!("table:{}", self.table_name))
    }

    async fn mount(
        &self,
        session: &CachedDataFusionSession,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let (cached_db, generation) = self.database.load_with_generation(context).await?;
        cached_db.ensure_flushed().await?;
        let db_guard = cached_db.db.read().await;
        let inner = db_guard.inner();

        let table_name = if self.table_name.is_empty() {
            inner.table_name().to_string()
        } else {
            self.table_name.clone()
        };

        let df_adapter = inner.to_datafusion().await?;
        drop(db_guard);

        // Retry-safe: a previous partially-failed run of this mount may already have
        // registered the name.
        let _ = session
            .ctx
            .deregister_table(TableReference::bare(table_name.clone()));
        session.ctx.register_table(
            TableReference::bare(table_name.clone()),
            Arc::new(df_adapter),
        )?;
        session
            .track_lance_table(table_name, self.database.clone(), generation)
            .await;

        Ok(())
    }
}

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
        let table_name: String = context.evaluate_pin("table_name").await?;

        let cached_session = session.load_lazy(context).await?;
        cached_session
            .defer_mount(Arc::new(LanceTableMount {
                database,
                table_name,
            }))
            .await;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
