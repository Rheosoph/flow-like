//! Aggregated telemetry stats for the admin dashboard.
//!
//! Windows of at most [`RAW_WINDOW_HOURS`] hours are answered from the raw
//! `TelemetryEvent` rows. Longer windows read the daily rollups instead, so a 90
//! day query stays a bounded aggregate over a few thousand rows rather than an
//! unbounded scan over every event ever ingested.

use super::{TOP_LIST_LIMIT, bucket_for};
use crate::entity::{
    telemetry_dimension_daily, telemetry_event, telemetry_event_daily, telemetry_install_daily,
};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{Duration, NaiveDateTime, NaiveTime, Utc};
use sea_orm::sea_query::ExprTrait;
use sea_orm::sea_query::{Alias, Expr, Func, Order as SeaOrder, Query as SeaQuery, SimpleExpr};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, FromQueryResult, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

/// Windows of at most this many hours are served from raw telemetry rows;
/// anything longer is served from the daily rollup tables. Sub-day buckets are
/// therefore only offered inside this window.
pub(super) const RAW_WINDOW_HOURS: i64 = 48;

pub(super) const GRANULARITY_RAW: &str = "raw";
pub(super) const GRANULARITY_DAILY: &str = "daily";

const DIMENSION_SOURCE: &str = "source";
const DIMENSION_PLATFORM: &str = "platform";
const DIMENSION_APP_VERSION: &str = "app_version";
const DIMENSION_COUNTRY: &str = "country";

const BACKEND_ANON_ID: &str = "backend";

pub(super) fn reads_raw(hours: i64) -> bool {
    hours <= RAW_WINDOW_HOURS
}

pub(super) fn granularity_for(hours: i64) -> &'static str {
    if reads_raw(hours) {
        GRANULARITY_RAW
    } else {
        GRANULARITY_DAILY
    }
}

/// Bucket granularity for a window, forced to whole UTC days outside the raw
/// window because the rollups store nothing finer.
pub(super) fn window_bucket(hours: i64, requested: Option<&str>) -> &'static str {
    if reads_raw(hours) {
        bucket_for(hours, requested)
    } else {
        "day"
    }
}

/// Inclusive UTC-midnight day range covering every day the window touches.
pub(super) fn day_window(now: NaiveDateTime, hours: i64) -> (NaiveDateTime, NaiveDateTime) {
    (
        (now - Duration::hours(hours))
            .date()
            .and_time(NaiveTime::MIN),
        now.date().and_time(NaiveTime::MIN),
    )
}

/// The equally long day range immediately preceding `start..=end`.
pub(super) fn previous_day_window(
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> (NaiveDateTime, NaiveDateTime) {
    let span = Duration::days((end - start).num_days() + 1);
    (start - span, start - Duration::days(1))
}

/// `SUM(column)` cast to BIGINT: Postgres widens `SUM` over 64 bit columns to
/// NUMERIC, which does not decode into `i64`.
pub(super) fn sum_i64<C: ColumnTrait>(column: C) -> SimpleExpr {
    Expr::col(column).sum().cast_as(Alias::new("BIGINT"))
}

pub(super) fn parse_retention_days(raw: Option<&str>, default_days: i64) -> i64 {
    std::cmp::Ord::max(
        raw.and_then(|value| value.trim().parse::<i64>().ok())
            .unwrap_or(default_days),
        1,
    )
}

/// How many days the sweeper still keeps raw rows for. Mirrors the sweeper's
/// env vars so read paths that no rollup can answer are able to report how far
/// back they actually reach instead of silently returning a partial window.
pub(super) fn retention_days(var: &str, default_days: i64) -> i64 {
    parse_retention_days(std::env::var(var).ok().as_deref(), default_days)
}

#[derive(Debug, FromQueryResult)]
struct GroupRow {
    key: Option<String>,
    cnt: i64,
    installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct DailyGroupRow {
    key: String,
    cnt: i64,
    installs: i64,
}

#[derive(Debug, FromQueryResult)]
struct ScalarCount {
    cnt: i64,
}

#[derive(Debug, FromQueryResult)]
struct ScalarSum {
    total: Option<i64>,
}

async fn group_counts<C: ConnectionTrait>(
    db: &C,
    column: telemetry_event::Column,
    cutoff: NaiveDateTime,
) -> Result<Vec<GroupRow>, ApiError> {
    let mut q = SeaQuery::select();
    q.from(telemetry_event::Entity)
        .expr_as(Expr::col(column), Alias::new("key"))
        .expr_as(
            Expr::col(telemetry_event::Column::Id).count(),
            Alias::new("cnt"),
        )
        .expr_as(
            Func::count_distinct(
                Expr::case(
                    Expr::col(telemetry_event::Column::AnonId).eq(BACKEND_ANON_ID),
                    sea_orm::Value::String(None),
                )
                .finally(Expr::col(telemetry_event::Column::AnonId)),
            ),
            Alias::new("installs"),
        )
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).gte(cutoff))
        .add_group_by([Expr::col(column).into()])
        .order_by_expr(
            Expr::col(telemetry_event::Column::Id).count(),
            SeaOrder::Desc,
        )
        .limit(TOP_LIST_LIMIT);

    let stmt = db.get_database_backend().build(&q);
    let rows = GroupRow::find_by_statement(stmt).all(db).await?;
    Ok(rows)
}

