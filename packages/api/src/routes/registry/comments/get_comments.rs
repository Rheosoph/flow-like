use crate::{
    entity::{comment, user, wasm_package},
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
    path = "/registry/package/{package_id}/comments",
    tag = "package-comments",
    description = "List review comments for a WASM package.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        CommentsQuery,
    ),
    responses(
        (status = 200, description = "Paginated comment list", body = CommentsResponse),
        (status = 404, description = "Package not found")
    ),
    security(
        (),
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /registry/package/{package_id}/comments",
    skip(state, _user)
)]
pub async fn get_comments(
    State(state): State<AppState>,
    Extension(_user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Query(query): Query<CommentsQuery>,
) -> Result<Json<CommentsResponse>, ApiError> {
    wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let offset = query.offset.unwrap_or(0);
    let limit = query.limit.unwrap_or(20).min(100);

    let total = comment::Entity::find()
        .filter(comment::Column::PackageId.eq(&package_id))
        .count(&state.db)
        .await?;

    let comments_with_users = comment::Entity::find()
        .filter(comment::Column::PackageId.eq(&package_id))
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
            user_id: c.user_id,
            user_name: u.as_ref().and_then(|u| u.name.clone()),
            user_avatar: u.as_ref().and_then(|u| u.avatar.clone()),
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
