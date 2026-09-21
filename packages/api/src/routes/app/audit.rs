//! Owner-facing audit export, nested at `/apps/{app_id}/audit`: pull the app's sealed
//! records as NDJSON or have the audit worker push them to a webhook.

pub mod export;
pub mod webhook;

use axum::{
    Router,
    routing::{get, post},
};

use crate::state::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/export", get(export::export_records))
        .route(
            "/webhook",
            get(webhook::get_webhook)
                .put(webhook::put_webhook)
                .delete(webhook::delete_webhook),
        )
        .route("/webhook/rotate", post(webhook::rotate_webhook_secret))
}
