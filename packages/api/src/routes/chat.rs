use axum::{
    Router,
    routing::{get, post},
};

use crate::state::AppState;

pub mod completions;
mod relay;
pub mod responses;
pub mod usage;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/completions", post(completions::invoke_llm))
        .route("/usage", get(usage::get_llm_usage))
}
