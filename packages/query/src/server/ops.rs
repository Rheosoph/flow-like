//! Running one request: clamp its limits to the service's, open its sources, execute, frame.

use super::{ServiceConfig, ServiceLimits};
use crate::frames::{FrameKind, encode_frame, encode_json_frame};
use crate::protocol::{
    ErrorCode, ExternalSource, Operation, QueryRequest, QueryShape, RequestLimits, SourceLocation,
    SourceSpec, SurfaceSpec, Trailer, WireError,
};
use arrow_ipc::CompressionType;
use arrow_ipc::writer::{IpcWriteOptions, StreamWriter};
use bytes::Bytes;
use flow_like_storage::databases::workbench::{
    CrossAppSources, ExecutionBudget, ExternalSchema, PreparedQuery, QueryDescription,
    WorkbenchSchema, WorkbenchSurface, WorkbenchView, describe_cross_app_query,
    describe_workbench_query, parameter_options_answer, parameter_options_query,
    prepare_cross_app_query, prepare_workbench_query,
};
use flow_like_storage::lancedb::Connection;
use flow_like_storage_contracts::column_metadata::ColumnReader;
use flow_like_types::Value;
use serde_json::json;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Arrow chunks are split so no frame grows near the decoder's limit.
const ARROW_FRAME_BYTES: usize = 4 * 1024 * 1024;
/// Warehouse statements end this long before the request does, so they fail as timed out and
/// Athena stops its query while the handler still runs.
const WAREHOUSE_MARGIN: Duration = Duration::from_secs(2);

pub(super) struct FrameSink {
    sender: mpsc::Sender<Bytes>,
}

impl FrameSink {
    pub(super) fn new(sender: mpsc::Sender<Bytes>) -> Self {
        Self { sender }
    }

    pub(super) async fn send(&self, frame: Bytes) -> Result<(), WireError> {
        self.sender
            .send(frame)
            .await
            .map_err(|_| WireError::new(ErrorCode::Internal, "The caller stopped reading"))
    }

    pub(super) fn closed(&self) -> bool {
        self.sender.is_closed()
    }
}

pub(super) fn invalid(message: impl Into<String>) -> WireError {
    WireError::new(ErrorCode::InvalidRequest, message)
}

pub(super) fn failed(error: flow_like_types::Error) -> WireError {
    WireError::from_query_error(&error)
}

pub(super) fn internal(context: &str, error: impl std::fmt::Display) -> WireError {
    WireError::new(ErrorCode::Internal, format!("{context}: {error}"))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or_default()
}

/// The request's limits within the service's ceilings, noting every reduction.
fn budget(
    limits: &ServiceLimits,
    requested: &RequestLimits,
    deadline_ms: i64,
    warnings: &mut Vec<String>,
) -> Result<ExecutionBudget, WireError> {
    let remaining = deadline_ms - now_ms();
    if remaining <= 0 {
        return Err(WireError::new(
            ErrorCode::Timeout,
            "The caller's deadline passed before the query started",
        ));
    }
    let timeout =
        Duration::from_millis(requested.timeout_ms.min(remaining as u64)).min(limits.max_timeout);
    let rows = if requested.rows > limits.max_rows {
        warnings.push(format!(
            "The row limit was lowered from {} to this service's maximum of {}",
            requested.rows, limits.max_rows
        ));
        limits.max_rows
    } else {
        requested.rows.max(1)
    };
    let memory_bytes = requested
        .memory_bytes
        .unwrap_or(limits.max_memory_bytes)
        .min(limits.max_memory_bytes);
    Ok(ExecutionBudget {
        rows,
        memory_bytes: Some(memory_bytes),
        spill: limits.spill.clone(),
        timeout,
    })
}

/// The request's sources, opened with the credentials it carried.
#[derive(Default)]
pub(super) struct Opened {
    /// Sources with a schema name, for cross-app shapes.
    pub(super) lance: Vec<WorkbenchSchema>,
    /// The one source without a schema: an app's own surface (workbench, graph, table reads).
    pub(super) bare: Option<(Connection, ColumnReader)>,
    pub(super) external: Vec<ExternalSchema>,
    pub(super) external_sources: Vec<ExternalSource>,
}

