//! Aggregation granularity shared by the app analytics and sales stats
//! endpoints (`?period=day|week|month`).
//!
//! Both endpoints compute day rows first - from their `*Daily` rollup tables
//! when those exist, from the raw tables otherwise - and then fold those rows
//! into the requested bucket. Counts fold by summing, rates fold as weighted
//! averages, and distinct counts cannot fold at all: they are re-queried per
//! bucket with [`StatsPeriod::bucket_expr`].

use chrono::{Datelike, Duration, NaiveDate};
use sea_orm::DbBackend;
use sea_orm::sea_query::{Expr, SimpleExpr};

/// Bucket granularity requested via `?period=`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatsPeriod {
    #[default]
    Day,
    Week,
    Month,
}

impl StatsPeriod {
    /// Lenient allowlist parse, mirroring `admin::telemetry::bucket_for`:
    /// anything unrecognised falls back to the documented default, so a client
    /// sending a value we never supported keeps the response it gets today
    /// instead of starting to receive a 400.
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "week" => Self::Week,
            "month" => Self::Month,
            _ => Self::Day,
        }
    }

    /// First day of the bucket `date` falls in. Weeks start on Monday, matching
    /// `week_start` in `admin::telemetry::engagement` and Postgres'
    /// `date_trunc('week', ...)`.
    pub fn bucket_start(self, date: NaiveDate) -> NaiveDate {
        match self {
            Self::Day => date,
            Self::Week => date - Duration::days(date.weekday().num_days_from_monday() as i64),
            Self::Month => NaiveDate::from_ymd_opt(date.year(), date.month(), 1).unwrap_or(date),
        }
    }

    /// SQL truncating a timestamp column to this bucket, rendered `YYYY-MM-DD`
    /// so it parses back with `NaiveDate::parse_from_str` and agrees with
    /// [`StatsPeriod::bucket_start`] day for day. `column` is always a
    /// compile-time constant, never caller input.
    ///
    /// The column is `timestamptz`, so `to_char`/`date_trunc` would otherwise
    /// render it in the session `TimeZone`. The callers' window filters and
    /// bucket keys are UTC by construction, so `AT TIME ZONE 'UTC'` pins the
    /// SQL half to the same clock — without it a non-UTC session GUC labels a
    /// row with a day outside the requested range and it silently drops out.
    pub fn bucket_expr(self, backend: DbBackend, column: &str) -> SimpleExpr {
        match backend {
            DbBackend::Postgres => match self {
                Self::Day => Expr::cust(format!(
                    r#"to_char("{column}" AT TIME ZONE 'UTC', 'YYYY-MM-DD')"#
                )),
                Self::Week => Expr::cust(format!(
                    r#"to_char(date_trunc('week', "{column}" AT TIME ZONE 'UTC'), 'YYYY-MM-DD')"#
                )),
                Self::Month => Expr::cust(format!(
                    r#"to_char(date_trunc('month', "{column}" AT TIME ZONE 'UTC'), 'YYYY-MM-DD')"#
                )),
            },
            // SQLite; the MySQL fallback path is unused in this project.
            _ => match self {
                Self::Day => Expr::cust(format!("strftime('%Y-%m-%d', {column})")),
                // 'weekday 0' moves to the coming Sunday, staying put when the
                // date already is one, so -6 days lands on that week's Monday.
                Self::Week => Expr::cust(format!(
                    "strftime('%Y-%m-%d', {column}, 'weekday 0', '-6 days')"
                )),
                Self::Month => Expr::cust(format!("strftime('%Y-%m-01', {column})")),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::{PostgresQueryBuilder, Query, SqliteQueryBuilder};

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    /// Render just the bucket expression by selecting it on its own.
    fn render(period: StatsPeriod, backend: DbBackend) -> String {
        let mut select = Query::select();
        select.expr(period.bucket_expr(backend, "createdAt"));
        let sql = match backend {
            DbBackend::Postgres => select.to_string(PostgresQueryBuilder),
            _ => select.to_string(SqliteQueryBuilder),
        };
        sql.trim_start_matches("SELECT ").to_string()
    }

    #[test]
    fn unknown_periods_fall_back_to_day() {
        assert_eq!(StatsPeriod::parse("week"), StatsPeriod::Week);
        assert_eq!(StatsPeriod::parse("MONTH"), StatsPeriod::Month);
        assert_eq!(StatsPeriod::parse("  Week  "), StatsPeriod::Week);
        assert_eq!(StatsPeriod::parse("quarter"), StatsPeriod::Day);
        assert_eq!(StatsPeriod::parse(""), StatsPeriod::Day);
        assert_eq!(StatsPeriod::default(), StatsPeriod::Day);
    }

    #[test]
    fn weeks_start_on_monday_and_months_on_the_first() {
        let sunday = day(2026, 8, 23);
        let monday = day(2026, 8, 17);
        assert_eq!(StatsPeriod::Week.bucket_start(sunday), monday);
        assert_eq!(StatsPeriod::Week.bucket_start(monday), monday);
        assert_eq!(StatsPeriod::Month.bucket_start(sunday), day(2026, 8, 1));
        assert_eq!(StatsPeriod::Day.bucket_start(sunday), sunday);
    }

    #[test]
    fn week_buckets_cross_month_and_year_boundaries() {
        // Tue 2026-09-01 belongs to the week starting Mon 2026-08-31.
        assert_eq!(
            StatsPeriod::Week.bucket_start(day(2026, 9, 1)),
            day(2026, 8, 31)
        );
        // Fri 2027-01-01 belongs to the week starting Mon 2026-12-28.
        assert_eq!(
            StatsPeriod::Week.bucket_start(day(2027, 1, 1)),
            day(2026, 12, 28)
        );
        // Leap day resolves against its own month.
        assert_eq!(
            StatsPeriod::Month.bucket_start(day(2028, 2, 29)),
            day(2028, 2, 1)
        );
        assert_eq!(
            StatsPeriod::Week.bucket_start(day(2028, 2, 29)),
            day(2028, 2, 28)
        );
    }

    #[test]
    fn day_bucket_sql_is_unchanged_per_backend() {
        assert_eq!(
            render(StatsPeriod::Day, DbBackend::Postgres),
            r#"to_char("createdAt" AT TIME ZONE 'UTC', 'YYYY-MM-DD')"#
        );
        assert_eq!(
            render(StatsPeriod::Day, DbBackend::Sqlite),
            "strftime('%Y-%m-%d', createdAt)"
        );
    }

    /// The Rust half of every caller (`utc_midnight` window filters,
    /// `NaiveDate::parse_from_str` bucket keys) is UTC, so the SQL half must be
    /// too regardless of the session `TimeZone`.
    #[test]
    fn postgres_buckets_are_pinned_to_utc() {
        for period in [StatsPeriod::Day, StatsPeriod::Week, StatsPeriod::Month] {
            let sql = render(period, DbBackend::Postgres);
            assert!(
                sql.contains(r#""createdAt" AT TIME ZONE 'UTC'"#),
                "{period:?} bucket is not pinned to UTC: {sql}"
            );
        }
    }

    #[test]
    fn week_and_month_bucket_sql_truncates() {
        assert_eq!(
            render(StatsPeriod::Week, DbBackend::Postgres),
            r#"to_char(date_trunc('week', "createdAt" AT TIME ZONE 'UTC'), 'YYYY-MM-DD')"#
        );
        assert_eq!(
            render(StatsPeriod::Month, DbBackend::Postgres),
            r#"to_char(date_trunc('month', "createdAt" AT TIME ZONE 'UTC'), 'YYYY-MM-DD')"#
        );
        assert_eq!(
            render(StatsPeriod::Week, DbBackend::Sqlite),
            "strftime('%Y-%m-%d', createdAt, 'weekday 0', '-6 days')"
        );
        assert_eq!(
            render(StatsPeriod::Month, DbBackend::Sqlite),
            "strftime('%Y-%m-01', createdAt)"
        );
    }
}
