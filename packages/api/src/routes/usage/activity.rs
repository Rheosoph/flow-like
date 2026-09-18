//! Period-scoped execution activity for the signed-in account.
//!
//! `GET /usage/executions` can only answer "the newest N records"; a home
//! widget asking for "the last 7 days" cannot be built from that page without
//! silently dropping every record the page cut off. This endpoint counts the
//! whole window in SQL instead, so the buckets, the per-app split and the
//! totals describe the period rather than the page size.

use crate::{
    entity::{execution_usage_tracking, sea_orm_active_enums::ExecutionStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    utils::{stats_period::StatsPeriod, time::utc_midnight},
};
use axum::{
    Extension, Json,
    extract::{Query, State},
};
use chrono::{Duration, NaiveDate, Utc};
use sea_orm::{
    ColumnTrait, DbBackend, EntityTrait, FromQueryResult, QueryFilter, QueryOrder, QuerySelect,
    Select,
    sea_query::{Alias, Expr, ExprTrait, SimpleExpr},
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ExecutionActivityBucket {
    /// UTC calendar day, `YYYY-MM-DD`.
    pub day: String,
    pub count: i64,
    pub attention_count: i64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ExecutionActivityApp {
    /// `None` for records recorded without an app.
    pub app_id: Option<String>,
    pub count: i64,
    pub attention_count: i64,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ExecutionActivityResponse {
    pub days: u32,
    /// Inclusive window start, midnight UTC of the first day.
    pub from: String,
    /// Instant the window was evaluated at.
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

#[derive(Debug, FromQueryResult)]
struct BucketStatusCount {
    bucket: String,
    status: ExecutionStatus,
    cnt: i64,
    total_microseconds: i64,
}

#[derive(Debug, FromQueryResult)]
struct AppStatusCount {
    app_id: Option<String>,
    status: ExecutionStatus,
    cnt: i64,
}

type ActivitySelect = Select<execution_usage_tracking::Entity>;

/// Records per UTC day and severity, with the duration each group contributes.
///
/// `total_microseconds` is cast because Postgres widens `SUM` over a 64 bit
/// column to NUMERIC, which never decodes into the `i64` it lands in.
fn day_select(base: ActivitySelect, backend: DbBackend) -> ActivitySelect {
    let bucket_expr = StatsPeriod::Day.bucket_expr(backend, "createdAt");
    base.select_only()
        .expr_as(bucket_expr.clone(), "bucket")
        .column(execution_usage_tracking::Column::Status)
        .expr_as(
            Expr::col(execution_usage_tracking::Column::Id).count(),
            "cnt",
        )
        .expr_as(
            sum_i64(execution_usage_tracking::Column::Microseconds),
            "total_microseconds",
        )
        .group_by(bucket_expr)
        .group_by(Expr::col(execution_usage_tracking::Column::Status))
}

/// Records per app and severity.
///
/// The app column is aliased because its database label is `appId` while the
/// row it decodes into names the field `app_id`; without the alias every app
/// comes back as the null group.
fn app_select(base: ActivitySelect) -> ActivitySelect {
    base.select_only()
        .column_as(Expr::col(execution_usage_tracking::Column::AppId), "app_id")
        .column(execution_usage_tracking::Column::Status)
        .expr_as(
            Expr::col(execution_usage_tracking::Column::Id).count(),
            "cnt",
        )
        .group_by(Expr::col(execution_usage_tracking::Column::AppId))
        .group_by(Expr::col(execution_usage_tracking::Column::Status))
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
    Query(params): Query<ActivityParams>,
) -> Result<Json<ExecutionActivityResponse>, ApiError> {
    let sub = user.sub()?;
    let days = params.days.clamp(1, MAX_DAYS);
    let now = Utc::now();
    let keys = window_days(now.date_naive(), days);
    let from = utc_midnight(keys[0]);

    let base = || {
        let mut query = execution_usage_tracking::Entity::find()
            .filter(execution_usage_tracking::Column::UserId.eq(&sub))
            .filter(execution_usage_tracking::Column::CreatedAt.gte(from));
        if let Some(ref app_id) = params.app_id {
            query = query.filter(execution_usage_tracking::Column::AppId.eq(app_id));
        }
        query
    };

    let (day_rows, app_rows) = flow_like_types::tokio::join!(
        day_select(base(), state.db.get_database_backend())
            .into_model::<BucketStatusCount>()
            .all(&state.db),
        app_select(base())
            .into_model::<AppStatusCount>()
            .all(&state.db),
    );
    let day_rows = day_rows.map_err(|e| ApiError::internal_error(e.into()))?;
    let app_rows = app_rows.map_err(|e| ApiError::internal_error(e.into()))?;

    let total_microseconds: i64 = day_rows.iter().map(|row| row.total_microseconds).sum();
    let per_day = fold_counts(
        day_rows
            .into_iter()
            .map(|row| (row.bucket, row.status, row.cnt)),
    );

    let buckets: Vec<ExecutionActivityBucket> = keys
        .iter()
        .map(|day| {
            let key = day.format("%Y-%m-%d").to_string();
            let (count, attention_count) = per_day.get(&key).copied().unwrap_or((0, 0));
            ExecutionActivityBucket {
                day: key,
                count,
                attention_count,
            }
        })
        .collect();

    let total: i64 = buckets.iter().map(|bucket| bucket.count).sum();
    let attention_total: i64 = buckets.iter().map(|bucket| bucket.attention_count).sum();

    let mut apps: Vec<ExecutionActivityApp> = fold_counts(
        app_rows
            .into_iter()
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

    let attention = if attention_total > 0 {
        base()
            .filter(
                execution_usage_tracking::Column::Status
                    .is_in([ExecutionStatus::Error, ExecutionStatus::Fatal]),
            )
            .order_by_desc(execution_usage_tracking::Column::CreatedAt)
            .limit(ATTENTION_SAMPLE)
            .all(&state.db)
            .await
            .map_err(|e| ApiError::internal_error(e.into()))?
            .into_iter()
            .map(ExecutionUsageRecord::from)
            .collect()
    } else {
        Vec::new()
    };

    Ok(Json(ExecutionActivityResponse {
        days,
        from: from.to_rfc3339(),
        to: now.fixed_offset().to_rfc3339(),
        buckets,
        apps,
        total,
        attention_total,
        average_microseconds: (total > 0).then(|| total_microseconds as f64 / total as f64),
        attention,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::QueryTrait;

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
        let sql = day_select(
            execution_usage_tracking::Entity::find(),
            DbBackend::Postgres,
        )
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
        let sql = app_select(execution_usage_tracking::Entity::find())
            .build(DbBackend::Postgres)
            .to_string();
        assert!(sql.contains("\"appId\" AS \"app_id\""), "{sql}");
        assert!(sql.contains("GROUP BY"), "{sql}");
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
}
