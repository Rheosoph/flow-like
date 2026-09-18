use crate::{
    ensure_permission, entity::event, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use super::get_routes::RouteMapping;

#[derive(Deserialize, Debug, IntoParams, ToSchema)]
pub struct PathQuery {
    pub path: String,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/routes/by-path",
    tag = "routes",
    description = "Get the route mapping for a specific URL path.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        PathQuery
    ),
    responses(
        (status = 200, description = "Route mapping for the path", body = Option<RouteMapping>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/routes/by-path", skip(state, user, params))]
pub async fn get_route_by_path(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<PathQuery>,
) -> Result<Json<Option<RouteMapping>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ListEvents);

    let row = event::Entity::find()
        .filter(event::Column::AppId.eq(&app_id))
        .filter(event::Column::Route.eq(&params.path))
        .select_only()
        .columns([
            event::Column::Id,
            event::Column::Route,
            event::Column::IsDefault,
        ])
        .into_tuple::<(String, Option<String>, bool)>()
        .one(&state.db)
        .await?;

    let result = row.and_then(|(id, route, is_default)| {
        route.map(|path| RouteMapping {
            id: id.clone(),
            path,
            event_id: id,
            is_default,
        })
    });

    Ok(Json(result))
}
