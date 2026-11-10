//! Executions for the admin overview. Closed days come from the daily rollup
//! (`execution_usage_rollup`). Raw executions are read only for the days a
//! window starts inside, which the rollup cannot split at the start instant,
//! and for the days it does not cover yet, normally just today. Every rolling
//! window stays exact while the raw scan shrinks from the whole window to a
//! few days.

use super::{PeriodExecutionUsageRow, RecentUsageRow, count_since, sum_bigint};
use crate::{
    entity::{execution_usage_daily, execution_usage_tracking},
    execution_usage_rollup::{first_unrolled_day, last_closed_day},
    utils::{day_buckets::Buckets, time::utc_midnight},
};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate};
use sea_orm::{
    ColumnTrait, DatabaseConnection, DbErr, EntityTrait, FromQueryResult, QueryFilter, QuerySelect,
    Select,
    sea_query::{Expr, ExprTrait},
};
use tracing::Instrument;

/// Executions of one UTC day, user, technical user and app, counted per window.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ExecutionDay {
    pub day: NaiveDate,
    pub user_id: Option<String>,
    pub technical_user_id: Option<String>,
    pub app_id: Option<String>,
    pub period_executions: i64,
    pub period_microseconds: i64,
    pub monthly: i64,
    pub weekly: i64,
    pub daily: i64,
    /// Only raw days know when their last execution ran.
    pub last_seen: Option<DateTime<FixedOffset>>,
}

/// The starts of every window the overview counts in.
#[derive(Clone, Copy, Debug)]
pub(super) struct Windows {
    pub period: DateTime<FixedOffset>,
    pub monthly: DateTime<FixedOffset>,
    pub weekly: DateTime<FixedOffset>,
    pub daily: DateTime<FixedOffset>,
}

impl Windows {
    fn starts(&self) -> [DateTime<FixedOffset>; 4] {
        [self.period, self.monthly, self.weekly, self.daily]
    }

    fn earliest(&self) -> DateTime<FixedOffset> {
        self.starts().into_iter().min().unwrap_or(self.period)
    }

    fn split_days(&self) -> Vec<NaiveDate> {
        let mut days: Vec<NaiveDate> = self.starts().iter().map(utc_day).collect();
        days.sort_unstable();
        days.dedup();
        days
    }
}

fn utc_day(at: &DateTime<FixedOffset>) -> NaiveDate {
    at.naive_utc().date()
}

#[derive(Debug, FromQueryResult)]
struct RawDay {
    bucket: i64,
    user_id: Option<String>,
    technical_user_id: Option<String>,
    app_id: Option<String>,
    period_executions: i64,
    period_microseconds: i64,
    monthly: i64,
    weekly: i64,
    daily: i64,
    last_seen: DateTime<FixedOffset>,
}

impl RawDay {
    fn on(self, first: NaiveDate) -> ExecutionDay {
        ExecutionDay {
            day: first + Duration::days(self.bucket),
            user_id: self.user_id,
            technical_user_id: self.technical_user_id,
            app_id: self.app_id,
            period_executions: self.period_executions,
            period_microseconds: self.period_microseconds,
            monthly: self.monthly,
            weekly: self.weekly,
            daily: self.daily,
            last_seen: Some(self.last_seen),
        }
    }
}

#[derive(Debug, FromQueryResult)]
struct RolledDay {
    day: NaiveDate,
    user_id: Option<String>,
    technical_user_id: Option<String>,
    app_id: Option<String>,
    executions: i64,
    microseconds: i64,
}

impl RolledDay {
    /// A rolled day is never a split day, so it lies wholly inside or wholly
    /// before each window.
    fn within(self, windows: &Windows) -> ExecutionDay {
        let counts = |start: DateTime<FixedOffset>, value: i64| {
            if self.day > utc_day(&start) { value } else { 0 }
        };
        ExecutionDay {
            period_executions: counts(windows.period, self.executions),
            period_microseconds: counts(windows.period, self.microseconds),
            monthly: counts(windows.monthly, self.executions),
            weekly: counts(windows.weekly, self.executions),
            daily: counts(windows.daily, self.executions),
            day: self.day,
            user_id: self.user_id,
            technical_user_id: self.technical_user_id,
            app_id: self.app_id,
            last_seen: None,
        }
    }
}

