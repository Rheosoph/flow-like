//! Web-vitals and app performance percentiles.
//!
//! Windows of at most 48 hours are computed from the raw samples: with
//! `percentile_cont` on Postgres, and with a single capped fetch folded in Rust
//! on other backends. Longer windows read `TelemetryPerfDaily`, which already
//! stores p50/p75/p95 per (day, metric, source); those daily percentiles are
//! recombined as a count-weighted mean, which is an approximation of the exact
//! percentile over the whole window — the response says so through
//! `granularity`.
//!
//! The per-route breakdown has no rollup dimension, so it is always read from
//! the raw samples and clipped to the retention window the sweeper keeps them
//! for. `byPathWindowHours` reports how much it actually covers.

use super::overview::{
    GRANULARITY_DAILY, GRANULARITY_RAW, day_window, reads_raw, retention_days, window_bucket,
};
use super::trunc_to_bucket;
use crate::entity::{telemetry_perf_daily, telemetry_perf_metric};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use crate::{db::DbDialect, telemetry::percentiles_in_sql};
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Select, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

/// Metrics the dashboard knows how to rate. Rows outside this vocabulary are
/// ignored so a stray writer cannot poison the charts.
pub const PERF_METRICS: [&str; 7] = [
    "lcp",
    "inp",
    "cls",
    "ttfb",
    "fcp",
    "app_start",
    "screen_load",
];
const DEFAULT_PERF_HOURS: i64 = 24;
const MAX_PERF_HOURS: i64 = 24 * 90;
/// Upper bound on the rows folded in Rust when the backend has no
/// `percentile_cont`.
const PERF_ROW_CAP: u64 = 100_000;
const PERF_PATH_LIMIT: usize = 20;
/// Mirrors the telemetry sweeper's default raw performance retention.
const DEFAULT_PERF_RETENTION_DAYS: i64 = 30;
const PERF_RETENTION_VAR: &str = "FLOW_LIKE_PERF_RETENTION_DAYS";

