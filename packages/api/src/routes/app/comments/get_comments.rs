use crate::{
    ensure_in_project,
    entity::{app, comment, sea_orm_active_enums::Visibility, user},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, QuerySelect};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Deserialize, Debug, IntoParams, ToSchema)]
pub struct CommentsQuery {
    #[serde(default)]
    pub offset: Option<u64>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Serialize, Debug, ToSchema)]
pub struct CommentItem {
    pub id: String,
    pub text: String,
    pub rating: i64,
    pub user_id: String,
    pub user_name: Option<String>,
    pub user_avatar: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize, Debug, ToSchema)]
pub struct CommentsResponse {
    pub comments: Vec<CommentItem>,
    pub total: u64,
    pub offset: u64,
    pub limit: u64,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/comments",
    tag = "comments",
    description = "List review comments for an app. Public apps allow unauthenticated access.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        CommentsQuery,
    ),
    responses(
        (status = 200, description = "Paginated comment list", body = CommentsResponse),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        (),
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/comments", skip(state, user, query))]
pub async fn get_comments(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(query): Query<CommentsQuery>,
) -> Result<Json<CommentsResponse>, ApiError> {
    let app_model = app::Entity::find_by_id(&app_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let is_public = app_model.visibility == Visibility::Public
        || app_model.visibility == Visibility::PublicRequestAccess;

    if !is_public {
        ensure_in_project!(user, &app_id, &state);
    }

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(20).min(100);

    let total = comment::Entity::find()
        .filter(comment::Column::AppId.eq(&app_id))
        .count(&state.db)
        .await?;

    let comments_with_users = comment::Entity::find()
        .filter(comment::Column::AppId.eq(&app_id))
        .find_also_related(user::Entity)
        .order_by_desc(comment::Column::CreatedAt)
        .limit(Some(limit))
        .offset(Some(offset))
        .all(&state.db)
        .await?;

    let comments = comments_with_users
        .into_iter()
        .map(|(c, u)| CommentItem {
            id: c.id,
            text: c.text,
            rating: c.rating,
            user_name: u
                .as_ref()
                .and_then(|u| u.name.clone().or(u.username.clone()))
                .or_else(|| Some(c.user_id.clone())),
            user_avatar: u.as_ref().and_then(|u| u.avatar.clone()),
            user_id: c.user_id,
            created_at: c.created_at.to_string(),
            updated_at: c.updated_at.to_string(),
        })
        .collect();

    Ok(Json(CommentsResponse {
        comments,
        total,
        offset,
        limit,
    }))
}
