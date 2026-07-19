pub mod alias;
pub mod db;
pub mod delete_event;
pub mod get_event;
pub mod get_event_versions;
pub mod get_events;
pub mod invoke_event;
pub mod invoke_event_async;
pub mod prerun_event;
pub mod registrations;
pub mod remote_proxy;
pub mod setup_event;
pub mod upsert_event;
pub mod upsert_event_feedback;
pub mod validate_event;

use axum::{
    Router,
    routing::{any, get, post, put},
};

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};

fn connected_app_direct_event_allowed(event_type: &str, active: bool) -> bool {
    active && event_type == "simple_chat"
}

fn generic_event_endpoint_allowed(event_type: &str) -> bool {
    event_type != "ontology_action"
}

/// Connected apps call REST/MCP events through the proxy so exposure and
/// registration auth cannot be bypassed. Chat events have no public proxy
/// surface and remain directly invocable through the generic handler.
pub(crate) fn ensure_connected_app_direct_event_allowed(
    user: &AppUser,
    event_type: &str,
    active: bool,
) -> Result<(), ApiError> {
    if !generic_event_endpoint_allowed(event_type) {
        return Err(ApiError::forbidden(
            "Ontology action events must be accessed through their governed ontology action endpoint",
        ));
    }
    if user.is_connected_app() && !connected_app_direct_event_allowed(event_type, active) {
        return Err(ApiError::forbidden(
            "Connected apps may directly invoke only active simple-chat events; use the REST or MCP proxy for other event types",
        ));
    }
    Ok(())
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_events::get_events))
        .route(
            "/{event_id}",
            get(get_event::get_event)
                .put(upsert_event::upsert_event)
                .delete(delete_event::delete_event),
        )
        .route(
            "/{event_id}/versions",
            get(get_event_versions::get_event_versions),
        )
        .route("/{event_id}/validate", post(validate_event::validate_event))
        .route("/{event_id}/setup", post(setup_event::setup_event))
        .route("/{event_id}/prerun", get(prerun_event::prerun_event))
        .route("/{event_id}/invoke", post(invoke_event::invoke_event))
        .route(
            "/{event_id}/invoke/async",
            post(invoke_event_async::invoke_event_async),
        )
        .route(
            "/{event_id}/registrations",
            get(registrations::list_registrations),
        )
        .route("/{event_id}/rest", any(remote_proxy::proxy_rest_root))
        .route("/{event_id}/rest/{*path}", any(remote_proxy::proxy_rest))
        .route("/{event_id}/mcp", any(remote_proxy::proxy_mcp))
        .route("/{event_id}/mcp/{*path}", any(remote_proxy::proxy_mcp_path))
        .route("/{event_id}/alias", get(alias::list_aliases))
        .route(
            "/{event_id}/alias/{slug}",
            get(alias::get_alias)
                .put(alias::upsert_alias)
                .delete(alias::delete_alias),
        )
        .route(
            "/{event_id}/feedback",
            put(upsert_event_feedback::upsert_event_feedback),
        )
}

#[cfg(test)]
mod tests {
    use super::{connected_app_direct_event_allowed, generic_event_endpoint_allowed};

    #[test]
    fn connected_apps_can_directly_invoke_only_active_chat_events() {
        assert!(connected_app_direct_event_allowed("simple_chat", true));
        assert!(!connected_app_direct_event_allowed("simple_chat", false));
        assert!(!connected_app_direct_event_allowed("rest", true));
        assert!(!connected_app_direct_event_allowed("mcp", true));
        assert!(!connected_app_direct_event_allowed("webhook", true));
    }

    #[test]
    fn managed_ontology_actions_cannot_use_generic_event_endpoints() {
        assert!(!generic_event_endpoint_allowed("ontology_action"));
        assert!(generic_event_endpoint_allowed("generic"));
        assert!(generic_event_endpoint_allowed("simple_chat"));
    }
}