const RATING_GOOD: &str = "good";
const RATING_NEEDS_IMPROVEMENT: &str = "needs-improvement";
const RATING_POOR: &str = "poor";

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetryPerformanceQuery {
    /// Lookback window in hours. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by metric: "lcp", "inp", "cls", "ttfb", "fcp", "app_start" or "screen_load".
    #[serde(default)]
    pub metric: Option<String>,
    /// Filter by source: "desktop", "web", "desktop_native" or "backend".
    #[serde(default)]
    pub source: Option<String>,
    /// Filter by the reported route or screen. Routes are only recorded on the
    /// raw samples, so this filter always reads raw data.
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerfMetricSummary {
    pub metric: String,
    pub p50: f64,
    pub p75: f64,
    pub p95: f64,
    pub count: i64,
    /// Core Web Vitals rating of the p75: "good", "needs-improvement" or "poor".
    pub rating: String,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerfTrendPoint {
    /// ISO-8601 timestamp at the start of the bucket.
    pub ts: String,
    pub metric: String,
    pub p75: f64,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PerfPathRow {
    pub path: String,
    pub metric: String,
    pub p75: f64,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryPerformanceResponse {
    pub hours: i64,
    /// "raw" when the percentiles are computed over individual samples, "daily"
    /// when the stored daily percentiles are recombined as a count-weighted mean.
    pub granularity: String,
    pub metrics: Vec<PerfMetricSummary>,
    pub trend: Vec<PerfTrendPoint>,
    pub by_path: Vec<PerfPathRow>,
    /// Hours of raw samples the per-route breakdown could actually read.
    pub by_path_window_hours: i64,
}

#[derive(Debug, FromQueryResult)]
struct SummaryRow {
    metric: String,
    cnt: i64,
    p50: f64,
    p75: f64,
    p95: f64,
}

#[derive(Debug, FromQueryResult)]
struct TrendRow {
    bucket: DateTime<FixedOffset>,
    metric: String,
    p75: f64,
}

#[derive(Debug, FromQueryResult)]
struct PathRow {
    path: String,
    metric: String,
    cnt: i64,
    p75: f64,
}

#[derive(Clone, Debug, FromQueryResult)]
struct PerfSampleRow {
    metric: String,
    value: f64,
    path: Option<String>,
    created_at: DateTime<FixedOffset>,
}

#[derive(Debug, Default)]
struct PerfFold {
    metrics: Vec<PerfMetricSummary>,
    trend: Vec<PerfTrendPoint>,
    by_path: Vec<PerfPathRow>,
}

/// Core Web Vitals thresholds as `(good, poor)`: at most `good` rates "good",
/// above `poor` rates "poor", everything in between needs improvement.
fn rating_thresholds(metric: &str) -> Option<(f64, f64)> {
    match metric {
        "lcp" => Some((2500.0, 4000.0)),
        "inp" => Some((200.0, 500.0)),
        "cls" => Some((0.1, 0.25)),
        "ttfb" => Some((800.0, 1800.0)),
        "fcp" => Some((1800.0, 3000.0)),
        "app_start" | "screen_load" => Some((1000.0, 3000.0)),
        _ => None,
    }
}

fn rating(metric: &str, p75: f64) -> String {
    let Some((good, poor)) = rating_thresholds(metric) else {
        return RATING_NEEDS_IMPROVEMENT.to_string();
    };
    if p75 <= good {
        RATING_GOOD.to_string()
    } else if p75 > poor {
        RATING_POOR.to_string()
    } else {
        RATING_NEEDS_IMPROVEMENT.to_string()
    }
}

/// Linear-interpolating percentile over an ascending slice, matching the
/// semantics of SQL `percentile_cont`.
pub(super) fn percentile_cont(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let pos = q.clamp(0.0, 1.0) * (sorted.len() - 1) as f64;
    let lower = pos.floor();
    let upper = pos.ceil();
    if (upper - lower).abs() < f64::EPSILON {
        return sorted[lower as usize];
    }

    let low = sorted[lower as usize];
    let high = sorted[upper as usize];
    low + (pos - lower) * (high - low)
}

pub(super) fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn sorted_values(mut values: Vec<f64>) -> Vec<f64> {
    values.sort_by(|a, b| a.total_cmp(b));
    values
}

fn summary(metric: &str, values: Vec<f64>) -> PerfMetricSummary {
    let sorted = sorted_values(values);
    let p75 = percentile_cont(&sorted, 0.75);
    PerfMetricSummary {
        metric: metric.to_string(),
        p50: round3(percentile_cont(&sorted, 0.5)),
        p75: round3(p75),
        p95: round3(percentile_cont(&sorted, 0.95)),
        count: sorted.len() as i64,
        rating: rating(metric, p75),
    }
}

/// Recombines stored daily percentiles into one number by weighting each day by
/// its sample count. Exact percentiles cannot be summed, so this is a documented
/// approximation used only outside the raw window.
#[derive(Debug, Default)]
struct WeightedPercentiles {
    count: i64,
    p50: f64,
    p75: f64,
    p95: f64,
}

impl WeightedPercentiles {
    fn push(&mut self, count: i32, p50: f64, p75: f64, p95: f64) {
        let weight = i64::from(count.max(0));
        if weight == 0 {
            return;
        }
        self.count = self.count.saturating_add(weight);
        self.p50 += p50 * weight as f64;
        self.p75 += p75 * weight as f64;
        self.p95 += p95 * weight as f64;
    }

    fn resolve(&self) -> (f64, f64, f64) {
        if self.count <= 0 {
            return (0.0, 0.0, 0.0);
        }
        let weight = self.count as f64;
        (
            round3(self.p50 / weight),
            round3(self.p75 / weight),
            round3(self.p95 / weight),
        )
    }
}

fn fold_perf_samples(rows: Vec<PerfSampleRow>, bucket: &str) -> PerfFold {
    let mut per_metric: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    let mut per_bucket: BTreeMap<(DateTime<FixedOffset>, String), Vec<f64>> = BTreeMap::new();
    let mut per_path: BTreeMap<(String, String), Vec<f64>> = BTreeMap::new();

    for row in rows {
        per_metric
            .entry(row.metric.clone())
            .or_default()
            .push(row.value);
        per_bucket
            .entry((trunc_to_bucket(row.created_at, bucket), row.metric.clone()))
            .or_default()
            .push(row.value);
        if let Some(path) = row.path {
            per_path
                .entry((path, row.metric))
                .or_default()
                .push(row.value);
        }
    }

    let metrics = per_metric
        .into_iter()
        .map(|(metric, values)| summary(&metric, values))
        .collect();

    let trend = per_bucket
        .into_iter()
        .map(|((ts, metric), values)| PerfTrendPoint {
            ts: ts.to_rfc3339(),
            metric,
            p75: round3(percentile_cont(&sorted_values(values), 0.75)),
        })
        .collect();

    let by_path = rank_paths(
        per_path
            .into_iter()
            .map(|((path, metric), values)| {
                let sorted = sorted_values(values);
                PerfPathRow {
                    path,
                    metric,
                    p75: round3(percentile_cont(&sorted, 0.75)),
                    count: sorted.len() as i64,
                }
            })
            .collect(),
    );

    PerfFold {
        metrics,
        trend,
        by_path,
    }
}

fn rank_paths(mut rows: Vec<PerfPathRow>) -> Vec<PerfPathRow> {
    rows.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.path.cmp(&b.path))
            .then_with(|| a.metric.cmp(&b.metric))
    });
    rows.truncate(PERF_PATH_LIMIT);
    rows
}

/// Folds the daily rollup into per-metric summaries and a per-day p75 trend.
fn fold_perf_daily(
    rows: &[telemetry_perf_daily::Model],
) -> (Vec<PerfMetricSummary>, Vec<PerfTrendPoint>) {
    let mut per_metric: BTreeMap<&str, WeightedPercentiles> = BTreeMap::new();
    let mut per_day: BTreeMap<(DateTime<FixedOffset>, &str), WeightedPercentiles> = BTreeMap::new();

    for row in rows {
        per_metric
            .entry(row.metric.as_str())
            .or_default()
            .push(row.count, row.p50, row.p75, row.p95);
        per_day
            .entry((row.day, row.metric.as_str()))
            .or_default()
            .push(row.count, row.p50, row.p75, row.p95);
    }

    let metrics = per_metric
        .into_iter()
        .map(|(metric, weighted)| {
            let (p50, p75, p95) = weighted.resolve();
            PerfMetricSummary {
                metric: metric.to_string(),
                p50,
                p75,
                p95,
                count: weighted.count,
                rating: rating(metric, p75),
            }
        })
        .collect();

    let trend = per_day
        .into_iter()
        .map(|((day, metric), weighted)| PerfTrendPoint {
            ts: day.to_rfc3339(),
            metric: metric.to_string(),
            p75: weighted.resolve().1,
        })
        .collect();

    (metrics, trend)
}

fn validate_metric(metric: &str) -> Result<(), ApiError> {
    if PERF_METRICS.contains(&metric) {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "Unknown metric '{}', expected one of {}",
        metric,
        PERF_METRICS.join(", ")
    )))
}

