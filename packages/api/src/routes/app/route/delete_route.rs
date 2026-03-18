use crate::{
    ensure_permission, entity::event, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension,
    extract::{Path, State},
    http::StatusCode,
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/routes/{route_id}",
    tag = "routes",
    description = "Delete a route mapping.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("route_id" = String, Path, description = "Route ID (event ID)")
    ),
    responses(
        (status = 204, description = "Route mapping deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Route not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/routes/{route_id}", skip(state, user))]
pub async fn delete_route(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, route_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);

    let model = event::Entity::find_by_id(&route_id)
        .filter(event::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active: event::ActiveModel = model.into();
    active.route = Set(None);
    active.is_default = Set(false);
    active.update(&state.db).await?;

    Ok(StatusCode::NO_CONTENT)
}
