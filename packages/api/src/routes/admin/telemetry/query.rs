//! Structured ad-hoc analytics over the anonymous telemetry tables.
//!
//! SECURITY INVARIANT — nothing a caller sends is ever interpolated into SQL.
//! Every identifier that reaches the statement (table, column, aggregate
//! function, bucket unit, LIMIT) is a `&'static str` constant looked up in the
//! static allowlist below; a dataset, field, metric, operator or interval that
//! is not in the allowlist is a 400 and never a query. Every caller-supplied
//! *value* — including the `LIKE` pattern behind `contains` — is pushed onto the
//! bound parameter list and referenced by an ordinal placeholder, so no value is
//! ever concatenated into SQL text. There is deliberately no endpoint that
//! accepts raw SQL.

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use sea_orm::{FromQueryResult, Statement, Value};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const DEFAULT_QUERY_HOURS: i64 = 24;
/// 90 days, matching the longest retention window the sweeper keeps.
const MAX_QUERY_HOURS: i64 = 2160;
/// Distinct breakdown values a single query may return.
const MAX_BREAKDOWN_GROUPS: usize = 50;
/// Hard row cap, enforced as a forced `LIMIT` on every statement. Rows are read
/// newest first, so hitting the cap drops the oldest buckets and never the ones
/// an operator is actually looking at.
const MAX_QUERY_ROWS: usize = 5000;
const MAX_FILTERS: usize = 12;
const MAX_IN_VALUES: usize = 100;
const MAX_FILTER_TEXT_LEN: usize = 512;
/// Bucket label for rows whose breakdown column is NULL.
const NULL_GROUP: &str = "unknown";
const TS_COLUMN: &str = "ts";
const GROUP_COLUMN: &str = "group_key";
const VALUE_COLUMN: &str = "metric_value";
const NULL_TS: &str = "CAST(NULL AS TIMESTAMP)";
const NULL_GROUP_EXPR: &str = "CAST(NULL AS TEXT)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FieldKind {
    Text,
    Numeric,
    Timestamp,
    Bool,
}

/// One allowlisted field: the name a caller may use and the constant physical
/// column it resolves to. Nothing else ever becomes an identifier.
#[derive(Clone, Copy, Debug)]
struct FieldDef {
    name: &'static str,
    column: &'static str,
    kind: FieldKind,
}

#[derive(Clone, Copy, Debug)]
struct DatasetDef {
    name: &'static str,
    table: &'static str,
    time_column: &'static str,
    fields: &'static [FieldDef],
}

impl DatasetDef {
    fn field(&self, name: &str) -> Option<FieldDef> {
        self.fields.iter().copied().find(|f| f.name == name)
    }

    fn numeric_fields(&self) -> Vec<&'static str> {
        self.fields
            .iter()
            .filter(|f| f.kind == FieldKind::Numeric)
            .map(|f| f.name)
            .collect()
    }
}

const fn text(name: &'static str, column: &'static str) -> FieldDef {
    FieldDef {
        name,
        column,
        kind: FieldKind::Text,
    }
}

const fn numeric(name: &'static str, column: &'static str) -> FieldDef {
    FieldDef {
        name,
        column,
        kind: FieldKind::Numeric,
    }
}

const fn timestamp(name: &'static str, column: &'static str) -> FieldDef {
    FieldDef {
        name,
        column,
        kind: FieldKind::Timestamp,
    }
}

const fn boolean(name: &'static str, column: &'static str) -> FieldDef {
    FieldDef {
        name,
        column,
        kind: FieldKind::Bool,
    }
}

const EVENT_FIELDS: &[FieldDef] = &[
    text("name", "name"),
    text("source", "source"),
    text("anon_id", "anonId"),
    text("app_version", "appVersion"),
    text("platform", "platform"),
    text("country", "country"),
    timestamp("client_ts", "clientTs"),
    timestamp("created_at", "createdAt"),
];

const ERROR_FIELDS: &[FieldDef] = &[
    text("issue_id", "issueId"),
    text("anon_id", "anonId"),
    text("source", "source"),
    text("platform", "platform"),
    text("app_version", "appVersion"),
    text("release", "release"),
    text("kind", "kind"),
    text("title", "title"),
    text("culprit", "culprit"),
    text("level", "level"),
    text("country", "country"),
    timestamp("client_ts", "clientTs"),
    timestamp("created_at", "createdAt"),
];

const SPAN_FIELDS: &[FieldDef] = &[
    text("trace_id", "traceId"),
    text("span_id", "spanId"),
    text("parent_span_id", "parentSpanId"),
    text("name", "name"),
    text("kind", "kind"),
    text("source", "source"),
    text("anon_id", "anonId"),
    text("release", "release"),
    text("platform", "platform"),
    text("status", "status"),
    numeric("duration_ms", "durationMs"),
    timestamp("started_at", "startedAt"),
    timestamp("created_at", "createdAt"),
];

const PERF_FIELDS: &[FieldDef] = &[
    text("anon_id", "anonId"),
    text("source", "source"),
    text("platform", "platform"),
    text("release", "release"),
    text("metric", "metric"),
    text("path", "path"),
    text("country", "country"),
    numeric("value", "value"),
    timestamp("client_ts", "clientTs"),
    timestamp("created_at", "createdAt"),
];

const SESSION_FIELDS: &[FieldDef] = &[
    text("anon_id", "anonId"),
    text("source", "source"),
    text("release", "release"),
    text("platform", "platform"),
    text("status", "status"),
    numeric("duration_ms", "durationMs"),
    timestamp("started_at", "startedAt"),
    timestamp("created_at", "createdAt"),
    timestamp("updated_at", "updatedAt"),
];

const LLM_FIELDS: &[FieldDef] = &[
    text("anon_id", "anonId"),
    text("source", "source"),
    text("release", "release"),
    text("provider", "provider"),
    text("model", "model"),
    text("operation", "operation"),
    text("status", "status"),
    text("error_kind", "errorKind"),
    numeric("duration_ms", "durationMs"),
    numeric("prompt_tokens", "promptTokens"),
    numeric("completion_tokens", "completionTokens"),
    numeric("total_tokens", "totalTokens"),
    numeric("tool_calls", "toolCalls"),
    boolean("streamed", "streamed"),
    timestamp("created_at", "createdAt"),
];

const DATASETS: &[DatasetDef] = &[
    DatasetDef {
        name: "events",
        table: "TelemetryEvent",
        time_column: "createdAt",
        fields: EVENT_FIELDS,
    },
    DatasetDef {
        name: "errors",
        table: "TelemetryErrorEvent",
        time_column: "createdAt",
        fields: ERROR_FIELDS,
    },
    DatasetDef {
        name: "spans",
        table: "TelemetrySpan",
        time_column: "startedAt",
        fields: SPAN_FIELDS,
    },
    DatasetDef {
        name: "performance",
        table: "TelemetryPerfMetric",
        time_column: "createdAt",
        fields: PERF_FIELDS,
    },
    DatasetDef {
        name: "sessions",
        table: "TelemetrySession",
        time_column: "startedAt",
        fields: SESSION_FIELDS,
    },
    DatasetDef {
        name: "llm",
        table: "TelemetryLlmCall",
        time_column: "createdAt",
        fields: LLM_FIELDS,
    },
];

