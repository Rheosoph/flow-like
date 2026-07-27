use crate::{
    ensure_in_project, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::event::Event;
use serde::Deserialize;
use utoipa::ToSchema;

use super::db::{filter_event_list_execution, filter_event_secrets, get_event_from_db};

#[derive(Deserialize, Debug, ToSchema)]
pub struct VersionQuery {
    /// expected format: "MAJOR_MINOR_PATCH", e.g. "1_0_3"
    pub version: Option<String>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}",
    tag = "events",
    description = "Get an event by ID and optional version.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("version" = Option<String>, Query, description = "Version in MAJOR_MINOR_PATCH format")
    ),
    responses(
        (status = 200, description = "Event payload", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
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
    name = "GET /apps/{app_id}/events/{event_id}",
    skip(state, user, query)
)]
pub async fn get_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<VersionQuery>,
) -> Result<Json<Event>, ApiError> {
    let permission = ensure_in_project!(user, &app_id, &state);
    if !permission.has_permission(RolePermissions::ReadEvents)
        && !permission.has_permission(RolePermissions::ExecuteEvents)
    {
        return Err(ApiError::FORBIDDEN);
    }
    let sub = permission.sub()?;
    let has_read = permission.has_permission(RolePermissions::ReadEvents);

    let version_opt = match query.version.as_deref() {
        Some(ver_str) => Some(super::parse_version_tuple(ver_str).ok_or_else(|| {
            ApiError::bad_request("version must be in MAJOR_MINOR_PATCH format")
        })?),
        None => None,
    };

    // For current version, use database lookup
    // For historical versions, fall back to bucket
    let event = if version_opt.is_none() {
        get_event_from_db(&state.db, &event_id, &app_id).await?
    } else {
        let app = state.master_app(&sub, &app_id, &state).await?;
        app.get_event(&event_id, version_opt).await?
    };

    let event = filter_event_secrets(event);
    let event = if has_read {
        event
    } else {
        filter_event_list_execution(event)
    };

    Ok(Json(event))
}