struct PerfFilters {
    cutoff: DateTime<FixedOffset>,
    metric: Option<String>,
    source: Option<String>,
    path: Option<String>,
}

impl PerfFilters {
    fn new(q: &TelemetryPerformanceQuery, cutoff: DateTime<FixedOffset>) -> Self {
        Self {
            cutoff,
            metric: q.metric.clone().filter(|v| !v.is_empty()),
            source: q.source.clone().filter(|v| !v.is_empty()),
            path: q.path.clone().filter(|v| !v.is_empty()),
        }
    }
}

/// Raw samples only exist for as long as the sweeper keeps them, so a raw read
/// is clipped to that retention and reports the hours it really covers.
fn raw_window(
    now: DateTime<FixedOffset>,
    hours: i64,
    retention_hours: i64,
) -> (DateTime<FixedOffset>, i64) {
    let effective = hours.min(retention_hours);
    (now - Duration::hours(effective), effective)
}

fn perf_retention_hours() -> i64 {
    retention_days(PERF_RETENTION_VAR, DEFAULT_PERF_RETENTION_DAYS) * 24
}

/// Shared `WHERE` fragment plus its bound values, in placeholder order. Only
/// the Postgres path builds SQL by hand; other backends fold in Rust.
fn perf_where(filters: &PerfFilters) -> (String, Vec<sea_orm::Value>) {
    let created_at = r#""createdAt""#;
    let metric = r#""metric""#;
    let source = r#""source""#;
    let path = r#""path""#;

    let vocabulary = PERF_METRICS
        .iter()
        .map(|m| format!("'{m}'"))
        .collect::<Vec<_>>()
        .join(", ");

    let mut values: Vec<sea_orm::Value> = vec![filters.cutoff.into()];
    let mut clauses = vec![
        format!("{created_at} >= $1"),
        format!("{metric} IN ({vocabulary})"),
    ];

    if let Some(value) = &filters.metric {
        values.push(value.clone().into());
        clauses.push(format!("{metric} = ${}", values.len()));
    }

    if let Some(value) = &filters.source {
        values.push(value.clone().into());
        clauses.push(format!("{source} = ${}", values.len()));
    }

    if let Some(value) = &filters.path {
        values.push(value.clone().into());
        clauses.push(format!("{path} = ${}", values.len()));
    }

    (clauses.join(" AND "), values)
}