fn dataset(name: &str) -> Result<&'static DatasetDef, ApiError> {
    DATASETS.iter().find(|d| d.name == name).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Unknown dataset '{}', expected one of {}",
            name,
            DATASETS
                .iter()
                .map(|d| d.name)
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Interval {
    Minute,
    Hour,
    Day,
    None,
}

impl Interval {
    fn parse(value: &str) -> Result<Self, ApiError> {
        match value {
            "minute" => Ok(Self::Minute),
            "hour" => Ok(Self::Hour),
            "day" => Ok(Self::Day),
            "none" => Ok(Self::None),
            other => Err(ApiError::bad_request(format!(
                "Unknown interval '{}', expected minute, hour, day or none",
                other
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Minute => "minute",
            Self::Hour => "hour",
            Self::Day => "day",
            Self::None => "none",
        }
    }
}

/// The aggregate, already reduced to constants. `func`, `fraction` and every
/// column are compile-time strings, never caller input.
#[derive(Clone, Copy, Debug)]
enum MetricPlan {
    Count,
    CountDistinct(&'static str),
    Aggregate {
        func: &'static str,
        column: &'static str,
    },
    Percentile {
        fraction: &'static str,
        column: &'static str,
    },
}

#[derive(Clone, Debug)]
enum FilterPlan {
    Compare {
        column: &'static str,
        sql_op: &'static str,
        value: Value,
    },
    /// `neq` is set complement, not SQL `<>`: a NULL column compares to NULL and
    /// would silently drop every row that has no value at all.
    NotEquals { column: &'static str, value: Value },
    Contains {
        column: &'static str,
        pattern: Value,
    },
    In {
        column: &'static str,
        values: Vec<Value>,
    },
}

#[derive(Clone, Debug)]
struct QueryPlan {
    dataset: &'static DatasetDef,
    metric: MetricPlan,
    filters: Vec<FilterPlan>,
    breakdown: Option<FieldDef>,
    interval: Interval,
    hours: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct TelemetryQueryMetric {
    /// Aggregate: "count", "count_distinct", "sum", "avg", "min", "max",
    /// "p50", "p75", "p95" or "p99".
    #[serde(rename = "type")]
    pub metric_type: String,
    /// Field the aggregate runs over. Required for everything but "count".
    #[serde(default)]
    pub field: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct TelemetryQueryFilter {
    /// Allowlisted field name of the dataset.
    pub field: String,
    /// Comparison: "eq", "neq", "contains", "gt", "lt", "gte", "lte" or "in".
    pub op: String,
    /// Bound parameter value. An array for "in", a scalar otherwise.
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct TelemetryQueryDefinition {
    /// Dataset: "events", "errors", "spans", "performance", "sessions" or "llm".
    pub dataset: String,
    pub metric: TelemetryQueryMetric,
    #[serde(default)]
    pub filters: Option<Vec<TelemetryQueryFilter>>,
    /// Text field to group by. At most 50 groups are returned.
    #[serde(default)]
    pub breakdown: Option<String>,
    /// Time bucket: "minute", "hour", "day" or "none". Defaults to "none".
    #[serde(default)]
    pub interval: Option<String>,
    /// Lookback window in hours, clamped to 1..=2160.
    #[serde(default)]
    pub hours: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryQueryResponse {
    /// Column headers in the same order as every entry of `rows`.
    pub columns: Vec<String>,
    pub rows: Vec<Vec<serde_json::Value>>,
    pub interval: String,
    /// Rows carried by this response.
    pub total: u64,
    /// True when a server-side cap dropped rows: buckets past the 5000 row limit
    /// or breakdown values past the top 50. The most recent buckets are always
    /// the ones that survive, so a truncated chart is missing its oldest end.
    pub truncated: bool,
}

#[derive(Debug, FromQueryResult)]
struct QueryRow {
    ts: Option<NaiveDateTime>,
    group_key: Option<String>,
    metric_value: Option<f64>,
}

/// Collects bound parameters and hands back the placeholder that refers to them.
/// Every caller-supplied value goes through here.
#[derive(Debug, Default)]
struct Binder {
    values: Vec<Value>,
}

impl Binder {
    fn bind(&mut self, value: Value) -> String {
        self.values.push(value);
        format!("${}", self.values.len())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    /// Ranks the breakdown values and caps them at `MAX_BREAKDOWN_GROUPS`.
    TopGroups,
    /// The result rows, restricted to the ranked groups when there is a breakdown.
    Rows,
}

#[derive(Debug)]
struct BoundQuery {
    sql: String,
    values: Vec<Value>,
}

fn quoted(ident: &'static str) -> String {
    format!("\"{}\"", ident)
}

/// One row past the cap, so a statement that comes back full proves there was
/// more to read instead of leaving the caller to guess.
const fn fetch_limit(cap: usize) -> usize {
    cap + 1
}

/// Applies a cap to a fetch that read one row past it and reports whether
/// anything was dropped.
fn cap_rows<T>(mut rows: Vec<T>, cap: usize) -> (Vec<T>, bool) {
    let truncated = rows.len() > cap;
    rows.truncate(cap);
    (rows, truncated)
}

/// Caps the result rows and restores ascending order. The statement reads newest
/// first so truncation sheds the oldest buckets, but a chart still needs the
/// rows left to right.
fn finalize_rows(rows: Vec<QueryRow>) -> (Vec<QueryRow>, bool) {
    let (mut rows, truncated) = cap_rows(rows, MAX_QUERY_ROWS);
    rows.reverse();
    (rows, truncated)
}

fn metric_sql(metric: &MetricPlan) -> String {
    match metric {
        MetricPlan::Count => "CAST(COUNT(*) AS DOUBLE PRECISION)".to_string(),
        MetricPlan::CountDistinct(column) => format!(
            "CAST(COUNT(DISTINCT {}) AS DOUBLE PRECISION)",
            quoted(column)
        ),
        MetricPlan::Aggregate { func, column } => {
            format!("CAST({}({}) AS DOUBLE PRECISION)", func, quoted(column))
        }
        MetricPlan::Percentile { fraction, column } => format!(
            "percentile_cont({}::float8) WITHIN GROUP (ORDER BY {}::float8)",
            fraction,
            quoted(column)
        ),
    }
}

fn filter_sql(filter: &FilterPlan, binder: &mut Binder) -> String {
    match filter {
        FilterPlan::Compare {
            column,
            sql_op,
            value,
        } => format!(
            "{} {} {}",
            quoted(column),
            sql_op,
            binder.bind(value.clone())
        ),
        FilterPlan::NotEquals { column, value } => format!(
            "({0} <> {1} OR {0} IS NULL)",
            quoted(column),
            binder.bind(value.clone())
        ),
        FilterPlan::Contains { column, pattern } => format!(
            "LOWER({}) LIKE {}",
            quoted(column),
            binder.bind(pattern.clone())
        ),
        FilterPlan::In { column, values } => {
            let placeholders = values
                .iter()
                .map(|value| binder.bind(value.clone()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("{} IN ({})", quoted(column), placeholders)
        }
    }
}

/// Compiles the plan into one statement. `groups` restricts the rows to the
/// breakdown values the `TopGroups` stage ranked; its entries are bound too even
/// though they come from the database and not from the caller. Every stage reads
/// one row past its cap and orders so the cap keeps the rows that matter.
fn build_query(
    plan: &QueryPlan,
    cutoff: NaiveDateTime,
    stage: Stage,
    groups: Option<&[String]>,
) -> BoundQuery {
    let mut binder = Binder::default();
    let interval = match stage {
        Stage::TopGroups => Interval::None,
        Stage::Rows => plan.interval,
    };

    let group_expr = match plan.breakdown {
        Some(field) => format!(
            "COALESCE({}, {})",
            quoted(field.column),
            binder.bind(NULL_GROUP.to_string().into())
        ),
        None => NULL_GROUP_EXPR.to_string(),
    };

    let ts_expr = match interval {
        Interval::None => NULL_TS.to_string(),
        bucket => format!(
            "date_trunc('{}', {})",
            bucket.as_str(),
            quoted(plan.dataset.time_column)
        ),
    };

    let mut clauses = vec![format!(
        "{} >= {}",
        quoted(plan.dataset.time_column),
        binder.bind(cutoff.into())
    )];
    for filter in &plan.filters {
        clauses.push(filter_sql(filter, &mut binder));
    }
    if let Some(groups) = groups.filter(|groups| !groups.is_empty())
        && plan.breakdown.is_some()
    {
        let placeholders = groups
            .iter()
            .map(|group| binder.bind(group.clone().into()))
            .collect::<Vec<_>>()
            .join(", ");
        clauses.push(format!("{} IN ({})", group_expr, placeholders));
    }

    let mut dimensions: Vec<&str> = Vec::new();
    if interval != Interval::None {
        dimensions.push(TS_COLUMN);
    }
    if plan.breakdown.is_some() {
        dimensions.push(GROUP_COLUMN);
    }
    let group_by = if dimensions.is_empty() {
        String::new()
    } else {
        format!(" GROUP BY {}", dimensions.join(", "))
    };

    let (order_by, limit) = match stage {
        Stage::TopGroups => (
            format!(" ORDER BY {VALUE_COLUMN} DESC NULLS LAST, {GROUP_COLUMN} ASC"),
            fetch_limit(MAX_BREAKDOWN_GROUPS),
        ),
        Stage::Rows => {
            let order = if dimensions.is_empty() {
                String::new()
            } else {
                format!(
                    " ORDER BY {}",
                    dimensions
                        .iter()
                        .map(|dimension| format!("{dimension} DESC"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            (order, fetch_limit(MAX_QUERY_ROWS))
        }
    };

    let sql = format!(
        "SELECT {ts_expr} AS {TS_COLUMN}, {group_expr} AS {GROUP_COLUMN}, {} AS {VALUE_COLUMN} \
         FROM {} WHERE {}{}{} LIMIT {}",
        metric_sql(&plan.metric),
        quoted(plan.dataset.table),
        clauses.join(" AND "),
        group_by,
        order_by,
        limit
    );

    BoundQuery {
        sql,
        values: binder.values,
    }
}

/// Escapes LIKE wildcards so a caller-supplied term stays a literal substring.
fn like_pattern(term: &str) -> String {
    let escaped = term
        .to_lowercase()
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{}%", escaped)
}

fn parse_timestamp(raw: &str) -> Option<NaiveDateTime> {
    if let Ok(parsed) = DateTime::parse_from_rfc3339(raw) {
        return Some(parsed.naive_utc());
    }
    raw.parse::<NaiveDateTime>().ok()
}

fn scalar_value(field: &FieldDef, raw: &serde_json::Value) -> Result<Value, ApiError> {
    let type_error = || {
        ApiError::bad_request(format!(
            "Filter on '{}' expects a {} value",
            field.name,
            match field.kind {
                FieldKind::Text => "string",
                FieldKind::Numeric => "number",
                FieldKind::Timestamp => "timestamp string",
                FieldKind::Bool => "boolean",
            }
        ))
    };

    match field.kind {
        FieldKind::Text => {
            let value = raw.as_str().ok_or_else(type_error)?;
            if value.len() > MAX_FILTER_TEXT_LEN {
                return Err(ApiError::bad_request(format!(
                    "Filter value for '{}' may be at most {} characters",
                    field.name, MAX_FILTER_TEXT_LEN
                )));
            }
            Ok(value.to_string().into())
        }
        FieldKind::Numeric => Ok(raw.as_f64().ok_or_else(type_error)?.into()),
        FieldKind::Bool => Ok(raw.as_bool().ok_or_else(type_error)?.into()),
        FieldKind::Timestamp => {
            let value = raw.as_str().ok_or_else(type_error)?;
            Ok(parse_timestamp(value).ok_or_else(type_error)?.into())
        }
    }
}

fn plan_filter(
    dataset: &'static DatasetDef,
    filter: &TelemetryQueryFilter,
) -> Result<FilterPlan, ApiError> {
    let field = dataset.field(&filter.field).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Unknown field '{}' for dataset '{}'",
            filter.field, dataset.name
        ))
    })?;

    let sql_op = match filter.op.as_str() {
        "eq" => "=",
        "neq" => {
            return Ok(FilterPlan::NotEquals {
                column: field.column,
                value: scalar_value(&field, &filter.value)?,
            });
        }
        "gt" => ">",
        "lt" => "<",
        "gte" => ">=",
        "lte" => "<=",
        "contains" => {
            if field.kind != FieldKind::Text {
                return Err(ApiError::bad_request(format!(
                    "Operator 'contains' needs a text field, '{}' is not one",
                    field.name
                )));
            }
            let term = filter.value.as_str().ok_or_else(|| {
                ApiError::bad_request(format!(
                    "Operator 'contains' on '{}' expects a string value",
                    field.name
                ))
            })?;
            if term.len() > MAX_FILTER_TEXT_LEN {
                return Err(ApiError::bad_request(format!(
                    "Filter value for '{}' may be at most {} characters",
                    field.name, MAX_FILTER_TEXT_LEN
                )));
            }
            return Ok(FilterPlan::Contains {
                column: field.column,
                pattern: like_pattern(term).into(),
            });
        }
        "in" => {
            let items = filter.value.as_array().ok_or_else(|| {
                ApiError::bad_request(format!(
                    "Operator 'in' on '{}' expects an array of values",
                    field.name
                ))
            })?;
            if items.is_empty() {
                return Err(ApiError::bad_request(format!(
                    "Operator 'in' on '{}' needs at least one value",
                    field.name
                )));
            }
            if items.len() > MAX_IN_VALUES {
                return Err(ApiError::bad_request(format!(
                    "Operator 'in' on '{}' accepts at most {} values",
                    field.name, MAX_IN_VALUES
                )));
            }
            let values = items
                .iter()
                .map(|item| scalar_value(&field, item))
                .collect::<Result<Vec<_>, _>>()?;
            return Ok(FilterPlan::In {
                column: field.column,
                values,
            });
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "Unknown filter operator '{}', expected eq, neq, contains, gt, lt, gte, lte or in",
                other
            )));
        }
    };

    if matches!(filter.op.as_str(), "gt" | "lt" | "gte" | "lte")
        && !matches!(field.kind, FieldKind::Numeric | FieldKind::Timestamp)
    {
        return Err(ApiError::bad_request(format!(
            "Operator '{}' needs a numeric or timestamp field, '{}' is not one",
            filter.op, field.name
        )));
    }

    Ok(FilterPlan::Compare {
        column: field.column,
        sql_op,
        value: scalar_value(&field, &filter.value)?,
    })
}

fn plan_metric(
    dataset: &'static DatasetDef,
    metric: &TelemetryQueryMetric,
) -> Result<MetricPlan, ApiError> {
    let requested = metric.metric_type.as_str();
    if requested == "count" {
        return Ok(MetricPlan::Count);
    }

    let name = metric.field.as_deref().unwrap_or_default();
    let field = dataset.field(name).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Metric '{}' needs a field of dataset '{}', '{}' is not one",
            requested, dataset.name, name
        ))
    })?;

    if requested == "count_distinct" {
        return Ok(MetricPlan::CountDistinct(field.column));
    }

    let numeric_required = || {
        ApiError::bad_request(format!(
            "Metric '{}' needs a numeric field of dataset '{}', expected one of {}",
            requested,
            dataset.name,
            dataset.numeric_fields().join(", ")
        ))
    };

    let plan = match requested {
        "sum" | "avg" | "min" | "max" => {
            if field.kind != FieldKind::Numeric {
                return Err(numeric_required());
            }
            let func = match requested {
                "sum" => "SUM",
                "avg" => "AVG",
                "min" => "MIN",
                _ => "MAX",
            };
            MetricPlan::Aggregate {
                func,
                column: field.column,
            }
        }
        "p50" | "p75" | "p95" | "p99" => {
            if field.kind != FieldKind::Numeric {
                return Err(numeric_required());
            }
            let fraction = match requested {
                "p50" => "0.5",
                "p75" => "0.75",
                "p95" => "0.95",
                _ => "0.99",
            };
            MetricPlan::Percentile {
                fraction,
                column: field.column,
            }
        }
        other => {
            return Err(ApiError::bad_request(format!(
                "Unknown metric type '{}', expected count, count_distinct, sum, avg, min, max, p50, p75, p95 or p99",
                other
            )));
        }
    };

    Ok(plan)
}

/// Turns a request into an executable plan, or a 400. This is the only path
/// from caller input to SQL, and it is shared by the query endpoint and by the
/// saved-query and dashboard write paths.
fn plan_query(request: &TelemetryQueryDefinition) -> Result<QueryPlan, ApiError> {
    let dataset = dataset(&request.dataset)?;
    let metric = plan_metric(dataset, &request.metric)?;

    let requested_filters = request.filters.as_deref().unwrap_or_default();
    if requested_filters.len() > MAX_FILTERS {
        return Err(ApiError::bad_request(format!(
            "A query may carry at most {} filters",
            MAX_FILTERS
        )));
    }
    let filters = requested_filters
        .iter()
        .map(|filter| plan_filter(dataset, filter))
        .collect::<Result<Vec<_>, _>>()?;

    let breakdown = match request.breakdown.as_deref().filter(|v| !v.is_empty()) {
        Some(name) => {
            let field = dataset.field(name).ok_or_else(|| {
                ApiError::bad_request(format!(
                    "Unknown breakdown field '{}' for dataset '{}'",
                    name, dataset.name
                ))
            })?;
            if field.kind != FieldKind::Text {
                return Err(ApiError::bad_request(format!(
                    "Breakdowns need a text field, '{}' is not one",
                    field.name
                )));
            }
            Some(field)
        }
        None => None,
    };

    let interval = match request.interval.as_deref().filter(|v| !v.is_empty()) {
        Some(value) => Interval::parse(value)?,
        None => Interval::None,
    };

    Ok(QueryPlan {
        dataset,
        metric,
        filters,
        breakdown,
        interval,
        hours: request
            .hours
            .unwrap_or(DEFAULT_QUERY_HOURS)
            .clamp(1, MAX_QUERY_HOURS),
    })
}

/// Parses and validates a stored query definition. Saved queries and dashboard
/// tiles run through exactly the same planner as the live endpoint, so nothing
/// can be persisted that the endpoint would refuse.
pub(super) fn validate_query_definition(definition: &serde_json::Value) -> Result<(), ApiError> {
    let request: TelemetryQueryDefinition =
        serde_json::from_value(definition.clone()).map_err(|err| {
            ApiError::bad_request(format!("Invalid telemetry query definition: {}", err))
        })?;
    plan_query(&request)?;
    Ok(())
}

pub(super) fn require_name(field: &str, value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!(
            "'{}' must not be empty",
            field
        )));
    }
    if trimmed.chars().count() > 120 {
        return Err(ApiError::bad_request(format!(
            "'{}' may be at most 120 characters",
            field
        )));
    }
    Ok(trimmed.to_string())
}

fn project(plan: &QueryPlan, rows: Vec<QueryRow>, truncated: bool) -> TelemetryQueryResponse {
    let mut columns: Vec<String> = Vec::new();
    if plan.interval != Interval::None {
        columns.push(TS_COLUMN.to_string());
    }
    if let Some(field) = plan.breakdown {
        columns.push(field.name.to_string());
    }
    columns.push("value".to_string());

    let projected: Vec<Vec<serde_json::Value>> = rows
        .into_iter()
        .map(|row| {
            let mut cells: Vec<serde_json::Value> = Vec::with_capacity(columns.len());
            if plan.interval != Interval::None {
                cells.push(match row.ts {
                    Some(ts) => serde_json::Value::String(
                        DateTime::<Utc>::from_naive_utc_and_offset(ts, Utc).to_rfc3339(),
                    ),
                    None => serde_json::Value::Null,
                });
            }
            if plan.breakdown.is_some() {
                cells.push(serde_json::Value::String(
                    row.group_key.unwrap_or_else(|| NULL_GROUP.to_string()),
                ));
            }
            cells.push(
                row.metric_value
                    .and_then(serde_json::Number::from_f64)
                    .map(serde_json::Value::Number)
                    .unwrap_or(serde_json::Value::Null),
            );
            cells
        })
        .collect();

    TelemetryQueryResponse {
        columns,
        total: projected.len() as u64,
        rows: projected,
        interval: plan.interval.as_str().to_string(),
        truncated,
    }
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/query",
    tag = "admin",
    request_body = TelemetryQueryDefinition,
    responses(
        (status = 200, description = "Result table of the structured query", body = TelemetryQueryResponse),
        (status = 400, description = "Unknown dataset, field, metric, operator or interval"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Run an ad-hoc analytics query over the anonymous telemetry datasets. The query is built from an allowlist of datasets, fields, aggregates and operators — raw SQL is never accepted. Large results keep the most recent buckets and report 'truncated'. Requires Admin permission."
)]
#[tracing::instrument(name = "POST /admin/telemetry/query", skip(state, user, payload))]
pub async fn run_telemetry_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<TelemetryQueryDefinition>,
) -> Result<Json<TelemetryQueryResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let plan = plan_query(&payload)?;
    if matches!(plan.metric, MetricPlan::Percentile { .. })
        && !state.db_dialect.supports_ordered_set_aggregates()
    {
        return Err(ApiError::not_implemented(
            "Percentile metrics need an ordered-set aggregate this database engine does not provide; use count, sum, avg, min or max instead",
        ));
    }
    let cutoff = Utc::now().naive_utc() - Duration::hours(plan.hours);
    let backend = state.db.get_database_backend();

    let mut groups: Vec<String> = Vec::new();
    let mut dropped_groups = false;
    if plan.breakdown.is_some() {
        let built = build_query(&plan, cutoff, Stage::TopGroups, None);
        let ranked = QueryRow::find_by_statement(Statement::from_sql_and_values(
            backend,
            built.sql,
            built.values,
        ))
        .all(&state.db)
        .await?;

        let (ranked, truncated) = cap_rows(ranked, MAX_BREAKDOWN_GROUPS);
        dropped_groups = truncated;

        if plan.interval == Interval::None {
            return Ok(Json(project(&plan, ranked, dropped_groups)));
        }

        groups = ranked.into_iter().filter_map(|row| row.group_key).collect();
        if groups.is_empty() {
            return Ok(Json(project(&plan, Vec::new(), dropped_groups)));
        }
    }

    let built = build_query(&plan, cutoff, Stage::Rows, Some(groups.as_slice()));
    let rows = QueryRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        built.sql,
        built.values,
    ))
    .all(&state.db)
    .await?;

    let (rows, dropped_rows) = finalize_rows(rows);

    Ok(Json(project(&plan, rows, dropped_groups || dropped_rows)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{
        telemetry_error_event, telemetry_event, telemetry_llm_call, telemetry_perf_metric,
        telemetry_session, telemetry_span,
    };
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use chrono::NaiveDate;
    use sea_orm::{EntityName, IdenStatic, Iterable};
    use std::collections::HashSet;

    const INJECTIONS: [&str; 6] = [
        "'; DROP TABLE \"TelemetryEvent\"; --",
        "\" UNION SELECT * FROM \"User\" --",
        "1 OR 1=1",
        "desktop'/**/OR/**/'1'='1",
        "\\'; DELETE FROM \"TelemetrySpan\"; --",
        "%'; TRUNCATE \"TelemetryIssue\"; --",
    ];

    fn cutoff() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 7, 26)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
    }

    fn status(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    fn assert_bad_request<T: std::fmt::Debug>(result: Result<T, ApiError>) {
        match result {
            Ok(value) => panic!("expected a rejection, planned {:?}", value),
            Err(error) => assert_eq!(status(error), StatusCode::BAD_REQUEST),
        }
    }

    fn metric(metric_type: &str, field: Option<&str>) -> TelemetryQueryMetric {
        TelemetryQueryMetric {
            metric_type: metric_type.to_string(),
            field: field.map(|f| f.to_string()),
        }
    }

    fn filter(field: &str, op: &str, value: serde_json::Value) -> TelemetryQueryFilter {
        TelemetryQueryFilter {
            field: field.to_string(),
            op: op.to_string(),
            value,
        }
    }

    fn definition(dataset: &str, metric: TelemetryQueryMetric) -> TelemetryQueryDefinition {
        TelemetryQueryDefinition {
            dataset: dataset.to_string(),
            metric,
            filters: None,
            breakdown: None,
            interval: None,
            hours: None,
        }
    }

    fn count_events() -> TelemetryQueryDefinition {
        definition("events", metric("count", None))
    }

    /// The whole point of the module: no fragment of caller input may survive
    /// into SQL text.
    fn assert_no_input_in_sql(sql: &str, input: &str) {
        assert!(
            !sql.contains(input),
            "caller input leaked into SQL: {sql} <- {input}"
        );
        for token in ["DROP", "DELETE", "TRUNCATE", "UNION", "--", ";"] {
            assert!(
                !sql.contains(token),
                "SQL carries an unexpected token {token}: {sql}"
            );
        }
    }

    fn bound_strings(values: &[Value]) -> Vec<String> {
        values
            .iter()
            .filter_map(|value| match value {
                Value::String(Some(text)) => Some(text.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn unknown_datasets_are_rejected() {
        assert_bad_request(plan_query(&definition("users", metric("count", None))));
        assert_bad_request(plan_query(&definition(
            "events; DROP TABLE \"TelemetryEvent\"",
            metric("count", None),
        )));
        assert_bad_request(plan_query(&definition("", metric("count", None))));

        for known in DATASETS {
            assert!(
                plan_query(&definition(known.name, metric("count", None))).is_ok(),
                "{}",
                known.name
            );
        }
    }

    #[test]
    fn filter_field_names_never_reach_sql() {
        for name in [
            "created_at\"; DROP TABLE \"TelemetryEvent\"; --",
            "created_at\") --",
            "name UNION SELECT",
            "1; DELETE FROM \"TelemetryEvent\"",
            "*",
        ] {
            let mut request = count_events();
            request.filters = Some(vec![filter(name, "eq", serde_json::json!("x"))]);
            assert_bad_request(plan_query(&request));
        }
    }

    #[test]
    fn breakdown_field_names_never_reach_sql() {
        for name in [
            "1; DELETE FROM \"TelemetryEvent\"",
            "platform\"; DROP TABLE \"TelemetryEvent\"; --",
            "(SELECT 1)",
        ] {
            let mut request = count_events();
            request.breakdown = Some(name.to_string());
            assert_bad_request(plan_query(&request));
        }
    }

    #[test]
    fn metric_field_names_never_reach_sql() {
        let mut request = definition(
            "spans",
            metric("p95", Some("durationMs\"; DROP TABLE x --")),
        );
        assert_bad_request(plan_query(&request));

        request = definition("spans", metric("count_distinct", Some("*")));
        assert_bad_request(plan_query(&request));

        request = definition("spans", metric("sum", None));
        assert_bad_request(plan_query(&request));
    }

    #[test]
    fn metric_types_come_from_the_allowlist() {
        for known in [
            "count",
            "count_distinct",
            "sum",
            "avg",
            "min",
            "max",
            "p50",
            "p75",
            "p95",
            "p99",
        ] {
            let field = if known == "count" {
                None
            } else {
                Some("duration_ms")
            };
            assert!(
                plan_query(&definition("spans", metric(known, field))).is_ok(),
                "{known}"
            );
        }

        assert_bad_request(plan_query(&definition(
            "spans",
            metric("count(*) FROM \"User\" --", Some("duration_ms")),
        )));
        assert_bad_request(plan_query(&definition(
            "spans",
            metric("median", Some("duration_ms")),
        )));
    }

    #[test]
    fn non_numeric_fields_are_rejected_for_numeric_metrics() {
        for aggregate in ["sum", "avg", "min", "max", "p50", "p75", "p95", "p99"] {
            assert_bad_request(plan_query(&definition(
                "spans",
                metric(aggregate, Some("name")),
            )));
            assert_bad_request(plan_query(&definition(
                "spans",
                metric(aggregate, Some("started_at")),
            )));
            assert_bad_request(plan_query(&definition(
                "events",
                metric(aggregate, Some("source")),
            )));
        }

        assert!(
            plan_query(&definition("spans", metric("p95", Some("duration_ms")))).is_ok(),
            "duration_ms is numeric"
        );
        assert!(
            plan_query(&definition(
                "events",
                metric("count_distinct", Some("anon_id"))
            ))
            .is_ok(),
            "count_distinct accepts text fields"
        );
    }

    #[test]
    fn filter_values_are_bound_and_never_interpolated() {
        for injection in INJECTIONS {
            let mut request = count_events();
            request.filters = Some(vec![filter("source", "eq", serde_json::json!(injection))]);
            let plan = plan_query(&request).expect("planned");
            let built = build_query(&plan, cutoff(), Stage::Rows, None);

            assert_no_input_in_sql(&built.sql, injection);
            assert!(built.sql.contains("\"source\" = $2"), "{}", built.sql);
            assert!(
                bound_strings(&built.values).contains(&injection.to_string()),
                "{:?}",
                built.values
            );
        }
    }

    #[test]
    fn contains_binds_an_escaped_like_pattern() {
        let injection = "100%_'; DROP TABLE \"TelemetryEvent\"; --";
        let mut request = count_events();
        request.filters = Some(vec![filter(
            "name",
            "contains",
            serde_json::json!(injection),
        )]);
        let plan = plan_query(&request).expect("planned");
        let built = build_query(&plan, cutoff(), Stage::Rows, None);

        assert_no_input_in_sql(&built.sql, injection);
        assert!(
            built.sql.contains("LOWER(\"name\") LIKE $2"),
            "{}",
            built.sql
        );
        assert_eq!(
            bound_strings(&built.values),
            vec![like_pattern(injection)],
            "the LIKE pattern is a bound parameter"
        );
        assert_eq!(like_pattern("100% a_b\\c"), "%100\\% a\\_b\\\\c%");
    }

    #[test]
    fn contains_needs_a_text_field() {
        let mut request = definition("spans", metric("count", None));
        request.filters = Some(vec![filter(
            "duration_ms",
            "contains",
            serde_json::json!("5"),
        )]);
        assert_bad_request(plan_query(&request));
    }

    #[test]
    fn in_filters_bind_every_element() {
        let mut request = count_events();
        request.filters = Some(vec![filter(
            "platform",
            "in",
            serde_json::json!(["macos", INJECTIONS[0], INJECTIONS[1]]),
        )]);
        let plan = plan_query(&request).expect("planned");
        let built = build_query(&plan, cutoff(), Stage::Rows, None);

        assert_no_input_in_sql(&built.sql, INJECTIONS[0]);
        assert!(
            built.sql.contains("\"platform\" IN ($2, $3, $4)"),
            "{}",
            built.sql
        );
        assert_eq!(built.values.len(), 4);
    }

    #[test]
    fn in_filters_reject_empty_and_oversized_lists() {
        let mut request = count_events();
        request.filters = Some(vec![filter("platform", "in", serde_json::json!([]))]);
        assert_bad_request(plan_query(&request));

        let overflow: Vec<serde_json::Value> = (0..=MAX_IN_VALUES)
            .map(|i| serde_json::json!(format!("p{i}")))
            .collect();
        request = count_events();
        request.filters = Some(vec![filter(
            "platform",
            "in",
            serde_json::Value::Array(overflow),
        )]);
        assert_bad_request(plan_query(&request));

        request = count_events();
        request.filters = Some(vec![filter("platform", "in", serde_json::json!("macos"))]);
        assert_bad_request(plan_query(&request));
    }

    #[test]
    fn unknown_operators_are_rejected() {
        for op in ["like", "regex", "=", "or", ""] {
            let mut request = count_events();
            request.filters = Some(vec![filter("source", op, serde_json::json!("web"))]);
            assert_bad_request(plan_query(&request));
        }
    }

    #[test]
    fn filter_values_must_match_the_field_type() {
        let mut request = definition("spans", metric("count", None));
        request.filters = Some(vec![filter(
            "duration_ms",
            "gt",
            serde_json::json!("50 OR 1=1"),
        )]);
        assert_bad_request(plan_query(&request));

        request = count_events();
        request.filters = Some(vec![filter(
            "created_at",
            "gte",
            serde_json::json!("now()"),
        )]);
        assert_bad_request(plan_query(&request));

        request = count_events();
        request.filters = Some(vec![filter("source", "eq", serde_json::json!(42))]);
        assert_bad_request(plan_query(&request));

        request = definition("llm", metric("count", None));
        request.filters = Some(vec![filter("streamed", "eq", serde_json::json!("true"))]);
        assert_bad_request(plan_query(&request));
    }

    #[test]
    fn ordering_operators_need_a_numeric_or_timestamp_field() {
        for op in ["gt", "lt", "gte", "lte"] {
            let mut request = count_events();
            request.filters = Some(vec![filter("source", op, serde_json::json!("web"))]);
            assert_bad_request(plan_query(&request));

            let mut spans = definition("spans", metric("count", None));
            spans.filters = Some(vec![filter("duration_ms", op, serde_json::json!(100))]);
            assert!(plan_query(&spans).is_ok(), "{op}");
        }
    }

    #[test]
    fn timestamp_filters_bind_a_parsed_value() {
        let mut request = count_events();
        request.filters = Some(vec![filter(
            "created_at",
            "gte",
            serde_json::json!("2026-07-26T10:00:00Z"),
        )]);
        let plan = plan_query(&request).expect("planned");
        let built = build_query(&plan, cutoff(), Stage::Rows, None);

        let expected = NaiveDate::from_ymd_opt(2026, 7, 26)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap();
        assert!(built.sql.contains("\"createdAt\" >= $2"), "{}", built.sql);
        assert!(
            built.values.iter().any(|value| matches!(
                value,
                Value::ChronoDateTime(Some(ts)) if ts == &expected
            )),
            "{:?}",
            built.values
        );
    }

    #[test]
    fn too_many_filters_are_rejected() {
        let mut request = count_events();
        request.filters = Some(
            (0..=MAX_FILTERS)
                .map(|_| filter("source", "eq", serde_json::json!("web")))
                .collect(),
        );
        assert_bad_request(plan_query(&request));
    }

    #[test]
    fn intervals_come_from_the_closed_set() {
        for known in ["minute", "hour", "day", "none"] {
            let mut request = count_events();
            request.interval = Some(known.to_string());
            assert!(plan_query(&request).is_ok(), "{known}");
        }

        for unknown in [
            "hour'); DROP TABLE \"TelemetryEvent\"; --",
            "week",
            "milliseconds",
        ] {
            let mut request = count_events();
            request.interval = Some(unknown.to_string());
            assert_bad_request(plan_query(&request));
        }
    }

    #[test]
    fn hours_are_clamped_into_the_supported_window() {
        for (requested, expected) in [
            (Some(-9_999_999), 1),
            (Some(0), 1),
            (Some(1), 1),
            (Some(24), 24),
            (Some(MAX_QUERY_HOURS), MAX_QUERY_HOURS),
            (Some(i64::MAX), MAX_QUERY_HOURS),
            (None, DEFAULT_QUERY_HOURS),
        ] {
            let mut request = count_events();
            request.hours = requested;
            assert_eq!(plan_query(&request).expect("planned").hours, expected);
        }
    }

    #[test]
    fn breakdowns_are_capped_and_rows_carry_a_forced_limit() {
        let mut request = count_events();
        request.breakdown = Some("platform".to_string());
        request.interval = Some("hour".to_string());
        let plan = plan_query(&request).expect("planned");

        let groups = build_query(&plan, cutoff(), Stage::TopGroups, None);
        assert!(
            groups
                .sql
                .contains(&format!("LIMIT {}", fetch_limit(MAX_BREAKDOWN_GROUPS))),
            "{}",
            groups.sql
        );
        assert!(
            groups.sql.contains("ORDER BY metric_value DESC NULLS LAST"),
            "{}",
            groups.sql
        );
        assert!(groups.sql.contains("GROUP BY group_key"), "{}", groups.sql);

        let ranked: Vec<String> = (0..MAX_BREAKDOWN_GROUPS)
            .map(|i| format!("platform-{i}"))
            .collect();
        let rows = build_query(&plan, cutoff(), Stage::Rows, Some(&ranked));
        assert!(
            rows.sql
                .contains(&format!("LIMIT {}", fetch_limit(MAX_QUERY_ROWS))),
            "{}",
            rows.sql
        );
        assert!(rows.sql.contains("GROUP BY ts, group_key"), "{}", rows.sql);
        assert!(
            rows.sql.contains("date_trunc('hour', \"createdAt\")"),
            "{}",
            rows.sql
        );
        assert_eq!(rows.values.len(), 2 + MAX_BREAKDOWN_GROUPS);
        assert!(bound_strings(&rows.values).contains(&"platform-0".to_string()));
    }

    #[test]
    fn ranked_group_keys_are_bound_even_though_they_come_from_the_database() {
        let mut request = count_events();
        request.breakdown = Some("platform".to_string());
        request.interval = Some("day".to_string());
        let plan = plan_query(&request).expect("planned");

        let poisoned = vec![INJECTIONS[0].to_string(), INJECTIONS[2].to_string()];
        let rows = build_query(&plan, cutoff(), Stage::Rows, Some(&poisoned));

        assert_no_input_in_sql(&rows.sql, INJECTIONS[0]);
        assert!(
            rows.sql.contains("COALESCE(\"platform\", $1) IN ($3, $4)"),
            "{}",
            rows.sql
        );
        assert!(bound_strings(&rows.values).contains(&INJECTIONS[0].to_string()));
    }

    #[test]
    fn breakdowns_need_a_text_field() {
        let mut request = definition("spans", metric("count", None));
        request.breakdown = Some("duration_ms".to_string());
        assert_bad_request(plan_query(&request));

        request = definition("llm", metric("count", None));
        request.breakdown = Some("streamed".to_string());
        assert_bad_request(plan_query(&request));

        request = definition("llm", metric("count", None));
        request.breakdown = Some("model".to_string());
        assert!(plan_query(&request).is_ok());
    }

    #[test]
    fn queries_without_dimensions_stay_a_single_aggregate_row() {
        let plan = plan_query(&count_events()).expect("planned");
        let built = build_query(&plan, cutoff(), Stage::Rows, None);

        assert!(!built.sql.contains("GROUP BY"), "{}", built.sql);
        assert!(!built.sql.contains("ORDER BY"), "{}", built.sql);
        assert!(built.sql.contains(NULL_TS), "{}", built.sql);
        assert!(built.sql.contains(NULL_GROUP_EXPR), "{}", built.sql);
        assert!(
            built
                .sql
                .starts_with("SELECT CAST(NULL AS TIMESTAMP) AS ts, CAST(NULL AS TEXT) AS group_key, CAST(COUNT(*) AS DOUBLE PRECISION) AS metric_value FROM \"TelemetryEvent\" WHERE \"createdAt\" >= $1"),
            "{}",
            built.sql
        );
    }

    #[test]
    fn percentiles_order_by_the_allowlisted_column() {
        let plan =
            plan_query(&definition("spans", metric("p95", Some("duration_ms")))).expect("planned");
        let built = build_query(&plan, cutoff(), Stage::Rows, None);

        assert!(
            built.sql.contains(
                "percentile_cont(0.95::float8) WITHIN GROUP (ORDER BY \"durationMs\"::float8)"
            ),
            "{}",
            built.sql
        );
    }

    #[test]
    fn every_allowlisted_field_resolves_against_its_entity_column() {
        fn columns_of(dataset: &str) -> HashSet<String> {
            match dataset {
                "events" => telemetry_event::Column::iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                "errors" => telemetry_error_event::Column::iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                "spans" => telemetry_span::Column::iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                "performance" => telemetry_perf_metric::Column::iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                "sessions" => telemetry_session::Column::iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                "llm" => telemetry_llm_call::Column::iter()
                    .map(|c| c.as_str().to_string())
                    .collect(),
                other => panic!("dataset '{other}' has no entity binding in this test"),
            }
        }

        fn table_of(dataset: &str) -> String {
            match dataset {
                "events" => telemetry_event::Entity.table_name().to_string(),
                "errors" => telemetry_error_event::Entity.table_name().to_string(),
                "spans" => telemetry_span::Entity.table_name().to_string(),
                "performance" => telemetry_perf_metric::Entity.table_name().to_string(),
                "sessions" => telemetry_session::Entity.table_name().to_string(),
                "llm" => telemetry_llm_call::Entity.table_name().to_string(),
                other => panic!("dataset '{other}' has no entity binding in this test"),
            }
        }

        for dataset in DATASETS {
            let columns = columns_of(dataset.name);
            assert_eq!(table_of(dataset.name), dataset.table, "{}", dataset.name);
            assert!(
                columns.contains(dataset.time_column),
                "{} has no column {}",
                dataset.name,
                dataset.time_column
            );

            let mut names = HashSet::new();
            for field in dataset.fields {
                assert!(
                    columns.contains(field.column),
                    "{}.{} maps to '{}', which is not a column of the entity",
                    dataset.name,
                    field.name,
                    field.column
                );
                assert!(
                    names.insert(field.name),
                    "{} declares '{}' twice",
                    dataset.name,
                    field.name
                );
            }
        }
    }

    #[test]
    fn allowlisted_identifiers_are_plain_identifiers() {
        let plain = |ident: &str| {
            !ident.is_empty()
                && ident
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_alphabetic())
                && ident.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        };

        for dataset in DATASETS {
            assert!(plain(dataset.table), "{}", dataset.table);
            assert!(plain(dataset.time_column), "{}", dataset.time_column);
            for field in dataset.fields {
                assert!(plain(field.column), "{}", field.column);
                assert!(plain(field.name), "{}", field.name);
            }
        }
    }

    #[test]
    fn stored_definitions_run_through_the_same_planner() {
        assert!(
            validate_query_definition(&serde_json::json!({
                "dataset": "llm",
                "metric": { "type": "p95", "field": "duration_ms" },
                "filters": [{ "field": "provider", "op": "eq", "value": "anthropic" }],
                "breakdown": "model",
                "interval": "hour",
                "hours": 168
            }))
            .is_ok()
        );

        for rejected in [
            serde_json::json!({ "dataset": "users", "metric": { "type": "count" } }),
            serde_json::json!({ "dataset": "events", "metric": { "type": "sum", "field": "name" } }),
            serde_json::json!({
                "dataset": "events",
                "metric": { "type": "count" },
                "breakdown": "1; DELETE FROM \"TelemetryEvent\""
            }),
            serde_json::json!({ "dataset": "events" }),
            serde_json::json!("SELECT * FROM \"User\""),
            serde_json::json!({
                "dataset": "events",
                "metric": { "type": "count" },
                "filters": [{ "field": "created_at\"; DROP TABLE x", "op": "eq", "value": "1" }]
            }),
        ] {
            assert_bad_request(validate_query_definition(&rejected));
        }
    }

    #[test]
    fn names_are_trimmed_and_bounded() {
        assert_eq!(require_name("name", "  Crashes  ").unwrap(), "Crashes");
        assert_bad_request(require_name("name", "   "));
        assert_bad_request(require_name("name", &"n".repeat(121)));
    }

    #[test]
    fn projection_matches_the_requested_dimensions() {
        let mut request = count_events();
        request.breakdown = Some("platform".to_string());
        request.interval = Some("hour".to_string());
        let plan = plan_query(&request).expect("planned");

        let response = project(
            &plan,
            vec![
                QueryRow {
                    ts: Some(cutoff()),
                    group_key: Some("macos".to_string()),
                    metric_value: Some(3.0),
                },
                QueryRow {
                    ts: Some(cutoff()),
                    group_key: None,
                    metric_value: None,
                },
            ],
            false,
        );

        assert_eq!(response.columns, vec!["ts", "platform", "value"]);
        assert_eq!(response.interval, "hour");
        assert_eq!(response.total, 2);
        assert!(!response.truncated);
        assert_eq!(
            response.rows[0],
            vec![
                serde_json::json!("2026-07-26T00:00:00+00:00"),
                serde_json::json!("macos"),
                serde_json::json!(3.0),
            ]
        );
        assert_eq!(
            response.rows[1],
            vec![
                serde_json::json!("2026-07-26T00:00:00+00:00"),
                serde_json::json!(NULL_GROUP),
                serde_json::Value::Null,
            ]
        );
    }

    #[test]
    fn neq_keeps_rows_whose_column_is_null() {
        let mut request = count_events();
        request.filters = Some(vec![filter(
            "platform",
            "neq",
            serde_json::json!(INJECTIONS[1]),
        )]);
        let plan = plan_query(&request).expect("planned");
        let built = build_query(&plan, cutoff(), Stage::Rows, None);

        assert_no_input_in_sql(&built.sql, INJECTIONS[1]);
        assert!(
            built
                .sql
                .contains("(\"platform\" <> $2 OR \"platform\" IS NULL)"),
            "{}",
            built.sql
        );
        assert!(
            bound_strings(&built.values).contains(&INJECTIONS[1].to_string()),
            "{:?}",
            built.values
        );
    }

    /// Rows in the order the statement hands them back: newest bucket first.
    fn descending_rows(count: usize) -> Vec<QueryRow> {
        (0..count)
            .map(|index| QueryRow {
                ts: Some(cutoff() - Duration::minutes(index as i64)),
                group_key: None,
                metric_value: Some(index as f64),
            })
            .collect()
    }

    #[test]
    fn result_rows_are_read_newest_first() {
        let mut request = count_events();
        request.interval = Some("minute".to_string());
        let plan = plan_query(&request).expect("planned");
        let built = build_query(&plan, cutoff(), Stage::Rows, None);

        assert!(built.sql.contains(" ORDER BY ts DESC"), "{}", built.sql);
        assert!(
            built
                .sql
                .ends_with(&format!("LIMIT {}", fetch_limit(MAX_QUERY_ROWS))),
            "{}",
            built.sql
        );
    }

    #[test]
    fn a_result_at_the_cap_is_complete() {
        let (rows, truncated) = finalize_rows(descending_rows(MAX_QUERY_ROWS));

        assert!(!truncated);
        assert_eq!(rows.len(), MAX_QUERY_ROWS);
        assert_eq!(rows.last().expect("newest").ts, Some(cutoff()));
    }

    #[test]
    fn hitting_the_cap_keeps_the_newest_buckets_and_reports_truncation() {
        let fetched = descending_rows(fetch_limit(MAX_QUERY_ROWS));
        let oldest = fetched.last().expect("oldest").ts;
        let (rows, truncated) = finalize_rows(fetched);

        assert!(truncated);
        assert_eq!(rows.len(), MAX_QUERY_ROWS);
        assert_eq!(
            rows.last().expect("newest").ts,
            Some(cutoff()),
            "the newest bucket survives the cap"
        );
        assert!(
            rows.iter().all(|row| row.ts != oldest),
            "the oldest bucket is the one dropped"
        );
        assert!(
            rows.windows(2).all(|pair| pair[0].ts <= pair[1].ts),
            "the response still reads left to right"
        );
    }

    #[test]
    fn a_result_under_the_cap_is_not_truncated() {
        let (rows, truncated) = finalize_rows(descending_rows(3));

        assert!(!truncated);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows.first().expect("oldest").metric_value, Some(2.0));
        assert_eq!(rows.last().expect("newest").ts, Some(cutoff()));
    }
}
