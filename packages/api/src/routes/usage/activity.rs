//! Period-scoped execution activity for the signed-in account.
//!
//! `GET /usage/executions` can only answer "the newest N records"; a home
//! widget asking for "the last 7 days" cannot be built from that page without
//! silently dropping every record the page cut off. This endpoint counts the
//! whole window in SQL instead, so the buckets, the per-app split and the
//! totals describe the period rather than the page size.
//!
//! Closed days come from the daily rollup that maintenance keeps
//! (`execution_usage_rollup`); only the days it does not cover yet, normally
//! just today, are counted from raw executions.

use crate::{
    entity::{
        execution_usage_daily, execution_usage_tracking, sea_orm_active_enums::ExecutionStatus,
    },
    error::ApiError,
    execution_usage_rollup::{first_unrolled_day, last_closed_day},
    middleware::jwt::AppUser,
    state::AppState,
    utils::{day_buckets::Buckets, time::utc_midnight},
};
use axum::{
    Extension, Json,
    extract::{Query, State},
    http::HeaderMap,
};
use chrono::{DateTime, Duration, FixedOffset, NaiveDate, Utc};
use sea_orm::{
    ColumnTrait, DatabaseConnection, EntityTrait, FromQueryResult, QueryFilter, QueryOrder,
    QuerySelect, Select,
    sea_query::{Alias, Expr, ExprTrait, SimpleExpr},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::Instrument;
use utoipa::{IntoParams, ToSchema};

use super::history::ExecutionUsageRecord;

/// Newest flagged records returned alongside the counts, so a caller listing
/// them does not have to re-read the newest page and miss every flagged record
/// that page cut off. Matches the highest item count a home widget can ask for.
const ATTENTION_SAMPLE: u64 = 50;
const MAX_DAYS: u32 = 90;

#[derive(Clone, Debug, Deserialize, IntoParams)]
pub struct ActivityParams {
    /// Length of the window in UTC days, counting today. Clamped to 1..=90.
    #[serde(default = "default_days")]
    pub days: u32,
    pub app_id: Option<String>,
}

fn default_days() -> u32 {
    7
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ExecutionActivityBucket {
    /// UTC calendar day, `YYYY-MM-DD`.
    pub day: String,
    pub count: i64,
    pub attention_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ExecutionActivityApp {
    /// `None` for records recorded without an app.
    pub app_id: Option<String>,
    pub count: i64,
    pub attention_count: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ExecutionActivityResponse {
    pub days: u32,
    /// Inclusive window start, midnight UTC of the first day.
    pub from: String,
    /// Instant the window was evaluated at. Snapshots are reused for up to five minutes.
    pub to: String,
    /// One entry per UTC day in the window, oldest first, zeros included.
    pub buckets: Vec<ExecutionActivityBucket>,
    /// Every app with records in the window, busiest first. Not truncated.
    pub apps: Vec<ExecutionActivityApp>,
    /// Records in the window. Counted in SQL, never a page size.
    pub total: i64,
    /// Records in the window with Error or Fatal severity.
    pub attention_total: i64,
    /// Mean recorded duration over the window, `None` when it holds no records.
    pub average_microseconds: Option<f64>,
    /// Newest flagged records in the window, capped for display.
    pub attention: Vec<ExecutionUsageRecord>,
}

/// Executions of one UTC day, app and severity, from either source.
struct ActivityCount {
    day: String,
    app_id: Option<String>,
    status: ExecutionStatus,
    cnt: i64,
    total_microseconds: i64,
}

#[derive(Debug, FromQueryResult)]
struct LiveCount {
    bucket: i64,
    app_id: Option<String>,
    status: ExecutionStatus,
    cnt: i64,
    total_microseconds: i64,
}

impl LiveCount {
    fn on(self, days: &Buckets) -> Option<ActivityCount> {
        let day = days.keys.get(usize::try_from(self.bucket).ok()?)?.clone();
        Some(ActivityCount {
            day,
            app_id: self.app_id,
            status: self.status,
            cnt: self.cnt,
            total_microseconds: self.total_microseconds,
        })
    }
}

#[derive(Debug, FromQueryResult)]
struct RolledCount {
    day: NaiveDate,
    app_id: Option<String>,
    status: ExecutionStatus,
    executions: i64,
    microseconds: i64,
}

impl From<RolledCount> for ActivityCount {
    fn from(row: RolledCount) -> Self {
        Self {
            day: row.day.format("%Y-%m-%d").to_string(),
            app_id: row.app_id,
            status: row.status,
            cnt: row.executions,
            total_microseconds: row.microseconds,
        }
    }
}

type ActivitySelect = Select<execution_usage_tracking::Entity>;

/// One scan of the days the rollup does not cover supplies both the daily and
/// per-app totals, including null app ids.
///
/// `total_microseconds` is cast because Postgres widens `SUM` over a 64 bit
/// column to NUMERIC, which never decodes into the `i64` it lands in.
fn live_select(base: ActivitySelect, days: &Buckets) -> ActivitySelect {
    days.group(base.select_only(), "bucket")
        .column_as(Expr::col(execution_usage_tracking::Column::AppId), "app_id")
        .column(execution_usage_tracking::Column::Status)
        .expr_as(
            Expr::col(execution_usage_tracking::Column::Id).count(),
            "cnt",
        )
        .expr_as(
            sum_i64(execution_usage_tracking::Column::Microseconds),
            "total_microseconds",
        )
        .group_by(Expr::col(execution_usage_tracking::Column::AppId))
        .group_by(Expr::col(execution_usage_tracking::Column::Status))
}

/// The account's rolled-up days in `from..until`, read from the
/// `(userId, day)` covering index alone.
fn rolled_select(
    sub: &str,
    from: NaiveDate,
    until: NaiveDate,
    app_id: Option<&str>,
) -> Select<execution_usage_daily::Entity> {
    use execution_usage_daily::Column;

    let query = execution_usage_daily::Entity::find()
        .select_only()
        .column(Column::Day)
        .column_as(Column::AppId, "app_id")
        .column(Column::Status)
        .column(Column::Executions)
        .column(Column::Microseconds)
        .filter(Column::UserId.eq(sub))
        .filter(Column::Day.gte(from))
        .filter(Column::Day.lt(until));
    match app_id {
        Some(app_id) => query.filter(Column::AppId.eq(app_id)),
        None => query,
    }
}

async fn rolled_counts(
    db: &DatabaseConnection,
    sub: &str,
    from: NaiveDate,
    until: NaiveDate,
    app_id: Option<&str>,
) -> Result<Vec<RolledCount>, sea_orm::DbErr> {
    if until <= from {
        return Ok(Vec::new());
    }
    rolled_select(sub, from, until, app_id)
        .into_model::<RolledCount>()
        .all(db)
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "usage.executions.activity.rolled",
            db.operation = "select",
            db.table = "ExecutionUsageDaily",
        ))
        .await
}

/// `SUM(column)` cast to BIGINT: Postgres widens `SUM` over 64 bit columns to
/// NUMERIC, which does not decode into `i64`. Mirrors `admin::telemetry`'s
/// helper, which is private to that module.
fn sum_i64<C: ColumnTrait>(column: C) -> SimpleExpr {
    Expr::col(column).sum().cast_as(Alias::new("BIGINT"))
}

fn is_attention(status: &ExecutionStatus) -> bool {
    matches!(status, ExecutionStatus::Error | ExecutionStatus::Fatal)
}

/// Zero-filled day keys for the window, oldest first.
fn window_days(today: NaiveDate, days: u32) -> Vec<NaiveDate> {
    let span = i64::from(days.saturating_sub(1));
    (0..=span)
        .map(|back| today - Duration::days(span - back))
        .collect()
}

/// Fold `(count, attention_count)` pairs keyed by whatever the query grouped on.
fn fold_counts<K: std::hash::Hash + Eq>(
    rows: impl IntoIterator<Item = (K, ExecutionStatus, i64)>,
) -> HashMap<K, (i64, i64)> {
    let mut folded: HashMap<K, (i64, i64)> = HashMap::new();
    for (key, status, count) in rows {
        let entry = folded.entry(key).or_insert((0, 0));
        entry.0 += count;
        if is_attention(&status) {
            entry.1 += count;
        }
    }
    folded
}

#[utoipa::path(
    get,
    path = "/usage/executions/activity",
    tag = "usage",
    params(ActivityParams),
    responses(
        (status = 200, description = "Execution records per UTC day and per app for the signed-in account", body = ExecutionActivityResponse)
    )
)]
#[tracing::instrument(name = "GET /usage/executions/activity", skip_all)]
pub async fn get_execution_activity(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(mut params): Query<ActivityParams>,
    headers: HeaderMap,
) -> Result<Json<ExecutionActivityResponse>, ApiError> {
    let sub = user.sub()?;
    params.days = params.days.clamp(1, MAX_DAYS);
    let now = Utc::now().fixed_offset();
    let key = super::cache::cache_key((
        "activity",
        &sub,
        &params.app_id,
        params.days,
        now.date_naive(),
    ));
    state
        .execution_usage_cache
        .get_or_compute(
            &state.cache,
            key,
            super::cache::CacheMode::from_headers(&headers),
            execution_activity(&state.db, &sub, params, now),
        )
        .await
        .map(Json)
}

