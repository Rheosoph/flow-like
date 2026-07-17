use axum::{
    Router,
    routing::{get, post},
};

use crate::{
    error::ApiError,
    middleware::jwt::AppUser,
    routes::app::db::{ScopeParams, resolve_connection},
    state::AppState,
};
use flow_like_storage::databases::graph::lancegraph::{self, GraphOverlayDef};
use flow_like_storage::lancedb::Connection;

pub mod actions;
pub mod analytics;
pub mod create_overlay;
pub mod cypher;
pub mod delete_overlay;
pub mod get_overlay;
pub mod list_imports;
pub mod list_overlays;
pub mod neighbors;
pub mod paths;
pub mod sample;
pub mod schema;
pub mod search;
pub mod sql;
pub mod subgraph;
pub mod update_overlay;
pub mod validate;

/// Resolves the scoped connection and loads the overlay, mapping a missing
/// overlay to 404 and enforcing the `exposed` contract for connected apps on
/// every query surface — not just actions.
pub(crate) async fn load_scoped_overlay(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    overlay_id: &str,
    scope: &ScopeParams,
) -> Result<(Connection, GraphOverlayDef), ApiError> {
    let connection = resolve_connection(state, user, app_id, scope).await?;
    let overlay = lancegraph::load_overlay(&connection, overlay_id)
        .await
        .map_err(|_| ApiError::not_found("Graph overlay not found"))?;
    if user.is_connected_app() && !overlay.exposed {
        return Err(ApiError::forbidden(
            "This ontology is not exposed to connected projects",
        ));
    }
    Ok((connection, overlay))
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/imports", get(list_imports::list_imports))
        .route(
            "/",
            get(list_overlays::list_overlays).post(create_overlay::create_overlay),
        )
        .route(
            "/{overlay_id}",
            get(get_overlay::get_overlay)
                .put(update_overlay::update_overlay)
                .delete(delete_overlay::delete_overlay),
        )
        .route("/{overlay_id}/schema", get(schema::graph_schema))
        .route("/{overlay_id}/validate", post(validate::validate_overlay))
        .route("/{overlay_id}/cypher", post(cypher::run_cypher))
        .route("/{overlay_id}/sql", post(sql::run_sql))
        .route("/{overlay_id}/neighbors", post(neighbors::neighbors))
        .route("/{overlay_id}/subgraph", post(subgraph::subgraph))
        .route("/{overlay_id}/paths", post(paths::find_paths))
        .route("/{overlay_id}/analytics", get(analytics::graph_analytics))
        .route("/{overlay_id}/search", post(search::search_nodes))
        .route("/{overlay_id}/sample", get(sample::sample_nodes))
        .route(
            "/{overlay_id}/actions/{action_id}/invoke",
            post(actions::invoke_ontology_action),
        )
        .route(
            "/{overlay_id}/actions/{action_id}/prerun",
            get(actions::prerun_ontology_action),
        )
}
