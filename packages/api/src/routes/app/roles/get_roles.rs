use crate::{
    entity::{app, role},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

/// The app's default role id (when set) together with all of its roles.
pub type AppRoles = (Option<String>, Vec<role::Model>);

#[utoipa::path(
    get,
    path = "/apps/{app_id}/roles",
    tag = "roles",
    description = "List roles for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Roles list", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/roles", skip(state, user))]
pub async fn get_roles(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AppRoles>, ApiError> {
    let permission = user.execution_app_permission(&app_id, &state).await?;
    if !permission.has_permission(RolePermissions::ReadRoles) {
        if let Ok(user_id) = permission.sub() {
            state.invalidate_permission(&user_id, &app_id);
        }
        return Err(ApiError::FORBIDDEN);
    }

    let txn = state.db.begin().await?;

    let app = app::Entity::find_by_id(app_id.clone())
        .one(&txn)
        .await?
        .ok_or_else(|| {
            tracing::warn!("App {} not found", app_id);
            ApiError::NOT_FOUND
        })?;

    let roles = role::Entity::find()
        .filter(role::Column::AppId.eq(app_id.clone()))
        .all(&txn)
        .await?;

    Ok(Json((app.default_role_id, roles)))
}