async fn perf_paths_from_sql<C: ConnectionTrait>(
    db: &C,
    filters: &PerfFilters,
) -> Result<Vec<PerfPathRow>, ApiError> {
    let backend = db.get_database_backend();
    let (where_sql, values) = perf_where(filters);

    let path_sql = format!(
        r#"SELECT "path" AS path,
                  "metric" AS metric,
                  CAST(COUNT(*) AS BIGINT) AS cnt,
                  percentile_cont(0.75::float8) WITHIN GROUP (ORDER BY "value") AS p75
           FROM "TelemetryPerfMetric"
           WHERE {where_sql} AND "path" IS NOT NULL
           GROUP BY "path", "metric"
           ORDER BY cnt DESC, path ASC, metric ASC
           LIMIT {PERF_PATH_LIMIT}"#
    );

    let rows =
        PathRow::find_by_statement(Statement::from_sql_and_values(backend, path_sql, values))
            .all(db)
            .await?;

    Ok(rows
        .into_iter()
        .map(|row| PerfPathRow {
            path: row.path,
            metric: row.metric,
            p75: round3(row.p75),
            count: row.cnt,
        })
        .collect())
}

async fn perf_from_sql<C: ConnectionTrait>(
    db: &C,
    filters: &PerfFilters,
    bucket: &str,
) -> Result<PerfFold, ApiError> {
    let backend = db.get_database_backend();
    let (where_sql, values) = perf_where(filters);

    let summary_sql = format!(
        r#"SELECT "metric" AS metric,
                  CAST(COUNT(*) AS BIGINT) AS cnt,
                  percentile_cont(0.5::float8) WITHIN GROUP (ORDER BY "value") AS p50,
                  percentile_cont(0.75::float8) WITHIN GROUP (ORDER BY "value") AS p75,
                  percentile_cont(0.95::float8) WITHIN GROUP (ORDER BY "value") AS p95
           FROM "TelemetryPerfMetric"
           WHERE {where_sql}
           GROUP BY "metric"
           ORDER BY "metric" ASC"#
    );
    let summaries = SummaryRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        summary_sql,
        values.clone(),
    ))
    .all(db)
    .await?;

    let trend_sql = format!(
        r#"SELECT date_trunc('{bucket}', "createdAt" AT TIME ZONE 'UTC') AT TIME ZONE 'UTC' AS bucket,
                  "metric" AS metric,
                  percentile_cont(0.75::float8) WITHIN GROUP (ORDER BY "value") AS p75
           FROM "TelemetryPerfMetric"
           WHERE {where_sql}
           GROUP BY bucket, "metric"
           ORDER BY bucket ASC, "metric" ASC"#
    );
    let trend_rows = TrendRow::find_by_statement(Statement::from_sql_and_values(
        backend,
        trend_sql,
        values.clone(),
    ))
    .all(db)
    .await?;

    Ok(PerfFold {
        metrics: summaries
            .into_iter()
            .map(|row| PerfMetricSummary {
                p50: round3(row.p50),
                p75: round3(row.p75),
                p95: round3(row.p95),
                count: row.cnt,
                rating: rating(&row.metric, row.p75),
                metric: row.metric,
            })
            .collect(),
        trend: trend_rows
            .into_iter()
            .map(|row| PerfTrendPoint {
                ts: row.bucket.to_rfc3339(),
                metric: row.metric,
                p75: round3(row.p75),
            })
            .collect(),
        by_path: perf_paths_from_sql(db, filters).await?,
    })
}