async fn active_installs<C: ConnectionTrait>(
    db: &C,
    cutoff: NaiveDateTime,
) -> Result<i64, ApiError> {
    let mut q = SeaQuery::select();
    q.from(telemetry_event::Entity)
        .expr_as(
            Expr::col(telemetry_event::Column::AnonId).count_distinct(),
            Alias::new("cnt"),
        )
        .and_where(Expr::col(telemetry_event::Column::CreatedAt).gte(cutoff))
        .and_where(Expr::col(telemetry_event::Column::AnonId).ne(BACKEND_ANON_ID));

    let stmt = db.get_database_backend().build(&q);
    let count = ScalarCount::find_by_statement(stmt)
        .one(db)
        .await?
        .map(|r| r.cnt)
        .unwrap_or(0);
    Ok(count)
}

async fn daily_event_total<C: ConnectionTrait>(
    db: &C,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<i64, ApiError> {
    Ok(telemetry_event_daily::Entity::find()
        .select_only()
        .column_as(sum_i64(telemetry_event_daily::Column::Count), "total")
        .filter(telemetry_event_daily::Column::Day.gte(start))
        .filter(telemetry_event_daily::Column::Day.lte(end))
        .into_model::<ScalarSum>()
        .one(db)
        .await?
        .and_then(|row| row.total)
        .unwrap_or(0))
}

async fn daily_top_events<C: ConnectionTrait>(
    db: &C,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<DailyGroupRow>, ApiError> {
    Ok(telemetry_event_daily::Entity::find()
        .select_only()
        .column_as(telemetry_event_daily::Column::Name, "key")
        .column_as(sum_i64(telemetry_event_daily::Column::Count), "cnt")
        .column_as(sum_i64(telemetry_event_daily::Column::Installs), "installs")
        .filter(telemetry_event_daily::Column::Day.gte(start))
        .filter(telemetry_event_daily::Column::Day.lte(end))
        .group_by(telemetry_event_daily::Column::Name)
        .order_by_desc(sum_i64(telemetry_event_daily::Column::Count))
        .limit(TOP_LIST_LIMIT)
        .into_model::<DailyGroupRow>()
        .all(db)
        .await?)
}

async fn daily_dimension<C: ConnectionTrait>(
    db: &C,
    dimension: &str,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<Vec<DailyGroupRow>, ApiError> {
    Ok(telemetry_dimension_daily::Entity::find()
        .select_only()
        .column_as(telemetry_dimension_daily::Column::Value, "key")
        .column_as(sum_i64(telemetry_dimension_daily::Column::Count), "cnt")
        .column_as(
            sum_i64(telemetry_dimension_daily::Column::Installs),
            "installs",
        )
        .filter(telemetry_dimension_daily::Column::Dimension.eq(dimension))
        .filter(telemetry_dimension_daily::Column::Day.gte(start))
        .filter(telemetry_dimension_daily::Column::Day.lte(end))
        .group_by(telemetry_dimension_daily::Column::Value)
        .order_by_desc(sum_i64(telemetry_dimension_daily::Column::Count))
        .limit(TOP_LIST_LIMIT)
        .into_model::<DailyGroupRow>()
        .all(db)
        .await?)
}

/// Distinct installs over the window, exact because `TelemetryInstallDaily`
/// stores one row per install per day rather than per event.
pub(super) async fn daily_active_installs<C: ConnectionTrait>(
    db: &C,
    start: NaiveDateTime,
    end: NaiveDateTime,
) -> Result<i64, ApiError> {
    Ok(telemetry_install_daily::Entity::find()
        .select_only()
        .column_as(
            Expr::col(telemetry_install_daily::Column::AnonId).count_distinct(),
            "cnt",
        )
        .filter(telemetry_install_daily::Column::Day.gte(start))
        .filter(telemetry_install_daily::Column::Day.lte(end))
        .filter(telemetry_install_daily::Column::AnonId.ne(BACKEND_ANON_ID))
        .into_model::<ScalarCount>()
        .one(db)
        .await?
        .map(|row| row.cnt)
        .unwrap_or(0))
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct TelemetryOverviewQuery {
    /// Lookback window in hours. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TopEventBucket {
    pub name: String,
    pub count: i64,
    pub installs: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SourceBucket {
    pub source: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlatformBucket {
    pub platform: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct VersionBucket {
    pub app_version: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CountryBucket {
    pub country: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetryOverviewResponse {
    pub hours: i64,
    /// "raw" when the numbers come from individual events, "daily" when they
    /// come from the daily rollups.
    pub granularity: String,
    pub total_events: i64,
    pub active_installs: i64,
    pub previous_total_events: i64,
    pub top_events: Vec<TopEventBucket>,
    pub sources: Vec<SourceBucket>,
    pub platforms: Vec<PlatformBucket>,
    pub versions: Vec<VersionBucket>,
    pub countries: Vec<CountryBucket>,
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/overview",
    tag = "admin",
    params(TelemetryOverviewQuery),
    responses(
        (status = 200, description = "Aggregated telemetry statistics for the admin dashboard", body = TelemetryOverviewResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Aggregate stats over anonymous telemetry: totals, active installs, top events, sources, platforms and versions. Windows of up to 48 hours are computed from individual events (granularity \"raw\"); longer windows are computed from the daily rollups (granularity \"daily\") and are therefore aligned to whole UTC days. Active installs are always an exact distinct count. In \"daily\" mode the per-event and per-dimension install counts are the sum of the daily distinct installs, an upper bound on the distinct installs across the whole window. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/overview", skip_all)]
pub async fn telemetry_overview(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TelemetryOverviewQuery>,
) -> Result<Json<TelemetryOverviewResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let hours = q.hours.unwrap_or(24).clamp(1, 24 * 90);
    let now = Utc::now().naive_utc();

    if reads_raw(hours) {
        let cutoff = now - Duration::hours(hours);
        let prev_cutoff = cutoff - Duration::hours(hours);

        let total_events = telemetry_event::Entity::find()
            .filter(telemetry_event::Column::CreatedAt.gte(cutoff))
            .count(&state.db)
            .await? as i64;

        let previous_total_events = telemetry_event::Entity::find()
            .filter(telemetry_event::Column::CreatedAt.gte(prev_cutoff))
            .filter(telemetry_event::Column::CreatedAt.lt(cutoff))
            .count(&state.db)
            .await? as i64;

        let active_installs = active_installs(&state.db, cutoff).await?;

        let top_events = group_counts(&state.db, telemetry_event::Column::Name, cutoff)
            .await?
            .into_iter()
            .map(|r| TopEventBucket {
                name: r.key.unwrap_or_default(),
                count: r.cnt,
                installs: r.installs,
            })
            .collect();

        let sources = group_counts(&state.db, telemetry_event::Column::Source, cutoff)
            .await?
            .into_iter()
            .map(|r| SourceBucket {
                source: r.key.unwrap_or_default(),
                count: r.cnt,
            })
            .collect();

        let platforms = group_counts(&state.db, telemetry_event::Column::Platform, cutoff)
            .await?
            .into_iter()
            .map(|r| PlatformBucket {
                platform: r.key.unwrap_or_else(|| "unknown".to_string()),
                count: r.cnt,
            })
            .collect();

        let versions = group_counts(&state.db, telemetry_event::Column::AppVersion, cutoff)
            .await?
            .into_iter()
            .map(|r| VersionBucket {
                app_version: r.key.unwrap_or_else(|| "unknown".to_string()),
                count: r.cnt,
            })
            .collect();

        let countries = group_counts(&state.db, telemetry_event::Column::Country, cutoff)
            .await?
            .into_iter()
            .map(|r| CountryBucket {
                country: r.key.unwrap_or_else(|| "unknown".to_string()),
                count: r.cnt,
            })
            .collect();

        return Ok(Json(TelemetryOverviewResponse {
            hours,
            granularity: GRANULARITY_RAW.to_string(),
            total_events,
            active_installs,
            previous_total_events,
            top_events,
            sources,
            platforms,
            versions,
            countries,
        }));
    }

    let (start, end) = day_window(now, hours);
    let (prev_start, prev_end) = previous_day_window(start, end);

    let total_events = daily_event_total(&state.db, start, end).await?;
    let previous_total_events = daily_event_total(&state.db, prev_start, prev_end).await?;
    let active_installs = daily_active_installs(&state.db, start, end).await?;

    let top_events = daily_top_events(&state.db, start, end)
        .await?
        .into_iter()
        .map(|r| TopEventBucket {
            name: r.key,
            count: r.cnt,
            installs: r.installs,
        })
        .collect();

    let sources = daily_dimension(&state.db, DIMENSION_SOURCE, start, end)
        .await?
        .into_iter()
        .map(|r| SourceBucket {
            source: r.key,
            count: r.cnt,
        })
        .collect();

    let platforms = daily_dimension(&state.db, DIMENSION_PLATFORM, start, end)
        .await?
        .into_iter()
        .map(|r| PlatformBucket {
            platform: r.key,
            count: r.cnt,
        })
        .collect();

    let versions = daily_dimension(&state.db, DIMENSION_APP_VERSION, start, end)
        .await?
        .into_iter()
        .map(|r| VersionBucket {
            app_version: r.key,
            count: r.cnt,
        })
        .collect();

    let countries = daily_dimension(&state.db, DIMENSION_COUNTRY, start, end)
        .await?
        .into_iter()
        .map(|r| CountryBucket {
            country: r.key,
            count: r.cnt,
        })
        .collect();

    Ok(Json(TelemetryOverviewResponse {
        hours,
        granularity: GRANULARITY_DAILY.to_string(),
        total_events,
        active_installs,
        previous_total_events,
        top_events,
        sources,
        platforms,
        versions,
        countries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn ts(y: i32, m: u32, d: u32, h: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(y, m, d)
            .unwrap()
            .and_hms_opt(h, 0, 0)
            .unwrap()
    }

    #[test]
    fn the_raw_window_boundary_is_inclusive() {
        assert!(reads_raw(1));
        assert!(reads_raw(47));
        assert!(reads_raw(48));
        assert!(!reads_raw(49));
        assert!(!reads_raw(24 * 90));
    }

    #[test]
    fn granularity_follows_the_boundary() {
        assert_eq!(granularity_for(47), GRANULARITY_RAW);
        assert_eq!(granularity_for(48), GRANULARITY_RAW);
        assert_eq!(granularity_for(49), GRANULARITY_DAILY);
    }

    #[test]
    fn sub_day_buckets_are_only_offered_inside_the_raw_window() {
        assert_eq!(window_bucket(6, None), "minute");
        assert_eq!(window_bucket(47, Some("minute")), "minute");
        assert_eq!(window_bucket(48, Some("hour")), "hour");
        assert_eq!(window_bucket(49, Some("minute")), "day");
        assert_eq!(window_bucket(49, Some("hour")), "day");
        assert_eq!(window_bucket(49, None), "day");
        assert_eq!(window_bucket(24 * 90, Some("minute")), "day");
    }

    #[test]
    fn the_day_window_covers_every_day_the_range_touches() {
        let now = ts(2026, 7, 27, 10);

        assert_eq!(
            day_window(now, 49),
            (ts(2026, 7, 25, 0), ts(2026, 7, 27, 0))
        );
        assert_eq!(
            day_window(now, 24 * 7),
            (ts(2026, 7, 20, 0), ts(2026, 7, 27, 0))
        );
    }

    #[test]
    fn retention_days_fall_back_to_the_sweeper_default_and_never_go_below_one() {
        assert_eq!(parse_retention_days(None, 30), 30);
        assert_eq!(parse_retention_days(Some(""), 30), 30);
        assert_eq!(parse_retention_days(Some("junk"), 90), 90);
        assert_eq!(parse_retention_days(Some(" 7 "), 30), 7);
        assert_eq!(parse_retention_days(Some("0"), 30), 1);
        assert_eq!(parse_retention_days(Some("-5"), 30), 1);
    }

    #[test]
    fn the_previous_day_window_is_equally_long_and_does_not_overlap() {
        let (start, end) = (ts(2026, 7, 25, 0), ts(2026, 7, 27, 0));
        let (prev_start, prev_end) = previous_day_window(start, end);

        assert_eq!(prev_end, ts(2026, 7, 24, 0));
        assert_eq!(prev_start, ts(2026, 7, 22, 0));
        assert_eq!((prev_end - prev_start).num_days(), (end - start).num_days());
    }
}
