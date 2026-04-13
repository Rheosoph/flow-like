use super::chart_data_utils::{
    clean_field_name, extract_from_csv_table, has_csv_table_data, parse_csv_text,
};
use super::element_utils::extract_element_id;
use flow_like::flow::{
    board::Board,
    execution::context::ExecutionContext,
    node::{Node, NodeLogic, remove_pin},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_types::{Value, async_trait, json::Map, json::json};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartLibrary {
    Nivo,
    Plotly,
}

impl ChartLibrary {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "nivo" => Some(Self::Nivo),
            "plotly" => Some(Self::Plotly),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChartType {
    Bar,
    Line,
    Pie,
    Scatter,
    Area,
    Radar,
    Heatmap,
    Calendar,
    Sankey,
    Tree,
}

impl ChartType {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bar" => Some(Self::Bar),
            "line" => Some(Self::Line),
            "pie" => Some(Self::Pie),
            "scatter" => Some(Self::Scatter),
            "area" => Some(Self::Area),
            "radar" => Some(Self::Radar),
            "heatmap" => Some(Self::Heatmap),
            "calendar" => Some(Self::Calendar),
            "sankey" => Some(Self::Sankey),
            "tree" | "treemap" => Some(Self::Tree),
            _ => None,
        }
    }
}

/// Push data to a chart element (Nivo or Plotly).
///
/// Select a data format (JSON or CSV) and only the relevant pins are shown.
/// JSON data is passed through directly; CSV/table data is auto-transformed.
#[crate::register_node]
#[derive(Default)]
pub struct PushCsvToChart;

impl PushCsvToChart {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl NodeLogic for PushCsvToChart {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "a2ui_push_csv_to_chart",
            "Push Data to Chart",
            "Push data to a Nivo or Plotly chart. Select JSON for pre-formatted data or CSV for auto-transformation.",
            "UI/Elements/Charts",
        );
        node.add_icon("/flow/icons/a2ui.svg");
        node.set_version(2);

        node.add_input_pin("exec_in", "▶", "", VariableType::Execution);

        node.add_input_pin(
            "element_ref",
            "Chart",
            "Reference to the chart element",
            VariableType::Struct,
        )
        .set_options(PinOptions::new().set_enforce_schema(false).build());