async fn execution_activity(
    db: &DatabaseConnection,
    sub: &str,
    params: ActivityParams,
    now: DateTime<FixedOffset>,
) -> Result<ExecutionActivityResponse, ApiError> {
    let days = params.days.clamp(1, MAX_DAYS);
    let today = now.date_naive();
    let keys = window_days(today, days);
    let first = keys[0];
    let from = utc_midnight(first);
    let live_from = first_unrolled_day(db, first, last_closed_day(now)).await;
    let live_days = Buckets::new(db.get_database_backend(), live_from, today, false);

    let base = |from: DateTime<FixedOffset>| {
        let mut query = execution_usage_tracking::Entity::find()
            .filter(execution_usage_tracking::Column::UserId.eq(sub))
            .filter(execution_usage_tracking::Column::CreatedAt.gte(from))
            .filter(execution_usage_tracking::Column::CreatedAt.lte(now.fixed_offset()));
        if let Some(ref app_id) = params.app_id {
            query = query.filter(execution_usage_tracking::Column::AppId.eq(app_id));
        }
        query
    };
    let (rolled, live, attention) = flow_like_types::tokio::try_join!(
        rolled_counts(db, sub, first, live_from, params.app_id.as_deref()),
        live_select(base(utc_midnight(live_from)), &live_days)
            .into_model::<LiveCount>()
            .all(db)
            .instrument(tracing::info_span!(
                target: "flow_like::observability",
                "usage.executions.activity",
                db.operation = "select",
                db.table = "ExecutionUsageTracking",
            )),
        base(from)
            .filter(
                execution_usage_tracking::Column::Status
                    .is_in([ExecutionStatus::Error, ExecutionStatus::Fatal]),
            )
            .order_by_desc(execution_usage_tracking::Column::CreatedAt)
            .order_by_desc(execution_usage_tracking::Column::Id)
            .limit(ATTENTION_SAMPLE)
            .all(db)
            .instrument(tracing::info_span!(
                target: "flow_like::observability",
                "usage.executions.attention",
                db.operation = "select",
                db.table = "ExecutionUsageTracking",
            )),
    )
    .map_err(|e| ApiError::internal_error(e.into()))?;

    let rows: Vec<ActivityCount> = rolled
        .into_iter()
        .map(ActivityCount::from)
        .chain(live.into_iter().filter_map(|row| row.on(&live_days)))
        .collect();
    let total_microseconds: i64 = rows.iter().map(|row| row.total_microseconds).sum();
    let buckets = day_buckets(&keys, &rows);
    let total: i64 = buckets.iter().map(|bucket| bucket.count).sum();
    let attention_total: i64 = buckets.iter().map(|bucket| bucket.attention_count).sum();

    Ok(ExecutionActivityResponse {
        days,
        from: from.to_rfc3339(),
        to: now.fixed_offset().to_rfc3339(),
        buckets,
        apps: busiest_apps(rows),
        total,
        attention_total,
        average_microseconds: (total > 0).then(|| total_microseconds as f64 / total as f64),
        attention: attention
            .into_iter()
            .map(ExecutionUsageRecord::from)
            .collect(),
    })
}

