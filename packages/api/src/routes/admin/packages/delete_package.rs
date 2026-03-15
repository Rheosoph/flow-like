//! Delete a package

use crate::audit;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};

#[utoipa::path(
    delete,
    path = "/admin/packages/{package_id}",
    tag = "admin",
    params(
        ("package_id" = String, Path, description = "Package ID to delete")
    ),
    responses(
        (status = 200, description = "Package deleted successfully"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
pub async fn delete_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ManagePackages)
        .await?;

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("WASM registry not configured"))?;

    registry.delete_package(&package_id).await?;

    audit!(state, user, "admin.package.delete", "WasmPackage", package_id, "Package deleted");
    Ok(Json(()))
}