        node.add_input_pin("library", "Library", "Nivo or Plotly", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["Nivo".to_string(), "Plotly".to_string()])
                    .build(),
            )
            .set_default_value(Some(json!("Nivo")));

        node.add_input_pin(
            "format",
            "Format",
            "Data format: JSON (passthrough) or CSV (auto-transform)",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["JSON".to_string(), "CSV".to_string()])
                .build(),
        )
        .set_default_value(Some(json!("JSON")));

        // Default: JSON mode → show data pin
        node.add_input_pin(
            "data",
            "Data",
            "Chart data as JSON array/object or JSON string",
            VariableType::Struct,
        );

        node.add_output_pin("exec_out", "▶", "", VariableType::Execution);

        node.set_long_running(true);

        node
    }

    async fn on_update(&self, node: &mut Node, _board: &Board) {
        let format = node
            .get_pin_by_name("format")
            .and_then(|pin| pin.default_value.clone())
            .and_then(|bytes| flow_like_types::json::from_slice::<String>(&bytes).ok())
            .unwrap_or_else(|| "JSON".to_string());

        let data_pin = node.get_pin_by_name("data").cloned();
        let csv_pin = node.get_pin_by_name("csv").cloned();
        let table_pin = node.get_pin_by_name("table").cloned();
        let chart_type_pin = node.get_pin_by_name("chart_type").cloned();
        let delimiter_pin = node.get_pin_by_name("delimiter").cloned();

        match format.as_str() {
            "JSON" => {
                remove_pin(node, csv_pin);
                remove_pin(node, table_pin);
                remove_pin(node, chart_type_pin);
                remove_pin(node, delimiter_pin);
                if data_pin.is_none() {
                    node.add_input_pin(
                        "data",
                        "Data",
                        "Chart data as JSON array/object or JSON string",
                        VariableType::Struct,
                    );
                }
            }
            "CSV" => {
                remove_pin(node, data_pin);
                if csv_pin.is_none() {
                    node.add_input_pin(
                        "csv",
                        "Data",
                        "CSV text with headers",
                        VariableType::String,
                    );
                }
                if table_pin.is_none() {
                    node.add_input_pin(
                        "table",
                        "Table",
                        "Table data from DataFusion query",
                        VariableType::Struct,
                    )
                    .set_options(PinOptions::new().set_enforce_schema(false).build());
                }
                if chart_type_pin.is_none() {
                    node.add_input_pin(
                        "chart_type",
                        "Chart Type",
                        "Chart type for auto-transformation",
                        VariableType::String,
                    )
                    .set_options(
                        PinOptions::new()
                            .set_valid_values(vec![
                                "Bar".to_string(),
                                "Line".to_string(),
                                "Pie".to_string(),
                                "Scatter".to_string(),
                                "Area".to_string(),
                                "Radar".to_string(),
                                "Heatmap".to_string(),
                                "Calendar".to_string(),
                                "Sankey".to_string(),
                                "Tree".to_string(),
                            ])
                            .build(),
                    )
                    .set_default_value(Some(json!("Bar")));
                }
                if delimiter_pin.is_none() {
                    node.add_input_pin(
                        "delimiter",
                        "Delimiter",
                        "CSV delimiter character",
                        VariableType::String,
                    )
                    .set_default_value(Some(json!(",")));
                }
            }
            _ => {}
        }
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let element_value: Value = context.evaluate_pin("element_ref").await?;
        let element_id = extract_element_id(&element_value)
            .ok_or_else(|| flow_like_types::anyhow!("Invalid chart element reference"))?;

        let library_str: String = context.evaluate_pin("library").await?;
        let library = ChartLibrary::from_str(&library_str)
            .ok_or_else(|| flow_like_types::anyhow!("Unknown library: {}", library_str))?;

        let format: String = context.evaluate_pin("format").await?;

        match format.as_str() {
            "JSON" => {
                let data: Value = context.evaluate_pin("data").await?;
                let json_value = parse_json_input(data)?;
                push_json_passthrough(context, &element_id, library, json_value).await
            }
            "CSV" => {
                let (headers, rows) =
                    if let Ok(table_value) = context.evaluate_pin::<Value>("table").await {
                        if has_csv_table_data(&table_value) {
                            extract_from_csv_table(&table_value)?
                        } else {
                            read_csv_input(context).await?
                        }
                    } else {
                        read_csv_input(context).await?
                    };
                push_tabular(context, &element_id, library, &headers, &rows).await
            }
            _ => Err(flow_like_types::anyhow!("Unknown format: {}", format)),
        }
    }
}

fn parse_json_input(data: Value) -> flow_like_types::Result<Value> {
    match data {
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(json!([]));
            }
            flow_like_types::json::from_str(trimmed)
                .map_err(|e| flow_like_types::anyhow!("Invalid JSON string: {}", e))
        }
        Value::Null => Ok(json!([])),
        other => Ok(other),
    }
}

async fn read_csv_input(
    context: &mut ExecutionContext,
) -> flow_like_types::Result<(Vec<String>, Vec<Vec<String>>)> {
    let csv_text: String = context.evaluate_pin("csv").await?;
    let delimiter: String = context.evaluate_pin("delimiter").await?;
    let delim = delimiter.chars().next().unwrap_or(',');
    parse_csv_text(&csv_text, delim)
}

