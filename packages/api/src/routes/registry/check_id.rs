//! Package ID availability check endpoint

use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckIdRequest {
    pub id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CheckIdResponse {
    pub available: bool,
    pub owned_by_caller: bool,
}

/// POST /registry/check-id
/// Check whether a package ID is available or already owned by the caller.
#[utoipa::path(
    post,
    path = "/registry/check-id",
    tag = "registry",
    request_body = CheckIdRequest,
    responses(
        (status = 200, description = "ID availability result", body = CheckIdResponse),
        (status = 401, description = "Authentication required"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn check_id(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<CheckIdRequest>,
) -> Result<Json<CheckIdResponse>, ApiError> {
    let sub = user.sub()?;

    if request.id.is_empty() {
        return Err(ApiError::bad_request("id is required"));
    }

    use crate::entity::wasm_package;
    use sea_orm::EntityTrait;

    let existing = wasm_package::Entity::find_by_id(&request.id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    if existing.is_none() {
        return Ok(Json(CheckIdResponse {
            available: true,
            owned_by_caller: false,
        }));
    }

    let perm = crate::check_wasm_access!(state, &sub, &request.id);
    let owned = perm
        .map(|p| {
            p.has_permission(
                crate::permission::wasm_package_permission::WasmPackagePermission::Maintainer,
            )
        })
        .unwrap_or(false);

    Ok(Json(CheckIdResponse {
        available: owned,
        owned_by_caller: owned,
    }))
}
