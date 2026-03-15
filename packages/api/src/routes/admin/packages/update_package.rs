//! Update package status

use crate::audit;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateRequest {
    pub status: Option<String>,
    pub verified: Option<bool>,
}

#[utoipa::path(
    patch,
    path = "/admin/packages/{package_id}",
    tag = "admin",
    params(
        ("package_id" = String, Path, description = "Package ID to update")
    ),
    request_body = UpdateRequest,
    responses(
        (status = 200, description = "Package updated successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
pub async fn update_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(request): Json<UpdateRequest>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ManagePackages)
        .await?;

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("WASM registry not configured"))?;

    if let Some(status) = &request.status {
        registry
            .update_status(&package_id, status, request.verified)
            .await?;
    } else if let Some(verified) = request.verified {
        registry
            .update_status(&package_id, "active", Some(verified))
            .await?;
    }

    audit!(state, user, "admin.package.update", "WasmPackage", package_id, "Package status updated");
    Ok(Json(()))
}
