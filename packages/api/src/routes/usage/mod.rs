use axum::{Router, routing::get};

use crate::state::AppState;

mod activity;
pub(crate) mod cache;
mod history;

#[cfg(test)]
pub(crate) mod test_db;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/llm", get(history::get_llm_history))
        .route("/embeddings", get(history::get_embedding_history))
        .route("/executions", get(history::get_execution_history))
        .route("/executions/recent", get(history::get_recent_executions))
        .route(
            "/executions/activity",
            get(activity::get_execution_activity),
        )
        .route("/summary", get(history::get_usage_summary))
}