async fn push_json_passthrough(
    context: &mut ExecutionContext,
    element_id: &str,
    library: ChartLibrary,
    data: Value,
) -> flow_like_types::Result<()> {
    let msg_type = match library {
        ChartLibrary::Nivo => "setNivoData",
        ChartLibrary::Plotly => "setChartData",
    };
    context
        .upsert_element(element_id, json!({ "type": msg_type, "data": data }))
        .await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

async fn push_tabular(
    context: &mut ExecutionContext,
    element_id: &str,
    library: ChartLibrary,
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<()> {
    if headers.is_empty() || rows.is_empty() {
        let msg_type = match library {
            ChartLibrary::Nivo => "setNivoData",
            ChartLibrary::Plotly => "setChartData",
        };
        context
            .upsert_element(element_id, json!({ "type": msg_type, "data": [] }))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        return Ok(());
    }

    let chart_type_str: String = context.evaluate_pin("chart_type").await?;
    let chart_type = ChartType::from_str(&chart_type_str)
        .ok_or_else(|| flow_like_types::anyhow!("Unknown chart type: {}", chart_type_str))?;

    match library {
        ChartLibrary::Nivo => {
            push_nivo_data(context, element_id, chart_type, headers, rows).await?
        }
        ChartLibrary::Plotly => {
            push_plotly_data(context, element_id, chart_type, headers, rows).await?
        }
    }

    context.activate_exec_pin("exec_out").await?;
    Ok(())
}

async fn push_nivo_data(
    context: &mut ExecutionContext,
    element_id: &str,
    chart_type: ChartType,
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<()> {
    let (data, config) = transform_for_nivo(chart_type, headers, rows)?;

    context
        .upsert_element(element_id, json!({ "type": "setNivoData", "data": data }))
        .await?;

    if let Some(cfg) = config {
        context
            .upsert_element(
                element_id,
                json!({ "type": "setNivoConfig", "config": cfg }),
            )
            .await?;
    }

    Ok(())
}

async fn push_plotly_data(
    context: &mut ExecutionContext,
    element_id: &str,
    chart_type: ChartType,
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<()> {
    let traces = transform_for_plotly(chart_type, headers, rows)?;

    context
        .upsert_element(
            element_id,
            json!({ "type": "setChartData", "data": traces }),
        )
        .await?;

    Ok(())
}

// ============================================================================
// NIVO TRANSFORMATIONS
// ============================================================================

fn transform_for_nivo(
    chart_type: ChartType,
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    match chart_type {
        ChartType::Bar | ChartType::Radar => nivo_bar(headers, rows),
        ChartType::Line | ChartType::Area => nivo_line(headers, rows),
        ChartType::Pie => nivo_pie(headers, rows),
        ChartType::Scatter => nivo_scatter(headers, rows),
        ChartType::Heatmap => nivo_heatmap(headers, rows),
        ChartType::Calendar => nivo_calendar(headers, rows),
        ChartType::Sankey => nivo_sankey(headers, rows),
        ChartType::Tree => nivo_tree(headers, rows),
    }
}

fn nivo_bar(
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    let index_field = clean_field_name(headers.first().map(|s| s.as_str()).unwrap_or("category"));
    let keys: Vec<String> = headers[1..].iter().map(|h| clean_field_name(h)).collect();

    let data: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut obj = Map::new();
            obj.insert(
                index_field.clone(),
                json!(row.first().cloned().unwrap_or_default()),
            );
            for (i, key) in keys.iter().enumerate() {
                let val: f64 = row.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                obj.insert(key.clone(), json!(val));
            }
            Value::Object(obj)
        })
        .collect();

    Ok((
        json!(data),
        Some(json!({ "keys": keys, "indexBy": index_field })),
    ))
}

fn nivo_line(
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    let series: Vec<Value> = (1..headers.len())
        .map(|y_idx| {
            let name = clean_field_name(&headers[y_idx]);
            let points: Vec<Value> = rows
                .iter()
                .map(|row| {
                    let x = row.first().cloned().unwrap_or_default();
                    let y: f64 = row.get(y_idx).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    json!({ "x": x, "y": y })
                })
                .collect();
            json!({ "id": name, "data": points })
        })
        .collect();

    Ok((json!(series), None))
}

fn nivo_pie(
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    let value_idx = if headers.len() > 1 { 1 } else { 0 };
    let data: Vec<Value> = rows
        .iter()
        .map(|row| {
            let label = row.first().cloned().unwrap_or_default();
            let value: f64 = row
                .get(value_idx)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            json!({ "id": clean_field_name(&label), "label": label, "value": value })
        })
        .collect();

    Ok((json!(data), None))
}

fn nivo_scatter(
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    // Group by series if 3+ columns, otherwise single series
    if headers.len() >= 3 {
        let mut groups: std::collections::HashMap<String, Vec<Value>> =
            std::collections::HashMap::new();
        for row in rows {
            let x: f64 = row.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let y: f64 = row.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let group = row.get(2).cloned().unwrap_or_else(|| "default".to_string());
            groups
                .entry(group)
                .or_default()
                .push(json!({ "x": x, "y": y }));
        }
        let series: Vec<Value> = groups
            .into_iter()
            .map(|(id, data)| json!({ "id": id, "data": data }))
            .collect();
        Ok((json!(series), None))
    } else {
        let points: Vec<Value> = rows
            .iter()
            .map(|row| {
                let x: f64 = row.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
                let y: f64 = row.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                json!({ "x": x, "y": y })
            })
            .collect();
        Ok((json!([{ "id": "data", "data": points }]), None))
    }
}

fn nivo_heatmap(
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    let col_headers: Vec<String> = headers[1..].iter().map(|s| clean_field_name(s)).collect();
    let data: Vec<Value> = rows
        .iter()
        .map(|row| {
            let row_id = clean_field_name(row.first().map(|s| s.as_str()).unwrap_or("row"));
            let cells: Vec<Value> = col_headers
                .iter()
                .enumerate()
                .map(|(i, col)| {
                    let val: f64 = row.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
                    json!({ "x": col, "y": val })
                })
                .collect();
            json!({ "id": row_id, "data": cells })
        })
        .collect();

    Ok((json!(data), None))
}

fn nivo_calendar(
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    let value_idx = if headers.len() > 1 { 1 } else { 0 };
    let data: Vec<Value> = rows
        .iter()
        .map(|row| {
            let day = row.first().cloned().unwrap_or_default();
            let value: f64 = row
                .get(value_idx)
                .and_then(|s| s.parse().ok())
                .unwrap_or(0.0);
            json!({ "day": day, "value": value })
        })
        .collect();

    Ok((json!(data), None))
}

fn nivo_sankey(
    _headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    let mut nodes_set = std::collections::HashSet::new();
    let links: Vec<Value> = rows
        .iter()
        .map(|row| {
            let source = row.first().cloned().unwrap_or_default();
            let target = row.get(1).cloned().unwrap_or_default();
            let value: f64 = row.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            nodes_set.insert(source.clone());
            nodes_set.insert(target.clone());
            json!({ "source": source, "target": target, "value": value })
        })
        .collect();

    let nodes: Vec<Value> = nodes_set
        .into_iter()
        .map(|id| json!({ "id": id }))
        .collect();
    Ok((json!({ "nodes": nodes, "links": links }), None))
}

fn nivo_tree(
    _headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<(Value, Option<Value>)> {
    let nodes: Vec<Value> = rows
        .iter()
        .map(|row| {
            let id = row.first().cloned().unwrap_or_default();
            let parent = row.get(1).cloned().unwrap_or_default();
            let value: f64 = row.get(2).and_then(|s| s.parse().ok()).unwrap_or(1.0);
            json!({
                "id": id,
                "parent": if parent.is_empty() { Value::Null } else { json!(parent) },
                "value": value
            })
        })
        .collect();

    Ok((json!(nodes), None))
}

// ============================================================================
// PLOTLY TRANSFORMATIONS
// ============================================================================

fn transform_for_plotly(
    chart_type: ChartType,
    headers: &[String],
    rows: &[Vec<String>],
) -> flow_like_types::Result<Value> {
    match chart_type {
        ChartType::Bar => plotly_bar(headers, rows),
        ChartType::Line => plotly_line(headers, rows),
        ChartType::Pie => plotly_pie(headers, rows),
        ChartType::Scatter => plotly_scatter(headers, rows),
        ChartType::Area => plotly_area(headers, rows),
        ChartType::Heatmap => plotly_heatmap(headers, rows),
        // Plotly doesn't have native equivalents for these - fallback to bar
        ChartType::Radar | ChartType::Calendar | ChartType::Sankey | ChartType::Tree => {
            plotly_bar(headers, rows)
        }
    }
}

fn plotly_bar(headers: &[String], rows: &[Vec<String>]) -> flow_like_types::Result<Value> {
    let x: Vec<String> = rows
        .iter()
        .map(|r| r.first().cloned().unwrap_or_default())
        .collect();

    let traces: Vec<Value> = (1..headers.len())
        .map(|i| {
            let y: Vec<f64> = rows
                .iter()
                .map(|r| r.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0))
                .collect();
            json!({
                "x": x,
                "y": y,
                "name": clean_field_name(&headers[i]),
                "type": "bar"
            })
        })
        .collect();

    Ok(json!(traces))
}

fn plotly_line(headers: &[String], rows: &[Vec<String>]) -> flow_like_types::Result<Value> {
    let x: Vec<String> = rows
        .iter()
        .map(|r| r.first().cloned().unwrap_or_default())
        .collect();

    let traces: Vec<Value> = (1..headers.len())
        .map(|i| {
            let y: Vec<f64> = rows
                .iter()
                .map(|r| r.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0))
                .collect();
            json!({
                "x": x,
                "y": y,
                "name": clean_field_name(&headers[i]),
                "type": "scatter",
                "mode": "lines+markers"
            })
        })
        .collect();

    Ok(json!(traces))
}

fn plotly_scatter(headers: &[String], rows: &[Vec<String>]) -> flow_like_types::Result<Value> {
    if headers.len() >= 3 {
        let mut groups: std::collections::HashMap<String, (Vec<f64>, Vec<f64>)> =
            std::collections::HashMap::new();
        for row in rows {
            let x: f64 = row.first().and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let y: f64 = row.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0);
            let group = row.get(2).cloned().unwrap_or_else(|| "data".to_string());
            let entry = groups.entry(group).or_insert_with(|| (vec![], vec![]));
            entry.0.push(x);
            entry.1.push(y);
        }
        let traces: Vec<Value> = groups
            .into_iter()
            .map(|(name, (x, y))| {
                json!({ "x": x, "y": y, "name": name, "type": "scatter", "mode": "markers" })
            })
            .collect();
        Ok(json!(traces))
    } else {
        let x: Vec<f64> = rows
            .iter()
            .map(|r| r.first().and_then(|s| s.parse().ok()).unwrap_or(0.0))
            .collect();
        let y: Vec<f64> = rows
            .iter()
            .map(|r| r.get(1).and_then(|s| s.parse().ok()).unwrap_or(0.0))
            .collect();
        Ok(json!([{ "x": x, "y": y, "type": "scatter", "mode": "markers" }]))
    }
}

