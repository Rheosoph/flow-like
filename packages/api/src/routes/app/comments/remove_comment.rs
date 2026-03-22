use crate::{
    ensure_in_project, entity::comment, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, ModelTrait, QueryFilter, TransactionTrait};

use super::upsert_comment::adjust_app_ratings;

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/comments/{comment_id}",
    tag = "comments",
    description = "Delete a review comment. Users can delete their own; admins and owners can delete any.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("comment_id" = String, Path, description = "Comment ID")
    ),
    responses(
        (status = 200, description = "Comment deleted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/comments/{comment_id}",
    skip(state, user)
)]
pub async fn remove_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, comment_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let permission = ensure_in_project!(user, &app_id, &state);
    let sub = permission.sub()?;

    let txn = state.db.begin().await?;

    let comment = comment::Entity::find_by_id(&comment_id)
        .filter(comment::Column::AppId.eq(&app_id))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let is_owner = comment.user_id == sub;
    let is_admin = permission.has_permission(RolePermissions::Admin)
        || permission.has_permission(RolePermissions::Owner);

    if !is_owner && !is_admin {
        return Err(ApiError::FORBIDDEN);
    }

    let rating = comment.rating;
    comment.delete(&txn).await?;
    adjust_app_ratings(&txn, &app_id, -rating, -1).await?;
    txn.commit().await?;

    Ok(Json(()))
}
