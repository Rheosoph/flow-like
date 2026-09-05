use flow_like::{
    a2ui::components::NivoChartProps,
    bit::Bit,
    flow::{
        execution::context::ExecutionContext,
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_types::{async_trait, json::json};

#[cfg(feature = "execute")]
use flow_like_catalog_data_support::data::datafusion::query::batches_to_rows;
use flow_like_catalog_data_support::data::datafusion::session::DataFusionSession;
#[cfg(feature = "execute")]
use flow_like_model_provider::history::{History, HistoryMessage, Role};
#[cfg(feature = "execute")]
use flow_like_storage::datafusion::arrow::array::StringArray;
#[cfg(feature = "execute")]
use flow_like_types::Value;

const CHART_TYPE_OPTIONS: &[&str] = &[
    "auto", "bar", "line", "pie", "radar", "heatmap", "scatter", "funnel",
];

#[cfg(feature = "execute")]
fn get_chart_format_description(chart_type: &str) -> &'static str {
    match chart_type {
        "bar" | "auto" => {
            r#"Bar chart — array of category objects with numeric values.
Example: [{"category":"Q1","sales":100,"profit":20},{"category":"Q2","sales":150,"profit":35}]
Alias the category column to "category" and each metric to a descriptive key."#
        }
        "line" => {
            r#"Line chart — array of series, each with an id and data: [{x, y}].
Example: [{"id":"Revenue","data":[{"x":"Jan","y":100},{"x":"Feb","y":120}]}]"#
        }
        "pie" => {
            r#"Pie/Donut chart — array of slices with id, label, value.
Example: [{"id":"chrome","label":"Chrome","value":45},{"id":"firefox","label":"Firefox","value":30}]"#
        }
        "radar" => {
            r#"Radar chart — array of dimension rows with values per series.
Example: [{"dimension":"Speed","a":70,"b":90},{"dimension":"Cost","a":50,"b":60}]"#
        }
        "heatmap" => {
            r#"Heatmap — array of row objects, each with id and data: [{x, y}].
Example: [{"id":"Mon","data":[{"x":"9am","y":5},{"x":"10am","y":12}]}]"#
        }
        "scatter" => {
            r#"Scatter plot — array of series with data: [{x, y}] (numeric).
Example: [{"id":"GroupA","data":[{"x":10,"y":20},{"x":15,"y":30}]}]"#
        }
        "funnel" => {
            r#"Funnel chart — array of stages with id, label, value (descending).
Example: [{"id":"visits","label":"Visits","value":10000},{"id":"signups","label":"Sign Ups","value":2000}]"#
        }
        _ => "Array of flat objects with string/numeric fields suitable for the requested chart.",
    }
}

#[cfg(feature = "execute")]
fn build_system_prompt(chart_type: &str, schema_desc: &str, description: &str) -> String {
    let fmt = get_chart_format_description(chart_type);
    format!(
        r#"You are a data analyst. Generate a DataFusion SQL query for the task below and format the output for a {chart_type} chart.

## Table Schema
{schema_desc}

## Chart Format ({chart_type})
{fmt}

## Task
{description}

## Rules
- Write a single SELECT statement only; no DDL or DML.
- Use column aliases so results directly match the chart format keys.
- Aggregate where appropriate (SUM, COUNT, AVG).
- Limit to <= 50 rows for visualisation.

## Response
Respond with ONLY a JSON object (no markdown fences):
{{"sql":"<query>","explanation":"<one-sentence description>"}}"#,
        chart_type = chart_type,
        schema_desc = schema_desc,
        fmt = fmt,
        description = description,
    )
}

#[cfg(feature = "execute")]
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end >= start {
        Some(&text[start..=end])
    } else {
        None
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ChartDataAgent;

impl ChartDataAgent {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for ChartDataAgent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_chart_data_agent",
            "Chart Data Agent",
            "Uses an LLM to write and run SQL against a DataFusion session, returning chart-ready struct data.",
            "UI/Elements/Charts/Agent",
        );
        node.set_flowscript_name("ui", "chartDataAgent");
        node.set_version(5);
        node.add_icon("/flow/icons/a2ui.svg");

        node.set_scores(
            NodeScores::new()
                .set_privacy(6)
                .set_security(7)
                .set_performance(5)
                .set_governance(7)
                .set_reliability(6)
                .set_cost(4)
                .build(),
        );

        node.add_input_pin(
            "exec_in",
            "Generate",
            "Trigger data generation",
            VariableType::Execution,
        );

        node.add_input_pin("model", "Model", "LLM model (Bit)", VariableType::Struct)
            .set_schema::<Bit>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "session",
            "Session",
            "DataFusion session to query",
            VariableType::Struct,
        )
        .set_schema::<DataFusionSession>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "table",
            "Table",
            "Table name within the session to query",
            VariableType::String,
        );

        node.add_input_pin(
            "description",
            "Description",
            "Natural language task (e.g. 'monthly sales by region')",
            VariableType::String,
        );

        node.add_input_pin(
            "chart_type",
            "Chart Type",
            "Target chart type",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(CHART_TYPE_OPTIONS.iter().map(|s| s.to_string()).collect())
                .build(),
        )
        .set_default_value(Some(json!("bar")));

        node.add_input_pin(
            "element",
            "Element",
            "Chart element reference (from Get Element) to bind the data to",
            VariableType::Struct,
        )
        .set_schema::<NivoChartProps>()
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires when generation is complete",
            VariableType::Execution,
        );

        node.add_output_pin(
            "data",
            "Data",
            "Query results as an array of row structs (chart-ready)",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build())
        .set_open_schema();

        node.add_output_pin("sql", "SQL", "Generated SQL query", VariableType::String);

        node.add_output_pin(
            "explanation",
            "Explanation",
            "AI explanation of the query",
            VariableType::String,
        );

        node.set_long_running(true);
        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let model_bit: Bit = context.evaluate_pin("model").await?;
        let session: DataFusionSession = context.evaluate_pin("session").await?;
        let table: String = context.evaluate_pin("table").await?;
        let description: String = context.evaluate_pin("description").await?;
        let chart_type: String = context.evaluate_pin("chart_type").await?;

        let mut model_name = model_bit.id.clone();
        if let Some(meta) = model_bit.meta.get("en") {
            model_name = meta.name.clone();
        }

        let cached_session = session.load(context).await?;

        let describe_sql = format!("DESCRIBE \"{}\"", table.replace('"', ""));
        let schema_df = cached_session.ctx.sql(&describe_sql).await?;
        let schema_batches = schema_df.collect().await?;

        let mut schema_desc = format!("Table: {table}\nColumns:\n");
        for batch in &schema_batches {
            for row_idx in 0..batch.num_rows() {
                let col_name = batch
                    .column(0)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .map(|a| a.value(row_idx))
                    .unwrap_or("?");
                let col_type = batch
                    .column(1)
                    .as_any()
                    .downcast_ref::<StringArray>()
                    .map(|a| a.value(row_idx))
                    .unwrap_or("?");
                schema_desc.push_str(&format!("  - {col_name} ({col_type})\n"));
            }
        }

        let system_prompt = build_system_prompt(&chart_type, &schema_desc, &description);

        let model_factory = context.app_state.model_factory.clone();
        let model = model_factory
            .lock()
            .await
            .build(
                &model_bit,
                context.app_state.clone(),
                context.token.clone(),
                context.model_usage_context(),
            )
            .await?;

        let mut history = History::new(model_name, vec![]);
        history.set_system_prompt(system_prompt);
        history.push_message(HistoryMessage::from_string(
            Role::User,
            &format!("Generate the SQL and explanation for: {description}"),
        ));

        let response = model.invoke(&history, None).await?;
        let raw = response.content().unwrap_or_default();

        let (sql, explanation) = extract_json_object(&raw)
            .and_then(|s| flow_like_types::json::from_str::<Value>(s).ok())
            .map(|v| {
                let sql = v
                    .get("sql")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string();
                let expl = v
                    .get("explanation")
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string();
                (sql, expl)
            })
            .unwrap_or_else(|| (String::new(), raw.clone()));

        if sql.is_empty() {
            return Err(flow_like_types::anyhow!(
                "LLM did not return a valid SQL query. Raw response: {}",
                raw
            ));
        }

        // Model-authored SQL over a session whose Lance tables accept DML —
        // this surface only ever charts data, so enforce read-only.
        flow_like_storage::databases::sql_guard::validate_readonly_sql(&sql).map_err(|error| {
            flow_like_types::anyhow!(
                "Chart Data Agent generated a non-SELECT statement and refused to run it: {error}"
            )
        })?;

        let data_df = cached_session.ctx.sql(&sql).await?;
        let data_batches = data_df.collect().await?;
        let rows = batches_to_rows(&data_batches)?;

        context.set_pin_value("data", json!(rows)).await?;
        context.set_pin_value("sql", json!(sql)).await?;
        context
            .set_pin_value("explanation", json!(explanation))
            .await?;

        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "ChartDataAgent requires the 'execute' feature"
        ))
    }
}
