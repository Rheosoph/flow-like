use crate::{
    ensure_fresh_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::events::db::{
        filter_event_list_execution, is_listed_event_type, is_user_facing_event,
        redact_page_event_board_metadata,
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::event::Event;

use super::db::{filter_event_secrets, get_events_for_app, get_events_with_fallback};

#[tracing::instrument(name = "GET /apps/{app_id}/events", skip(state, user))]
#[utoipa::path(
    get,
    path = "/apps/{app_id}/events",
    tag = "events",
    description = "List events for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Event list", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
pub async fn get_events(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<Event>>, ApiError> {
    let permission = ensure_fresh_permission!(user, &app_id, &state, RolePermissions::ListEvents);

    // Prefer the database mirror, but recover legacy/interrupted-sync apps when
    // the mirror is empty. The fallback also backfills rows for future reads.
    let mut events = get_events_for_app(&state.db, &app_id).await?;
    if events.is_empty() {
        let principal_id = permission.identifier();
        let app = state.master_app(&principal_id, &app_id, &state).await?;
        events = get_events_with_fallback(&state.db, state.db_dialect, &app).await?;
    }

    // Filter out secret variable values from all events
    let mut events: Vec<Event> = events
        .into_iter()
        .filter(|event| is_listed_event_type(&event.event_type))
        .map(filter_event_secrets)
        .collect();

    let can_read_events = permission.has_permission(RolePermissions::ReadEvents);
    if !can_read_events {
        events = events
            .into_iter()
            .filter(|e| e.active)
            .filter(is_user_facing_event)
            .map(filter_event_list_execution)
            .collect();
    }

    let can_use_direct_board = permission.has_permission(RolePermissions::ReadBoards)
        && permission.has_permission(RolePermissions::ExecuteBoards);
    // ReadEvents callers may edit and PUT these complete definitions. The
    // runtime-only projection hides Page Board selectors.
    if !can_read_events && !can_use_direct_board {
        events = events
            .into_iter()
            .map(redact_page_event_board_metadata)
            .collect();
    }

    Ok(Json(events))
}
