use std::sync::Arc;

use chrono::Utc;
use flow_like_storage::databases::workbench::{
    WorkbenchLimits, WorkbenchSchema, execute_readonly_sql_across_apps,
};
use serde_json::{Value, json};

use super::session::{Source, WORKSPACE_SCHEMA, app_aliases, resolve_source, session_of};
use super::{AttachMode, AttachSpec, ConnectionSpec, DataSourcesArgs, SourceKind, SourceOperation};
use crate::data_backend::{DataBackend, NetworkTable, SourceTable};
use crate::env::{Environment, ToolEnv};
use crate::error::{ToolError, ToolResult, code};
use crate::files::{resolve, workspace_of};
use crate::outcome::ToolOutput;
use crate::tables::{
    SOURCES_TABLE, all_json, delete_where, drop_table, ensure_not_memory_table,
    replace_with_batches, table_names, upsert_json, workspace_table_names,
};

const SAMPLE_ROWS: usize = 5;
const PREFIX: &str = "ext_";

fn source_name(name: &str) -> ToolResult<String> {
    let name = name.trim().to_lowercase();
    let valid = !name.is_empty()
        && name.len() <= 30
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if valid {
        Ok(name)
    } else {
        Err(ToolError::invalid(format!(
            "'{name}' is not a valid source name: use 1 to 30 characters of a-z, 0-9 and '_', starting with a letter"
        )))
    }
}

fn clean_table(name: &str) -> String {
    let clean: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    clean.trim_matches('_').to_string()
}

fn snapshot_table(source: &str, table: &str) -> String {
    let table = clean_table(table);
    if table.is_empty() {
        format!("{PREFIX}{source}")
    } else {
        format!("{PREFIX}{source}_{table}")
    }
}

fn quoted(identifier: &str) -> ToolResult<String> {
    if !identifier.is_empty()
        && identifier
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        Ok(format!("\"{identifier}\""))
    } else {
        Err(ToolError::invalid(format!(
            "'{identifier}' is not a plain table name"
        )))
    }
}

fn backend_of(env: &dyn ToolEnv) -> ToolResult<Arc<dyn DataBackend>> {
    env.data_backend().ok_or_else(|| {
        ToolError::new(
            code::UNSUPPORTED_FILE,
            "This host cannot read CSV or Excel files or connect to databases",
        )
    })
}

pub async fn data_sources(env: &dyn ToolEnv, args: DataSourcesArgs) -> ToolResult<ToolOutput> {
    match args.operation {
        SourceOperation::List => list(env).await,
        SourceOperation::Describe => describe(env, args).await,
        SourceOperation::Attach => {
            let spec = args
                .attach
                .ok_or_else(|| ToolError::invalid("attach needs an 'attach' description"))?;
            attach(env, spec).await
        }
        SourceOperation::Detach => detach(env, &named(args.name)?).await,
        SourceOperation::Refresh => refresh(env, &named(args.name)?).await,
    }
}

fn named(name: Option<String>) -> ToolResult<String> {
    source_name(
        &name.ok_or_else(|| ToolError::invalid("This operation needs the source's 'name'"))?,
    )
}

async fn list(env: &dyn ToolEnv) -> ToolResult<ToolOutput> {
    let workspace = workspace_of(env, "list its tables")?;
    let session = session_of(env);
    let ws = session.open(env, &workspace).await?;
    let mut apps = Vec::new();
    for (alias, app_id) in app_aliases(env.scope()) {
        let open = session.open(env, &app_id).await?;
        apps.push(json!({
            "schema": alias,
            "app_id": app_id,
            "tables": table_names(&open.connection).await?,
        }));
    }
    let attached: Vec<Value> = all_json(&ws.connection, SOURCES_TABLE)
        .await?
        .into_iter()
        .map(|row| {
            json!({
                "name": row["id"],
                "kind": row["kind"],
                "tables": serde_json::from_str::<Value>(row["tables"].as_str().unwrap_or("[]")).unwrap_or(Value::Null),
                "mode": row["mode"],
                "attached_at": row["attached_at"],
            })
        })
        .collect();
    Ok(ToolOutput::new(json!({
        "workspace": { "schema": WORKSPACE_SCHEMA, "tables": workspace_table_names(&ws.connection).await? },
        "apps": apps,
        "attached": attached,
    })))
}

