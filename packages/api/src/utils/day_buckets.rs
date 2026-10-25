//! Grouping rows by UTC day or month without grouping by `createdAt`.

use chrono::{Datelike, Duration, NaiveDate};
use sea_orm::{
    DbBackend, EntityTrait, QuerySelect, QueryTrait, Select,
    sea_query::{Alias, Expr, ExprTrait, JoinType, Query, SimpleExpr, ValueTuple},
};

use super::time::utc_midnight;

const BUCKET_SERIES: &str = "bucket_series";

/// The UTC days (or months) from a window's first day through its last, oldest
/// first. Queries group rows by the index of their bucket, joined against a
/// `VALUES` list of every index, never by an expression of `createdAt`: the
/// planner gives such an expression as many values as `createdAt` itself,
/// expects one group per row and sorts the whole window, which on DSQL runs
/// past the 128 MiB of temp space a query may use. A `VALUES` list has a known
/// size, so the aggregate stays hashed and sized by its real groups.
pub(crate) struct Buckets {
    backend: DbBackend,
    pub(crate) monthly: bool,
    start: NaiveDate,
    pub(crate) keys: Vec<String>,
}

impl Buckets {
    pub(crate) fn new(backend: DbBackend, start: NaiveDate, end: NaiveDate, monthly: bool) -> Self {
        let mut keys = Vec::new();
        if monthly {
            let (mut year, mut month) = (start.year(), start.month());
            while (year, month) <= (end.year(), end.month()) {
                keys.push(format!("{year:04}-{month:02}"));
                (year, month) = if month == 12 {
                    (year + 1, 1)
                } else {
                    (year, month + 1)
                };
            }
        } else {
            let mut day = start;
            while day <= end {
                keys.push(day.format("%Y-%m-%d").to_string());
                day += Duration::days(1);
            }
        }
        Self {
            backend,
            monthly,
            start,
            keys,
        }
    }

    fn created_at_index(&self) -> SimpleExpr {
        let sqlite = self.backend == DbBackend::Sqlite;
        if self.monthly {
            let origin = self.start.year() * 12 + self.start.month() as i32;
            Expr::cust(if sqlite {
                format!(
                    r#"CAST(strftime('%Y', "createdAt") AS INTEGER) * 12 + CAST(strftime('%m', "createdAt") AS INTEGER) - {origin}"#
                )
            } else {
                format!(
                    r#"CAST(date_part('year', "createdAt" AT TIME ZONE 'UTC') * 12 + date_part('month', "createdAt" AT TIME ZONE 'UTC') AS INTEGER) - {origin}"#
                )
            })
        } else {
            let origin = utc_midnight(self.start).timestamp();
            Expr::cust(if sqlite {
                format!(
                    r#"CAST((CAST(strftime('%s', "createdAt") AS INTEGER) - {origin}) / 86400 AS INTEGER)"#
                )
            } else {
                format!(
                    r#"CAST(FLOOR((date_part('epoch', "createdAt") - {origin}) / 86400) AS INTEGER)"#
                )
            })
        }
    }

    fn series_index() -> SimpleExpr {
        Expr::col((Alias::new(BUCKET_SERIES), Alias::new("i")))
    }

    /// Groups `select` by bucket, selecting the bucket's index as `alias`.
    pub(crate) fn group<E: EntityTrait>(&self, select: Select<E>, alias: &str) -> Select<E> {
        let indexes = (0..self.keys.len() as i32).map(|index| ValueTuple::One(index.into()));
        let series = Query::select()
            .expr_as(Expr::col(Alias::new("column1")), Alias::new("i"))
            .from_values(indexes, Alias::new("bucket_values"))
            .take();
        let mut select = select
            .expr_as(
                Self::series_index().cast_as(Alias::new("BIGINT")),
                alias.to_string(),
            )
            .group_by(Self::series_index());
        QueryTrait::query(&mut select).join_subquery(
            JoinType::InnerJoin,
            series,
            Alias::new(BUCKET_SERIES),
            Self::series_index().eq(self.created_at_index()),
        );
        select
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::user;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    fn render(buckets: &Buckets) -> String {
        buckets
            .group(user::Entity::find().select_only(), "bucket")
            .build(DbBackend::Postgres)
            .to_string()
    }

    #[test]
    fn buckets_span_every_day_or_month_through_the_last() {
        let days = Buckets::new(
            DbBackend::Postgres,
            day(2026, 9, 21),
            day(2026, 10, 21),
            false,
        );
        assert_eq!(days.keys.len(), 31);
        assert_eq!(days.keys.first().map(String::as_str), Some("2026-09-21"));
        assert_eq!(days.keys.last().map(String::as_str), Some("2026-10-21"));

        let months = Buckets::new(
            DbBackend::Postgres,
            day(2025, 10, 21),
            day(2026, 10, 21),
            true,
        );
        assert_eq!(months.keys.len(), 13);
        assert_eq!(months.keys[2], "2025-12");
        assert_eq!(months.keys[3], "2026-01");
    }

    /// DSQL caps a query at 128 MiB of temp space. Grouping by an expression
    /// of `createdAt` makes the planner sort every row of the window; the
    /// bucket must come from the joined list of indexes.
    #[test]
    fn daily_index_joins_the_bucket_list_instead_of_grouping_by_created_at() {
        let sql = render(&Buckets::new(
            DbBackend::Postgres,
            day(2026, 9, 21),
            day(2026, 9, 23),
            false,
        ));

        assert!(
            sql.contains(
                r#"INNER JOIN (SELECT "column1" AS "i" FROM (VALUES (0), (1), (2)) AS "bucket_values") AS "bucket_series""#
            ),
            "{sql}"
        );
        assert!(
            sql.contains(r#"FLOOR((date_part('epoch', "createdAt") - 1789948800) / 86400)"#),
            "{sql}"
        );
        assert!(sql.contains(r#"GROUP BY "bucket_series"."i""#), "{sql}");
        assert!(!sql.contains("to_char"), "{sql}");
    }

    #[test]
    fn monthly_index_counts_months_from_the_first_one() {
        let sql = render(&Buckets::new(
            DbBackend::Postgres,
            day(2025, 10, 21),
            day(2026, 10, 21),
            true,
        ));

        assert!(sql.contains("(11), (12)) AS \"bucket_values\""), "{sql}");
        assert!(sql.contains("AS INTEGER) - 24310"), "{sql}");
    }
}
