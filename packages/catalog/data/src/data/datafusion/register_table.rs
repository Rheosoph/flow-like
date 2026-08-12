use crate::data::datafusion::session::{CachedDataFusionSession, DataFusionSession, DeferredMount};
use crate::data::excel::CSVTable;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    variable::VariableType,
};
use flow_like_storage::datafusion::common::TableReference;
use flow_like_types::{async_trait, json::json};
use std::sync::Arc;

/// Building the Arrow MemTable is deferred to the first query, so a cached query never
/// pays for the conversion.
struct CsvTableMount {
    table: CSVTable,
    table_name: String,
}

#[async_trait]
impl DeferredMount for CsvTableMount {
    fn describe(&self) -> String {
        format!("table '{}' from an in-memory CSVTable", self.table_name)
    }

    fn dedupe_key(&self) -> Option<String> {
        Some(format!("table:{}", self.table_name))
    }

    async fn mount(
        &self,
        session: &CachedDataFusionSession,
        _context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let _ = session
            .ctx
            .deregister_table(TableReference::bare(self.table_name.clone()));
        self.table
            .register_with_datafusion(&session.ctx, &self.table_name)
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterCSVTableNode {}

impl RegisterCSVTableNode {
    pub fn new() -> Self {
        RegisterCSVTableNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterCSVTableNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_csv_table",
            "Register Table",
            "Register a CSVTable (from Excel/CSV extraction) into a DataFusion session for SQL queries. Converts the table to an in-memory Arrow table.",
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
            "table",
            "Table",
            "CSVTable to register (from Excel/CSV extraction nodes)",
            VariableType::Struct,
        )
        .set_schema::<CSVTable>();

        node.add_input_pin(
            "table_name",
            "Table Name",
            "Name to register the table as in the DataFusion catalog",
            VariableType::String,
        )
        .set_default_value(Some(json!("data")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Table registered successfully",
            VariableType::Execution,
        );

        node.scores = Some(NodeScores {
            privacy: 10,
            security: 10,
            performance: 8,
            governance: 9,
            reliability: 9,
            cost: 10,
        });

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let table: CSVTable = context.evaluate_pin("table").await?;
        let table_name: String = context.evaluate_pin("table_name").await?;

        let cached_session = session.load_lazy(context).await?;
        cached_session
            .defer_mount(Arc::new(CsvTableMount { table, table_name }))
            .await;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}
