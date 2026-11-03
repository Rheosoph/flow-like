use std::time::Instant;

use flow_like_storage::databases::sql_guard::validate_readonly_sql;
use flow_like_storage::databases::workbench::{
    WorkbenchLimits, WorkbenchSchema, execute_readonly_sql_across_apps,
};
use flow_like_types::Bytes;
use serde_json::{Value, json};

use super::session::{WORKSPACE_SCHEMA, app_aliases, session_of};
use super::{DataQueryArgs, SaveAs};
use crate::env::ToolEnv;
use crate::error::{ToolError, ToolResult, code};
use crate::files::{handle_for, workspace_of};
use crate::layout::normalize_path;
use crate::outcome::ToolOutput;
use crate::tables::{SaveMode, save_rows, validate_table_name};

enum Target {
    Table { name: String, mode: SaveMode },
    File { path: String, format: Format },
}

#[derive(Clone, Copy)]
enum Format {
    Csv,
    Json,
}

fn target_of(save_as: Option<&SaveAs>) -> ToolResult<Option<Target>> {
    Ok(match save_as {
        None => None,
        Some(SaveAs::Table(table)) => Some(Target::Table {
            name: validate_table_name(&table.table)?,
            mode: match table.mode.as_deref().unwrap_or("replace") {
                "replace" => SaveMode::Replace,
                "append" => SaveMode::Append,
                other => {
                    return Err(ToolError::invalid(format!(
                        "Unknown save mode '{other}'; use 'replace' or 'append'"
                    )));
                }
            },
        }),
        Some(SaveAs::File(file)) => Some(Target::File {
            path: normalize_path(&file.file)?,
            format: match file.format.as_str() {
                "csv" => Format::Csv,
                "json" => Format::Json,
                other => {
                    return Err(ToolError::invalid(format!(
                        "Unknown file format '{other}'; use 'csv' or 'json'"
                    )));
                }
            },
        }),
    })
}

fn mentions(sql: &str, alias: &str) -> bool {
    let sql = sql.to_lowercase();
    let needle = format!("{alias}.");
    sql.match_indices(&needle).any(|(at, _)| {
        sql[..at]
            .chars()
            .next_back()
            .is_none_or(|before| !(before.is_ascii_alphanumeric() || before == '_'))
    })
}

fn csv_cell(value: &Value) -> String {
    let text = match value {
        Value::Null => String::new(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn render_csv(columns: &[String], rows: &[Value]) -> String {
    let mut out = columns
        .iter()
        .map(|name| csv_cell(&Value::String(name.clone())))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    for row in rows {
        let line: Vec<String> = columns
            .iter()
            .map(|name| csv_cell(row.get(name).unwrap_or(&Value::Null)))
            .collect();
        out.push_str(&line.join(","));
        out.push('\n');
    }
    out
}

pub async fn data_query(env: &dyn ToolEnv, args: DataQueryArgs) -> ToolResult<ToolOutput> {
    let started = Instant::now();
    let limits = env.limits().clone();
    if args.sql.trim().is_empty() {
        return Err(ToolError::invalid("The SQL is empty"));
    }
    validate_readonly_sql(&args.sql).map_err(|error| {
        ToolError::refused(format!(
            "Only one read-only SELECT or WITH statement is allowed: {error:#}. To keep a result, use save_as"
        ))
    })?;
    let target = target_of(args.save_as.as_ref())?;
    let max_rows = args
        .max_rows
        .unwrap_or(limits.data_default_rows)
        .clamp(1, limits.data_max_rows);

    let workspace = workspace_of(env, "query its tables")?;
    let session = session_of(env);
    let workspace_tables = session.open(env, &workspace).await?;
    let mut schemas = vec![WorkbenchSchema {
        name: WORKSPACE_SCHEMA.to_string(),
        connection: workspace_tables.connection.clone(),
        reader: workspace_tables.reader.clone(),
    }];
    for (alias, app_id) in app_aliases(env.scope()) {
        if mentions(&args.sql, &alias) {
            let open = session.open(env, &app_id).await?;
            schemas.push(WorkbenchSchema {
                name: alias,
                connection: open.connection,
                reader: open.reader,
            });
        }
    }
    if target.is_some() && !workspace_tables.writable {
        return Err(ToolError::refused(
            "The workspace tables are read-only here, so a result cannot be saved",
        ));
    }

    let wanted = if target.is_some() {
        limits.snapshot_max_rows
    } else {
        max_rows
    };
    let outcome = execute_readonly_sql_across_apps(
        &schemas,
        &args.sql,
        &args.params.clone().unwrap_or(Value::Null),
        WorkbenchLimits {
            rows: Some(wanted),
            memory_bytes: Some(limits.data_memory_bytes),
        },
    )
    .await
    .map_err(|error| ToolError::new(code::FAILED, format!("The query failed: {error:#}")))?;

    let result = outcome.result;
    let column_names: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
    let mut output = ToolOutput::new(Value::Null).classified(outcome.sensitivity.classification);
    let saved = match target {
        Some(Target::Table { name, mode }) => {
            let count = save_rows(
                &workspace_tables.connection,
                &name,
                result.rows.clone(),
                mode,
            )
            .await?;
            Some(json!({ "table": format!("{WORKSPACE_SCHEMA}.{name}"), "rows": count }))
        }
        Some(Target::File { path, format }) => {
            let (bytes, mime) = match format {
                Format::Csv => (
                    render_csv(&column_names, &result.rows).into_bytes(),
                    "text/csv",
                ),
                Format::Json => (
                    serde_json::to_vec_pretty(&result.rows).map_err(|error| {
                        ToolError::new(code::FAILED, format!("Encoding the result failed: {error}"))
                    })?,
                    "application/json",
                ),
            };
            let bytes = Bytes::from(bytes);
            env.write_workspace(&path, bytes.clone(), mime).await?;
            let handle = handle_for(&workspace, &path, &bytes, mime);
            output = output.saved(handle.clone());
            Some(json!({ "file": handle }))
        }
        None => None,
    };
    if result.truncated {
        output.warnings.push(format!(
            "The result hit the {wanted} row limit and is incomplete"
        ));
    }

    let shown: Vec<Value> = result.rows.iter().take(max_rows).cloned().collect();
    let cut = result.rows.len() > max_rows || result.truncated;
    output.truncated = cut && saved.is_none();
    output.result = json!({
        "columns": result.columns.iter().map(|c| json!({
            "name": c.name,
            "type": c.type_name,
            "classification": c.metadata.get(flow_like_storage::contracts::column_metadata::CLASSIFICATION_METADATA_KEY),
        })).collect::<Vec<_>>(),
        "rows": shown,
        "row_count": result.row_count,
        "truncated": cut,
        "saved": saved,
        "sources_touched": outcome.tables,
        "pii": outcome.sensitivity.pii,
        "elapsed_ms": started.elapsed().as_millis() as u64,
    });
    Ok(output)
}
