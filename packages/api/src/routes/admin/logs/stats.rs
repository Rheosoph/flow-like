//! Aggregate error stats: totals, top buckets, recent activity.

use crate::entity::error_report;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::admin::logs::list_errors::ErrorReportRecord;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{Duration, Utc};
use sea_orm::sea_query::{Alias, Expr, Order as SeaOrder, Query as SeaQuery};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbBackend, EntityTrait, FromQueryResult, Order, PaginatorTrait,
    QueryFilter, QueryOrder, QuerySelect, Statement,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct StatsQuery {
    /// Lookback window in hours. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Limit how many entries each top-list returns. Default 5, max 25.
    #[serde(default)]
    pub top: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorBucket {
    pub key: String,
    pub label: String,
    pub count: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorStatsResponse {
    pub window_hours: i64,
    pub total_errors: i64,
    pub server_errors: i64,
    pub client_errors: i64,
    pub unique_users_affected: i64,
    pub unique_paths: i64,
    pub previous_window_total: i64,
    pub change_percent: Option<f64>,
    pub recent: Vec<ErrorReportRecord>,
    pub top_codes: Vec<ErrorBucket>,
    pub top_paths: Vec<ErrorBucket>,
    pub top_users: Vec<ErrorBucket>,
}

#[derive(Debug, FromQueryResult)]
struct CountRow {
    key: Option<String>,
    cnt: i64,
}

#[derive(Debug, FromQueryResult)]
struct ScalarCount {
    cnt: i64,
}

async fn group_count<C: ConnectionTrait>(
    db: &C,
    column: error_report::Column,
    cutoff: chrono::NaiveDateTime,
    limit: u64,
) -> Result<Vec<CountRow>, ApiError> {
    use sea_orm::sea_query::ExprTrait;

    let mut q = SeaQuery::select();
    q.from(error_report::Entity)
        .expr_as(Expr::col(column), Alias::new("key"))
        .expr_as(
            Expr::col(error_report::Column::Id).count(),
            Alias::new("cnt"),
        )
        .and_where(Expr::col(error_report::Column::CreatedAt).gte(cutoff))
        .add_group_by([Expr::col(column).into()])
        .order_by_expr(Expr::col(error_report::Column::Id).count(), SeaOrder::Desc)
        .limit(limit);

    let stmt = db.get_database_backend().build(&q);
    let rows = CountRow::find_by_statement(stmt).all(db).await?;
    Ok(rows)
}

async fn distinct_count<C: ConnectionTrait>(
    db: &C,
    column: error_report::Column,
    cutoff: chrono::NaiveDateTime,
    only_non_null: bool,
) -> Result<i64, ApiError> {
    use sea_orm::sea_query::ExprTrait;

    let mut q = SeaQuery::select();
    q.from(error_report::Entity)
        .expr_as(Expr::col(column).count_distinct(), Alias::new("cnt"))
        .and_where(Expr::col(error_report::Column::CreatedAt).gte(cutoff));
    if only_non_null {
        q.and_where(Expr::col(column).is_not_null());
    }

    let stmt = db.get_database_backend().build(&q);
    let row = ScalarCount::find_by_statement(stmt)
        .one(db)
        .await?
        .map(|r| r.cnt)
        .unwrap_or(0);
    Ok(row)
}

#[utoipa::path(
    get,
    path = "/admin/logs/stats",
    tag = "admin",
    params(StatsQuery),
    responses(
        (status = 200, description = "Aggregated error statistics for the dashboard", body = ErrorStatsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Aggregate stats over recent API errors: totals, top codes, top paths, top users."
)]
pub async fn error_stats(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<StatsQuery>,
) -> Result<Json<ErrorStatsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadLogs)
        .await?;

    let hours = q.hours.unwrap_or(24).clamp(1, 24 * 30);
    let top = q.top.unwrap_or(5).clamp(1, 25);

    let now = Utc::now().naive_utc();
    let cutoff = now - Duration::hours(hours);
    let prev_cutoff = cutoff - Duration::hours(hours);

    let total_errors = error_report::Entity::find()
        .filter(error_report::Column::CreatedAt.gte(cutoff))
        .count(&state.db)
        .await? as i64;

    let server_errors = error_report::Entity::find()
        .filter(error_report::Column::CreatedAt.gte(cutoff))
        .filter(error_report::Column::StatusCode.gte(500))
        .count(&state.db)
        .await? as i64;

    let client_errors = error_report::Entity::find()
        .filter(error_report::Column::CreatedAt.gte(cutoff))
        .filter(error_report::Column::StatusCode.gte(400))
        .filter(error_report::Column::StatusCode.lt(500))
        .count(&state.db)
        .await? as i64;

    let previous_window_total = error_report::Entity::find()
        .filter(error_report::Column::CreatedAt.gte(prev_cutoff))
        .filter(error_report::Column::CreatedAt.lt(cutoff))
        .count(&state.db)
        .await? as i64;

    let change_percent = if previous_window_total > 0 {
        Some(((total_errors - previous_window_total) as f64 / previous_window_total as f64) * 100.0)
    } else if total_errors > 0 {
        Some(100.0)
    } else {
        None
    };

    let unique_users_affected =
        distinct_count(&state.db, error_report::Column::UserId, cutoff, true).await?;

    let unique_paths = distinct_count(&state.db, error_report::Column::Path, cutoff, false).await?;

    let recent_models = error_report::Entity::find()
        .filter(error_report::Column::CreatedAt.gte(cutoff))
        .order_by(error_report::Column::CreatedAt, Order::Desc)
        .limit(top.min(25))
        .all(&state.db)
        .await?;
    let recent: Vec<ErrorReportRecord> = recent_models.into_iter().map(Into::into).collect();

    let top_codes_rows =
        group_count(&state.db, error_report::Column::PublicCode, cutoff, top).await?;
    let top_codes = top_codes_rows
        .into_iter()
        .map(|r| {
            let key = r.key.unwrap_or_else(|| "UNKNOWN".to_string());
            ErrorBucket {
                label: key.clone(),
                key,
                count: r.cnt,
            }
        })
        .collect();

    let top_paths_rows = group_count(&state.db, error_report::Column::Path, cutoff, top).await?;
    let top_paths = top_paths_rows
        .into_iter()
        .map(|r| {
            let key = r.key.unwrap_or_else(|| "/".to_string());
            ErrorBucket {
                label: key.clone(),
                key,
                count: r.cnt,
            }
        })
        .collect();

    // Top users: SeaORM doesn't allow easy filtering of NULL keys in group_by helper, do raw SQL
    let backend = state.db.get_database_backend();
    let users_sql = match backend {
        DbBackend::Postgres => r#"SELECT "userId" AS "key", COUNT(*) AS "cnt"
FROM "ErrorReport"
WHERE "createdAt" >= $1 AND "userId" IS NOT NULL
GROUP BY "userId"
ORDER BY "cnt" DESC
LIMIT $2"#
            .to_string(),
        _ => r#"SELECT user_id AS key, COUNT(*) AS cnt
FROM error_report
WHERE created_at >= $1 AND user_id IS NOT NULL
GROUP BY user_id
ORDER BY cnt DESC
LIMIT $2"#
            .to_string(),
    };
    let stmt =
        Statement::from_sql_and_values(backend, users_sql, [cutoff.into(), (top as i64).into()]);
    let user_rows = CountRow::find_by_statement(stmt).all(&state.db).await?;
    let top_users = user_rows
        .into_iter()
        .map(|r| {
            let key = r.key.unwrap_or_default();
            ErrorBucket {
                label: key.clone(),
                key,
                count: r.cnt,
            }
        })
        .collect();

    Ok(Json(ErrorStatsResponse {
        window_hours: hours,
        total_errors,
        server_errors,
        client_errors,
        unique_users_affected,
        unique_paths,
        previous_window_total,
        change_percent,
        recent,
        top_codes,
        top_paths,
        top_users,
    }))
}
