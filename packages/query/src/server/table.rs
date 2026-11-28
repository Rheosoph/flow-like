//! Table browsing reads. Lane L6 of `todo/1140-query-service` owns this file.

use super::ServiceConfig;
use super::ops::{FrameSink, Opened, internal, invalid, send_json, workbench_source};
use crate::protocol::table::TableOperation;
use crate::protocol::{ErrorCode, Trailer, WireError};
use flow_like_storage::databases::table_summary::summarize_tables_for;
use flow_like_storage::databases::vector::schema::{PrimaryKeyRejected, TableInputRejected};
use flow_like_storage::databases::vector::{
    VectorStore,
    lancedb::{LanceDBVectorStore, record_batches_to_vec},
};
use flow_like_storage::databases::workbench::ExecutionBudget;
use flow_like_storage::datafusion::{
    arrow::error::ArrowError, error::DataFusionError, prelude::SessionContext,
};
use flow_like_storage::lancedb::Connection;
use flow_like_storage_contracts::column_metadata::ColumnReader;
use flow_like_storage_contracts::database::DatabaseSelector;
use flow_like_types::Value;
use serde_json::json;
use std::time::Instant;

const MAX_TABLE_NAME_CHARS: usize = 256;

pub(super) async fn execute(
    _config: &ServiceConfig,
    opened: &Opened,
    operation: TableOperation,
    budget: &ExecutionBudget,
    sink: &FrameSink,
    warnings: Vec<String>,
    started: Instant,
) -> Result<(), WireError> {
    let (connection, reader) = workbench_source(opened)?;
    let answer = tokio::time::timeout(budget.timeout, answer(connection, reader, operation))
        .await
        .map_err(|_| WireError::new(ErrorCode::Timeout, "The table read timed out"))??;
    let trailer = Trailer {
        row_count: answer.as_array().map_or(1, Vec::len),
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
        ..Trailer::default()
    };
    send_json(sink, &answer, trailer).await
}

async fn answer(
    connection: &Connection,
    reader: &ColumnReader,
    operation: TableOperation,
) -> Result<Value, WireError> {
    match operation {
        TableOperation::Query {
            table,
            selector,
            sql,
            sql_params,
            vector,
            filter,
            fts_term,
            rerank,
            select,
            limit,
            offset,
        } => {
            let db = open_table(connection, reader, table.clone(), selector).await?;
            let items = match (sql, vector, fts_term, filter) {
                (Some(sql), ..) => run_sql_query(&db, table, &sql, &sql_params).await?,
                (None, Some(vector), None, filter) => db
                    .vector_search(vector, filter.as_deref(), select, limit, offset)
                    .await
                    .map_err(input_error)?,
                (None, None, Some(term), filter) => db
                    .fts_search(&term, filter.as_deref(), select, None, limit, offset)
                    .await
                    .map_err(input_error)?,
                (None, Some(vector), Some(term), filter) => db
                    .hybrid_search(
                        vector,
                        &term,
                        filter.as_deref(),
                        select,
                        None,
                        limit,
                        offset,
                        rerank.unwrap_or(true),
                    )
                    .await
                    .map_err(input_error)?,
                (None, None, None, Some(filter)) => db
                    .filter(&filter, select, limit, offset)
                    .await
                    .map_err(input_error)?,
                (None, None, None, None) => {
                    return Err(invalid("No valid query parameters provided"));
                }
            };
            Ok(Value::Array(items))
        }
        TableOperation::List {
            table,
            selector,
            limit,
            offset,
        } => {
            let db = open_table(connection, reader, table, selector).await?;
            let items = db.list(None, limit, offset).await.map_err(storage_error)?;
            Ok(Value::Array(items))
        }
        TableOperation::Count { table, selector } => {
            let db = open_table(connection, reader, table, selector).await?;
            let count = db.count(None).await.map_err(storage_error)?;
            Ok(json!(count))
        }
        TableOperation::Schema { table, selector } => {
            let db = open_table(connection, reader, table, selector).await?;
            to_json(&db.schema().await.map_err(storage_error)?)
        }
        TableOperation::Indices { table, selector } => {
            let db = open_table(connection, reader, table, selector).await?;
            let indices = db.list_indices().await.map_err(storage_error)?;
            Ok(Value::Array(
                indices
                    .into_iter()
                    .map(|index| {
                        json!({
                            "name": index.name,
                            "index_type": index.index_type,
                            "columns": index.columns,
                        })
                    })
                    .collect(),
            ))
        }
        TableOperation::View {
            table,
            selector,
            limit,
            offset,
        } => {
            let db = open_table(connection, reader, table, selector).await?;
            let schema = db.schema().await.map_err(storage_error)?;
            let count = db.count(None).await.map_err(storage_error)?;
            let items = db.list(None, limit, offset).await.map_err(storage_error)?;
            Ok(json!({ "schema": to_json(&schema)?, "count": count, "items": items }))
        }
        TableOperation::History { table, selector } => {
            validate_table(&table)?;
            validate_selector(&selector)?;
            let db = LanceDBVectorStore::from_connection_with_selector(
                connection.clone(),
                table,
                selector,
            )
            .await
            .map_err(storage_error)?;
            to_json(&db.history().await.map_err(storage_error)?)
        }
        TableOperation::Compare {
            table,
            selector,
            other,
            key,
            limit,
        } => {
            validate_selector(&other)?;
            let source = open_table(connection, reader, table.clone(), selector).await?;
            let target = open_table(connection, reader, table, other).await?;
            let diff = source
                .compare(&target, &key, limit.min(1000))
                .await
                .map_err(storage_error)?;
            to_json(&diff)
        }
        TableOperation::ListTables { summary } => list_tables(connection, reader, summary).await,
    }
}

