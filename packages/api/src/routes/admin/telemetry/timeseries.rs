//! Time-bucketed telemetry event counts.
//!
//! Windows of at most 48 hours are bucketed from the raw `TelemetryEvent` rows
//! and may use minute or hour buckets. Longer windows are read from
//! `TelemetryEventDaily` / `TelemetryInstallDaily`, which only store whole UTC
//! days, so the bucket is forced to "day" there.

use super::overview::{day_window, granularity_for, reads_raw, sum_i64, window_bucket};
use crate::entity::{telemetry_event_daily, telemetry_install_daily};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Select, Statement,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::{IntoParams, ToSchema};

const BACKEND_ANON_ID: &str = "backend";

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetryTimeseriesQuery {
    /// Lookback window in hours. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Bucket granularity: "minute", "hour", "day". Default chosen from the
    /// window. Sub-day buckets are ignored beyond 48 hours.
    #[serde(default)]
    pub bucket: Option<String>,
    /// Filter by exact event name.
    #[serde(default)]
    pub name: Option<String>,
    /// Filter by source.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryTimeseriesPoint {
    /// ISO-8601 timestamp at the start of the bucket.
    pub ts: String,
    pub count: i64,
    pub installs: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryTimeseriesResponse {
    pub bucket: String,
    /// "raw" when the points come from individual events, "daily" when they come
    /// from the daily rollups.
    pub granularity: String,
    pub points: Vec<TelemetryTimeseriesPoint>,
}

#[derive(Debug, FromQueryResult)]
struct TimeseriesRow {
    bucket: DateTime<FixedOffset>,
    cnt: i64,
    installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct DailyCountRow {
    day: DateTime<FixedOffset>,
    cnt: i64,
    installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct DailyInstallRow {
    day: DateTime<FixedOffset>,
    installs: i64,
}

fn non_empty(value: &Option<String>) -> Option<&str> {
    value.as_deref().filter(|value| !value.is_empty())
}

async fn raw_points<C: ConnectionTrait>(
    db: &C,
    cutoff: DateTime<FixedOffset>,
    bucket: &str,
    name: Option<&str>,
    source: Option<&str>,
) -> Result<Vec<TelemetryTimeseriesPoint>, ApiError> {
    let backend = db.get_database_backend();
    let mut values: Vec<sea_orm::Value> = vec![cutoff.into()];
    let mut name_param = None;
    let mut source_param = None;

    if let Some(name) = name {
        values.push(name.to_string().into());
        name_param = Some(values.len());
    }

    if let Some(source) = source {
        values.push(source.to_string().into());
        source_param = Some(values.len());
    }

    let sql = match backend {
        DbBackend::Postgres => {
            let mut conditions = r#""createdAt" >= $1"#.to_string();
            if let Some(idx) = name_param {
                conditions.push_str(&format!(r#" AND "name" = ${}"#, idx));
            }
            if let Some(idx) = source_param {
                conditions.push_str(&format!(r#" AND "source" = ${}"#, idx));
            }
            format!(
                r#"SELECT date_trunc('{bucket}', "createdAt" AT TIME ZONE 'UTC') AT TIME ZONE 'UTC' AS bucket,
                          COUNT(*) AS cnt,
                          COUNT(DISTINCT "anonId") FILTER (WHERE "anonId" <> '{BACKEND_ANON_ID}') AS installs
                   FROM "TelemetryEvent"
                   WHERE {conditions}
                   GROUP BY bucket
                   ORDER BY bucket ASC"#,
            )
        }
        _ => {
            let mut conditions = "created_at >= $1".to_string();
            if let Some(idx) = name_param {
                conditions.push_str(&format!(" AND name = ${}", idx));
            }
            if let Some(idx) = source_param {
                conditions.push_str(&format!(" AND source = ${}", idx));
            }
            format!(
                r#"SELECT date_trunc('{bucket}', created_at) AS bucket,
                          COUNT(*) AS cnt,
                          COUNT(DISTINCT CASE WHEN anon_id = '{BACKEND_ANON_ID}' THEN NULL ELSE anon_id END) AS installs
                   FROM telemetry_event
                   WHERE {conditions}
                   GROUP BY bucket
                   ORDER BY bucket ASC"#,
            )
        }
    };

    let stmt = Statement::from_sql_and_values(backend, sql, values);
    let rows = TimeseriesRow::find_by_statement(stmt).all(db).await?;

    Ok(rows
        .into_iter()
        .map(|r| TelemetryTimeseriesPoint {
            ts: r.bucket.to_rfc3339(),
            count: r.cnt,
            installs: r.installs,
        })
        .collect())
}

fn daily_counts_query(
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    name: Option<&str>,
    source: Option<&str>,
) -> Select<telemetry_event_daily::Entity> {
    let mut select = telemetry_event_daily::Entity::find()
        .select_only()
        .column_as(telemetry_event_daily::Column::Day, "day")
        .column_as(sum_i64(telemetry_event_daily::Column::Count), "cnt")
        .column_as(sum_i64(telemetry_event_daily::Column::Installs), "installs")
        .filter(telemetry_event_daily::Column::Day.gte(start))
        .filter(telemetry_event_daily::Column::Day.lte(end))
        .group_by(telemetry_event_daily::Column::Day)
        .order_by_asc(telemetry_event_daily::Column::Day);

    if let Some(name) = name {
        select = select.filter(telemetry_event_daily::Column::Name.eq(name));
    }
    if let Some(source) = source {
        select = select.filter(telemetry_event_daily::Column::Source.eq(source));
    }
    select
}

/// Exact distinct installs per day. Only usable without an event-name filter,
/// because `TelemetryInstallDaily` is not broken down by event.
fn daily_installs_query(
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    source: Option<&str>,
) -> Select<telemetry_install_daily::Entity> {
    let mut select = telemetry_install_daily::Entity::find()
        .select_only()
        .column_as(telemetry_install_daily::Column::Day, "day")
        .column_as(
            Expr::col(telemetry_install_daily::Column::AnonId).count_distinct(),
            "installs",
        )
        .filter(telemetry_install_daily::Column::Day.gte(start))
        .filter(telemetry_install_daily::Column::Day.lte(end))
        .filter(telemetry_install_daily::Column::AnonId.ne(BACKEND_ANON_ID))
        .group_by(telemetry_install_daily::Column::Day);

    if let Some(source) = source {
        select = select.filter(telemetry_install_daily::Column::Source.eq(source));
    }
    select
}

async fn daily_points<C: ConnectionTrait>(
    db: &C,
    start: DateTime<FixedOffset>,
    end: DateTime<FixedOffset>,
    name: Option<&str>,
    source: Option<&str>,
) -> Result<Vec<TelemetryTimeseriesPoint>, ApiError> {
    let counts = daily_counts_query(start, end, name, source)
        .into_model::<DailyCountRow>()
        .all(db)
        .await?;

    let mut points: BTreeMap<DateTime<FixedOffset>, (i64, i64)> = counts
        .into_iter()
        .map(|row| (row.day, (row.cnt, row.installs)))
        .collect();

    if name.is_none() {
        let installs = daily_installs_query(start, end, source)
            .into_model::<DailyInstallRow>()
            .all(db)
            .await?;
        for row in installs {
            points.entry(row.day).or_insert((0, 0)).1 = row.installs;
        }
    }

    Ok(points
        .into_iter()
        .map(|(day, (count, installs))| TelemetryTimeseriesPoint {
            ts: day.to_rfc3339(),
            count,
            installs,
        })
        .collect())
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/timeseries",
    tag = "admin",
    params(TelemetryTimeseriesQuery),
    responses(
        (status = 200, description = "Bucketed telemetry event counts for charts", body = TelemetryTimeseriesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Time-bucketed counts of anonymous telemetry events, optionally filtered by event name and source. Windows of up to 48 hours are bucketed from individual events (granularity \"raw\") and may use minute or hour buckets; longer windows are read from the daily rollups (granularity \"daily\"), which forces whole UTC day buckets. Without an event-name filter the daily install counts are exact distinct installs; with one they are the daily distinct installs recorded for that event. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/timeseries", skip_all)]
pub async fn telemetry_timeseries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetryTimeseriesQuery>,
) -> Result<Json<TelemetryTimeseriesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q.hours.unwrap_or(24).clamp(1, 24 * 90);
    let bucket = window_bucket(hours, q.bucket.as_deref());
    let now = Utc::now().fixed_offset();
    let name = non_empty(&q.name);
    let source = non_empty(&q.source);

    let points = if reads_raw(hours) {
        raw_points(
            &state.db,
            now - Duration::hours(hours),
            bucket,
            name,
            source,
        )
        .await?
    } else {
        let (start, end) = day_window(now, hours);
        daily_points(&state.db, start, end, name, source).await?
    };

    Ok(Json(TelemetryTimeseriesResponse {
        bucket: bucket.to_string(),
        granularity: granularity_for(hours).to_string(),
        points,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::admin::telemetry::overview::{GRANULARITY_DAILY, GRANULARITY_RAW};
    use chrono::NaiveDate;
    use sea_orm::QueryTrait;

    fn ts(y: i32, m: u32, d: u32) -> DateTime<FixedOffset> {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .fixed_offset()
    }

    #[test]
    fn the_bucket_is_forced_to_days_once_the_rollups_take_over() {
        assert_eq!(window_bucket(47, Some("minute")), "minute");
        assert_eq!(window_bucket(48, Some("minute")), "minute");
        assert_eq!(window_bucket(49, Some("minute")), "day");
        assert_eq!(granularity_for(48), GRANULARITY_RAW);
        assert_eq!(granularity_for(49), GRANULARITY_DAILY);
    }

    #[test]
    fn daily_counts_sum_the_rollup_and_keep_the_filters() {
        let sql = daily_counts_query(ts(2026, 7, 20), ts(2026, 7, 27), Some("page_view"), None)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(r#"CAST(SUM("count") AS BIGINT) AS "cnt""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#"CAST(SUM("installs") AS BIGINT) AS "installs""#),
            "{sql}"
        );
        assert!(sql.contains(r#""name" = 'page_view'"#), "{sql}");
        assert!(
            sql.contains(r#"GROUP BY "TelemetryEventDaily"."day""#),
            "{sql}"
        );
    }

    #[test]
    fn daily_installs_count_distinct_anon_ids_and_skip_the_backend() {
        let sql = daily_installs_query(ts(2026, 7, 20), ts(2026, 7, 27), Some("web"))
            .build(DbBackend::Postgres)
            .to_string();

        assert!(
            sql.contains(r#"COUNT(DISTINCT "anonId") AS "installs""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""TelemetryInstallDaily"."anonId" <> 'backend'"#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""TelemetryInstallDaily"."source" = 'web'"#),
            "{sql}"
        );
    }
}
