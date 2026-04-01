use crate::{
    audit_branch, ensure_permission, entity::technical_user, error::ApiError,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, ModelTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/api/{key_id}",
    tag = "api-keys",
    description = "Delete an API key.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("key_id" = String, Path, description = "API key ID")
    ),
    responses(
        (status = 200, description = "API key deleted", body = ()),
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
#[tracing::instrument(name = "DELETE /apps/{app_id}/api/{key_id}", skip(state, user))]
pub async fn delete_api_key(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, key_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let technical_user = technical_user::Entity::find_by_id(&key_id)
        .filter(technical_user::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    technical_user.delete(&state.db).await?;

    // Invalidate any cached auth for this key
    state.auth_cache.invalidate_all();

    audit_branch!(
        state,
        user,
        app_id,
        "apikey.delete",
        "ApiKey",
        key_id,
        "API key deleted"
    );
    Ok(Json(()))
}
