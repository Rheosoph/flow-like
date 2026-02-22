//! Package publish endpoint — two-step flow
//!
//! 1. Client uploads WASM via presigned URL to tmp/wasm/{uuid}.wasm
//! 2. Client calls this endpoint with manifest + tmp_path
//! 3. Server fetches WASM from tmp, hashes, moves to final path, compiles in parallel

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use super::types::PublishResponse;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct TwoStepPublishRequest {
    pub manifest: flow_like_wasm::manifest::PackageManifest,
    pub tmp_path: String,
}

/// POST /registry/publish
/// Publish a package using two-step flow: WASM already uploaded to tmp_path
#[utoipa::path(
    post,
    path = "/registry/publish",
    tag = "registry",
    request_body = TwoStepPublishRequest,
    responses(
        (status = 200, description = "Package published successfully", body = PublishResponse),
        (status = 400, description = "Invalid manifest or WASM binary"),
        (status = 401, description = "Authentication required"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn publish(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<TwoStepPublishRequest>,
) -> Result<Json<PublishResponse>, ApiError> {
    let sub = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required for publishing"))?;

    if sub.is_empty() {
        return Err(ApiError::unauthorized(
            "Authentication required for publishing",
        ));
    }

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    if let Err(errors) = request.manifest.validate() {
        return Err(ApiError::bad_request(format!(
            "Invalid manifest: {}",
            errors.join(", ")
        )));
    }

    if !request.tmp_path.starts_with("tmp/wasm/") || !request.tmp_path.ends_with(".wasm") {
        return Err(ApiError::bad_request(
            "Invalid tmp_path: must be tmp/wasm/<id>.wasm",
        ));
    }

    let email = {
        use crate::entity::user;
        use sea_orm::EntityTrait;
        user::Entity::find_by_id(&sub)
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .and_then(|u| u.email)
    };

    let response = registry
        .publish_from_tmp(
            request.manifest.clone(),
            &request.tmp_path,
            Some(sub),
            email,
        )
        .await?;

    Ok(Json(response))
}
