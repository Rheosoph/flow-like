use crate::{
    ensure_permission,
    entity::feedback,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, Condition, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct FeedbackQuery {
    /// Pagination offset
    #[serde(default)]
    pub offset: u64,
    /// Items per page (max 100)
    #[serde(default = "default_limit")]
    pub limit: u64,
    /// Minimum rating filter
    pub min_rating: Option<i64>,
    /// Maximum rating filter
    pub max_rating: Option<i64>,
}

fn default_limit() -> u64 {
    50
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackItem {
    pub id: String,
    pub user_id: Option<String>,
    pub event_id: Option<String>,
    pub rating: i64,
    pub comment: String,
    pub created_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PaginatedFeedback {
    pub items: Vec<FeedbackItem>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

/// GET /apps/{app_id}/analytics/feedback - List feedback entries
#[utoipa::path(
    get,
    path = "/apps/{app_id}/analytics/feedback",
    tag = "analytics",
    description = "List feedback entries for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("offset" = Option<u64>, Query, description = "Pagination offset"),
        ("limit" = Option<u64>, Query, description = "Items per page (max 100)"),
        ("min_rating" = Option<i64>, Query, description = "Minimum rating filter"),
        ("max_rating" = Option<i64>, Query, description = "Maximum rating filter")
    ),
    responses(
        (status = 200, description = "Feedback list", body = PaginatedFeedback),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/analytics/feedback", skip(state, user))]
pub async fn list_feedback(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<FeedbackQuery>,
) -> Result<Json<PaginatedFeedback>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadAnalytics);

    let limit = query.limit.min(100);
    let offset = query.offset;

    let mut condition = Condition::all().add(feedback::Column::AppId.eq(&app_id));

    if let Some(min) = query.min_rating {
        condition = condition.add(feedback::Column::Rating.gte(min));
    }
    if let Some(max) = query.max_rating {
        condition = condition.add(feedback::Column::Rating.lte(max));
    }

    let total = feedback::Entity::find()
        .filter(condition.clone())
        .count(&state.db)
        .await?;

    let records = feedback::Entity::find()
        .filter(condition)
        .order_by_desc(feedback::Column::CreatedAt)
        .offset(Some(offset))
        .limit(Some(limit))
        .all(&state.db)
        .await?;

    let items = records
        .into_iter()
        .map(|r| FeedbackItem {
            id: r.id,
            user_id: r.user_id,
            event_id: r.event_id,
            rating: r.rating,
            comment: r.comment,
            created_at: r.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        })
        .collect();

    Ok(Json(PaginatedFeedback {
        items,
        total,
        offset,
        limit,
    }))
}
