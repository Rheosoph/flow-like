pub mod delete_route;
pub mod get_default_route;
pub mod get_route_by_path;
pub mod get_routes;
pub mod upsert_route;

use axum::{
    Router,
    routing::{get, put},
};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(get_routes::get_routes).post(upsert_route::create_route),
        )
        .route("/by-path", get(get_route_by_path::get_route_by_path))
        .route("/default", get(get_default_route::get_default_route))
        .route(
            "/{route_id}",
            put(upsert_route::update_route).delete(delete_route::delete_route),
        )
}