async fn describe(env: &dyn ToolEnv, args: DataSourcesArgs) -> ToolResult<ToolOutput> {
    let source = args.source.as_deref().unwrap_or(WORKSPACE_SCHEMA);
    let (schema, app_id) = match resolve_source(env.scope(), source)? {
        Source::Workspace => (
            WORKSPACE_SCHEMA.to_string(),
            workspace_of(env, "describe its tables")?,
        ),
        Source::App { alias, app_id } => (alias, app_id),
    };
    let session = session_of(env);
    let open = session.open(env, &app_id).await?;
    let Some(table) = args.table.as_deref() else {
        let tables = if env.scope().is_workspace(&app_id) {
            workspace_table_names(&open.connection).await?
        } else {
            table_names(&open.connection).await?
        };
        return Ok(ToolOutput::new(json!({
            "schema": schema,
            "tables": tables,
        })));
    };
    if env.scope().is_workspace(&app_id) {
        ensure_not_memory_table(table)?;
    }
    let table = quoted(table)?;
    let schemas = [WorkbenchSchema {
        name: schema.clone(),
        connection: open.connection,
        reader: open.reader,
    }];
    let limits = WorkbenchLimits {
        rows: Some(if args.include_sample { SAMPLE_ROWS } else { 1 }),
        memory_bytes: Some(env.limits().data_memory_bytes),
    };
    let shape = run_sql(
        &schemas,
        &format!("SELECT * FROM {schema}.{table} LIMIT {SAMPLE_ROWS}"),
        limits,
    )
    .await?;
    let counted = run_sql(
        &schemas,
        &format!("SELECT COUNT(*) AS n FROM {schema}.{table}"),
        WorkbenchLimits {
            rows: Some(1),
            memory_bytes: limits.memory_bytes,
        },
    )
    .await?;
    let rows = counted
        .result
        .rows
        .first()
        .and_then(|row| row.get("n"))
        .cloned();
    let columns: Vec<Value> = shape
        .result
        .columns
        .iter()
        .map(|column| {
            json!({
                "name": column.name,
                "type": column.type_name,
                "classification": column.metadata.get(flow_like_storage::contracts::column_metadata::CLASSIFICATION_METADATA_KEY),
                "description": column.metadata.get(flow_like_storage::contracts::column_metadata::DESCRIPTION_KEY),
            })
        })
        .collect();
    let mut result = json!({ "schema": schema, "columns": columns, "row_count": rows });
    if args.include_sample {
        result["sample"] = json!(shape.result.rows);
    }
    Ok(ToolOutput::new(result).classified(shape.sensitivity.classification))
}

async fn run_sql(
    schemas: &[WorkbenchSchema],
    sql: &str,
    limits: WorkbenchLimits,
) -> ToolResult<flow_like_storage::databases::workbench::CrossAppQueryResult> {
    execute_readonly_sql_across_apps(schemas, sql, &Value::Null, limits)
        .await
        .map_err(|error| {
            ToolError::new(code::FAILED, format!("Reading the table failed: {error:#}"))
        })
}

async fn existing(env: &dyn ToolEnv, name: &str) -> ToolResult<Option<Value>> {
    let ws = session_of(env)
        .open(env, &workspace_of(env, "keep attached sources")?)
        .await?;
    Ok(all_json(&ws.connection, SOURCES_TABLE)
        .await?
        .into_iter()
        .find(|row| row["id"] == name))
}

