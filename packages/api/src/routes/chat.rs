use axum::{
    Router,
    routing::{get, post},
};

use crate::state::AppState;

mod bedrock_auth;
pub mod completions;
mod hosted_bit_policy;
mod hosted_connection;
pub(crate) mod hosted_worker;
mod relay;
pub(crate) use relay::current_instance_model_tier;
pub mod responses;
pub mod usage;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/completions", post(completions::invoke_llm))
        .route("/usage", get(usage::get_llm_usage))
}
