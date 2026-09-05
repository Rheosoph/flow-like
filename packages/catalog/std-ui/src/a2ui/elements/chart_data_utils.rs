//! Shared utilities for converting CSV and CSVTable data to chart formats.

use flow_like_types::Value;

pub fn has_csv_table_data(value: &Value) -> bool {
    match value {
        Value::Object(obj) => {
            let has_headers = obj
                .get("headers")
                .and_then(|headers| headers.as_array())
                .is_some_and(|headers| !headers.is_empty());
            let has_rows = obj
                .get("rows")
                .and_then(|rows| rows.as_array())
                .is_some_and(|rows| !rows.is_empty());
            has_headers || has_rows
        }
        _ => false,
    }
}

/// Extract headers and rows from a CSVTable JSON value (from DataFusion queries)
pub fn extract_from_csv_table(
    value: &Value,
) -> flow_like_types::Result<(Vec<String>, Vec<Vec<String>>)> {
    let headers = value
        .get("headers")
        .and_then(|h| h.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let rows = value
        .get("rows")
        .and_then(|r| r.as_array())
        .map(|arr| {
            arr.iter()
                .map(|row| {
                    row.as_array()
                        .map(|cells| cells.iter().map(cell_to_string).collect())
                        .unwrap_or_default()
                })
                .collect()
        })
        .unwrap_or_default();

    Ok((headers, rows))
}

/// Convert a CSVTable cell value to string
pub fn cell_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Object(obj) => {
            // Handle Cell enum variants like { "Int": 42 } or { "Str": "hello" }
            if let Some(v) = obj
                .get("Int")
                .or(obj.get("Float"))
                .or(obj.get("Bool"))
                .or(obj.get("Str"))
            {
                return cell_to_string(v);
            }
            if let Some(date_obj) = obj.get("Date")
                && let Some(iso) = date_obj.get("iso").and_then(|v| v.as_str())
            {
                return iso.to_string();
            }
            String::new()
        }
        Value::Array(_) => String::new(),
    }
}

/// Parse CSV text into headers and rows
pub fn parse_csv_text(
    csv_text: &str,
    delimiter: char,
) -> flow_like_types::Result<(Vec<String>, Vec<Vec<String>>)> {
    let lines: Vec<&str> = csv_text
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() {
        return Ok((vec![], vec![]));
    }
    let headers = parse_csv_row(lines[0], delimiter);
    let rows: Vec<Vec<String>> = lines[1..]
        .iter()
        .map(|line| parse_csv_row(line, delimiter))
        .collect();
    Ok((headers, rows))
}

/// Parse a single CSV row with quote handling
pub fn parse_csv_row(line: &str, delimiter: char) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for c in line.chars() {
        match c {
            '"' => in_quotes = !in_quotes,
            c if c == delimiter && !in_quotes => {
                result.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(c),
        }
    }
    result.push(current.trim().to_string());
    result
}

/// Parse column reference (name or 0-based index)
pub fn parse_column_ref(col_ref: &str, headers: &[String]) -> usize {
    if let Ok(idx) = col_ref.parse::<usize>() {
        return idx;
    }
    headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case(col_ref))
        .unwrap_or(0)
}

/// Clean field name for use in chart data (lowercase, replace spaces/dashes with underscores)
pub fn clean_field_name(name: &str) -> String {
    name.to_lowercase().replace([' ', '-'], "_")
}

/// Get data from either CSVTable (table pin) or CSV text (csv pin)
/// Returns (headers, rows)
pub async fn get_chart_data(
    context: &mut flow_like::flow::execution::context::ExecutionContext,
    delimiter: char,
) -> flow_like_types::Result<(Vec<String>, Vec<Vec<String>>)> {
    // Try to get data from CSVTable first, then fall back to CSV text
    if let Ok(table_value) = context.evaluate_pin::<Value>("table").await
        && has_csv_table_data(&table_value)
    {
        return extract_from_csv_table(&table_value);
    }

    let csv_text: String = context.evaluate_pin("csv").await?;
    parse_csv_text(&csv_text, delimiter)
}