/// Runs of consecutive days in `first..=today` read from raw executions,
/// inclusive at both ends.
fn raw_ranges(
    first: NaiveDate,
    today: NaiveDate,
    live_from: NaiveDate,
    split_days: &[NaiveDate],
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut ranges: Vec<(NaiveDate, NaiveDate)> = Vec::new();
    for day in first.iter_days().take_while(|day| *day <= today) {
        if day < live_from && !split_days.contains(&day) {
            continue;
        }
        match ranges.last_mut() {
            Some((_, last)) if *last + Duration::days(1) == day => *last = day,
            _ => ranges.push((day, day)),
        }
    }
    ranges
}

fn raw_select(
    days: &Buckets,
    from: DateTime<FixedOffset>,
    until: DateTime<FixedOffset>,
    windows: &Windows,
) -> Select<execution_usage_tracking::Entity> {
    use execution_usage_tracking::Column;

    let since = |start| count_since(Column::Id, Column::CreatedAt, start);
    days.group(
        execution_usage_tracking::Entity::find().select_only(),
        "bucket",
    )
    .column_as(Column::UserId, "user_id")
    .column_as(Column::TechnicalUserId, "technical_user_id")
    .column_as(Column::AppId, "app_id")
    .expr_as(since(windows.period), "period_executions")
    .expr_as(
        sum_bigint(Expr::expr(Expr::case(
            Column::CreatedAt.gte(windows.period),
            Expr::col(Column::Microseconds),
        ))),
        "period_microseconds",
    )
    .expr_as(since(windows.monthly), "monthly")
    .expr_as(since(windows.weekly), "weekly")
    .expr_as(since(windows.daily), "daily")
    .expr_as(Expr::col(Column::CreatedAt).max(), "last_seen")
    .filter(Column::CreatedAt.gte(from))
    .filter(Column::CreatedAt.lt(until))
    .group_by(Column::UserId)
    .group_by(Column::TechnicalUserId)
    .group_by(Column::AppId)
}

fn rolled_select(
    first: NaiveDate,
    until: NaiveDate,
    split_days: &[NaiveDate],
) -> Select<execution_usage_daily::Entity> {
    use execution_usage_daily::Column;

    execution_usage_daily::Entity::find()
        .select_only()
        .column(Column::Day)
        .column_as(Column::UserId, "user_id")
        .column_as(Column::TechnicalUserId, "technical_user_id")
        .column_as(Column::AppId, "app_id")
        .expr_as(sum_bigint(Expr::col(Column::Executions)), "executions")
        .expr_as(sum_bigint(Expr::col(Column::Microseconds)), "microseconds")
        .filter(Column::Day.gte(first))
        .filter(Column::Day.lt(until))
        .filter(Column::Day.is_not_in(split_days.iter().copied()))
        .group_by(Column::Day)
        .group_by(Column::UserId)
        .group_by(Column::TechnicalUserId)
        .group_by(Column::AppId)
}

async fn rolled_days(
    db: &DatabaseConnection,
    first: NaiveDate,
    until: NaiveDate,
    split_days: &[NaiveDate],
) -> Result<Vec<RolledDay>, DbErr> {
    if until <= first {
        return Ok(Vec::new());
    }
    rolled_select(first, until, split_days)
        .into_model::<RolledDay>()
        .all(db)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "admin.usage.executions.rolled",
            db.operation = "select",
            db.table = "ExecutionUsageDaily",
        ))
        .await
}

async fn raw_days(
    db: &DatabaseConnection,
    (first, last): (NaiveDate, NaiveDate),
    windows: &Windows,
) -> Result<Vec<ExecutionDay>, DbErr> {
    let days = Buckets::new(db.get_database_backend(), first, last, false);
    let from = Ord::max(utc_midnight(first), windows.earliest());
    let until = utc_midnight(last + Duration::days(1));
    let rows = raw_select(&days, from, until, windows)
        .into_model::<RawDay>()
        .all(db)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "admin.usage.executions.raw",
            db.operation = "select",
            db.table = "ExecutionUsageTracking",
        ))
        .await?;
    Ok(rows.into_iter().map(|row| row.on(first)).collect())
}

/// Every execution since the earliest window start, one entry per day, user,
/// technical user and app and per source.
pub(super) async fn load(
    db: &DatabaseConnection,
    windows: &Windows,
    now: DateTime<FixedOffset>,
) -> Result<Vec<ExecutionDay>, DbErr> {
    let first = utc_day(&windows.earliest());
    let split_days = windows.split_days();
    let live_from = first_unrolled_day(db, first, last_closed_day(now)).await;
    let raw = raw_ranges(first, utc_day(&now), live_from, &split_days)
        .into_iter()
        .map(|range| raw_days(db, range, windows));

    let (rolled, raw) = flow_like_types::tokio::try_join!(
        rolled_days(db, first, live_from, &split_days),
        futures::future::try_join_all(raw),
    )?;
    Ok(rolled
        .into_iter()
        .map(|row| row.within(windows))
        .chain(raw.into_iter().flatten())
        .collect())
}

