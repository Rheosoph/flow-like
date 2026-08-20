pub mod context;
pub mod presign;

use axum::{Router, routing::get};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/presign", get(presign::presign))
        .route("/context", get(context::execution_context))
}