async fn list_tables(
    connection: &Connection,
    reader: &ColumnReader,
    summary: bool,
) -> Result<Value, WireError> {
    let names: Vec<String> = connection
        .table_names()
        .execute()
        .await
        .map_err(|error| storage_error(error.into()))?
        .into_iter()
        .filter(|name| !is_reserved_table(name))
        .collect();
    if summary {
        to_json(&summarize_tables_for(connection, names, reader).await)
    } else {
        to_json(&names)
    }
}

async fn open_table(
    connection: &Connection,
    reader: &ColumnReader,
    table: String,
    selector: DatabaseSelector,
) -> Result<LanceDBVectorStore, WireError> {
    validate_table(&table)?;
    validate_selector(&selector)?;
    Ok(
        LanceDBVectorStore::from_connection_with_selector(connection.clone(), table, selector)
            .await
            .map_err(storage_error)?
            .with_column_reader(reader.clone()),
    )
}

fn is_reserved_table(name: &str) -> bool {
    name.starts_with("__") && name.ends_with("__") && name.len() > 4
}

fn validate_table(name: &str) -> Result<(), WireError> {
    if name.is_empty() || name.len() > MAX_TABLE_NAME_CHARS {
        return Err(invalid(format!(
            "Table name must be 1-{MAX_TABLE_NAME_CHARS} characters"
        )));
    }
    if name.contains("..")
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(invalid(format!("'{name}' is not a valid table name")));
    }
    if is_reserved_table(name) {
        return Err(invalid("Table name is reserved for internal use"));
    }
    Ok(())
}

fn validate_selector(selector: &DatabaseSelector) -> Result<(), WireError> {
    selector
        .validate()
        .map_err(|error| invalid(error.to_string()))
}

fn to_json(value: &impl serde::Serialize) -> Result<Value, WireError> {
    serde_json::to_value(value).map_err(|error| internal("Encoding the table answer failed", error))
}

fn storage_error(error: flow_like_types::Error) -> WireError {
    internal("The table read failed", format!("{error:#}"))
}

fn rejection_message<'a>(
    mut causes: impl Iterator<Item = &'a (dyn std::error::Error + 'static)>,
) -> Option<String> {
    causes.find_map(|cause| {
        cause
            .downcast_ref::<TableInputRejected>()
            .map(ToString::to_string)
            .or_else(|| {
                cause
                    .downcast_ref::<PrimaryKeyRejected>()
                    .map(ToString::to_string)
            })
    })
}

