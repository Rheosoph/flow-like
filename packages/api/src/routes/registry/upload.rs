//! Presigned upload URL endpoint for WASM binaries

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use flow_like_types::create_id;
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct UploadUrlResponse {
    pub upload_url: String,
    pub tmp_path: String,
    pub expires_in_secs: u64,
}

/// POST /registry/upload-url
/// Get a presigned PUT URL for uploading a WASM binary to temporary storage.
#[utoipa::path(
    post,
    path = "/registry/upload-url",
    tag = "registry",
    responses(
        (status = 200, description = "Presigned upload URL", body = UploadUrlResponse),
        (status = 401, description = "Authentication required"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_upload_url(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<UploadUrlResponse>, ApiError> {
    let sub = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    if sub.is_empty() {
        return Err(ApiError::unauthorized("Authentication required"));
    }

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let tmp_path = format!("tmp/wasm/{}.wasm", create_id());

    let upload_url = registry
        .get_upload_url(&tmp_path)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to generate upload URL: {}", e)))?;

    Ok(Json(UploadUrlResponse {
        upload_url,
        tmp_path,
        expires_in_secs: 3600,
    }))
}
