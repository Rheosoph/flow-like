use axum::{
    Router,
    routing::{get, post},
};
use flow_like::bit::LlmModelEvaluation;

use crate::{entity::llm_model, state::AppState};

pub mod get_bit;
pub mod get_with_dependencies;
pub mod search_bits;

pub(crate) fn llm_model_to_evaluation(model: llm_model::Model) -> LlmModelEvaluation {
    LlmModelEvaluation {
        slug: model.slug,
        name: model.name,
        release_date: model.release_date.map(|date| date.to_rfc3339()),
        creator_name: model.creator_name,
        creator_slug: model.creator_slug,
        evaluations: model.evaluations,
        pricing: model.pricing,
        median_output_tokens_per_second: model.median_output_tokens_per_second,
        median_time_to_first_token_seconds: model.median_time_to_first_token_seconds,
        median_time_to_first_answer_token: model.median_time_to_first_answer_token,
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", post(search_bits::search_bits))
        .route("/{bit_id}", get(get_bit::get_bit))
        .route(
            "/{bit_id}/dependencies",
            get(get_with_dependencies::get_with_dependencies),
        )
}
