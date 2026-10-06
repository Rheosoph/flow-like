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

use super::current_inputs::with_current_inputs;
use super::db::{filter_event_secrets, get_events_for_app, get_events_with_fallback};

/// Readers see every listed event; everyone else only the active, user-facing ones.
fn is_listed_for(event: &Event, can_read_events: bool) -> bool {
    is_listed_event_type(&event.event_type)
        && (can_read_events || (event.active && is_user_facing_event(event)))
}

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

    let can_read_events = permission.has_permission(RolePermissions::ReadEvents);
    let can_use_direct_board = permission.has_permission(RolePermissions::ReadBoards)
        && permission.has_permission(RolePermissions::ExecuteBoards);
    let events: Vec<Event> = events
        .into_iter()
        .filter(|event| is_listed_for(event, can_read_events))
        .collect();
    let events = with_current_inputs(&state, &app_id, events)
        .await
        .into_iter()
        .map(|event| redact_listed(event, can_read_events, can_use_direct_board))
        .collect();

    Ok(Json(events))
}

/// Secret values are blanked for everyone. ReadEvents callers may edit and PUT these complete
/// definitions; the runtime-only projection also drops the execution details and, without direct
/// board access, the Page Board selector.
fn redact_listed(event: Event, can_read_events: bool, can_use_direct_board: bool) -> Event {
    let event = filter_event_secrets(event);
    if can_read_events {
        return event;
    }
    let event = filter_event_list_execution(event);
    if can_use_direct_board {
        event
    } else {
        redact_page_event_board_metadata(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{
        pin::ValueType,
        variable::{Variable, VariableType},
    };
    use std::collections::HashMap;

    fn event(event_type: &str, active: bool, page: Option<&str>) -> Event {
        let mut token = Variable::new("token", VariableType::String, ValueType::Normal);
        token.secret = true;
        token.default_value = Some(b"hush".to_vec());
        Event {
            id: "event".into(),
            name: "Event".into(),
            description: String::new(),
            board_id: "board".into(),
            board_version: Some((1, 0, 0)),
            node_id: "node".into(),
            variables: HashMap::from([("token".to_string(), token)]),
            config: Vec::new(),
            active,
            canary: None,
            variants: Vec::new(),
            priority: 0,
            event_type: event_type.into(),
            notes: None,
            event_version: (1, 0, 0),
            created_at: std::time::UNIX_EPOCH,
            updated_at: std::time::UNIX_EPOCH,
            default_page_id: page.map(Into::into),
            inputs: Vec::new(),
            route: None,
            is_default: false,
            execution_mode: Default::default(),
            exposure: Default::default(),
            correlation_mappings: None,
        }
    }

    #[test]
    fn readers_list_every_listed_event_and_runtime_callers_the_active_user_facing_ones() {
        let cases = [
            (event("ontology_action", true, None), false, false),
            (event("generic_form", false, None), true, false),
            (event("api", true, None), true, false),
            (event("api", true, Some("page")), true, true),
            (event("generic_form", true, None), true, true),
        ];
        for (event, reader, runtime) in cases {
            let kind = (&event.event_type, event.active, &event.default_page_id);
            assert_eq!(is_listed_for(&event, true), reader, "reader, {kind:?}");
            assert_eq!(is_listed_for(&event, false), runtime, "runtime, {kind:?}");
        }
    }

    #[test]
    fn runtime_callers_get_the_redacted_projection() {
        let reader = redact_listed(event("generic_form", true, Some("page")), true, false);
        assert_eq!(reader.variables["token"].default_value, None);
        assert_eq!(reader.board_id, "board");

        let direct = redact_listed(event("generic_form", true, Some("page")), false, true);
        assert!(direct.variables.is_empty());
        assert_eq!(direct.board_id, "board");

        let runtime = redact_listed(event("generic_form", true, Some("page")), false, false);
        assert!(runtime.variables.is_empty());
        assert!(runtime.board_id.is_empty() && runtime.node_id.is_empty());
        assert_eq!(runtime.board_version, None);
    }
}
