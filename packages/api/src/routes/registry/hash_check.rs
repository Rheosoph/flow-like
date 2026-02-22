use crate::entity::wasm_package_version;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct HashCheckRequest {
    pub hash: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct HashCheckResponse {
    pub exists: bool,
    pub package_id: Option<String>,
    pub version: Option<String>,
}

/// POST /registry/hash-check
/// Check if a WASM binary with the given blake3 hash already exists in the registry
#[utoipa::path(
    post,
    path = "/registry/hash-check",
    tag = "registry",
    request_body = HashCheckRequest,
    responses(
        (status = 200, description = "Hash check result", body = HashCheckResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn hash_check(
    Extension(user): Extension<AppUser>,
    State(state): State<AppState>,
    Json(body): Json<HashCheckRequest>,
) -> Result<Json<HashCheckResponse>, ApiError> {
    let _user_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    if body.hash.is_empty() {
        return Err(ApiError::bad_request("Hash must not be empty"));
    }

    let existing = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::WasmHash.eq(&body.hash))
        .one(&state.db)
        .await?;

    let response = match existing {
        Some(version) => HashCheckResponse {
            exists: true,
            package_id: Some(version.package_id),
            version: Some(version.version),
        },
        None => HashCheckResponse {
            exists: false,
            package_id: None,
            version: None,
        },
    };

    Ok(Json(response))
}
