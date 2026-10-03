//! Execution routes module
//!
//! Contains routes for:
//! - Executor → API: progress reporting, event pushing
//! - User → API: long polling, status queries
//! - Public: JWKS for JWT verification

use crate::state::AppState;
use axum::{
    Router,
    routing::{get, post},
};

pub mod cancel;
pub mod progress;
pub mod public_key;
pub mod quota;
pub mod widgets;

pub fn routes() -> Router<AppState> {
    Router::new()
        // Executor endpoints (require executor JWT)
        .route("/progress", post(progress::report_progress))
        .route("/quota", post(quota::report))
        .route("/events", post(progress::push_events))
        .route(
            "/apps/{app_id}/teams/send",
            post(crate::teams::runtime::send),
        )
        .route(
            "/apps/{app_id}/teams/messages",
            post(crate::teams::lookup::messages),
        )
        .route(
            "/apps/{app_id}/teams/members",
            post(crate::teams::lookup::members),
        )
        .route(
            "/apps/{app_id}/mail/send",
            post(crate::routes::app::mail::executor_send_mail),
        )
        .route(
            "/apps/{app_id}/mail/reply",
            post(crate::routes::app::mail::executor_reply_mail),
        )
        .route("/result", get(progress::executor_result))
        .route("/apps/{app_id}/widgets", get(widgets::get_app_widgets))
        .route(
            "/apps/{app_id}/package-widgets",
            get(widgets::get_app_package_widgets),
        )
        // User endpoints (require user JWT)
        .route("/poll", get(progress::poll_status))
        // App-auth endpoints (require normal app access)
        .route(
            "/run/{run_id}",
            get(progress::get_run_status).delete(cancel::cancel_run),
        )
        // Public endpoints
        .route(
            "/.well-known/jwks.json",
            get(public_key::get_execution_jwks),
        )
}