pub(super) fn period_rows(
    days: &[ExecutionDay],
    buckets: &Buckets,
) -> Vec<PeriodExecutionUsageRow> {
    days.iter()
        .filter(|day| day.period_executions > 0)
        .filter_map(|day| {
            Some(PeriodExecutionUsageRow {
                bucket: buckets.index_of(day.day)?,
                user_id: day.user_id.clone(),
                technical_user_id: day.technical_user_id.clone(),
                app_id: day.app_id.clone(),
                executions: day.period_executions,
                microseconds: day.period_microseconds,
            })
        })
        .collect()
}

pub(super) fn recent_rows(days: &[ExecutionDay], recent_days: &Buckets) -> Vec<RecentUsageRow> {
    days.iter()
        .filter(|day| day.monthly > 0)
        .filter_map(|day| {
            Some(RecentUsageRow {
                day: recent_days.index_of(day.day)?,
                user_id: day.user_id.clone(),
                app_id: day.app_id.clone(),
                interactions: day.monthly,
                daily_interactions: day.daily,
                weekly_interactions: day.weekly,
                price: 0,
                tokens: 0,
                last_seen: day.last_seen,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};

    fn day(month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, month, day).expect("valid date")
    }

    fn at(month: u32, d: u32, hour: u32) -> DateTime<FixedOffset> {
        day(month, d)
            .and_hms_opt(hour, 0, 0)
            .expect("valid time")
            .and_utc()
            .fixed_offset()
    }

    fn monthly_windows() -> Windows {
        Windows {
            period: at(10, 11, 14),
            monthly: at(10, 11, 14),
            weekly: at(11, 3, 14),
            daily: at(11, 9, 14),
        }
    }

    #[test]
    fn raw_ranges_cover_split_days_and_the_unrolled_tail_only() {
        let windows = monthly_windows();
        let ranges = raw_ranges(day(10, 11), day(11, 10), day(11, 10), &windows.split_days());
        assert_eq!(
            ranges,
            [
                (day(10, 11), day(10, 11)),
                (day(11, 3), day(11, 3)),
                (day(11, 9), day(11, 10)),
            ]
        );
    }

    #[test]
    fn raw_ranges_fall_back_to_raw_from_the_first_unrolled_day() {
        let windows = monthly_windows();
        let ranges = raw_ranges(day(10, 11), day(11, 10), day(11, 1), &windows.split_days());
        assert_eq!(
            ranges,
            [(day(10, 11), day(10, 11)), (day(11, 1), day(11, 10))]
        );
    }

    #[test]
    fn rolled_days_count_only_in_the_windows_they_lie_inside() {
        let windows = monthly_windows();
        let rolled = |d: NaiveDate| {
            RolledDay {
                day: d,
                user_id: Some("u".into()),
                technical_user_id: None,
                app_id: None,
                executions: 4,
                microseconds: 40,
            }
            .within(&windows)
        };

        let old = rolled(day(10, 20));
        assert_eq!(
            (old.period_executions, old.monthly, old.weekly, old.daily),
            (4, 4, 0, 0)
        );
        let recent = rolled(day(11, 5));
        assert_eq!(
            (
                recent.period_microseconds,
                recent.monthly,
                recent.weekly,
                recent.daily
            ),
            (40, 4, 4, 0)
        );
        assert_eq!(recent.last_seen, None);
    }

    /// See `utils::day_buckets`: raw rows must take their bucket from the
    /// joined series, or DSQL sorts the whole window.
    #[test]
    fn raw_rows_group_by_the_bucket_series_not_by_created_at() {
        let windows = monthly_windows();
        let buckets = Buckets::new(DbBackend::Postgres, day(11, 9), day(11, 10), false);
        let sql = raw_select(&buckets, at(11, 9, 0), at(11, 11, 0), &windows)
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#"AS "bucket_series" ON"#), "{sql}");
        assert!(
            sql.contains(r#"GROUP BY "bucket_series"."i", "ExecutionUsageTracking"."userId""#),
            "{sql}"
        );
        assert!(!sql.contains("to_char"), "{sql}");
    }

    #[test]
    fn rolled_reads_skip_split_days() {
        let windows = monthly_windows();
        let sql = rolled_select(day(10, 11), day(11, 10), &windows.split_days())
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#""day" NOT IN ("#), "{sql}");
        assert!(sql.contains("'2026-11-03'"), "{sql}");
    }
}
