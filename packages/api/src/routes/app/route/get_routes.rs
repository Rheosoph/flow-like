use crate::{
    ensure_permission,
    entity::event,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::events::db::{is_listed_event_type, is_user_facing_event_parts},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RouteMapping {
    pub id: String,
    pub path: String,
    pub event_id: String,
    pub is_default: bool,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/routes",
    tag = "routes",
    description = "List all route mappings for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Route mappings", body = Vec<RouteMapping>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/routes", skip(state, user))]
pub async fn get_routes(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<RouteMapping>>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ListEvents);

    let events = event::Entity::find()
        .filter(event::Column::AppId.eq(&app_id))
        .filter(event::Column::Route.is_not_null())
        .select_only()
        .columns([
            event::Column::Id,
            event::Column::Route,
            event::Column::IsDefault,
            event::Column::Active,
            event::Column::EventType,
            event::Column::PageId,
        ])
        .into_tuple::<(String, Option<String>, bool, bool, String, Option<String>)>()
        .all(&state.db)
        .await?;

    // A route lives on the event row, so this list must hide exactly what
    // `GET /apps/{app_id}/events` hides. Any surplus here reads as a route
    // without an event.
    let can_read_events = permission.has_permission(RolePermissions::ReadEvents);
    let routes = events
        .into_iter()
        .filter(|(_, _, _, _, event_type, _)| is_listed_event_type(event_type))
        .filter(|(_, _, _, active, event_type, page_id)| {
            can_read_events
                || (*active && is_user_facing_event_parts(page_id.as_deref(), event_type))
        })
        .filter_map(|(id, route, is_default, _, _, _)| {
            route.map(|path| RouteMapping {
                id: id.clone(),
                path,
                event_id: id,
                is_default,
            })
        })
        .collect();

    Ok(Json(routes))
}
