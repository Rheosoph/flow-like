use crate::{error::InternalError, state::AppState};
use axum::{Json, Router, extract::State, routing::get};
use flow_like::hub::{Contact, Features};

pub mod get_profile_templates;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/home-defaults", get(super::admin::home_defaults::get_home_defaults))
        .route("/legal", get(legal_notice))
        .route("/privacy", get(privacy_policy))
        .route("/terms", get(terms_of_service))
        .route("/contact", get(contact))
        .route("/features", get(features))
        .route(
            "/profiles",
            get(get_profile_templates::get_profile_templates),
        )
}

#[tracing::instrument(name = "GET /info/legal", skip(state))]
async fn legal_notice(State(state): State<AppState>) -> Result<String, InternalError> {
    let notice = state.platform_config.legal_notice.clone();
    Ok(notice)
}

#[tracing::instrument(name = "GET /info/privacy", skip(state))]
async fn privacy_policy(State(state): State<AppState>) -> Result<String, InternalError> {
    let privacy_policy = state.platform_config.privacy_policy.clone();
    Ok(privacy_policy)
}

#[tracing::instrument(name = "GET /info/terms", skip(state))]
async fn terms_of_service(State(state): State<AppState>) -> Result<String, InternalError> {
    let terms_of_service = state.platform_config.terms_of_service.clone();
    Ok(terms_of_service)
}

#[tracing::instrument(name = "GET /info/contact", skip(state))]
async fn contact(State(state): State<AppState>) -> Result<Json<Contact>, InternalError> {
    let contact = state.platform_config.contact.clone();
    Ok(Json(contact))
}

#[tracing::instrument(name = "GET /info/features", skip(state))]
async fn features(State(state): State<AppState>) -> Result<Json<Features>, InternalError> {
    let features = state.platform_config.features.clone();
    Ok(Json(features))
}
