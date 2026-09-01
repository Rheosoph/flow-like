use crate::{
    error::ApiError, middleware::jwt::AppUser, permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like::flow::event::Event;
use serde::Deserialize;
use utoipa::ToSchema;

use super::db::{
    filter_event_list_execution, filter_event_secrets, get_event_from_db_opt,
    redact_page_event_board_metadata, sync_event_to_db,
};

/// The event artifact lookup reports a plain missing object as an internal
/// error. After membership and permission checks passed, an absent event must
/// be a precise 404 — the desktop client keys local-tombstone behavior on it.
fn map_missing_event_artifact(event_id: &str, error: flow_like_types::Error) -> ApiError {
    if matches!(
        error.downcast_ref::<flow_like_storage::object_store::Error>(),
        Some(flow_like_storage::object_store::Error::NotFound { .. })
    ) {
        return ApiError::not_found(format!("Event {event_id} not found"));
    }
    ApiError::from(error)
}

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
    let permission = user.app_permission_fresh(&app_id, &state).await?;
    if !permission.has_permission(RolePermissions::ReadEvents)
        && !permission.has_permission(RolePermissions::ExecuteEvents)
    {
        return Err(ApiError::FORBIDDEN);
    }
    let sub = permission.sub()?;
    let has_read = permission.has_permission(RolePermissions::ReadEvents);
    let can_use_direct_board = permission.has_permission(RolePermissions::ReadBoards)
        && permission.has_permission(RolePermissions::ExecuteBoards);

    let version_opt =
        match query.version.as_deref() {
            Some(ver_str) => Some(super::parse_version_tuple(ver_str).ok_or_else(|| {
                ApiError::bad_request("version must be in MAJOR_MINOR_PATCH format")
            })?),
            None => None,
        };

    // Current events normally come from the database mirror. Older apps (or an
    // interrupted sync) can still have a valid event artifact without that row,
    // so repair the mirror from bucket storage on a miss.
    let event = if version_opt.is_none() {
        if let Some(event) = get_event_from_db_opt(&state.db, &event_id, &app_id).await? {
            event
        } else {
            let app = state.master_app(&sub, &app_id, &state).await?;
            let event = app
                .get_event(&event_id, None)
                .await
                .map_err(|error| map_missing_event_artifact(&event_id, error))?;
            if event.id != event_id {
                tracing::error!(
                    expected_event_id = %event_id,
                    artifact_event_id = %event.id,
                    app_id = %app_id,
                    "Event artifact ID does not match the requested event"
                );
                return Err(ApiError::internal("Event artifact ID mismatch"));
            }
            if let Err(error) = sync_event_to_db(&state.db, &app_id, &event).await {
                tracing::warn!(
                    event_id = %event_id,
                    app_id = %app_id,
                    %error,
                    "Failed to repair event database mirror"
                );
            }
            event
        }
    } else {
        let app = state.master_app(&sub, &app_id, &state).await?;
        app.get_event(&event_id, version_opt)
            .await
            .map_err(|error| map_missing_event_artifact(&event_id, error))?
    };

    let event = filter_event_secrets(event);
    let mut event = if has_read {
        event
    } else {
        filter_event_list_execution(event)
    };
    // ReadEvents callers may round-trip this definition through PUT. Runtime
    // callers have no such editing contract and receive no direct Board path.
    if !has_read && !can_use_direct_board {
        event = redact_page_event_board_metadata(event);
    }

    Ok(Json(event))
}
