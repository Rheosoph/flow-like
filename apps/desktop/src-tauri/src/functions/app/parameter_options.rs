//! The values a `$parameter` compared with `table.column` can take in a local-only app. Built as
//! the query service builds its `parameter_options` operation for online apps
//! (`packages/query/src/server/ops.rs`), so both answer the same shape.

use flow_like::flow_like_storage::databases::workbench::SqlQueryResult;
use flow_like_types::{Result, Value, anyhow};
use serde_json::json;

const OPTION_VALUES: usize = 200;
const MAX_NAME_CHARS: usize = 256;

pub(super) struct OptionsQuery {
    pub sql: String,
    pub params: Value,
    pub rows: usize,
}

fn quote(identifier: &str) -> Result<String> {
    if identifier.is_empty()
        || identifier.contains('\0')
        || identifier.chars().count() > MAX_NAME_CHARS
    {
        return Err(anyhow!("'{identifier}' is not a valid identifier"));
    }
    Ok(format!("\"{}\"", identifier.replace('"', "\"\"")))
}

/// `schema.table` or `table`, quoted part by part.
fn quote_table(table: &str) -> Result<String> {
    match table.split_once('.') {
        Some((schema, name)) => Ok(format!("{}.{}", quote(schema)?, quote(name)?)),
        None => quote(table),
    }
}

/// Distinct values (optionally containing `search`), or the minimum and maximum with `range`.
pub(super) fn options_query(
    table: &str,
    column: &str,
    search: Option<&str>,
    range: bool,
) -> Result<OptionsQuery> {
    let source = quote_table(table)?;
    let field = quote(column)?;
    if range {
        return Ok(OptionsQuery {
            sql: format!("SELECT MIN({field}) AS \"min\", MAX({field}) AS \"max\" FROM {source}"),
            params: Value::Null,
            rows: 1,
        });
    }
    let search = search.filter(|text| !text.is_empty());
    if search.is_some_and(|text| text.chars().count() > MAX_NAME_CHARS) {
        return Err(anyhow!(
            "The search text is longer than {MAX_NAME_CHARS} characters"
        ));
    }
    let filter = if search.is_some() {
        format!(" AND strpos(lower(CAST({field} AS VARCHAR)), lower($__search)) > 0")
    } else {
        String::new()
    };
    Ok(OptionsQuery {
        sql: format!(
            "SELECT DISTINCT {field} AS \"value\" FROM {source} WHERE {field} IS NOT NULL{filter} ORDER BY 1"
        ),
        params: search.map_or(Value::Null, |text| json!({ "__search": text })),
        rows: OPTION_VALUES,
    })
}

/// `{values, truncated}`, or the `{min, max}` row for a range.
pub(super) fn options_answer(result: SqlQueryResult, range: bool) -> Value {
    if range {
        return result.rows.into_iter().next().unwrap_or(Value::Null);
    }
    json!({
        "values": result
            .rows
            .into_iter()
            .map(|row| row.get("value").cloned().unwrap_or(Value::Null))
            .collect::<Vec<_>>(),
        "truncated": result.truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_queries_quote_every_identifier_part() {
        let distinct = options_query("crm.customers", "re\"gion", Some("we"), false).unwrap();
        assert_eq!(
            distinct.sql,
            "SELECT DISTINCT \"re\"\"gion\" AS \"value\" FROM \"crm\".\"customers\" \
             WHERE \"re\"\"gion\" IS NOT NULL AND strpos(lower(CAST(\"re\"\"gion\" AS VARCHAR)), \
             lower($__search)) > 0 ORDER BY 1"
        );
        assert_eq!(distinct.params, json!({ "__search": "we" }));
        assert_eq!(distinct.rows, OPTION_VALUES);

        let unfiltered = options_query("orders", "region", Some(""), false).unwrap();
        assert!(!unfiltered.sql.contains("strpos"));
        assert_eq!(unfiltered.params, Value::Null);

        let range = options_query("orders", "total", None, true).unwrap();
        assert_eq!(
            range.sql,
            "SELECT MIN(\"total\") AS \"min\", MAX(\"total\") AS \"max\" FROM \"orders\""
        );
        assert_eq!(range.rows, 1);

        assert!(options_query("crm.", "region", None, false).is_err());
        assert!(options_query("orders", "", None, false).is_err());
        assert!(options_query("orders", "a\0b", None, false).is_err());
        assert!(options_query("orders", "region", Some(&"s".repeat(257)), false).is_err());
    }

    #[test]
    fn answers_take_the_service_shape() {
        let result = |rows: Vec<Value>, truncated| SqlQueryResult {
            columns: Vec::new(),
            row_count: rows.len(),
            rows,
            truncated,
        };
        assert_eq!(
            options_answer(
                result(
                    vec![json!({ "value": "East" }), json!({ "value": "West" })],
                    true
                ),
                false
            ),
            json!({ "values": ["East", "West"], "truncated": true })
        );
        assert_eq!(
            options_answer(result(vec![json!({ "min": 1, "max": 9 })], false), true),
            json!({ "min": 1, "max": 9 })
        );
    }
}
