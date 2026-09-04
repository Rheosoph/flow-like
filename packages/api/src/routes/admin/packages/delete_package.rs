//! Delete a package

use crate::audit;
use crate::deletion::{self, AcceptedDeletion, Deleted, DeletionRoot};
use crate::entity::wasm_package;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::Extension;
use axum::extract::{Path, State};
use sea_orm::EntityTrait;

#[utoipa::path(
    delete,
    path = "/admin/packages/{package_id}",
    tag = "admin",
    params(
        ("package_id" = String, Path, description = "Package ID to delete")
    ),
    responses(
        (status = 200, description = "Package deleted successfully"),
        (status = 202, description = "Package queued for deletion; follow the job on `GET /admin/deletions/{job_id}`", body = AcceptedDeletion),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
pub async fn delete_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
) -> Result<Deleted<()>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ManagePackages)
        .await?;

    if wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await?
        .is_none()
    {
        return Ok(Deleted::Completed(()));
    }

    let requested_by = user.sub().ok();
    let deleted = deletion::delete_now(
        &state,
        DeletionRoot::WasmPackage,
        &package_id,
        requested_by.as_deref(),
        (),
    )
    .await?;

    audit!(
        state,
        user,
        "admin.package.delete",
        "WasmPackage",
        package_id,
        "Package deleted"
    );
    Ok(deleted)
}