async fn open(
    config: &ServiceConfig,
    sources: Vec<SourceSpec>,
    external_deadline: Option<Instant>,
) -> Result<Opened, WireError> {
    let mut opened = Opened::default();
    for spec in sources {
        match spec.location {
            SourceLocation::Lance {
                app_id,
                owner,
                credentials,
            } => {
                let unavailable = |error: flow_like_types::Error| {
                    WireError::new(
                        ErrorCode::Unavailable,
                        format!("Opening the database of app '{app_id}' failed: {error:#}"),
                    )
                };
                let builder = match &owner {
                    Some(sub) => credentials.to_db_scoped(sub, &app_id).await,
                    None => credentials.to_db(&app_id).await,
                }
                .map_err(unavailable)?;
                let connection = builder
                    .execute()
                    .await
                    .map_err(|error| unavailable(error.into()))?;
                let reader = spec.reader.to_reader();
                match spec.schema {
                    Some(name) => opened.lance.push(WorkbenchSchema {
                        name,
                        connection,
                        reader,
                    }),
                    None if opened.bare.is_none() => opened.bare = Some((connection, reader)),
                    None => {
                        return Err(invalid(
                            "Only one source may be registered without a schema",
                        ));
                    }
                }
            }
            SourceLocation::External(source) => {
                let name = spec.schema.ok_or_else(|| {
                    invalid(format!(
                        "External source '{}' needs a schema name",
                        source.source_id
                    ))
                })?;
                let resolver = config
                    .connector
                    .resolver(&source, external_deadline)
                    .await?;
                opened.external.push(ExternalSchema { name, resolver });
                opened.external_sources.push(source);
            }
        }
    }
    Ok(opened)
}

pub(super) fn surface_of(surface: SurfaceSpec) -> WorkbenchSurface {
    match surface {
        SurfaceSpec::Native => WorkbenchSurface::Native,
        SurfaceSpec::Overlay { overlay } => WorkbenchSurface::Overlay(*overlay),
        SurfaceSpec::RemoteOverlay { overlay } => WorkbenchSurface::RemoteOverlay(*overlay),
    }
}

pub(super) fn workbench_source(opened: &Opened) -> Result<&(Connection, ColumnReader), WireError> {
    opened
        .bare
        .as_ref()
        .ok_or_else(|| invalid("This read needs one app source without a schema"))
}

async fn prepare(
    opened: &Opened,
    shape: QueryShape,
    sql: &str,
    params: &Value,
    budget: &ExecutionBudget,
) -> Result<PreparedQuery, WireError> {
    match shape {
        QueryShape::CrossApp => prepare_cross_app_query(
            CrossAppSources {
                lance: &opened.lance,
                external: &opened.external,
            },
            sql,
            params,
            budget,
        )
        .await
        .map_err(failed),
        QueryShape::Workbench { surface, views } => {
            let (connection, reader) = workbench_source(opened)?;
            prepare_workbench_query(
                connection,
                surface_of(surface),
                views.iter().map(WorkbenchView::from).collect(),
                sql,
                params,
                reader,
                &opened.external,
                budget,
            )
            .await
            .map_err(failed)
        }
    }
}

async fn describe(
    opened: &Opened,
    shape: QueryShape,
    sql: &str,
) -> Result<QueryDescription, WireError> {
    match shape {
        QueryShape::CrossApp => describe_cross_app_query(
            CrossAppSources {
                lance: &opened.lance,
                external: &opened.external,
            },
            sql,
        )
        .await
        .map_err(failed),
        QueryShape::Workbench { surface, views } => {
            let (connection, reader) = workbench_source(opened)?;
            describe_workbench_query(
                connection,
                surface_of(surface),
                views.iter().map(WorkbenchView::from).collect(),
                sql,
                reader,
                &opened.external,
            )
            .await
            .map_err(failed)
        }
    }
}

/// One JSON value and the trailer: the response of every non-tabular operation.
pub(super) async fn send_json(
    sink: &FrameSink,
    value: &impl serde::Serialize,
    trailer: Trailer,
) -> Result<(), WireError> {
    sink.send(encode_json_frame(FrameKind::Json, value)).await?;
    sink.send(encode_json_frame(FrameKind::Trailer, &trailer))
        .await
}

async fn flush(writer: &mut StreamWriter<Vec<u8>>, sink: &FrameSink) -> Result<(), WireError> {
    let bytes = std::mem::take(writer.get_mut());
    for chunk in bytes.chunks(ARROW_FRAME_BYTES) {
        sink.send(encode_frame(FrameKind::Arrow, chunk)).await?;
    }
    Ok(())
}

