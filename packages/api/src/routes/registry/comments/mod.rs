pub mod get_comments;
pub mod remove_comment;
pub mod upsert_comment;

use axum::{Router, routing::get};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/",
            get(get_comments::get_comments).put(upsert_comment::upsert_comment),
        )
        .route(
            "/{comment_id}",
            axum::routing::delete(remove_comment::remove_comment),
        )
}
