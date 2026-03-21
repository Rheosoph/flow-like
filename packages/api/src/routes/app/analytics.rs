use axum::{
    Router,
    routing::get,
};

use crate::state::AppState;

pub mod dashboard;
pub mod feedback;
pub mod overview;
pub mod update_aggregations;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(overview::get_analytics_overview))
        .route("/stats", get(overview::get_analytics_stats))
        .route("/feedback", get(feedback::list_feedback))
        .route("/dashboard", get(dashboard::dashboard))
}