async fn perf_from_fold<C: ConnectionTrait>(
    db: &C,
    filters: &PerfFilters,
    bucket: &str,
) -> Result<PerfFold, ApiError> {
    let mut select = telemetry_perf_metric::Entity::find()
        .select_only()
        .column_as(telemetry_perf_metric::Column::Metric, "metric")
        .column_as(telemetry_perf_metric::Column::Value, "value")
        .column_as(telemetry_perf_metric::Column::Path, "path")
        .column_as(telemetry_perf_metric::Column::CreatedAt, "created_at")
        .filter(telemetry_perf_metric::Column::CreatedAt.gte(filters.cutoff))
        .filter(telemetry_perf_metric::Column::Metric.is_in(PERF_METRICS));

    if let Some(value) = &filters.metric {
        select = select.filter(telemetry_perf_metric::Column::Metric.eq(value));
    }
    if let Some(value) = &filters.source {
        select = select.filter(telemetry_perf_metric::Column::Source.eq(value));
    }
    if let Some(value) = &filters.path {
        select = select.filter(telemetry_perf_metric::Column::Path.eq(value));
    }

    let rows = select
        .limit(PERF_ROW_CAP)
        .into_model::<PerfSampleRow>()
        .all(db)
        .await?;

    if rows.len() as u64 == PERF_ROW_CAP {
        tracing::warn!(
            cap = PERF_ROW_CAP,
            "Performance percentile fold hit the row cap; results are truncated"
        );
    }

    Ok(fold_perf_samples(rows, bucket))
}

async fn perf_raw<C: ConnectionTrait>(
    db: &C,
    dialect: DbDialect,
    filters: &PerfFilters,
    bucket: &str,
) -> Result<PerfFold, ApiError> {
    if percentiles_in_sql(db.get_database_backend(), dialect) {
        perf_from_sql(db, filters, bucket).await
    } else {
        perf_from_fold(db, filters, bucket).await
    }
}

async fn perf_paths<C: ConnectionTrait>(
    db: &C,
    dialect: DbDialect,
    filters: &PerfFilters,
    bucket: &str,
) -> Result<Vec<PerfPathRow>, ApiError> {
    if percentiles_in_sql(db.get_database_backend(), dialect) {
        perf_paths_from_sql(db, filters).await
    } else {
        Ok(perf_from_fold(db, filters, bucket).await?.by_path)
    }
}

