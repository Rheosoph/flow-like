use crate::{
    ensure_permission, entity::event, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};

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

    let row = event::Entity::find()
        .filter(event::Column::AppId.eq(&app_id))
        .filter(event::Column::IsDefault.eq(true))
        .select_only()
        .columns([event::Column::Id, event::Column::Route])
        .into_tuple::<(String, Option<String>)>()
        .one(&state.db)
        .await?;

    let result = row.map(|(id, route)| RouteMapping {
        id: id.clone(),
        path: route.unwrap_or_else(|| "/".to_string()),
        event_id: id,
        is_default: true,
    });

    Ok(Json(result))
}