async fn write_snapshots(
    env: &dyn ToolEnv,
    name: &str,
    tables: &[SourceTable],
) -> ToolResult<Vec<Value>> {
    let ws = session_of(env)
        .open(env, &workspace_of(env, "keep attached sources")?)
        .await?;
    if !ws.writable {
        return Err(ToolError::refused(
            "The workspace tables are read-only here, so sources cannot be attached",
        ));
    }
    let mut written = Vec::new();
    for table in tables {
        let Some(first) = table.batches.first() else {
            continue;
        };
        let target = snapshot_table(name, &table.name);
        let rows = replace_with_batches(
            &ws.connection,
            &target,
            first.schema().as_ref(),
            &table.batches,
        )
        .await?;
        written.push(json!({ "table": format!("{WORKSPACE_SCHEMA}.{target}"), "name": target, "rows": rows }));
    }
    if written.is_empty() {
        return Err(ToolError::new(
            code::UNSUPPORTED_FILE,
            "The source has no rows to copy",
        ));
    }
    Ok(written)
}

fn scrub(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        text.to_string()
    } else {
        text.replace(secret, "***")
    }
}

async fn read_file_tables(
    env: &dyn ToolEnv,
    spec: &AttachSpec,
    backend: &Arc<dyn DataBackend>,
) -> ToolResult<(Vec<SourceTable>, Vec<String>)> {
    let file = spec
        .file
        .as_ref()
        .ok_or_else(|| ToolError::invalid("Files need a 'file' to read"))?;
    let resolved = resolve(env, file, env.limits().max_ingest_bytes).await?;
    if spec.kind == SourceKind::Csv {
        let csv = spec.csv.clone().unwrap_or(super::CsvSpec {
            delimiter: None,
            has_header: None,
        });
        let delimiter = csv
            .delimiter
            .as_deref()
            .and_then(|d| d.bytes().next())
            .unwrap_or(b',');
        let table = backend
            .csv(resolved.bytes, delimiter, csv.has_header.unwrap_or(true))
            .await
            .map_err(|error| {
                ToolError::new(
                    code::UNSUPPORTED_FILE,
                    format!("Reading '{}' failed: {error:#}", resolved.name),
                )
            })?;
        return Ok((vec![table], Vec::new()));
    }
    let excel = spec.excel.clone();
    let detect = excel
        .as_ref()
        .and_then(|excel| excel.mode.as_deref())
        .is_none_or(|mode| mode != "sheet_as_table");
    backend
        .workbook(
            resolved.bytes,
            excel.as_ref().and_then(|excel| excel.sheet.as_deref()),
            detect,
        )
        .await
        .map_err(|error| {
            ToolError::new(
                code::UNSUPPORTED_FILE,
                format!("Reading '{}' failed: {error:#}", resolved.name),
            )
        })
}

async fn read_network_tables(
    env: &dyn ToolEnv,
    connection: &ConnectionSpec,
    kind: SourceKind,
    backend: &Arc<dyn DataBackend>,
) -> ToolResult<Vec<SourceTable>> {
    let password = env.secret(&connection.secret).await?.ok_or_else(|| {
        ToolError::not_found(format!(
            "The bot has no secret named '{}'; call secret_list for the stored names, or secret_request to ask the owner for it, then attach again",
            connection.secret
        ))
    })?;
    let server = env.environment() == Environment::Server;
    let port = connection.port.unwrap_or(match kind {
        SourceKind::Mysql => 3306,
        SourceKind::Clickhouse => 9000,
        _ => 5432,
    });
    let mut tables = Vec::new();
    for table in &connection.tables {
        if env.is_cancelled() {
            return Err(ToolError::new(code::CANCELLED, "The call was cancelled"));
        }
        let source = NetworkTable {
            kind: kind.as_str().to_string(),
            host: connection.host.clone(),
            port,
            database: connection.database.clone(),
            schema: connection.schema.clone(),
            table: table.clone(),
            username: connection.username.clone(),
            password: password.clone(),
            ssl_mode: connection.ssl_mode.clone(),
        };
        let read = backend
            .network(&source, server, server, env.limits().snapshot_max_rows)
            .await
            .map_err(|error| {
                ToolError::new(
                    code::FAILED,
                    format!(
                        "Connecting to {}:{} failed: {}",
                        connection.host,
                        port,
                        scrub(&format!("{error:#}"), &password)
                    ),
                )
            })?;
        tables.push(SourceTable {
            name: table.clone(),
            batches: read.batches,
        });
    }
    Ok(tables)
}

