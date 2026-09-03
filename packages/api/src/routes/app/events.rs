pub mod alias;
pub mod canary;
pub mod db;
pub mod delete_event;
pub mod get_event;
pub mod get_event_runs;
pub mod get_event_timeline;
pub mod get_event_versions;
pub mod get_events;
pub mod invoke_event;
pub mod invoke_event_async;
pub mod page_trigger;
pub mod prerun_event;
pub mod registrations;
pub mod regression;
pub mod remote_proxy;
pub mod restore_event;
pub mod setup_event;
pub mod upsert_event;
pub mod upsert_event_feedback;
pub mod validate_event;

use axum::{
    Router,
    routing::{any, get, patch, post, put},
};

use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};

/// Parse a version string in `MAJOR_MINOR_PATCH` (or dotted `MAJOR.MINOR.PATCH`)
/// form into a numeric tuple. Returns `None` for malformed input (wrong arity or
/// non-numeric components) so callers can surface a 400 instead of a 500.
pub(crate) fn parse_version_tuple(raw: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = raw.split(['_', '.']).collect();
    match parts.as_slice() {
        [major, minor, patch] => Some((
            major.parse().ok()?,
            minor.parse().ok()?,
            patch.parse().ok()?,
        )),
        _ => None,
    }
}

/// Dotted `MAJOR.MINOR.PATCH` — the one event-version key format, shared by
/// timeline entries and the Lance `runs` table's `event_version` column. Every
/// producer must use this helper: the board `version` column in the same table
/// uses `v{major}-{minor}-{patch}`, and mixing the two silently breaks
/// version grouping.
pub(crate) fn dotted_version_key(version: (u32, u32, u32)) -> String {
    format!("{}.{}.{}", version.0, version.1, version.2)
}

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
        .route(
            "/{event_id}/timeline",
            get(get_event_timeline::get_event_timeline),
        )
        .route("/{event_id}/runs", get(get_event_runs::get_event_runs))
        .route("/{event_id}/restore", post(restore_event::restore_event))
        .route("/{event_id}/validate", post(validate_event::validate_event))
        .route("/{event_id}/setup", post(setup_event::setup_event))
        .route("/{event_id}/setups", get(canary::list_event_setups))
        .route(
            "/{event_id}/prerun",
            get(prerun_event::prerun_event).post(prerun_event::prerun_page_event),
        )
        .route("/{event_id}/invoke", post(invoke_event::invoke_event))
        .route(
            "/{event_id}/invoke/async",
            post(invoke_event_async::invoke_event_async),
        )
        .route(
            "/{event_id}/registrations",
            get(registrations::list_registrations),
        )
        .route("/{event_id}/corpus", get(regression::get_event_corpus))
        .route(
            "/{event_id}/corpus/{run_id}/payload",
            get(regression::get_corpus_payload),
        )
        .route(
            "/{event_id}/regression/fixtures",
            post(regression::promote_regression_fixture),
        )
        .route(
            "/{event_id}/regression/fixtures/{fixture_id}",
            axum::routing::delete(regression::delete_regression_fixture),
        )
        .route(
            "/{event_id}/regression/suite",
            get(regression::get_regression_suite).put(regression::put_regression_suite),
        )
        .route(
            "/{event_id}/regression/run",
            post(regression::run_regression_suite),
        )
        .route(
            "/{event_id}/regression/runs",
            get(regression::list_regression_runs),
        )
        .route(
            "/{event_id}/regression/runs/{suite_run_id}",
            get(regression::get_regression_run),
        )
        .route("/{event_id}/canary/explain", get(canary::explain_canary))
        .route("/{event_id}/canary/stats", get(canary::canary_stats))
        .route("/{event_id}/canary/promote", post(canary::promote_canary))
        .route("/{event_id}/canary/abort", post(canary::abort_canary))
        .route("/{event_id}/canary", patch(canary::patch_canary))
        .route("/{event_id}/variants", put(canary::put_event_variants))
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
    use super::{
        connected_app_direct_event_allowed, dotted_version_key, generic_event_endpoint_allowed,
        parse_version_tuple,
    };

    /// `StoredLogMeta.event_version` is dotted while `StoredLogMeta.version`
    /// (the board) is `v{major}-{minor}-{patch}` — this fails if the shared
    /// helper ever drifts toward the board format.
    #[test]
    fn dotted_version_key_is_the_lance_event_version_format() {
        assert_eq!(dotted_version_key((1, 0, 3)), "1.0.3");
        assert_ne!(dotted_version_key((1, 0, 3)), "v1-0-3");
        assert_eq!(
            parse_version_tuple(&dotted_version_key((4, 5, 6))),
            Some((4, 5, 6))
        );
    }

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
