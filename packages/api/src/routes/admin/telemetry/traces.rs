//! Distributed traces: a paginated trace list and the span waterfall behind a
//! single trace.
//!
//! Both endpoints derive the trace root and the span count from grouped
//! queries — a trace list page never issues a query per trace.

use crate::entity::telemetry_span;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbBackend, EntityTrait, FromQueryResult, QueryFilter,
    QueryOrder, QuerySelect, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

const SPAN_STATUSES: [&str; 2] = ["ok", "error"];
const ERROR_STATUS: &str = "error";
const OK_STATUS: &str = "ok";
const UNKNOWN: &str = "unknown";
const DEFAULT_TRACE_HOURS: i64 = 24;
const MAX_TRACE_HOURS: i64 = 24 * 90;
/// Upper bound on the spans returned for a single trace waterfall.
const MAX_TRACE_SPANS: u64 = 2_000;
/// Upper bound on the root candidates fetched for one page of traces.
const ROOT_CANDIDATE_CAP: u64 = 2_000;

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListTelemetryTracesQuery {
    /// Lookback window in hours over the span start time. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Only traces containing a span with this exact name.
    #[serde(default)]
    pub name: Option<String>,
    /// Only traces containing a span from this source.
    #[serde(default)]
    pub source: Option<String>,
    /// Filter by trace status: "ok" or "error". A trace is "error" as soon as
    /// one of its spans failed.
    #[serde(default)]
    pub status: Option<String>,
    /// Only traces whose longest span ran at least this many milliseconds.
    #[serde(default)]
    pub min_duration_ms: Option<i64>,
    #[serde(default)]
    pub page: Option<u64>,
    /// Page size, capped at 100. Default 25.
    #[serde(default)]
    pub page_size: Option<u64>,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryTraceRow {
    pub trace_id: String,
    /// Name of the root span, or of the earliest span when no root was stored.
    pub root_name: String,
    pub source: String,
    /// ISO-8601 timestamp of the earliest span in the trace.
    pub started_at: String,
    /// Wall-clock length of the trace, taken from its longest span.
    pub duration_ms: i64,
    pub span_count: i64,
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetryTracesResponse {
    pub traces: Vec<TelemetryTraceRow>,
    pub total: u64,
    pub page: u64,
    pub page_size: u64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryTraceSpan {
    pub id: String,
    pub span_id: String,
    pub parent_span_id: Option<String>,
    pub name: String,
    pub kind: String,
    pub source: String,
    /// ISO-8601 timestamp at which the span started.
    pub started_at: String,
    pub duration_ms: i64,
    pub status: String,
    pub attributes: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryTraceDetailResponse {
    pub trace_id: String,
    /// Spans ordered by start time, ready to render as a waterfall.
    pub spans: Vec<TelemetryTraceSpan>,
    pub root_name: String,
    pub total_duration_ms: i64,
    pub span_count: i64,
}

#[derive(Debug, FromQueryResult)]
struct TraceAggregateRow {
    trace_id: String,
    started_at: DateTime<FixedOffset>,
    span_count: i64,
    duration_ms: i64,
    error_flag: i64,
}

#[derive(Debug, FromQueryResult)]
struct ScalarCount {
    cnt: i64,
}

#[derive(Clone, Debug, FromQueryResult)]
struct RootSpanRow {
    trace_id: String,
    span_id: String,
    parent_span_id: Option<String>,
    name: String,
    source: String,
    started_at: DateTime<FixedOffset>,
}

struct TraceFilters {
    cutoff: DateTime<FixedOffset>,
    name: Option<String>,
    source: Option<String>,
    status: Option<String>,
    min_duration_ms: Option<i64>,
}

fn validate_status(status: &str) -> Result<(), ApiError> {
    if SPAN_STATUSES.contains(&status) {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "Unknown span status '{}', expected one of {}",
        status,
        SPAN_STATUSES.join(", ")
    )))
}

fn iso(ts: DateTime<FixedOffset>) -> String {
    ts.to_rfc3339()
}

/// Ranks candidates for the trace root: a span without a parent wins, otherwise
/// the earliest span does, with the span id as a stable tiebreak.
fn root_rank(
    parent_span_id: Option<&str>,
    started_at: DateTime<FixedOffset>,
    span_id: &str,
) -> (u8, DateTime<FixedOffset>, String) {
    (
        u8::from(parent_span_id.is_some()),
        started_at,
        span_id.to_string(),
    )
}

fn index_roots(rows: Vec<RootSpanRow>) -> HashMap<String, RootSpanRow> {
    let mut roots: HashMap<String, RootSpanRow> = HashMap::new();
    for row in rows {
        let rank = root_rank(row.parent_span_id.as_deref(), row.started_at, &row.span_id);
        let keep = roots.get(&row.trace_id).is_none_or(|current| {
            rank < root_rank(
                current.parent_span_id.as_deref(),
                current.started_at,
                &current.span_id,
            )
        });
        if keep {
            roots.insert(row.trace_id.clone(), row);
        }
    }
    roots
}

fn trace_status(error_flag: i64) -> &'static str {
    if error_flag > 0 {
        ERROR_STATUS
    } else {
        OK_STATUS
    }
}

/// Builds the page and total queries over one grouped scan of the window.
/// Filters that describe the whole trace live in `HAVING` so the span count
/// still covers every span of a matched trace.
fn trace_queries(
    backend: DbBackend,
    filters: &TraceFilters,
    limit: u64,
    offset: u64,
) -> (String, String, Vec<sea_orm::Value>) {
    let pg = matches!(backend, DbBackend::Postgres);
    let table = if pg {
        r#""TelemetrySpan""#
    } else {
        "telemetry_span"
    };
    let trace_id = if pg { r#""traceId""# } else { "trace_id" };
    let started_at = if pg { r#""startedAt""# } else { "started_at" };
    let duration_ms = if pg { r#""durationMs""# } else { "duration_ms" };
    let name = if pg { r#""name""# } else { "name" };
    let source = if pg { r#""source""# } else { "source" };
    let status = if pg { r#""status""# } else { "status" };

    let mut values: Vec<sea_orm::Value> = vec![filters.cutoff.into()];
    let mut having: Vec<String> = Vec::new();

    if let Some(value) = filters.name.as_ref().filter(|v| !v.is_empty()) {
        values.push(value.clone().into());
        having.push(format!(
            "MAX(CASE WHEN {name} = ${} THEN 1 ELSE 0 END) = 1",
            values.len()
        ));
    }

    if let Some(value) = filters.source.as_ref().filter(|v| !v.is_empty()) {
        values.push(value.clone().into());
        having.push(format!(
            "MAX(CASE WHEN {source} = ${} THEN 1 ELSE 0 END) = 1",
            values.len()
        ));
    }

    if let Some(value) = filters.status.as_ref().filter(|v| !v.is_empty()) {
        let expect = i32::from(value == ERROR_STATUS);
        having.push(format!(
            "MAX(CASE WHEN {status} = '{ERROR_STATUS}' THEN 1 ELSE 0 END) = {expect}"
        ));
    }

    if let Some(value) = filters.min_duration_ms.filter(|v| *v > 0) {
        values.push(value.into());
        having.push(format!("MAX({duration_ms}) >= ${}", values.len()));
    }

    let having = if having.is_empty() {
        String::new()
    } else {
        format!(" HAVING {}", having.join(" AND "))
    };

    let core = format!("FROM {table} WHERE {started_at} >= $1 GROUP BY {trace_id}{having}");

    let page_sql = format!(
        r#"SELECT {trace_id} AS trace_id,
                  MIN({started_at}) AS started_at,
                  CAST(COUNT(*) AS BIGINT) AS span_count,
                  CAST(MAX({duration_ms}) AS BIGINT) AS duration_ms,
                  CAST(MAX(CASE WHEN {status} = '{ERROR_STATUS}' THEN 1 ELSE 0 END) AS BIGINT) AS error_flag
           {core}
           ORDER BY MIN({started_at}) DESC
           LIMIT {limit} OFFSET {offset}"#
    );

    let count_sql =
        format!("SELECT CAST(COUNT(*) AS BIGINT) AS cnt FROM (SELECT {trace_id} {core}) t");

    (page_sql, count_sql, values)
}

/// One grouped query for the whole page: every candidate root, i.e. the
/// parentless spans plus the earliest span of each trace.
async fn trace_roots<C: ConnectionTrait>(
    db: &C,
    rows: &[TraceAggregateRow],
) -> Result<HashMap<String, RootSpanRow>, ApiError> {
    if rows.is_empty() {
        return Ok(HashMap::new());
    }

    let mut condition = Condition::any();
    for row in rows {
        condition = condition.add(
            Condition::all()
                .add(telemetry_span::Column::TraceId.eq(row.trace_id.clone()))
                .add(
                    Condition::any()
                        .add(telemetry_span::Column::ParentSpanId.is_null())
                        .add(telemetry_span::Column::StartedAt.eq(row.started_at)),
                ),
        );
    }

    let candidates = telemetry_span::Entity::find()
        .select_only()
        .column_as(telemetry_span::Column::TraceId, "trace_id")
        .column_as(telemetry_span::Column::SpanId, "span_id")
        .column_as(telemetry_span::Column::ParentSpanId, "parent_span_id")
        .column_as(telemetry_span::Column::Name, "name")
        .column_as(telemetry_span::Column::Source, "source")
        .column_as(telemetry_span::Column::StartedAt, "started_at")
        .filter(condition)
        .limit(ROOT_CANDIDATE_CAP)
        .into_model::<RootSpanRow>()
        .all(db)
        .await?;

    if candidates.len() as u64 == ROOT_CANDIDATE_CAP {
        tracing::warn!(
            cap = ROOT_CANDIDATE_CAP,
            "Trace root lookup hit the candidate cap; some roots may fall back to the earliest span"
        );
    }

    Ok(index_roots(candidates))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/traces",
    tag = "admin",
    params(ListTelemetryTracesQuery),
    responses(
        (status = 200, description = "Paginated list of traces, most recent first", body = ListTelemetryTracesResponse),
        (status = 400, description = "Unknown status filter"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List recorded traces with their root operation, duration and span count, filtered by operation, source, status or minimum duration. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/traces", skip_all)]
pub async fn list_telemetry_traces(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListTelemetryTracesQuery>,
) -> Result<Json<ListTelemetryTracesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    if let Some(status) = &q.status
        && !status.is_empty()
    {
        validate_status(status)?;
    }

    let hours = q
        .hours
        .unwrap_or(DEFAULT_TRACE_HOURS)
        .clamp(1, MAX_TRACE_HOURS);
    let page = q.page.unwrap_or(0);
    let page_size = q.page_size.unwrap_or(25).clamp(1, 100);
    let filters = TraceFilters {
        cutoff: Utc::now().fixed_offset() - Duration::hours(hours),
        name: q.name.clone(),
        source: q.source.clone(),
        status: q.status.clone(),
        min_duration_ms: q.min_duration_ms,
    };

    let backend = state.db.get_database_backend();
    let (page_sql, count_sql, values) =
        trace_queries(backend, &filters, page_size, page.saturating_mul(page_size));

    let rows = TraceAggregateRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        page_sql,
        values.clone(),
    ))
    .all(&state.db)
    .await?;

    let total =
        ScalarCount::find_by_statement(Statement::from_sql_and_values(backend, count_sql, values))
            .one(&state.db)
            .await?
            .map(|row| row.cnt.max(0) as u64)
            .unwrap_or(0);

    let roots = trace_roots(&state.db, &rows).await?;

    let traces = rows
        .into_iter()
        .map(|row| {
            let root = roots.get(&row.trace_id);
            TelemetryTraceRow {
                root_name: root
                    .map(|r| r.name.clone())
                    .unwrap_or_else(|| UNKNOWN.to_string()),
                source: root
                    .map(|r| r.source.clone())
                    .unwrap_or_else(|| UNKNOWN.to_string()),
                started_at: iso(row.started_at),
                duration_ms: row.duration_ms,
                span_count: row.span_count,
                status: trace_status(row.error_flag).to_string(),
                trace_id: row.trace_id,
            }
        })
        .collect();

    Ok(Json(ListTelemetryTracesResponse {
        traces,
        total,
        page,
        page_size,
    }))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/traces/{trace_id}",
    tag = "admin",
    params(("trace_id" = String, Path, description = "Trace identifier")),
    responses(
        (status = 200, description = "All spans of the trace ordered by start time", body = TelemetryTraceDetailResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Trace not found")
    ),
    description = "Inspect a single trace as a span waterfall, including each span's parent, duration, status and attributes. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/traces/{trace_id}", skip(state, user))]
pub async fn get_telemetry_trace(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(trace_id): Path<String>,
) -> Result<Json<TelemetryTraceDetailResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let models = telemetry_span::Entity::find()
        .filter(telemetry_span::Column::TraceId.eq(&trace_id))
        .order_by_asc(telemetry_span::Column::StartedAt)
        .limit(MAX_TRACE_SPANS)
        .all(&state.db)
        .await?;

    if models.is_empty() {
        return Err(ApiError::NOT_FOUND);
    }

    if models.len() as u64 == MAX_TRACE_SPANS {
        tracing::warn!(
            trace_id = %trace_id,
            cap = MAX_TRACE_SPANS,
            "Trace waterfall truncated at the span cap"
        );
    }

    let root = models
        .iter()
        .min_by_key(|span| {
            root_rank(
                span.parent_span_id.as_deref(),
                span.started_at,
                &span.span_id,
            )
        })
        .expect("models is not empty");
    let root_name = root.name.clone();
    let total_duration_ms = models
        .iter()
        .map(|span| span.duration_ms as i64)
        .max()
        .unwrap_or(0);

    let span_count = models.len() as i64;
    let spans = models
        .into_iter()
        .map(|span| TelemetryTraceSpan {
            id: span.id,
            span_id: span.span_id,
            parent_span_id: span.parent_span_id,
            name: span.name,
            kind: span.kind,
            source: span.source,
            started_at: iso(span.started_at),
            duration_ms: span.duration_ms as i64,
            status: span.status,
            attributes: span.attributes,
        })
        .collect();

    Ok(Json(TelemetryTraceDetailResponse {
        trace_id,
        spans,
        root_name,
        total_duration_ms,
        span_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn ts(minute: u32) -> DateTime<FixedOffset> {
        NaiveDate::from_ymd_opt(2026, 7, 26)
            .unwrap()
            .and_hms_opt(10, minute, 0)
            .unwrap()
            .and_utc()
            .fixed_offset()
    }

    fn candidate(
        trace: &str,
        span: &str,
        parent: Option<&str>,
        minute: u32,
        name: &str,
    ) -> RootSpanRow {
        RootSpanRow {
            trace_id: trace.to_string(),
            span_id: span.to_string(),
            parent_span_id: parent.map(|p| p.to_string()),
            name: name.to_string(),
            source: "backend".to_string(),
            started_at: ts(minute),
        }
    }

    fn filters() -> TraceFilters {
        TraceFilters {
            cutoff: ts(0),
            name: None,
            source: None,
            status: None,
            min_duration_ms: None,
        }
    }

    #[test]
    fn the_parentless_span_wins_even_when_it_started_late() {
        let roots = index_roots(vec![
            candidate("t1", "b", Some("a"), 1, "child"),
            candidate("t1", "a", None, 5, "root"),
            candidate("t1", "c", Some("a"), 2, "other-child"),
        ]);

        assert_eq!(roots.get("t1").map(|r| r.name.as_str()), Some("root"));
    }

    #[test]
    fn the_earliest_span_is_the_root_when_parent_span_id_is_absent() {
        let roots = index_roots(vec![
            candidate("t1", "c", Some("missing"), 7, "late"),
            candidate("t1", "b", Some("missing"), 3, "earliest"),
            candidate("t1", "d", Some("missing"), 9, "latest"),
        ]);

        assert_eq!(roots.get("t1").map(|r| r.name.as_str()), Some("earliest"));
    }

    #[test]
    fn ties_on_start_time_fall_back_to_the_span_id() {
        let roots = index_roots(vec![
            candidate("t1", "z", Some("missing"), 3, "z-span"),
            candidate("t1", "a", Some("missing"), 3, "a-span"),
        ]);

        assert_eq!(roots.get("t1").map(|r| r.name.as_str()), Some("a-span"));
    }

    #[test]
    fn roots_are_indexed_per_trace() {
        let roots = index_roots(vec![
            candidate("t1", "a", None, 1, "root-1"),
            candidate("t2", "b", Some("x"), 4, "fallback-2"),
            candidate("t2", "c", None, 6, "root-2"),
        ]);

        assert_eq!(roots.len(), 2);
        assert_eq!(roots.get("t1").map(|r| r.name.as_str()), Some("root-1"));
        assert_eq!(roots.get("t2").map(|r| r.name.as_str()), Some("root-2"));
    }

    #[test]
    fn trace_status_is_error_as_soon_as_one_span_failed() {
        assert_eq!(trace_status(0), "ok");
        assert_eq!(trace_status(1), "error");
    }

    #[test]
    fn only_known_span_statuses_are_accepted() {
        assert!(validate_status("ok").is_ok());
        assert!(validate_status("error").is_ok());
        assert!(validate_status("failed").is_err());
    }

    #[test]
    fn the_page_and_total_queries_share_one_grouped_scan() {
        let (page_sql, count_sql, values) = trace_queries(DbBackend::Postgres, &filters(), 25, 50);

        assert_eq!(values.len(), 1);
        assert!(page_sql.contains(r#"GROUP BY "traceId""#), "{page_sql}");
        assert!(page_sql.contains("LIMIT 25 OFFSET 50"), "{page_sql}");
        assert!(
            page_sql.contains(r#"CAST(COUNT(*) AS BIGINT) AS span_count"#),
            "{page_sql}"
        );
        assert!(!page_sql.contains("HAVING"), "{page_sql}");
        assert!(
            count_sql.contains(r#"FROM (SELECT "traceId" FROM "TelemetrySpan""#),
            "{count_sql}"
        );
    }

    #[test]
    fn trace_filters_are_applied_as_having_clauses_over_the_whole_trace() {
        let filters = TraceFilters {
            name: Some("http.request".to_string()),
            source: Some("backend".to_string()),
            status: Some("error".to_string()),
            min_duration_ms: Some(250),
            ..filters()
        };
        let (page_sql, count_sql, values) = trace_queries(DbBackend::Postgres, &filters, 25, 0);

        assert_eq!(values.len(), 4);
        assert!(
            page_sql.contains(r#"MAX(CASE WHEN "name" = $2 THEN 1 ELSE 0 END) = 1"#),
            "{page_sql}"
        );
        assert!(
            page_sql.contains(r#"MAX(CASE WHEN "source" = $3 THEN 1 ELSE 0 END) = 1"#),
            "{page_sql}"
        );
        assert!(
            page_sql.contains(r#"MAX(CASE WHEN "status" = 'error' THEN 1 ELSE 0 END) = 1"#),
            "{page_sql}"
        );
        assert!(
            page_sql.contains(r#"MAX("durationMs") >= $4"#),
            "{page_sql}"
        );
        assert!(count_sql.contains("HAVING"), "{count_sql}");
    }

    #[test]
    fn a_status_filter_of_ok_excludes_traces_with_failed_spans() {
        let filters = TraceFilters {
            status: Some("ok".to_string()),
            ..filters()
        };
        let (page_sql, _, _) = trace_queries(DbBackend::Postgres, &filters, 25, 0);

        assert!(
            page_sql.contains(r#"MAX(CASE WHEN "status" = 'error' THEN 1 ELSE 0 END) = 0"#),
            "{page_sql}"
        );
    }

    #[test]
    fn non_postgres_backends_use_snake_case_identifiers() {
        let (page_sql, _, _) = trace_queries(DbBackend::Sqlite, &filters(), 10, 0);

        assert!(page_sql.contains("FROM telemetry_span"), "{page_sql}");
        assert!(page_sql.contains("GROUP BY trace_id"), "{page_sql}");
        assert!(page_sql.contains("MAX(duration_ms)"), "{page_sql}");
    }
}