/// Rejected values, column names and key rules are the caller's to fix; anything else stays internal.
fn input_error(error: flow_like_types::Error) -> WireError {
    match rejection_message(error.chain()) {
        Some(message) => invalid(message),
        None => storage_error(error),
    }
}

async fn run_sql_query(
    db: &LanceDBVectorStore,
    table: String,
    sql: &str,
    sql_params: &Value,
) -> Result<Vec<Value>, WireError> {
    flow_like_storage::databases::sql_guard::validate_readonly_sql(sql)
        .map_err(|error| invalid(format!("Invalid query SQL: {error}")))?;
    let context = SessionContext::new_with_state(
        flow_like_storage::databases::df_provider::with_lance_order_pushdown(
            SessionContext::new().state(),
        ),
    );
    flow_like_storage::databases::register_sql_functions(&context);
    let fusion = db.to_datafusion().await.map_err(storage_error)?;
    context
        .register_table(table, fusion)
        .map_err(|error| internal("Registering the table failed", error))?;
    let param_values = flow_like_storage::databases::sql_params::bind_params(sql_params)
        .map_err(|error| invalid(format!("Invalid query parameters: {error}")))?;
    let df = context
        .sql(sql)
        .await
        .and_then(|df| df.with_param_values(param_values))
        .map_err(|error| invalid_query(&error))?;
    let batches = df.collect().await.map_err(query_execution_error)?;
    record_batches_to_vec(Some(batches)).map_err(|error| invalid(error.to_string()))
}

fn invalid_query(error: &DataFusionError) -> WireError {
    invalid(format!("Invalid query: {}", error.strip_backtrace()))
}

