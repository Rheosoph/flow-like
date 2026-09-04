//! Bucketed time-series of error counts for charts in the control tower.

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{Duration, Utc};
use sea_orm::{DbBackend, FromQueryResult, Statement};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct TimeseriesQuery {
    /// Lookback window in hours. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    /// Bucket granularity: "minute", "hour", "day". Default chosen from the window.
    #[serde(default)]
    pub bucket: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeseriesPoint {
    /// ISO-8601 timestamp at the start of the bucket.
    pub bucket: String,
    pub total: i64,
    pub server: i64,
    pub client: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeseriesResponse {
    pub window_hours: i64,
    pub bucket: String,
    pub points: Vec<TimeseriesPoint>,
}

#[derive(Debug, FromQueryResult)]
struct Row {
    bucket: chrono::DateTime<chrono::FixedOffset>,
    total: i64,
    server: i64,
    client: i64,
}

fn bucket_for(hours: i64, requested: Option<&str>) -> &'static str {
    if let Some(r) = requested {
        match r {
            "minute" => return "minute",
            "hour" => return "hour",
            "day" => return "day",
            _ => {}
        }
    }
    if hours <= 6 {
        "minute"
    } else if hours <= 24 * 7 {
        "hour"
    } else {
        "day"
    }
}

#[utoipa::path(
    get,
    path = "/admin/logs/timeseries",
    tag = "admin",
    params(TimeseriesQuery),
    responses(
        (status = 200, description = "Bucketed error counts for charts", body = TimeseriesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Time-bucketed error counts for the control tower charts."
)]
pub async fn error_timeseries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<TimeseriesQuery>,
) -> Result<Json<TimeseriesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadLogs)
        .await?;

    let hours = q.hours.unwrap_or(24).clamp(1, 24 * 90);
    let bucket = bucket_for(hours, q.bucket.as_deref());
    let cutoff = Utc::now().fixed_offset() - Duration::hours(hours);

    let backend = state.db.get_database_backend();
    let sql = match backend {
        DbBackend::Postgres => format!(
            r#"SELECT date_trunc('{bucket}', "createdAt" AT TIME ZONE 'UTC') AT TIME ZONE 'UTC' AS bucket,
                      COUNT(*) AS total,
                      COUNT(*) FILTER (WHERE "statusCode" >= 500) AS server,
                      COUNT(*) FILTER (WHERE "statusCode" >= 400 AND "statusCode" < 500) AS client
               FROM "ErrorReport"
               WHERE "createdAt" >= $1
               GROUP BY bucket
               ORDER BY bucket ASC"#,
        ),
        _ => format!(
            r#"SELECT date_trunc('{bucket}', created_at) AS bucket,
                      COUNT(*) AS total,
                      SUM(CASE WHEN status_code >= 500 THEN 1 ELSE 0 END) AS server,
                      SUM(CASE WHEN status_code >= 400 AND status_code < 500 THEN 1 ELSE 0 END) AS client
               FROM error_report
               WHERE created_at >= $1
               GROUP BY bucket
               ORDER BY bucket ASC"#,
        ),
    };

    let stmt = Statement::from_sql_and_values(backend, sql, [cutoff.into()]);
    let rows = Row::find_by_statement(stmt).all(&state.db).await?;

    let points = rows
        .into_iter()
        .map(|r| TimeseriesPoint {
            bucket: r.bucket.to_rfc3339(),
            total: r.total,
            server: r.server,
            client: r.client,
        })
        .collect();

    Ok(Json(TimeseriesResponse {
        window_hours: hours,
        bucket: bucket.to_string(),
        points,
    }))
}
