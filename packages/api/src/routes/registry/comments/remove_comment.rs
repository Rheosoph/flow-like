use crate::{
    entity::comment, error::ApiError, middleware::jwt::AppUser,
    permission::wasm_package_permission::WasmPackagePermission, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, ModelTrait, QueryFilter};

use super::upsert_comment::adjust_package_ratings;

#[utoipa::path(
    delete,
    path = "/registry/package/{package_id}/comments/{comment_id}",
    tag = "package-comments",
    description = "Delete a package review. Users can delete their own; package maintainers can delete any.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
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
    name = "DELETE /registry/package/{package_id}/comments/{comment_id}",
    skip(state, user)
)]
pub async fn remove_comment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((package_id, comment_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let sub = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let is_maintainer = crate::check_wasm_access!(state, &sub, &package_id)
        .map(|p| p.has_permission(WasmPackagePermission::Maintainer))
        .unwrap_or(false);

    state
        .transaction(|txn| {
            let sub = sub.clone();
            let package_id = package_id.clone();
            let comment_id = comment_id.clone();
            Box::pin(async move {
                let comment = comment::Entity::find_by_id(&comment_id)
                    .filter(comment::Column::PackageId.eq(&package_id))
                    .one(txn)
                    .await?
                    .ok_or(ApiError::NOT_FOUND)?;

                if comment.user_id != sub && !is_maintainer {
                    return Err(ApiError::FORBIDDEN);
                }

                let rating = comment.rating;
                comment.delete(txn).await?;
                adjust_package_ratings(txn, &package_id, -rating, -1).await?;
                Ok::<_, ApiError>(())
            })
        })
        .await?;

    Ok(Json(()))
}