fn plotly_area(headers: &[String], rows: &[Vec<String>]) -> flow_like_types::Result<Value> {
    let x: Vec<String> = rows
        .iter()
        .map(|r| r.first().cloned().unwrap_or_default())
        .collect();

    let traces: Vec<Value> = (1..headers.len())
        .map(|i| {
            let y: Vec<f64> = rows
                .iter()
                .map(|r| r.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0))
                .collect();
            json!({
                "x": x,
                "y": y,
                "name": clean_field_name(&headers[i]),
                "type": "scatter",
                "fill": "tozeroy"
            })
        })
        .collect();

    Ok(json!(traces))
}

fn plotly_pie(headers: &[String], rows: &[Vec<String>]) -> flow_like_types::Result<Value> {
    let labels: Vec<String> = rows
        .iter()
        .map(|r| r.first().cloned().unwrap_or_default())
        .collect();
    let value_idx = if headers.len() > 1 { 1 } else { 0 };
    let values: Vec<f64> = rows
        .iter()
        .map(|r| r.get(value_idx).and_then(|s| s.parse().ok()).unwrap_or(0.0))
        .collect();

    Ok(json!([{ "labels": labels, "values": values, "type": "pie" }]))
}

fn plotly_heatmap(headers: &[String], rows: &[Vec<String>]) -> flow_like_types::Result<Value> {
    let y: Vec<String> = rows
        .iter()
        .map(|r| r.first().cloned().unwrap_or_default())
        .collect();
    let x: Vec<String> = headers[1..].iter().map(|h| clean_field_name(h)).collect();

    let z: Vec<Vec<f64>> = rows
        .iter()
        .map(|row| {
            (1..headers.len())
                .map(|i| row.get(i).and_then(|s| s.parse().ok()).unwrap_or(0.0))
                .collect()
        })
        .collect();

    Ok(json!([{ "x": x, "y": y, "z": z, "type": "heatmap" }]))
}
