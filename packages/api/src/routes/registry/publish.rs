//! Package publish endpoint — two-step flow
//!
//! 1. Client uploads WASM via presigned URL to tmp/wasm/{sub}/{id}/{version}.wasm
//! 2. Client calls this endpoint with manifest (server constructs tmp path from sub)
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
}

/// POST /registry/publish
/// Publish a package using two-step flow: WASM already uploaded via upload-url
#[utoipa::path(
    post,
    path = "/registry/publish",
    tag = "registry",
    request_body = TwoStepPublishRequest,
    responses(
        (status = 200, description = "Package published successfully", body = PublishResponse),
        (status = 400, description = "Invalid manifest or WASM binary"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Not authorized to publish to this package"),
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

    // If the package already exists, verify the caller is owner or maintainer
    {
        use crate::entity::wasm_package;
        use sea_orm::EntityTrait;

        if let Some(_existing) = wasm_package::Entity::find_by_id(&request.manifest.id)
            .one(&state.db)
            .await
            .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        {
            crate::ensure_wasm_permission!(
                state,
                &sub,
                &request.manifest.id,
                crate::permission::wasm_package_permission::WasmPackagePermission::Maintainer
            );
        }
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
        .finalize_publish(
            request.manifest.clone(),
            &sub,
            email,
        )
        .await?;

    Ok(Json(response))
}