async fn attach(env: &dyn ToolEnv, spec: AttachSpec) -> ToolResult<ToolOutput> {
    let name = source_name(&spec.name)?;
    if spec.kind.is_network() && !env.scope().data_connections {
        return Err(ToolError::refused(
            "Attaching a database needs the data_connections capability, which this bot does not have",
        ));
    }
    if spec.mode == AttachMode::Live {
        return Err(ToolError::new(
            code::NOT_YET,
            "Live sources are not available yet: use mode 'snapshot' and refresh it when the data changes",
        ));
    }
    let backend = backend_of(env)?;
    let (tables, warnings) = if spec.kind.is_network() {
        let connection = spec
            .connection
            .as_ref()
            .ok_or_else(|| ToolError::invalid("A database needs a 'connection'"))?;
        if connection.tables.is_empty() {
            return Err(ToolError::invalid(
                "Name the tables to copy in connection.tables",
            ));
        }
        (
            read_network_tables(env, connection, spec.kind, &backend).await?,
            Vec::new(),
        )
    } else {
        read_file_tables(env, &spec, &backend).await?
    };

    let previous = existing(env, &name).await?;
    let written = write_snapshots(env, &name, &tables).await?;
    let ws = session_of(env)
        .open(env, &workspace_of(env, "keep attached sources")?)
        .await?;
    if let Some(previous) = previous {
        let names: Vec<String> =
            serde_json::from_str(previous["tables"].as_str().unwrap_or("[]")).unwrap_or_default();
        for old in names {
            if !written.iter().any(|w| w["name"] == old) {
                drop_table(&ws.connection, &old).await?;
            }
        }
    }
    let table_names: Vec<&Value> = written.iter().map(|w| &w["name"]).collect();
    upsert_json(
        &ws.connection,
        SOURCES_TABLE,
        json!({
            "id": name,
            "kind": spec.kind.as_str(),
            "spec": serde_json::to_string(&spec).map_err(|e| ToolError::new(code::FAILED, e.to_string()))?,
            "tables": serde_json::to_string(&table_names).unwrap_or_default(),
            "mode": "snapshot",
            "attached_at": Utc::now().timestamp_millis(),
        }),
    )
    .await?;
    let mut output =
        ToolOutput::new(json!({ "name": name, "kind": spec.kind.as_str(), "tables": written }));
    output.warnings = warnings;
    Ok(output)
}

async fn detach(env: &dyn ToolEnv, name: &str) -> ToolResult<ToolOutput> {
    let row = existing(env, name)
        .await?
        .ok_or_else(|| ToolError::not_found(format!("No attached source '{name}'")))?;
    let ws = session_of(env)
        .open(env, &workspace_of(env, "keep attached sources")?)
        .await?;
    let names: Vec<String> =
        serde_json::from_str(row["tables"].as_str().unwrap_or("[]")).unwrap_or_default();
    for table in &names {
        drop_table(&ws.connection, table).await?;
    }
    delete_where(&ws.connection, SOURCES_TABLE, &format!("id = '{name}'")).await?;
    Ok(ToolOutput::new(
        json!({ "name": name, "dropped_tables": names }),
    ))
}

async fn refresh(env: &dyn ToolEnv, name: &str) -> ToolResult<ToolOutput> {
    let row = existing(env, name)
        .await?
        .ok_or_else(|| ToolError::not_found(format!("No attached source '{name}'")))?;
    let spec: AttachSpec =
        serde_json::from_str(row["spec"].as_str().unwrap_or("{}")).map_err(|error| {
            ToolError::new(
                code::FAILED,
                format!("The stored description of '{name}' is unreadable: {error}"),
            )
        })?;
    attach(env, spec).await
}