/// Rows of a prepared query as Arrow frames, then the trailer.
pub(super) async fn stream_rows(
    prepared: PreparedQuery,
    sink: &FrameSink,
    warnings: Vec<String>,
    started: Instant,
) -> Result<(), WireError> {
    let mut trailer = Trailer {
        tables: prepared.tables.clone(),
        versions: prepared.versions.clone(),
        unversioned_tables: prepared.unversioned_tables.clone(),
        sensitivity: prepared.sensitivity,
        warnings,
        ..Trailer::default()
    };
    let mut stream = prepared.stream().await.map_err(failed)?;
    let options = IpcWriteOptions::default()
        .try_with_compression(Some(CompressionType::LZ4_FRAME))
        .map_err(|error| internal("Configuring Arrow compression failed", error))?;
    let mut writer = StreamWriter::try_new_with_options(Vec::new(), &stream.schema(), options)
        .map_err(|error| internal("Starting the Arrow stream failed", error))?;
    flush(&mut writer, sink).await?;
    let mut rows = 0;
    while let Some(batch) = stream.next_batch().await.map_err(failed)? {
        rows += batch.num_rows();
        writer
            .write(&batch)
            .map_err(|error| internal("Encoding a result batch failed", error))?;
        flush(&mut writer, sink).await?;
    }
    writer
        .finish()
        .map_err(|error| internal("Finishing the Arrow stream failed", error))?;
    flush(&mut writer, sink).await?;
    trailer.row_count = rows;
    trailer.truncated = stream.truncated();
    trailer.elapsed_ms = started.elapsed().as_millis() as u64;
    sink.send(encode_json_frame(FrameKind::Trailer, &trailer))
        .await
}

