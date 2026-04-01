use crate::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};

pub mod get_publication;
pub mod upsert_publication;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_publication::get_publication_requests))
        .route("/request", post(upsert_publication::request_publication))
}
