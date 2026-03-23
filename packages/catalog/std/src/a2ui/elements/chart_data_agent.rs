use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, NodeScores},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::json};

/// AI Agent for generating chart-ready data from DataFusion SQL sessions.
///
/// Takes one or more DataFusion sessions, a description of what data is needed,
/// and a target chart type. Uses an LLM to generate SQL and transform the results
/// into the appropriate format for Nivo or Plotly charts.
///
/// **Supported Chart Types:**
/// - Bar: `[{ category: "A", value1: 10 }, ...]`
/// - Line: `[{ id: "Series", data: [{x, y}] }]`
/// - Pie: `[{ id, label, value }]`
/// - Radar: `[{ dimension: "Speed", series1: 70 }, ...]`
/// - Heatmap: `[{ id: "Row", data: [{x, y}] }]`
/// - Sankey: `{ nodes: [{id}], links: [{source, target, value}] }`
/// - Calendar: `[{ day: "YYYY-MM-DD", value }]`
/// - Tree/Treemap: `{ name, children: [...] | value }`
/// - Scatter: `[{ id: "Series", data: [{x, y}] }]`
/// - Funnel: `[{ id, label, value }]`
#[crate::register_node]
#[derive(Default)]
pub struct ChartDataAgent;

impl ChartDataAgent {
    pub fn new() -> Self {
        Self
    }
}

const CHART_TYPE_OPTIONS: &[&str] = &[
    "bar", "line", "pie", "radar", "heatmap", "sankey", "calendar", "treemap", "scatter", "funnel",
    "chord",
];

fn get_chart_format_description(chart_type: &str) -> &'static str {
    match chart_type {
        "bar" => {
            r#"Bar chart format: Array of objects where each object is a category with numeric values.
Example: [{ "category": "Q1", "sales": 100, "profit": 20 }, { "category": "Q2", "sales": 150, "profit": 35 }]
Output includes: data (array), keys (value field names), indexBy (category field name)"#
        }

        "line" => {
            r#"Line chart format: Array of series objects, each with id and data array of {x, y} points.
Example: [{ "id": "Revenue", "data": [{"x": "Jan", "y": 100}, {"x": "Feb", "y": 120}] }]
X values can be dates, categories, or numbers. Y values must be numeric."#
        }

        "pie" => {
            r#"Pie/Donut chart format: Array of slice objects with id, label, and value.
Example: [{ "id": "chrome", "label": "Chrome", "value": 45 }, { "id": "firefox", "label": "Firefox", "value": 30 }]
Values should sum to a meaningful total (percentages or counts)."#
        }

        "radar" => {
            r#"Radar/Spider chart format: Array of dimension objects with values for each series.
Example: [{ "dimension": "Speed", "product_a": 70, "product_b": 90 }, { "dimension": "Cost", "product_a": 50, "product_b": 60 }]
Output includes: data (array), keys (series names), indexBy (dimension field name)"#
        }

        "heatmap" => {
            r#"Heatmap format: Array of row objects, each with id and data array of {x, y} cells.
Example: [{ "id": "Monday", "data": [{"x": "9am", "y": 5}, {"x": "10am", "y": 12}] }]
Y values represent the heat/intensity at each cell."#
        }

        "sankey" => {
            r#"Sankey diagram format: Object with nodes and links arrays.
Example: { "nodes": [{"id": "A"}, {"id": "B"}, {"id": "C"}], "links": [{"source": "A", "target": "B", "value": 10}, {"source": "B", "target": "C", "value": 8}] }
Links represent flow from source to target with a value/weight."#
        }

        "calendar" => {
            r#"Calendar heatmap format: Array of day/value pairs with YYYY-MM-DD dates.
Example: [{ "day": "2024-01-15", "value": 50 }, { "day": "2024-01-16", "value": 30 }]
Output includes: data (array), from_date, to_date (date range strings)"#
        }

        "treemap" => {
            r#"Treemap/Sunburst hierarchical format: Nested object with name and children/value.
Example: { "name": "root", "children": [{ "name": "Category A", "children": [{ "name": "Item 1", "value": 100 }] }] }
Leaf nodes have value, branch nodes have children array."#
        }

        "scatter" => {
            r#"Scatter plot format: Array of series with data points containing x and y coordinates.
Example: [{ "id": "Group A", "data": [{"x": 10, "y": 20}, {"x": 15, "y": 30}] }]
Both x and y should be numeric for scatter plots."#
        }

        "funnel" => {
            r#"Funnel chart format: Array of stage objects with id, label, and value (ordered by value descending).
Example: [{ "id": "visits", "label": "Website Visits", "value": 10000 }, { "id": "signups", "label": "Sign Ups", "value": 2000 }]
Values typically decrease through the funnel stages."#
        }

        "chord" => {
            r#"Chord diagram format: Square matrix of relationships between entities.
Example: { "keys": ["A", "B", "C"], "matrix": [[0, 10, 5], [10, 0, 8], [5, 8, 0]] }
Matrix[i][j] represents flow/relationship from keys[i] to keys[j]."#
        }

        _ => "Generic chart data format",
    }
}