/// One bucket per window day, oldest first, zeros included.
fn day_buckets(keys: &[NaiveDate], rows: &[ActivityCount]) -> Vec<ExecutionActivityBucket> {
    let per_day = fold_counts(
        rows.iter()
            .map(|row| (row.day.clone(), row.status.clone(), row.cnt)),
    );
    keys.iter()
        .map(|day| {
            let key = day.format("%Y-%m-%d").to_string();
            let (count, attention_count) = per_day.get(&key).copied().unwrap_or((0, 0));
            ExecutionActivityBucket {
                day: key,
                count,
                attention_count,
            }
        })
        .collect()
}

fn busiest_apps(rows: Vec<ActivityCount>) -> Vec<ExecutionActivityApp> {
    let mut apps: Vec<ExecutionActivityApp> = fold_counts(
        rows.into_iter()
            .map(|row| (row.app_id, row.status, row.cnt)),
    )
    .into_iter()
    .map(|(app_id, (count, attention_count))| ExecutionActivityApp {
        app_id,
        count,
        attention_count,
    })
    .collect();
    apps.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.app_id.cmp(&b.app_id)));
    apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::DbDialect,
        execution_usage_rollup::{RollupSettings, maintain_with},
    };
    use sea_orm::{DbBackend, QueryTrait};

    fn today_only() -> Buckets {
        Buckets::new(
            DbBackend::Postgres,
            day(2026, 9, 11),
            day(2026, 9, 11),
            false,
        )
    }

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
    }

    #[test]
    fn window_covers_the_requested_days_inclusive_of_today() {
        let keys = window_days(day(2026, 9, 11), 7);
        assert_eq!(keys.len(), 7);
        assert_eq!(keys[0], day(2026, 9, 5));
        assert_eq!(keys[6], day(2026, 9, 11));
    }

    #[test]
    fn a_single_day_window_is_today_only() {
        assert_eq!(window_days(day(2026, 9, 11), 1), vec![day(2026, 9, 11)]);
    }

    #[test]
    fn windows_cross_month_boundaries() {
        let keys = window_days(day(2026, 9, 2), 7);
        assert_eq!(keys[0], day(2026, 8, 27));
        assert_eq!(keys.len(), 7);
    }

    #[test]
    fn only_error_and_fatal_are_flagged() {
        assert!(is_attention(&ExecutionStatus::Error));
        assert!(is_attention(&ExecutionStatus::Fatal));
        assert!(!is_attention(&ExecutionStatus::Warn));
        assert!(!is_attention(&ExecutionStatus::Info));
        assert!(!is_attention(&ExecutionStatus::Debug));
    }

    /// Both of these guard a decode failure, not a compile failure: the query
    /// builds either way and only comes back wrong at runtime.
    #[test]
    fn the_day_rollup_casts_its_sum_so_postgres_numeric_still_decodes() {
        let sql = live_select(execution_usage_tracking::Entity::find(), &today_only())
            .build(DbBackend::Postgres)
            .to_string();
        assert!(
            sql.contains("CAST(SUM(\"microseconds\") AS BIGINT)"),
            "{sql}"
        );
        assert!(sql.contains("AS \"total_microseconds\""), "{sql}");
    }

    #[test]
    fn the_app_rollup_aliases_the_column_its_row_reads() {
        let sql = live_select(execution_usage_tracking::Entity::find(), &today_only())
            .build(DbBackend::Postgres)
            .to_string();
        assert!(sql.contains("\"appId\" AS \"app_id\""), "{sql}");
        assert!(sql.contains("GROUP BY \"bucket_series\".\"i\""), "{sql}");
    }

    #[test]
    fn rolled_days_are_read_by_account_and_half_open_day_range() {
        let sql = rolled_select("alice", day(2026, 9, 5), day(2026, 9, 11), Some("a"))
            .build(DbBackend::Postgres)
            .to_string();
        assert!(sql.contains(r#""userId" = 'alice'"#), "{sql}");
        assert!(sql.contains(r#""day" >= '2026-09-05'"#), "{sql}");
        assert!(sql.contains(r#""day" < '2026-09-11'"#), "{sql}");
        assert!(sql.contains(r#""appId" = 'a'"#), "{sql}");
        assert!(!sql.contains("GROUP BY"), "{sql}");
    }

    #[test]
    fn severities_fold_into_one_total_and_a_flagged_subset() {
        let folded = fold_counts([
            ("2026-09-11", ExecutionStatus::Info, 40),
            ("2026-09-11", ExecutionStatus::Error, 2),
            ("2026-09-11", ExecutionStatus::Fatal, 1),
            ("2026-09-10", ExecutionStatus::Warn, 5),
        ]);
        assert_eq!(folded["2026-09-11"], (43, 3));
        assert_eq!(folded["2026-09-10"], (5, 0));
    }

    #[tokio::test]
    async fn one_rollup_preserves_scoping_null_apps_window_totals_and_attention_limit() {
        use super::super::test_db::{database, insert};

        let db = database().await;
        let now = DateTime::parse_from_rfc3339("2026-10-04T12:00:00Z").unwrap();
        for i in 0..70 {
            insert(
                &db,
                &format!("a-{i:03}"),
                "alice",
                Some("a"),
                now - Duration::hours(1),
                ExecutionStatus::Error,
                100,
            )
            .await;
        }
        for i in 0..20 {
            insert(
                &db,
                &format!("b-{i:03}"),
                "alice",
                Some("b"),
                now - Duration::days(1),
                ExecutionStatus::Info,
                200,
            )
            .await;
        }
        for i in 0..3 {
            insert(
                &db,
                &format!("null-{i}"),
                "alice",
                None,
                now,
                ExecutionStatus::Fatal,
                300,
            )
            .await;
        }
        for (id, user, at) in [
            ("foreign", "bob", now),
            ("too-old", "alice", now - Duration::days(4)),
            ("future", "alice", now + Duration::days(1)),
        ] {
            insert(&db, id, user, Some("a"), at, ExecutionStatus::Fatal, 9_000).await;
        }
        let report = execution_activity(
            &db,
            "alice",
            ActivityParams {
                days: 3,
                app_id: None,
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!((report.total, report.attention_total), (93, 73));
        assert_eq!(
            report
                .buckets
                .iter()
                .map(|b| (b.count, b.attention_count))
                .collect::<Vec<_>>(),
            [(0, 0), (20, 0), (73, 73)]
        );
        assert_eq!(
            report
                .apps
                .iter()
                .map(|a| (a.app_id.as_deref(), a.count, a.attention_count))
                .collect::<Vec<_>>(),
            [(Some("a"), 70, 70), (Some("b"), 20, 0), (None, 3, 3)]
        );
        assert_eq!(report.average_microseconds, Some(11_900.0 / 93.0));
        assert_eq!(report.attention.len(), ATTENTION_SAMPLE as usize);
        assert_eq!(report.attention[0].id, "null-2");
        assert!(
            report
                .attention
                .iter()
                .all(|r| r.id.starts_with("null-") || r.id.starts_with("a-"))
        );
        let scoped = execution_activity(
            &db,
            "alice",
            ActivityParams {
                days: 3,
                app_id: Some("b".into()),
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!((scoped.total, scoped.attention_total), (20, 0));
        assert!(scoped.attention.is_empty());
        assert_eq!(scoped.apps.len(), 1);
        assert_eq!(scoped.average_microseconds, Some(200.0));
        let empty = execution_activity(
            &db,
            "nobody",
            ActivityParams {
                days: 0,
                app_id: None,
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!((empty.days, empty.total), (1, 0));
        assert!(empty.apps.is_empty());
        assert_eq!(empty.average_microseconds, None);
    }

    fn one_pass() -> RollupSettings {
        RollupSettings {
            retention_days: 3,
            budget: std::time::Duration::from_secs(60),
            part_rows: 100,
        }
    }

    #[tokio::test]
    async fn closed_days_come_from_the_rollup_once_maintenance_ran() {
        use super::super::test_db::{database, insert};
        use crate::{entity::execution_usage_day, execution_usage_rollup::count_late_execution};
        use sea_orm::PaginatorTrait;

        let db = database().await;
        let now = DateTime::parse_from_rfc3339("2026-10-04T12:00:00Z").unwrap();
        let yesterday = now - Duration::days(1);
        let window = || ActivityParams {
            days: 3,
            app_id: None,
        };
        let alice = |id, at, status| insert(&db, id, "alice", Some("a"), at, status, 10);
        let counts = || async {
            let report = execution_activity(&db, "alice", window(), now).await;
            let report = report.unwrap();
            let days: Vec<_> = report.buckets.iter().map(|bucket| bucket.count).collect();
            (days, report.attention_total)
        };
        alice("first", yesterday, ExecutionStatus::Info).await;
        alice("second", yesterday, ExecutionStatus::Info).await;
        alice("today", now, ExecutionStatus::Info).await;

        assert_eq!(counts().await, (vec![0, 2, 1], 0));
        let marked = execution_usage_day::Entity::find().count(&db).await;
        assert_eq!(marked.unwrap(), 0, "reads never roll up");

        maintain_with(&db, DbDialect::Postgres, now, &one_pass())
            .await
            .unwrap();
        assert_eq!(counts().await, (vec![0, 2, 1], 0));

        alice("unreported", yesterday, ExecutionStatus::Info).await;
        alice("late", yesterday, ExecutionStatus::Error).await;
        let late = execution_usage_tracking::Entity::find_by_id("late").one(&db);
        count_late_execution(&db, &late.await.unwrap().unwrap(), now)
            .await
            .unwrap();
        assert_eq!(counts().await, (vec![0, 3, 1], 1));
    }

    /// The same read on PostgreSQL, through the committed migration: closed
    /// days from the rollup, today from the joined bucket list, and a deleted
    /// account's days released from both tables.
    #[tokio::test]
    #[ignore = "requires FLOW_LIKE_ROLLUP_TEST_DATABASE_URL"]
    async fn postgres_reads_closed_days_from_the_rollup() {
        use crate::execution_usage_rollup::postgres::{database, execution};
        use sea_orm::{ActiveModelTrait, ConnectionTrait};

        let db = database().await;
        let now = DateTime::parse_from_rfc3339("2026-10-04T12:00:00Z").unwrap();
        for (id, user, at) in [
            ("first", "alice", now - Duration::days(1)),
            ("second", "alice", now - Duration::days(1)),
            ("today", "alice", now - Duration::hours(1)),
            ("foreign", "bob", now - Duration::days(1)),
        ] {
            execution(id, user, at).insert(&db).await.unwrap();
        }
        maintain_with(&db, DbDialect::Postgres, now, &one_pass())
            .await
            .unwrap();
        let read = |sub: &'static str| {
            execution_activity(
                &db,
                sub,
                ActivityParams {
                    days: 3,
                    app_id: Some("a".into()),
                },
                now,
            )
        };

        let report = read("alice").await.unwrap();
        assert_eq!(
            report
                .buckets
                .iter()
                .map(|bucket| bucket.count)
                .collect::<Vec<_>>(),
            [0, 2, 1]
        );
        assert_eq!(report.average_microseconds, Some(10.0));

        db.execute_unprepared(r#"DELETE FROM "User" WHERE "id" = 'alice'"#)
            .await
            .unwrap();
        assert_eq!(read("alice").await.unwrap().total, 0);
        assert_eq!(read("bob").await.unwrap().total, 1);
    }
}
