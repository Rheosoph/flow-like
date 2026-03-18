use crate::{
    ensure_permission, entity::event, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::ToSchema;

use super::get_routes::RouteMapping;

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct CreateRouteBody {
    pub path: String,
    pub event_id: String,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRouteBody {
    pub event_id: Option<String>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/routes",
    tag = "routes",
    description = "Create a new route mapping (path → event).",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = CreateRouteBody,
    responses(
        (status = 200, description = "Route mapping created", body = RouteMapping),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Event not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/routes", skip(state, user, body))]
pub async fn create_route(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<CreateRouteBody>,
) -> Result<Json<RouteMapping>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);

    let model = event::Entity::find_by_id(&body.event_id)
        .filter(event::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if body.is_default {
        clear_default_flag(&state, &app_id).await?;
    }

    let mut active: event::ActiveModel = model.into();
    active.route = Set(Some(body.path.clone()));
    active.is_default = Set(body.is_default);
    let updated = active.update(&state.db).await?;

    Ok(Json(RouteMapping {
        id: updated.id.clone(),
        path: body.path,
        event_id: updated.id,
        is_default: updated.is_default,
    }))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/routes/{route_id}",
    tag = "routes",
    description = "Update an existing route mapping.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("route_id" = String, Path, description = "Route ID (event ID)")
    ),
    request_body = UpdateRouteBody,
    responses(
        (status = 200, description = "Route mapping updated", body = RouteMapping),
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
#[tracing::instrument(name = "PUT /apps/{app_id}/routes/{route_id}", skip(state, user, body))]
pub async fn update_route(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, route_id)): Path<(String, String)>,
    Json(body): Json<UpdateRouteBody>,
) -> Result<Json<RouteMapping>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);

    let model = event::Entity::find_by_id(&route_id)
        .filter(event::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let path = model.route.clone().ok_or(ApiError::NOT_FOUND)?;

    if body.is_default == Some(true) {
        clear_default_flag(&state, &app_id).await?;
    }

    let mut active: event::ActiveModel = model.into();
    if let Some(event_id) = &body.event_id {
        // Reassign this route path to a different event
        let target = event::Entity::find_by_id(event_id)
            .filter(event::Column::AppId.eq(&app_id))
            .one(&state.db)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;

        // Clear route from old event
        active.route = Set(None);
        active.is_default = Set(false);
        active.update(&state.db).await?;

        // Set route on new event
        let mut target_active: event::ActiveModel = target.into();
        target_active.route = Set(Some(path.clone()));
        target_active.is_default = Set(body.is_default.unwrap_or(false));
        let updated = target_active.update(&state.db).await?;

        return Ok(Json(RouteMapping {
            id: updated.id.clone(),
            path,
            event_id: updated.id,
            is_default: updated.is_default,
        }));
    }

    if let Some(is_default) = body.is_default {
        active.is_default = Set(is_default);
    }
    let updated = active.update(&state.db).await?;

    Ok(Json(RouteMapping {
        id: updated.id.clone(),
        path,
        event_id: updated.id,
        is_default: updated.is_default,
    }))
}

async fn clear_default_flag(state: &AppState, app_id: &str) -> Result<(), ApiError> {
    let defaults = event::Entity::find()
        .filter(event::Column::AppId.eq(app_id))
        .filter(event::Column::IsDefault.eq(true))
        .all(&state.db)
        .await?;

    for model in defaults {
        let mut active: event::ActiveModel = model.into();
        active.is_default = Set(false);
        active.update(&state.db).await?;
    }

    Ok(())
}
