use crate::{
    ensure_permission, entity::event, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use super::get_routes::RouteMapping;

#[utoipa::path(
    get,
    path = "/apps/{app_id}/routes/default",
    tag = "routes",
    description = "Get the default route mapping for the app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Default route mapping", body = Option<RouteMapping>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/routes/default", skip(state, user))]
pub async fn get_default_route(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Option<RouteMapping>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ListEvents);

    let model = event::Entity::find()
        .filter(event::Column::AppId.eq(&app_id))
        .filter(event::Column::IsDefault.eq(true))
        .one(&state.db)
        .await?;

    let result = model.map(|e| {
        let path = e.route.clone().unwrap_or_else(|| "/".to_string());
        RouteMapping {
            id: e.id.clone(),
            path,
            event_id: e.id,
            is_default: true,
        }
    });

    Ok(Json(result))
}
