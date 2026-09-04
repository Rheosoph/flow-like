//! Per-operation span statistics: the operations that dominate trace time.

use super::performance::{percentile_cont, round3};
use crate::entity::telemetry_span;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use crate::telemetry::percentiles_in_sql;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{Duration, NaiveDateTime, Utc};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::{IntoParams, ToSchema};

const ERROR_STATUS: &str = "error";
const DEFAULT_SPAN_STATS_HOURS: i64 = 24;
const MAX_SPAN_STATS_HOURS: i64 = 24 * 90;
/// Operations returned, ranked by the time they account for.
const SPAN_STATS_LIMIT: usize = 20;
/// Upper bound on the spans folded in Rust when the backend has no
/// `percentile_cont`.
const SPAN_ROW_CAP: u64 = 100_000;

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetrySpanStatsQuery {
    /// Lookback window in hours over the span start time. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Filter by source: "desktop", "web", "desktop_native" or "backend".
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SpanOperationStats {
    pub name: String,
    pub count: i64,
    pub p50: f64,
    pub p95: f64,
    /// Failed spans divided by all spans of this operation, between 0 and 1.
    pub error_rate: f64,
    /// Total milliseconds spent in this operation across the window.
    pub total_ms: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySpanStatsResponse {
    pub operations: Vec<SpanOperationStats>,
}

#[derive(Debug, FromQueryResult)]
struct SpanStatsRow {
    name: String,
    cnt: i64,
    p50: f64,
    p95: f64,
    errors: i64,
    total_ms: i64,
}

#[derive(Clone, Debug, FromQueryResult)]
struct SpanSampleRow {
    name: String,
    duration_ms: i32,
    status: String,
}

#[derive(Default)]
struct OperationAccumulator {
    durations: Vec<f64>,
    errors: i64,
    total_ms: i64,
}

fn error_rate(errors: i64, count: i64) -> f64 {
    if count <= 0 {
        return 0.0;
    }
    round3((errors as f64 / count as f64).clamp(0.0, 1.0))
}

fn rank_operations(mut operations: Vec<SpanOperationStats>) -> Vec<SpanOperationStats> {
    operations.sort_by(|a, b| {
        b.total_ms
            .cmp(&a.total_ms)
            .then_with(|| a.name.cmp(&b.name))
    });
    operations.truncate(SPAN_STATS_LIMIT);
    operations
}

fn fold_span_samples(rows: Vec<SpanSampleRow>) -> Vec<SpanOperationStats> {
    let mut per_name: HashMap<String, OperationAccumulator> = HashMap::new();

    for row in rows {
        let entry = per_name.entry(row.name).or_default();
        entry.durations.push(row.duration_ms as f64);
        entry.total_ms += row.duration_ms as i64;
        if row.status == ERROR_STATUS {
            entry.errors += 1;
        }
    }

    let operations = per_name
        .into_iter()
        .map(|(name, mut acc)| {
            acc.durations.sort_by(|a, b| a.total_cmp(b));
            let count = acc.durations.len() as i64;
            SpanOperationStats {
                name,
                count,
                p50: round3(percentile_cont(&acc.durations, 0.5)),
                p95: round3(percentile_cont(&acc.durations, 0.95)),
                error_rate: error_rate(acc.errors, count),
                total_ms: acc.total_ms,
            }
        })
        .collect();

    rank_operations(operations)
}

async fn span_stats_from_sql<C: ConnectionTrait>(
    db: &C,
    cutoff: NaiveDateTime,
    source: Option<&str>,
) -> Result<Vec<SpanOperationStats>, ApiError> {
    let backend = db.get_database_backend();
    let mut values: Vec<sea_orm::Value> = vec![cutoff.into()];
    let mut conditions = r#""startedAt" >= $1"#.to_string();

    if let Some(source) = source {
        values.push(source.to_string().into());
        conditions.push_str(&format!(r#" AND "source" = ${}"#, values.len()));
    }

    let sql = format!(
        r#"SELECT "name" AS name,
                  CAST(COUNT(*) AS BIGINT) AS cnt,
                  percentile_cont(0.5::float8) WITHIN GROUP (ORDER BY "durationMs"::float8) AS p50,
                  percentile_cont(0.95::float8) WITHIN GROUP (ORDER BY "durationMs"::float8) AS p95,
                  CAST(SUM(CASE WHEN "status" = '{ERROR_STATUS}' THEN 1 ELSE 0 END) AS BIGINT) AS errors,
                  CAST(SUM("durationMs") AS BIGINT) AS total_ms
           FROM "TelemetrySpan"
           WHERE {conditions}
           GROUP BY "name"
           ORDER BY total_ms DESC, name ASC
           LIMIT {SPAN_STATS_LIMIT}"#
    );

    let rows =
        SpanStatsRow::find_by_statement(Statement::from_sql_and_values(backend, sql, values))
            .all(db)
            .await?;

    Ok(rows
        .into_iter()
        .map(|row| SpanOperationStats {
            name: row.name,
            count: row.cnt,
            p50: round3(row.p50),
            p95: round3(row.p95),
            error_rate: error_rate(row.errors, row.cnt),
            total_ms: row.total_ms,
        })
        .collect())
}

async fn span_stats_from_fold<C: ConnectionTrait>(
    db: &C,
    cutoff: NaiveDateTime,
    source: Option<&str>,
) -> Result<Vec<SpanOperationStats>, ApiError> {
    let mut select = telemetry_span::Entity::find()
        .select_only()
        .column_as(telemetry_span::Column::Name, "name")
        .column_as(telemetry_span::Column::DurationMs, "duration_ms")
        .column_as(telemetry_span::Column::Status, "status")
        .filter(telemetry_span::Column::StartedAt.gte(cutoff));

    if let Some(source) = source {
        select = select.filter(telemetry_span::Column::Source.eq(source));
    }

    let rows = select
        .limit(SPAN_ROW_CAP)
        .into_model::<SpanSampleRow>()
        .all(db)
        .await?;

    if rows.len() as u64 == SPAN_ROW_CAP {
        tracing::warn!(
            cap = SPAN_ROW_CAP,
            "Span statistics fold hit the row cap; results are truncated"
        );
    }

    Ok(fold_span_samples(rows))
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/span-stats",
    tag = "admin",
    params(TelemetrySpanStatsQuery),
    responses(
        (status = 200, description = "The 20 operations accounting for the most trace time", body = TelemetrySpanStatsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Rank traced operations by the total time they account for, with call counts, p50/p95 latency and error rate. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/span-stats", skip_all)]
pub async fn telemetry_span_stats(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetrySpanStatsQuery>,
) -> Result<Json<TelemetrySpanStatsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q
        .hours
        .unwrap_or(DEFAULT_SPAN_STATS_HOURS)
        .clamp(1, MAX_SPAN_STATS_HOURS);
    let cutoff = Utc::now().naive_utc() - Duration::hours(hours);
    let source = q.source.as_deref().filter(|value| !value.is_empty());

    let operations = if percentiles_in_sql(state.db.get_database_backend(), state.db_dialect) {
        span_stats_from_sql(&state.db, cutoff, source).await?
    } else {
        span_stats_from_fold(&state.db, cutoff, source).await?
    };

    Ok(Json(TelemetrySpanStatsResponse { operations }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(name: &str, duration_ms: i32, status: &str) -> SpanSampleRow {
        SpanSampleRow {
            name: name.to_string(),
            duration_ms,
            status: status.to_string(),
        }
    }

    #[test]
    fn operations_are_ranked_by_the_time_they_account_for() {
        let rows = vec![
            sample("db.query", 10, "ok"),
            sample("db.query", 30, "ok"),
            sample("db.query", 20, "error"),
            sample("http.request", 500, "ok"),
        ];

        let operations = fold_span_samples(rows);

        assert_eq!(operations.len(), 2);
        assert_eq!(operations[0].name, "http.request");
        assert_eq!(operations[0].total_ms, 500);
        assert_eq!(operations[1].name, "db.query");
        assert_eq!(operations[1].count, 3);
        assert_eq!(operations[1].total_ms, 60);
        assert_eq!(operations[1].p50, 20.0);
        assert_eq!(operations[1].p95, 29.0);
        assert_eq!(operations[1].error_rate, 0.333);
    }

    #[test]
    fn error_rates_stay_inside_the_unit_interval() {
        assert_eq!(error_rate(0, 0), 0.0);
        assert_eq!(error_rate(0, 4), 0.0);
        assert_eq!(error_rate(4, 4), 1.0);
        assert_eq!(error_rate(1, 3), 0.333);
    }

    #[test]
    fn only_the_top_operations_by_total_time_are_returned() {
        let rows: Vec<SpanSampleRow> = (0..30)
            .map(|i| sample(&format!("op-{i:02}"), i + 1, "ok"))
            .collect();

        let operations = rank_operations(fold_span_samples(rows));

        assert_eq!(operations.len(), SPAN_STATS_LIMIT);
        assert_eq!(operations[0].name, "op-29");
        assert_eq!(operations[SPAN_STATS_LIMIT - 1].name, "op-10");
    }
}
