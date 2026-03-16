//! Package version availability check endpoint

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
pub struct CheckVersionRequest {
    pub id: String,
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CheckVersionResponse {
    pub available: bool,
}

/// POST /registry/check-version
/// Check whether a specific version of a package is available.
#[utoipa::path(
    post,
    path = "/registry/check-version",
    tag = "registry",
    request_body = CheckVersionRequest,
    responses(
        (status = 200, description = "Version availability result", body = CheckVersionResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Authentication required"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn check_version(
    State(state): State<AppState>,
    Extension(_user): Extension<AppUser>,
    Json(request): Json<CheckVersionRequest>,
) -> Result<Json<CheckVersionResponse>, ApiError> {
    if request.id.is_empty() || request.version.is_empty() {
        return Err(ApiError::bad_request("id and version are required"));
    }

    let existing = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(&request.id))
        .filter(wasm_package_version::Column::Version.eq(&request.version))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    Ok(Json(CheckVersionResponse {
        available: existing.is_none(),
    }))
}
