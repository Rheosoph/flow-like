use axum::{
    Router,
    routing::{get, post},
};

use crate::state::AppState;

pub mod create_overlay;
pub mod cypher;
pub mod delete_overlay;
pub mod get_overlay;
pub mod list_overlays;
pub mod neighbors;
pub mod sample;
pub mod schema;
pub mod search;
pub mod sql;
pub mod subgraph;
pub mod update_overlay;
pub mod validate;

pub fn routes() -> Router<AppState> {
    Router::new()
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
        .route("/{overlay_id}/search", post(search::search_nodes))
        .route("/{overlay_id}/sample", get(sample::sample_nodes))
}
