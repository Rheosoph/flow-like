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
pub mod setup_event;
pub mod upsert_event;
pub mod upsert_event_feedback;
pub mod validate_event;

use axum::{
    Router,
    routing::{get, post, put},
};

use crate::state::AppState;

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
