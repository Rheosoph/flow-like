use axum::{Router, routing::post};

use crate::state::AppState;

pub mod embed;
mod providers;

pub fn routes() -> Router<AppState> {
    Router::new().route("/embed", post(embed::embed_text))
}
