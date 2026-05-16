//! List and filter API error reports for the admin control tower.

use crate::entity::error_report;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Query, State};
use axum::{Extension, Json};
use chrono::{Duration, NaiveDateTime, Utc};
use sea_orm::{
    ColumnTrait, Condition, EntityTrait, Order, PaginatorTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams)]
pub struct ListErrorsQuery {
    /// Free text search applied to id, summary, public code, path, user id.
    #[serde(default)]
    pub query: Option<String>,
    /// Filter by exact error id (e.g. references shared with users).
    #[serde(default)]
    pub error_id: Option<String>,
    /// Filter by HTTP method.
    #[serde(default)]
    pub method: Option<String>,
    /// Filter by exact path.
    #[serde(default)]
    pub path: Option<String>,
    /// Filter by exact public code.
    #[serde(default)]
    pub public_code: Option<String>,
    /// Filter by status code.
    #[serde(default)]
    pub status_code: Option<i32>,
    /// Filter by associated user id.
    #[serde(default)]
    pub user_id: Option<String>,
    /// Severity bucket: "client" (4xx), "server" (5xx). Anything else is ignored.
    #[serde(default)]
    pub severity: Option<String>,
    /// Lookback window in hours. Default 24.
    #[serde(default)]
    pub hours: Option<i64>,
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ErrorReportRecord {
    pub id: String,
    pub user_id: Option<String>,
    pub method: String,
    pub path: String,
    pub status_code: i32,
    pub public_code: String,
    pub summary: String,
    pub details: Option<serde_json::Value>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ListErrorsResponse {
    pub errors: Vec<ErrorReportRecord>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

impl From<error_report::Model> for ErrorReportRecord {
    fn from(m: error_report::Model) -> Self {
        Self {
            id: m.id,
            user_id: m.user_id,
            method: m.method,
            path: m.path,
            status_code: m.status_code,
            public_code: m.public_code,
            summary: m.summary,
            details: m.details,
            created_at: m.created_at.and_utc().to_rfc3339(),
            updated_at: m.updated_at.and_utc().to_rfc3339(),
        }
    }
}

fn cutoff(hours: Option<i64>) -> Option<NaiveDateTime> {
    let h = hours.unwrap_or(24).max(0);
    if h == 0 {
        None
    } else {
        Some((Utc::now() - Duration::hours(h)).naive_utc())
    }
}

#[utoipa::path(
    get,
    path = "/admin/logs/errors",
    tag = "admin",
    params(ListErrorsQuery),
    responses(
        (status = 200, description = "Paginated list of recent error reports", body = ListErrorsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List and filter API error reports. Requires ReadLogs permission."
)]
pub async fn list_errors(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(q): Query<ListErrorsQuery>,
) -> Result<Json<ListErrorsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadLogs)
        .await?;

    let offset = q.offset.unwrap_or(0);
    let limit = q.limit.unwrap_or(50).min(200);

    let mut select = error_report::Entity::find();

    if let Some(cutoff) = cutoff(q.hours) {
        select = select.filter(error_report::Column::CreatedAt.gte(cutoff));
    }

    if let Some(id) = &q.error_id
        && !id.is_empty()
    {
        select = select.filter(error_report::Column::Id.eq(id));
    }

    if let Some(method) = &q.method
        && !method.is_empty()
    {
        select = select.filter(error_report::Column::Method.eq(method.to_uppercase()));
    }

    if let Some(path) = &q.path
        && !path.is_empty()
    {
        select = select.filter(error_report::Column::Path.eq(path));
    }

    if let Some(code) = &q.public_code
        && !code.is_empty()
    {
        select = select.filter(error_report::Column::PublicCode.eq(code));
    }

    if let Some(sc) = q.status_code {
        select = select.filter(error_report::Column::StatusCode.eq(sc));
    }

    if let Some(uid) = &q.user_id
        && !uid.is_empty()
    {
        select = select.filter(error_report::Column::UserId.eq(uid));
    }

    match q.severity.as_deref() {
        Some("client") => {
            select = select.filter(
                error_report::Column::StatusCode
                    .gte(400)
                    .and(error_report::Column::StatusCode.lt(500)),
            );
        }
        Some("server") => {
            select = select.filter(error_report::Column::StatusCode.gte(500));
        }
        _ => {}
    }

    if let Some(text) = &q.query
        && !text.is_empty()
    {
        let pattern = format!("%{}%", text);
        select = select.filter(
            Condition::any()
                .add(error_report::Column::Id.like(&pattern))
                .add(error_report::Column::Summary.like(&pattern))
                .add(error_report::Column::PublicCode.like(&pattern))
                .add(error_report::Column::Path.like(&pattern))
                .add(error_report::Column::UserId.like(&pattern)),
        );
    }

    let total = select.clone().count(&state.db).await?;

    let records = select
        .order_by(error_report::Column::CreatedAt, Order::Desc)
        .offset(offset)
        .limit(limit)
        .all(&state.db)
        .await?;

    Ok(Json(ListErrorsResponse {
        errors: records.into_iter().map(Into::into).collect(),
        total,
        offset,
        limit,
    }))
}
