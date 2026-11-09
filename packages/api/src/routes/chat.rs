use axum::{
    Router,
    extract::DefaultBodyLimit,
    routing::{get, post},
};

use crate::state::AppState;

/// The largest request a hosted model takes, inline media included. A run's media handling keeps
/// what it inlines for a hosted model within this, and links travel instead where they can.
pub(crate) const MAX_MODEL_REQUEST_BYTES: usize =
    flow_like::flow_like_model_provider::llm::relay::MAX_REQUEST_BYTES;

pub(crate) fn model_request_limit() -> DefaultBodyLimit {
    DefaultBodyLimit::max(MAX_MODEL_REQUEST_BYTES)
}

mod bedrock_auth;
pub mod completions;
mod hosted_bit_policy;
mod hosted_connection;
pub(crate) mod hosted_worker;
mod relay;
pub(crate) use relay::{
    UsageRequestContext, current_instance_model_tier, enforce_tier, required_model_tier,
    resolve_usage_context, track_llm_usage,
};
pub mod responses;
pub mod systemone;
pub mod usage;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/completions",
            post(completions::invoke_llm).layer(model_request_limit()),
        )
        .route("/usage", get(usage::get_llm_usage))
}