pub(super) async fn execute(
    config: &ServiceConfig,
    request: QueryRequest,
    sink: &FrameSink,
) -> Result<(), WireError> {
    let started = Instant::now();
    let mut warnings = Vec::new();
    let budget = budget(
        &config.limits,
        &request.limits,
        request.deadline_ms,
        &mut warnings,
    )?;
    let external_deadline = (started + budget.timeout).checked_sub(WAREHOUSE_MARGIN);
    let opened = open(config, request.sources, external_deadline).await?;
    let finished = |warnings: Vec<String>| Trailer {
        warnings,
        elapsed_ms: started.elapsed().as_millis() as u64,
        ..Trailer::default()
    };
    match request.operation {
        Operation::Sql {
            sql,
            params,
            shape,
            reuse_if_unchanged,
        } => {
            let prepared = prepare(&opened, shape, &sql, &params, &budget).await?;
            if reuse_if_unchanged
                .as_ref()
                .is_some_and(|versions| prepared.answers_like(versions))
            {
                let mut trailer = finished(warnings);
                trailer.unchanged = true;
                return sink
                    .send(encode_json_frame(FrameKind::Trailer, &trailer))
                    .await;
            }
            stream_rows(prepared, sink, warnings, started).await
        }
        Operation::Describe { sql, shape } => {
            let description = describe(&opened, shape, &sql).await?;
            send_json(sink, &description, finished(warnings)).await
        }
        Operation::ParameterOptions {
            table,
            column,
            search,
            range,
            shape,
        } => {
            let options = parameter_options_query(&table, &column, search.as_deref(), range)
                .map_err(|error| invalid(format!("{error:#}")))?;
            let budget = ExecutionBudget {
                rows: options.rows,
                ..budget
            };
            let result = prepare(&opened, shape, &options.sql, &options.params, &budget)
                .await?
                .collect()
                .await
                .map_err(failed)?;
            let value = parameter_options_answer(result, range);
            send_json(sink, &value, finished(warnings)).await
        }
        Operation::ExternalTables | Operation::ExternalTest => {
            let mut sources = Vec::with_capacity(opened.external_sources.len());
            for source in &opened.external_sources {
                sources.push(json!({
                    "source_id": source.source_id,
                    "tables": config.connector.tables(source).await?,
                }));
            }
            send_json(sink, &json!({ "sources": sources }), finished(warnings)).await
        }
        Operation::Graph { graph } => {
            super::graph::execute(config, &opened, graph, &budget, sink, warnings, started).await
        }
        Operation::Table { table } => {
            super::table::execute(config, &opened, table, &budget, sink, warnings, started).await
        }
        Operation::Forecast { forecast } => {
            super::forecast::execute(config, &opened, forecast, &budget, sink, warnings, started)
                .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frames::ResponseAssembler;
    use flow_like_storage::arrow_array::{Int64Array, RecordBatch, StringArray};
    use flow_like_storage::arrow_schema::{DataType, Field, Schema};
    use std::sync::Arc;

    fn limits() -> ServiceLimits {
        ServiceLimits {
            max_rows: 100,
            max_memory_bytes: 64 * 1024 * 1024,
            max_timeout: Duration::from_secs(10),
            spill: None,
        }
    }

    fn requested(rows: usize, timeout_ms: u64) -> RequestLimits {
        RequestLimits {
            rows,
            memory_bytes: Some(usize::MAX),
            timeout_ms,
        }
    }

    #[test]
    fn budgets_stay_within_the_service_and_say_so() {
        let mut warnings = Vec::new();
        let capped = budget(
            &limits(),
            &requested(5_000, 60_000),
            now_ms() + 120_000,
            &mut warnings,
        )
        .unwrap();
        assert_eq!(capped.rows, 100);
        assert_eq!(capped.memory_bytes, Some(64 * 1024 * 1024));
        assert_eq!(capped.timeout, Duration::from_secs(10));
        assert_eq!(warnings.len(), 1, "{warnings:?}");

        let mut quiet = Vec::new();
        let short = budget(
            &limits(),
            &requested(10, 2_000),
            now_ms() + 120_000,
            &mut quiet,
        )
        .unwrap();
        assert_eq!((short.rows, short.timeout), (10, Duration::from_secs(2)));
        assert!(quiet.is_empty());

        let late = budget(
            &limits(),
            &requested(10, 2_000),
            now_ms() - 1,
            &mut Vec::new(),
        );
        assert_eq!(late.unwrap_err().code, ErrorCode::Timeout);
    }

    async fn orders_app(rows: &[(i64, &str)]) -> (std::path::PathBuf, WorkbenchSchema) {
        let path =
            std::env::temp_dir().join(format!("flow-like-query-{}", flow_like_types::create_id()));
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
                Arc::new(Int64Array::from(
                    rows.iter().map(|row| row.0).collect::<Vec<_>>(),
                )),
                Arc::new(StringArray::from(
                    rows.iter().map(|row| row.1).collect::<Vec<_>>(),
                )),
            ],
        )
        .unwrap();
        connection
            .create_table("orders", vec![batch])
            .execute()
            .await
            .unwrap();
        (
            path,
            WorkbenchSchema {
                name: "shop".to_string(),
                connection,
                reader: ColumnReader::Unrestricted,
            },
        )
    }

    async fn frames_of(prepared: PreparedQuery) -> crate::frames::QueryResponse {
        let (sender, mut receiver) = mpsc::channel(64);
        let sink = FrameSink::new(sender);
        stream_rows(prepared, &sink, vec!["note".to_string()], Instant::now())
            .await
            .unwrap();
        drop(sink);
        let mut assembler = ResponseAssembler::default();
        while let Some(frame) = receiver.recv().await {
            assembler.push(&frame).unwrap();
        }
        assembler.finish().unwrap()
    }

    #[tokio::test]
    async fn rows_round_trip_through_arrow_frames() {
        let (path, shop) = orders_app(&[(1, "West"), (2, "East"), (3, "West")]).await;
        let schemas = [shop];
        let budget = ExecutionBudget {
            rows: 2,
            memory_bytes: None,
            spill: None,
            timeout: Duration::from_secs(10),
        };
        let prepared = prepare_cross_app_query(
            CrossAppSources {
                lance: &schemas,
                external: &[],
            },
            "SELECT id, region FROM shop.orders ORDER BY id",
            &Value::Null,
            &budget,
        )
        .await
        .unwrap();
        let response = frames_of(prepared).await;
        assert_eq!(response.trailer.row_count, 2);
        assert!(response.trailer.truncated);
        assert_eq!(response.trailer.tables, vec!["shop.orders"]);
        assert_eq!(
            response.trailer.versions.keys().collect::<Vec<_>>(),
            vec!["shop.orders"]
        );
        assert_eq!(response.trailer.warnings, vec!["note"]);
        let result = response.to_sql_result().unwrap();
        assert_eq!(
            result.rows,
            vec![
                json!({ "id": 1, "region": "West" }),
                json!({ "id": 2, "region": "East" }),
            ]
        );
        assert_eq!(
            result
                .columns
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            vec!["id", "region"]
        );

        let empty = prepare_cross_app_query(
            CrossAppSources {
                lance: &schemas,
                external: &[],
            },
            "SELECT id FROM shop.orders WHERE id > 99",
            &Value::Null,
            &budget,
        )
        .await
        .unwrap();
        let response = frames_of(empty).await;
        assert_eq!(response.trailer.row_count, 0);
        assert!(!response.trailer.truncated);
        let result = response.to_sql_result().unwrap();
        assert!(result.rows.is_empty());
        assert_eq!(
            result.columns.len(),
            1,
            "an empty result still names its columns"
        );
        std::fs::remove_dir_all(path).ok();
    }
}
