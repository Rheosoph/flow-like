use crate::data::datafusion::session::DataFusionSession;
use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::data::datafusion::session::{CachedDataFusionSession, DeferredMount};
#[cfg(feature = "execute")]
use crate::data::excel::table_detect::{ExtractConfig, SheetTableMode, extract_workbook_tables};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
#[cfg(feature = "execute")]
use flow_like_types::tokio;
#[cfg(feature = "execute")]
use std::sync::Arc;

/// Download, parse and register the workbook's tables, returning the registered names.
/// Shared by the eager path and the deferred mount.
#[cfg(feature = "execute")]
async fn extract_and_register(
    context: &mut ExecutionContext,
    session: &CachedDataFusionSession,
    flow_path: &FlowPath,
    sheet_filter: Option<String>,
    mode: SheetTableMode,
    prefix: String,
) -> flow_like_types::Result<Vec<String>> {
    let file_buffer = flow_path.get(context, false).await?;
    let source = flow_path.clone();

    let result = tokio::task::spawn_blocking(move || {
        extract_workbook_tables(
            file_buffer,
            sheet_filter.as_deref(),
            &ExtractConfig::default(),
            mode,
            &prefix,
            Some(source),
        )
    })
    .await??;

    for warning in &result.warnings {
        context.log_message(warning, LogLevel::Warn);
    }
    if result.tables.is_empty() {
        return Err(flow_like_types::anyhow!(
            "No tables could be extracted from the Excel file"
        ));
    }

    let mut names: Vec<String> = Vec::with_capacity(result.tables.len());
    for table in &result.tables {
        let name = table
            .name
            .clone()
            .unwrap_or_else(|| format!("table_{}", names.len() + 1));
        // Retry-safe: a previous partially-failed run may already have registered
        // some of the workbook's tables.
        let _ = session.ctx.deregister_table(
            flow_like_storage::datafusion::common::TableReference::bare(name.clone()),
        );
        table.register_with_datafusion(&session.ctx, &name)?;
        context.log_message(
            &format!(
                "Registered '{}' ({} rows{})",
                name,
                table.row_count(),
                table
                    .range
                    .as_deref()
                    .map(|r| format!(", range {r}"))
                    .unwrap_or_default()
            ),
            LogLevel::Debug,
        );
        names.push(name);
    }

    Ok(names)
}

/// Deferred variant: the download and whole-workbook parse — the most expensive mount
/// in the catalog — only happen once a consumer actually queries the session.
#[cfg(feature = "execute")]
struct ExcelWorkbookMount {
    flow_path: FlowPath,
    sheet_filter: Option<String>,
    mode: SheetTableMode,
    prefix: String,
}

#[cfg(feature = "execute")]
#[async_trait]
impl DeferredMount for ExcelWorkbookMount {
    fn describe(&self) -> String {
        format!("Excel workbook '{}'", self.flow_path.path)
    }

    fn dedupe_key(&self) -> Option<String> {
        Some(format!(
            "excel:{}:{}:{}:{:?}:{}",
            self.flow_path.store_ref,
            self.flow_path.path,
            self.sheet_filter.as_deref().unwrap_or(""),
            self.mode,
            self.prefix
        ))
    }

    async fn mount(
        &self,
        session: &CachedDataFusionSession,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        extract_and_register(
            context,
            session,
            &self.flow_path,
            self.sheet_filter.clone(),
            self.mode,
            self.prefix.clone(),
        )
        .await
        .map(|_| ())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct RegisterExcelNode {}

impl RegisterExcelNode {
    pub fn new() -> Self {
        RegisterExcelNode {}
    }
}

#[async_trait]
impl NodeLogic for RegisterExcelNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "df_register_excel",
            "Register Excel (DataFusion)",
            "Registers an Excel workbook's sheets as SQL tables in a DataFusion session. Tables are named after their normalized sheet names (e.g. 'Sales Data (2024)' becomes 'sales_data_2024'); additional tables on the same sheet get numeric suffixes. The download and parse are deferred until a query actually uses the session — unless the Table Names output is connected, which requires parsing here.",
            "Data/DataFusion",
        );
        node.set_flowscript_name("df", "registerExcel");
        node.set_receiver("session");
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
            "DataFusion session to register the tables into",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>();

        node.add_input_pin("file", "File", "Excel file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "sheet",
            "Sheet",
            "Worksheet name (optional - if empty, registers all sheets)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "mode",
            "Mode",
            "'Sheet as table' registers each sheet's used range as one table; 'Detect tables' finds and registers every table on each sheet",
            VariableType::String,
        )
        .set_default_value(Some(json!("Sheet as table")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "Sheet as table".to_string(),
                    "Detect tables".to_string(),
                ])
                .build(),
        );

        node.add_input_pin(
            "prefix",
            "Name Prefix",
            "Optional prefix for the registered table names",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Tables registered successfully",
            VariableType::Execution,
        );

        node.add_output_pin(
            "table_names",
            "Table Names",
            "Names the tables were registered under. Connecting this pin makes the workbook parse eagerly at this node instead of at the first query.",
            VariableType::String,
        )
        .set_value_type(ValueType::Array);

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

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let flow_path: FlowPath = context.evaluate_pin("file").await?;
        let sheet_input: String = context.evaluate_pin("sheet").await.unwrap_or_default();
        let mode_input: String = context.evaluate_pin("mode").await.unwrap_or_default();
        let prefix: String = context.evaluate_pin("prefix").await.unwrap_or_default();

        let mode = if mode_input == "Detect tables" {
            SheetTableMode::DetectTables
        } else {
            SheetTableMode::WholeSheet
        };
        let sheet_filter = (!sheet_input.trim().is_empty()).then_some(sheet_input);
        let prefix = prefix.trim().to_string();

        // The workbook has to be parsed to know the table names, so the parse can only
        // be deferred while nothing consumes them.
        let names_consumed = !context
            .get_pin_by_name("table_names")
            .await?
            .connected_to()
            .is_empty();

        if names_consumed {
            // Eager registration must not jump ahead of mounts deferred earlier in the
            // board, so drain the queue first via a full load.
            let cached_session = session.load(context).await?;
            let names = extract_and_register(
                context,
                &cached_session,
                &flow_path,
                sheet_filter,
                mode,
                prefix,
            )
            .await?;
            context.set_pin_value("table_names", json!(names)).await?;
        } else {
            let cached_session = session.load_lazy(context).await?;
            context.log_message(
                "Deferring the Excel workbook parse until the session is queried",
                LogLevel::Debug,
            );
            cached_session
                .defer_mount(Arc::new(ExcelWorkbookMount {
                    flow_path,
                    sheet_filter,
                    mode,
                    prefix,
                }))
                .await;
            context
                .set_pin_value("table_names", json!(Vec::<String>::new()))
                .await?;
        }

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Excel registration requires the 'execute' feature"
        ))
    }
}
