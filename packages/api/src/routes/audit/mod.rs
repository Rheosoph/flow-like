pub mod query;
pub mod verify;

use axum::{Router, routing::get};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/entries", get(query::query_audit_entries))
        .route("/verify", get(verify::verify_chain))
}