fn daily_perf_query(
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    metric: Option<&str>,
    source: Option<&str>,
) -> Select<telemetry_perf_daily::Entity> {
    let mut select = telemetry_perf_daily::Entity::find()
        .filter(telemetry_perf_daily::Column::Day.gte(start))
        .filter(telemetry_perf_daily::Column::Day.lte(end))
        .filter(telemetry_perf_daily::Column::Metric.is_in(PERF_METRICS))
        .order_by_asc(telemetry_perf_daily::Column::Day);

    if let Some(metric) = metric {
        select = select.filter(telemetry_perf_daily::Column::Metric.eq(metric));
    }
    if let Some(source) = source {
        select = select.filter(telemetry_perf_daily::Column::Source.eq(source));
    }
    select
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/performance",
    tag = "admin",
    params(TelemetryPerformanceQuery),
    responses(
        (status = 200, description = "Web-vitals percentiles, their trend and the slowest routes", body = TelemetryPerformanceResponse),
        (status = 400, description = "Unknown metric"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Performance percentiles (p50/p75/p95) per web vital or app metric, rated against the Core Web Vitals thresholds, with a trend over time and a per-route breakdown. Windows of up to 48 hours are computed over individual samples and are exact (granularity \"raw\"). Longer windows recombine the stored daily percentiles weighted by sample count (granularity \"daily\"), which approximates the exact percentile over the window; the daily trend values are exact. Filtering by route always falls back to raw samples. The per-route breakdown is never rolled up, so it only covers the days raw samples are still kept for, reported as \"byPathWindowHours\". Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/performance", skip_all)]
pub async fn telemetry_performance(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetryPerformanceQuery>,
) -> Result<Json<TelemetryPerformanceResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    if let Some(metric) = &q.metric
        && !metric.is_empty()
    {
        validate_metric(metric)?;
    }

    if let Some(path) = &q.path
        && (path.contains('?') || path.contains('#'))
    {
        return Err(ApiError::bad_request(
            "Path filter must not contain a query string or fragment",
        ));
    }

    let hours = q
        .hours
        .unwrap_or(DEFAULT_PERF_HOURS)
        .clamp(1, MAX_PERF_HOURS);
    let bucket = window_bucket(hours, None);
    let now = Utc::now().fixed_offset();
    let (raw_cutoff, by_path_window_hours) = raw_window(now, hours, perf_retention_hours());
    let filters = PerfFilters::new(&q, raw_cutoff);

    // A route filter can only be answered by the raw samples: the rollup has no
    // path dimension, so silently ignoring it would return the wrong numbers.
    if reads_raw(hours) || filters.path.is_some() {
        let fold = perf_raw(&state.db, state.db_dialect, &filters, bucket).await?;
        return Ok(Json(TelemetryPerformanceResponse {
            hours,
            granularity: GRANULARITY_RAW.to_string(),
            metrics: fold.metrics,
            trend: fold.trend,
            by_path: fold.by_path,
            by_path_window_hours,
        }));
    }

    let (start, end) = day_window(now, hours);
    let rows = daily_perf_query(
        start,
        end,
        filters.metric.as_deref(),
        filters.source.as_deref(),
    )
    .all(&state.db)
    .await?;
    let (metrics, trend) = fold_perf_daily(&rows);
    let by_path = perf_paths(&state.db, state.db_dialect, &filters, bucket).await?;

    Ok(Json(TelemetryPerformanceResponse {
        hours,
        granularity: GRANULARITY_DAILY.to_string(),
        metrics,
        trend,
        by_path,
        by_path_window_hours,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use sea_orm::DbBackend;

    fn ts(hour: u32, minute: u32) -> DateTime<FixedOffset> {
        NaiveDate::from_ymd_opt(2026, 7, 26)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
            .and_utc()
            .fixed_offset()
    }

    fn day(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .fixed_offset()
    }

    fn sample(metric: &str, value: f64, path: Option<&str>, hour: u32) -> PerfSampleRow {
        PerfSampleRow {
            metric: metric.to_string(),
            value,
            path: path.map(|p| p.to_string()),
            created_at: ts(hour, 30),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn daily(
        y: i32,
        m: u32,
        d: u32,
        metric: &str,
        source: &str,
        count: i32,
        p50: f64,
        p75: f64,
        p95: f64,
    ) -> telemetry_perf_daily::Model {
        telemetry_perf_daily::Model {
            id: format!("{y}-{m}-{d}-{metric}-{source}"),
            day: day(y, m, d),
            metric: metric.to_string(),
            source: source.to_string(),
            count,
            p50,
            p75,
            p95,
            created_at: day(y, m, d),
            updated_at: day(y, m, d),
        }
    }

    #[test]
    fn the_window_decides_between_raw_samples_and_the_daily_rollup() {
        assert!(reads_raw(47));
        assert!(reads_raw(48));
        assert!(!reads_raw(49));
        assert_eq!(window_bucket(48, None), "hour");
        assert_eq!(window_bucket(49, None), "day");
    }

    #[test]
    fn a_raw_read_is_clipped_to_the_sample_retention() {
        let now = ts(12, 0);

        assert_eq!(
            raw_window(now, 24, 24 * 30),
            (now - Duration::hours(24), 24)
        );
        assert_eq!(
            raw_window(now, 24 * 90, 24 * 30),
            (now - Duration::hours(24 * 30), 24 * 30)
        );
        assert_eq!(raw_window(now, 48, 24), (now - Duration::hours(24), 24));
    }

    #[test]
    fn percentiles_interpolate_like_percentile_cont() {
        let values = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(percentile_cont(&values, 0.5), 2.5);
        assert_eq!(percentile_cont(&values, 0.75), 3.25);
        assert_eq!(round3(percentile_cont(&values, 0.95)), 3.85);
        assert_eq!(percentile_cont(&values, 0.0), 1.0);
        assert_eq!(percentile_cont(&values, 1.0), 4.0);
    }

    #[test]
    fn percentiles_handle_degenerate_inputs() {
        assert_eq!(percentile_cont(&[], 0.5), 0.0);
        assert_eq!(percentile_cont(&[42.0], 0.95), 42.0);
        assert_eq!(percentile_cont(&[1.0, 3.0], 0.5), 2.0);
    }

    #[test]
    fn percentiles_land_on_exact_samples_when_the_position_is_whole() {
        let values = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(percentile_cont(&values, 0.5), 30.0);
        assert_eq!(percentile_cont(&values, 0.25), 20.0);
    }

    #[test]
    fn ratings_treat_the_good_threshold_as_inclusive() {
        assert_eq!(rating("lcp", 2500.0), RATING_GOOD);
        assert_eq!(rating("inp", 200.0), RATING_GOOD);
        assert_eq!(rating("cls", 0.1), RATING_GOOD);
        assert_eq!(rating("ttfb", 800.0), RATING_GOOD);
        assert_eq!(rating("fcp", 1800.0), RATING_GOOD);
        assert_eq!(rating("app_start", 1000.0), RATING_GOOD);
        assert_eq!(rating("screen_load", 1000.0), RATING_GOOD);
    }

    #[test]
    fn ratings_treat_the_poor_threshold_as_exclusive() {
        assert_eq!(rating("lcp", 4000.0), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("lcp", 4000.1), RATING_POOR);
        assert_eq!(rating("inp", 500.0), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("inp", 500.1), RATING_POOR);
        assert_eq!(rating("cls", 0.25), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("cls", 0.26), RATING_POOR);
        assert_eq!(rating("ttfb", 1800.0), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("ttfb", 1801.0), RATING_POOR);
        assert_eq!(rating("fcp", 3000.0), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("fcp", 3000.5), RATING_POOR);
        assert_eq!(rating("app_start", 3000.0), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("screen_load", 3001.0), RATING_POOR);
    }

    #[test]
    fn ratings_between_the_thresholds_need_improvement() {
        assert_eq!(rating("lcp", 2500.1), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("cls", 0.11), RATING_NEEDS_IMPROVEMENT);
        assert_eq!(rating("app_start", 2000.0), RATING_NEEDS_IMPROVEMENT);
    }

    #[test]
    fn every_metric_in_the_vocabulary_has_thresholds() {
        for metric in PERF_METRICS {
            assert!(rating_thresholds(metric).is_some(), "{metric}");
            assert!(validate_metric(metric).is_ok(), "{metric}");
        }
        assert!(validate_metric("fid").is_err());
    }

    #[test]
    fn folding_produces_summaries_trend_and_paths() {
        let rows = vec![
            sample("lcp", 1000.0, Some("/home"), 10),
            sample("lcp", 3000.0, Some("/home"), 10),
            sample("lcp", 5000.0, Some("/editor"), 11),
            sample("cls", 0.05, None, 10),
        ];

        let fold = fold_perf_samples(rows, "hour");

        assert_eq!(fold.metrics.len(), 2);
        assert_eq!(fold.metrics[0].metric, "cls");
        assert_eq!(fold.metrics[0].rating, RATING_GOOD);
        assert_eq!(fold.metrics[1].metric, "lcp");
        assert_eq!(fold.metrics[1].count, 3);
        assert_eq!(fold.metrics[1].p50, 3000.0);
        assert_eq!(fold.metrics[1].p75, 4000.0);
        assert_eq!(fold.metrics[1].rating, RATING_NEEDS_IMPROVEMENT);

        assert_eq!(
            fold.trend,
            vec![
                PerfTrendPoint {
                    ts: "2026-07-26T10:00:00+00:00".to_string(),
                    metric: "cls".to_string(),
                    p75: 0.05,
                },
                PerfTrendPoint {
                    ts: "2026-07-26T10:00:00+00:00".to_string(),
                    metric: "lcp".to_string(),
                    p75: 2500.0,
                },
                PerfTrendPoint {
                    ts: "2026-07-26T11:00:00+00:00".to_string(),
                    metric: "lcp".to_string(),
                    p75: 5000.0,
                },
            ]
        );

        assert_eq!(
            fold.by_path,
            vec![
                PerfPathRow {
                    path: "/home".to_string(),
                    metric: "lcp".to_string(),
                    p75: 2500.0,
                    count: 2,
                },
                PerfPathRow {
                    path: "/editor".to_string(),
                    metric: "lcp".to_string(),
                    p75: 5000.0,
                    count: 1,
                },
            ]
        );
    }

    #[test]
    fn daily_percentiles_are_weighted_by_sample_count() {
        let rows = vec![
            daily(2026, 7, 25, "lcp", "web", 10, 1000.0, 1500.0, 2000.0),
            daily(2026, 7, 25, "lcp", "desktop", 30, 2000.0, 2500.0, 4000.0),
            daily(2026, 7, 26, "lcp", "web", 60, 3000.0, 3500.0, 5000.0),
        ];

        let (metrics, trend) = fold_perf_daily(&rows);

        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].metric, "lcp");
        assert_eq!(metrics[0].count, 100);
        assert_eq!(metrics[0].p50, 2500.0);
        assert_eq!(metrics[0].p75, 3000.0);
        assert_eq!(metrics[0].p95, 4400.0);
        assert_eq!(metrics[0].rating, RATING_NEEDS_IMPROVEMENT);

        assert_eq!(
            trend,
            vec![
                PerfTrendPoint {
                    ts: "2026-07-25T00:00:00+00:00".to_string(),
                    metric: "lcp".to_string(),
                    p75: 2250.0,
                },
                PerfTrendPoint {
                    ts: "2026-07-26T00:00:00+00:00".to_string(),
                    metric: "lcp".to_string(),
                    p75: 3500.0,
                },
            ]
        );
    }

    #[test]
    fn daily_days_without_samples_never_divide_by_zero() {
        let rows = vec![daily(2026, 7, 25, "cls", "web", 0, 9.0, 9.0, 9.0)];
        let (metrics, trend) = fold_perf_daily(&rows);

        assert_eq!(metrics.len(), 1);
        assert_eq!(metrics[0].count, 0);
        assert_eq!(metrics[0].p50, 0.0);
        assert_eq!(metrics[0].p75, 0.0);
        assert_eq!(trend[0].p75, 0.0);
    }

    #[test]
    fn an_empty_rollup_window_folds_to_nothing() {
        let (metrics, trend) = fold_perf_daily(&[]);
        assert!(metrics.is_empty());
        assert!(trend.is_empty());
    }

    #[test]
    fn the_daily_query_pins_the_metric_vocabulary_and_the_day_range() {
        use sea_orm::QueryTrait;

        let sql = daily_perf_query(day(2026, 7, 20), day(2026, 7, 27), Some("lcp"), Some("web"))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(
                r#""metric" IN ('lcp', 'inp', 'cls', 'ttfb', 'fcp', 'app_start', 'screen_load')"#
            ),
            "{sql}"
        );
        assert!(sql.contains(r#""metric" = 'lcp'"#), "{sql}");
        assert!(sql.contains(r#""source" = 'web'"#), "{sql}");
        assert!(sql.contains(r#""TelemetryPerfDaily""#), "{sql}");
    }

    #[test]
    fn the_where_clause_pins_the_metric_vocabulary_and_numbers_placeholders() {
        let filters = PerfFilters {
            cutoff: ts(10, 0),
            metric: Some("lcp".to_string()),
            source: Some("web".to_string()),
            path: Some("/home".to_string()),
        };
        let (sql, values) = perf_where(&filters);

        assert_eq!(values.len(), 4);
        assert!(sql.contains(r#""createdAt" >= $1"#), "{sql}");
        assert!(
            sql.contains(
                r#""metric" IN ('lcp', 'inp', 'cls', 'ttfb', 'fcp', 'app_start', 'screen_load')"#
            ),
            "{sql}"
        );
        assert!(sql.contains(r#""metric" = $2"#), "{sql}");
        assert!(sql.contains(r#""source" = $3"#), "{sql}");
        assert!(sql.contains(r#""path" = $4"#), "{sql}");
    }
}
