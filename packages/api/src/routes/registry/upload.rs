//! Presigned upload URL endpoint for WASM binaries

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UploadUrlRequest {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UploadUrlResponse {
    pub upload_url: String,
    pub expires_in_secs: u64,
}

/// POST /registry/upload-url
/// Get a presigned PUT URL for uploading a WASM binary to temporary storage.
#[utoipa::path(
    post,
    path = "/registry/upload-url",
    tag = "registry",
    request_body = UploadUrlRequest,
    responses(
        (status = 200, description = "Presigned upload URL", body = UploadUrlResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Authentication required"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_upload_url(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<UploadUrlRequest>,
) -> Result<Json<UploadUrlResponse>, ApiError> {
    let sub = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    if sub.is_empty() {
        return Err(ApiError::unauthorized("Authentication required"));
    }

    if request.id.is_empty() || request.version.is_empty() {
        return Err(ApiError::bad_request("id and version are required"));
    }

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let tmp_path = format!("tmp/wasm/{}/{}/{}.wasm", sub, request.id, request.version);

    let upload_url = registry
        .get_upload_url(&tmp_path)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to generate upload URL: {}", e)))?;

    Ok(Json(UploadUrlResponse {
        upload_url,
        expires_in_secs: 3600,
    }))
}