/// Unplannable statements and function or cast failures on the query's own values surface only
/// once execution starts; those stay the caller's to fix, storage and resource failures do not.
fn query_execution_error(error: DataFusionError) -> WireError {
    let causes = std::iter::successors(
        Some(&error as &(dyn std::error::Error + 'static)),
        |cause| cause.source(),
    );
    if let Some(message) = rejection_message(causes) {
        return invalid(message);
    }
    match error.find_root() {
        DataFusionError::Plan(_)
        | DataFusionError::SchemaError(..)
        | DataFusionError::SQL(..)
        | DataFusionError::NotImplemented(_)
        | DataFusionError::Execution(_) => invalid_query(&error),
        DataFusionError::ArrowError(arrow_error, _) if is_value_error(arrow_error) => {
            invalid_query(&error)
        }
        _ => internal("The query failed", error.strip_backtrace()),
    }
}

fn is_value_error(error: &ArrowError) -> bool {
    matches!(
        error,
        ArrowError::CastError(_)
            | ArrowError::ParseError(_)
            | ArrowError::DivideByZero
            | ArrowError::ArithmeticOverflow(_)
            | ArrowError::InvalidArgumentError(_)
            | ArrowError::ComputeError(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::arrow_array::{Int64Array, RecordBatch, StringArray};
    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    use std::sync::Arc;

    const FEATURE_COLLECTION_REJECTION: &str =
        "ST_GeomFromGeoJSON: received a GeoJSON FeatureCollection; write one row per feature";

    #[test]
    fn planning_and_value_failures_are_invalid_queries() {
        let plan = DataFusionError::Plan("Invalid function 'st_asgeojsn'".to_string())
            .context("Failed to execute query on table 'sites'");
        let error = query_execution_error(plan);
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(
            error.message.starts_with("Invalid query: "),
            "{}",
            error.message
        );

        for error in [
            DataFusionError::Execution(FEATURE_COLLECTION_REJECTION.to_string()),
            DataFusionError::ArrowError(Box::new(ArrowError::DivideByZero), None),
        ] {
            let wire = query_execution_error(error.context("Query on table 'sites' failed"));
            assert_eq!(wire.code, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn table_input_rejections_keep_their_own_message() {
        let error = DataFusionError::External(Box::new(TableInputRejected(
            FEATURE_COLLECTION_REJECTION.to_string(),
        )))
        .context("Query on table 'sites' failed");
        let wire = query_execution_error(error);
        assert_eq!(wire.code, ErrorCode::InvalidRequest);
        assert_eq!(wire.message, FEATURE_COLLECTION_REJECTION);
    }

    #[test]
    fn runtime_failures_stay_internal() {
        for error in [
            DataFusionError::IoError(std::io::Error::other("connection reset")),
            DataFusionError::ResourcesExhausted("memory pool exhausted".into()),
            DataFusionError::Internal("unexpected plan shape".into()),
        ] {
            assert_eq!(query_execution_error(error).code, ErrorCode::Internal);
        }
    }

    async fn orders() -> (std::path::PathBuf, Connection) {
        let path =
            std::env::temp_dir().join(format!("flow-like-table-{}", flow_like_types::create_id()));
        std::fs::create_dir_all(&path).unwrap();
        let connection = flow_like_storage::lancedb::connect(&path.to_string_lossy())
            .execute()
            .await
            .unwrap();
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("region", DataType::Utf8, true),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int64Array::from(vec![1, 2, 3])),
                Arc::new(StringArray::from(vec!["West", "East", "West"])),
            ],
        )
        .unwrap();
        connection
            .create_table("orders", vec![batch])
            .execute()
            .await
            .unwrap();
        (path, connection)
    }

    fn search(
        table: &str,
        sql: Option<&str>,
        sql_params: Value,
        filter: Option<&str>,
    ) -> TableOperation {
        TableOperation::Query {
            table: table.to_string(),
            selector: DatabaseSelector::default(),
            sql: sql.map(str::to_string),
            sql_params,
            vector: None,
            filter: filter.map(str::to_string),
            fts_term: None,
            rerank: None,
            select: None,
            limit: 10,
            offset: 0,
        }
    }

    async fn run(connection: &Connection, operation: TableOperation) -> Result<Value, WireError> {
        answer(connection, &ColumnReader::Unrestricted, operation).await
    }

    #[tokio::test]
    async fn reads_rows_counts_schemas_and_listings() {
        let (path, connection) = orders().await;
        let sql = search(
            "orders",
            Some("SELECT id FROM orders WHERE region = $region ORDER BY id"),
            json!({ "region": "West" }),
            None,
        );
        assert_eq!(
            run(&connection, sql).await.unwrap(),
            json!([{ "id": 1 }, { "id": 3 }])
        );
        let filtered = search("orders", None, Value::Null, Some("id > 1"));
        assert_eq!(
            run(&connection, filtered)
                .await
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );

        let count = run(
            &connection,
            TableOperation::Count {
                table: "orders".into(),
                selector: DatabaseSelector::default(),
            },
        )
        .await
        .unwrap();
        assert_eq!(count, json!(3));

        let view = run(
            &connection,
            TableOperation::View {
                table: "orders".into(),
                selector: DatabaseSelector::default(),
                limit: 2,
                offset: 0,
            },
        )
        .await
        .unwrap();
        assert_eq!(view["count"], json!(3));
        assert_eq!(view["items"].as_array().unwrap().len(), 2);
        assert!(view["schema"]["fields"].is_array());

        let names = run(&connection, TableOperation::ListTables { summary: false })
            .await
            .unwrap();
        assert_eq!(names, json!(["orders"]));
        let summaries = run(&connection, TableOperation::ListTables { summary: true })
            .await
            .unwrap();
        assert_eq!(summaries.as_array().unwrap().len(), 1);
        std::fs::remove_dir_all(path).ok();
    }

    #[tokio::test]
    async fn rejects_bad_names_writes_and_empty_queries() {
        let (path, connection) = orders().await;
        for table in ["", "../x", "a/b", "__graph_overlays__"] {
            let error = run(
                &connection,
                search(table, None, Value::Null, Some("id > 1")),
            )
            .await
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::InvalidRequest, "{table}");
        }
        let empty = search("orders", None, Value::Null, None);
        assert_eq!(
            run(&connection, empty).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        let write = search("orders", Some("DELETE FROM orders"), Value::Null, None);
        assert_eq!(
            run(&connection, write).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        std::fs::remove_dir_all(path).ok();
    }
}
