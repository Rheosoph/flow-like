use crate::data::excel::CSVTable;
use crate::data::excel::table_detect::ExtractConfig;
use crate::data::path::FlowPath;
use flow_like::flow::node::NodeLogic;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::Node,
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use crate::data::excel::table_detect::{SheetTableMode, extract_workbook_tables};
#[cfg(feature = "execute")]
use flow_like::flow::execution::LogLevel;
#[cfg(feature = "execute")]
use flow_like_types::tokio;

#[crate::register_node]
#[derive(Default)]
pub struct ExtractExcelTablesNode {}

impl ExtractExcelTablesNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for ExtractExcelTablesNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "data_excel_extract_tables",
            "Extract Tables (Excel)",
            "Detects and extracts all tables from Excel worksheets, handling titles, multi-row headers, merged cells, footnotes and multiple tables per sheet",
            "Data/Excel",
        );
        node.set_flowscript_name("excel", "extractTables");
        node.add_icon("/flow/icons/file-spreadsheet.svg");
        node.set_version(2);

        node.add_input_pin("exec_in", "Input", "Trigger", VariableType::Execution);

        node.add_input_pin("file", "File", "Excel file", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "sheet",
            "Sheet",
            "Worksheet name (optional - if empty, extracts from all sheets)",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "extract_config",
            "Extract Config",
            "Table detection configuration",
            VariableType::Struct,
        )
        .set_schema::<ExtractConfig>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(ExtractConfig::default())));

        node.add_output_pin("exec_out", "Output", "Next", VariableType::Execution);
        node.add_output_pin(
            "tables",
            "Tables",
            "Extracted tables (name, title, A1 range and typed rows)",
            VariableType::Struct,
        )
        .set_schema::<CSVTable>()
        .set_value_type(ValueType::Array);

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let flow_path: FlowPath = context.evaluate_pin("file").await?;
        let sheet_input: String = context.evaluate_pin("sheet").await?;
        let cfg: ExtractConfig = context
            .evaluate_pin("extract_config")
            .await
            .unwrap_or_default();

        let file_buffer = flow_path.get(context, false).await?;
        let source = flow_path.clone();

        let result = tokio::task::spawn_blocking(move || {
            let filter = (!sheet_input.trim().is_empty()).then_some(sheet_input);
            extract_workbook_tables(
                file_buffer,
                filter.as_deref(),
                &cfg,
                SheetTableMode::DetectTables,
                "",
                Some(source),
            )
        })
        .await??;

        for warning in &result.warnings {
            context.log_message(warning, LogLevel::Warn);
        }
        context.log_message(
            &format!("Extracted {} table(s)", result.tables.len()),
            LogLevel::Debug,
        );

        context
            .set_pin_value("tables", json!(result.tables))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Table extraction requires the 'execute' feature"
        ))
    }
}