fn build_system_prompt(chart_type: &str, table_schemas: &str, description: &str) -> String {
    let chart_format = get_chart_format_description(chart_type);

    format!(
        r#"You are a data analyst assistant that generates SQL queries and transforms data for visualization.

## Task
Generate a SQL query to extract data matching the user's description, then format the results for a {chart_type} chart.

## Available Tables
{table_schemas}

## Target Chart Format ({chart_type})
{chart_format}

## User Request
{description}

## Instructions
1. Write a SQL query that extracts the relevant data
2. The query should return columns that can be directly mapped to the chart format
3. Use appropriate aggregations (SUM, COUNT, AVG) when needed
4. Order results appropriately for the chart type
5. Limit results to a reasonable number for visualization (usually 10-50 items)

## Response Format
Return a JSON object with:
- "sql": The SQL query string
- "column_mapping": Object mapping query columns to chart fields
- "explanation": Brief explanation of what the query does

Example response:
{{
  "sql": "SELECT category, SUM(amount) as total FROM sales GROUP BY category ORDER BY total DESC LIMIT 10",
  "column_mapping": {{"category": "category", "total": "value"}},
  "explanation": "Aggregates sales by category, showing top 10 categories by total sales"
}}"#,
        chart_type = chart_type,
        table_schemas = table_schemas,
        chart_format = chart_format,
        description = description
    )
}

#[async_trait]
impl NodeLogic for ChartDataAgent {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_chart_data_agent",
            "Chart Data Agent",
            "AI agent that generates SQL queries and chart-ready data from DataFusion sessions",
            "UI/Elements/Charts/Agent",
        );
        node.set_version(1);
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

        node.add_input_pin(
            "sessions",
            "Sessions",
            "One or more DataFusion sessions to query from (array)",
            VariableType::Generic,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin(
            "description",
            "Description",
            "Natural language description of what data you need (e.g., 'monthly sales by region')",
            VariableType::String,
        );

        node.add_input_pin(
            "chart_type",
            "Chart Type",
            "Target chart type for formatting the output",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(CHART_TYPE_OPTIONS.iter().map(|s| s.to_string()).collect())
                .build(),
        )
        .set_default_value(Some(json!("bar")));

        node.add_input_pin(
            "table_schemas",
            "Table Schemas",
            "JSON object mapping table names to their schema descriptions (optional, auto-discovered if not provided)",
            VariableType::Struct,
        )
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
            "Chart-ready data in the appropriate format",
            VariableType::Generic,
        );
        node.add_output_pin(
            "sql",
            "SQL",
            "The generated SQL query",
            VariableType::String,
        );
        node.add_output_pin(
            "keys",
            "Keys",
            "Series/value keys for the chart (if applicable)",
            VariableType::Generic,
        );
        node.add_output_pin(
            "index_by",
            "Index By",
            "Index/category field name (if applicable)",
            VariableType::String,
        );
        node.add_output_pin(
            "explanation",
            "Explanation",
            "AI explanation of the query and data",
            VariableType::String,
        );

        node.set_long_running(true);

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let description: String = context.evaluate_pin("description").await?;
        let chart_type: String = context.evaluate_pin("chart_type").await?;
        let table_schemas: Value = context
            .evaluate_pin("table_schemas")
            .await
            .unwrap_or(json!({}));

        let table_schemas_str = if table_schemas.is_null()
            || table_schemas
                .as_object()
                .map(|o| o.is_empty())
                .unwrap_or(true)
        {
            "No table schemas provided. Please provide table_schemas input with table names and column descriptions.".to_string()
        } else {
            flow_like_types::json::to_string_pretty(&table_schemas).unwrap_or_default()
        };

        let system_prompt = build_system_prompt(&chart_type, &table_schemas_str, &description);

        // For now, output the system prompt as explanation and empty data
        // Full implementation would invoke an LLM and execute the SQL
        context.set_pin_value("data", json!([])).await?;
        context
            .set_pin_value("sql", json!("-- SQL generation requires LLM integration"))
            .await?;
        context.set_pin_value("keys", json!([])).await?;
        context.set_pin_value("index_by", json!("")).await?;
        context
            .set_pin_value(
                "explanation",
                json!(format!(
                    "Chart Data Agent ready for {} chart.\n\nSystem prompt prepared:\n{}",
                    chart_type, system_prompt
                )),
            )
            .await?;

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}
