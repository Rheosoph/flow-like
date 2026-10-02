//! Registry index endpoints

use super::types::{PackageVersion, RegistryEntry};
use crate::entity::sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility};
use crate::entity::wasm_package;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::Extension;
use axum::Json;
use axum::extract::{Path, Query, State};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PackageQuery {
    /// Project to read the package through: members see the version it pins,
    /// even without access to the package itself.
    #[serde(default)]
    pub app_id: Option<String>,
}

/// GET /registry/package/{id}
/// Returns full package entry details.
/// - Public packages: accessible to anyone
/// - PublicRequestAccess packages: metadata visible, download gated separately
/// - Private packages: only accessible to users with a permission record, or
///   to members of the `app_id` project while it licenses the package
/// - With `app_id` pinning the package, the manifest describes the pinned
///   version, and a pinned version the viewer may not see is a 404
#[utoipa::path(
    get,
    path = "/registry/package/{id}",
    tag = "registry",
    description = "Get package details by ID. With a project, the details describe the version that project uses, and members of the project can read them even for a private package.",
    params(("id" = String, Path, description = "Package ID"), PackageQuery),
    responses(
        (status = 200, description = "Package entry"),
        (status = 403, description = "No access"),
        (status = 404, description = "Not found, or the version the project uses is not available"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(query): Query<PackageQuery>,
) -> Result<Json<RegistryEntry>, ApiError> {
    let sub = user.sub().ok();

    if !state.platform_config.features.unauthorized_read && sub.is_none() {
        return Err(ApiError::FORBIDDEN);
    }

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let package = wasm_package::Entity::find_by_id(&id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", id)))?;

    let non_active =
        package.status != crate::entity::sea_orm_active_enums::WasmPackageStatus::Active;

    if non_active {
        if let Some(ref uid) = sub {
            let access = crate::check_wasm_access!(state, uid, &id);
            if access.is_none() {
                return Err(ApiError::not_found(format!("Package '{}' not found", id)));
            }
        } else {
            return Err(ApiError::not_found(format!("Package '{}' not found", id)));
        }
    }

    let access = match sub.as_ref() {
        Some(uid) => crate::check_wasm_access!(state, uid, &id),
        None => None,
    };
    let pinned = match query.app_id.as_deref() {
        Some(app_id) => {
            crate::package_license::member_pinned_version(&state, &user, app_id, &id).await?
        }
        None => None,
    };

    let through_project_only =
        package.visibility == WasmPackageVisibility::Private && access.is_none();
    if through_project_only && pinned.is_none() {
        return Err(ApiError::FORBIDDEN);
    }

    let can_manage = access.is_some_and(|permission| {
        permission.has_permission(
            crate::permission::wasm_package_permission::WasmPackagePermission::Maintainer,
        )
    });

    // Access / visibility control is done above; build with correct version visibility.
    let mut entry = registry.entry_for_viewer(package, can_manage).await?;
    if let Some(pinned) = pinned.as_deref() {
        if entry.get_version(pinned).is_none() {
            return Err(ApiError::not_found(format!(
                "Version '{}' of package '{}', which this project uses, is not available",
                pinned, id
            )));
        }
        registry.pin_entry_to_version(&mut entry, pinned).await?;
        // A member who cannot read the project's boards only loads the
        // package's widgets: the node list, the node binary's hash and size
        // and the other versions stay with the package's own users.
        if through_project_only
            && let Some(app_id) = query.app_id.as_deref()
            && !crate::package_license::reads_project_boards(&state, &user, app_id).await
        {
            entry.nodes.clear();
            entry.versions.retain(|listed| listed.version == pinned);
            if super::server::manifest_has_wasm(&entry.manifest) {
                entry.manifest.withhold_nodes();
                for listed in &mut entry.versions {
                    listed.wasm_hash.clear();
                    listed.wasm_size = 0;
                }
            }
        }
    }
    entry.pinned_version = pinned;
    entry.current_user_permission = access.map(|a| a.bits() as i32);

    Ok(Json(entry))
}

/// GET /registry/package/{id}/versions
/// Returns all approved versions for a package.
/// Same visibility rules as get_package.
#[utoipa::path(
    get,
    path = "/registry/package/{id}/versions",
    tag = "registry",
    description = "List versions for a package.",
    params(("id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Package versions"),
        (status = 403, description = "No access"),
        (status = 404, description = "Not found"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_versions(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<Vec<PackageVersion>>, ApiError> {
    let sub = user.sub().ok();
    let mut access = None;

    if !state.platform_config.features.unauthorized_read && sub.is_none() {
        return Err(ApiError::FORBIDDEN);
    }

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let package = wasm_package::Entity::find_by_id(&id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", id)))?;

    if package.status != crate::entity::sea_orm_active_enums::WasmPackageStatus::Active {
        if let Some(ref uid) = sub {
            access = crate::check_wasm_access!(state, uid, &id);
            if access.is_none() {
                return Err(ApiError::not_found(format!("Package '{}' not found", id)));
            }
        } else {
            return Err(ApiError::not_found(format!("Package '{}' not found", id)));
        }
    }

    if package.visibility == WasmPackageVisibility::Private {
        let uid = sub.as_ref().ok_or(ApiError::FORBIDDEN)?;
        if access.is_none() {
            access = crate::check_wasm_access!(state, uid, &id);
        }
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    if access.is_none()
        && let Some(uid) = sub.as_ref()
    {
        access = crate::check_wasm_access!(state, uid, &id);
    }

    let can_manage = access.is_some_and(|permission| {
        permission.has_permission(
            crate::permission::wasm_package_permission::WasmPackagePermission::Maintainer,
        )
    });
    let show_all_versions = package.visibility == WasmPackageVisibility::Private || can_manage;
    let versions = if show_all_versions {
        registry.get_versions(&id).await?
    } else {
        registry.get_versions_approved(&id).await?
    };

    Ok(Json(versions))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DeletePackageResponse {
    pub message: String,
}

/// DELETE /registry/package/{id}
/// Soft-delete a package by setting its status to Disabled.
/// The package no longer appears in search results but WASM artifacts are
/// preserved so existing installs keep working.
#[utoipa::path(
    delete,
    path = "/registry/package/{id}",
    tag = "registry",
    description = "Soft-delete a package (sets status to Disabled). Artifacts are preserved for existing installs.",
    params(("id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Package disabled", body = DeletePackageResponse),
        (status = 403, description = "Forbidden – owner permission required"),
        (status = 404, description = "Package not found"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn delete_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<DeletePackageResponse>, ApiError> {
    let uid = user.sub().map_err(|_| ApiError::UNAUTHORIZED)?;

    crate::ensure_wasm_permission!(state, &uid, &id, WasmPackagePermission::Owner);

    let _pkg = wasm_package::Entity::find_by_id(&id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", id)))?;

    let model = wasm_package::ActiveModel {
        id: Set(id.clone()),
        status: Set(WasmPackageStatus::Disabled),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        ..Default::default()
    };
    model
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to disable package: {}", e)))?;

    Ok(Json(DeletePackageResponse {
        message: "Package disabled. Artifacts preserved for existing installs.".to_string(),
    }))
}

/// POST /registry/package/{id}/restore
/// Re-enable a previously disabled package.
#[utoipa::path(
    post,
    path = "/registry/package/{id}/restore",
    tag = "registry",
    description = "Restore a disabled package back to active status.",
    params(("id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Package restored", body = DeletePackageResponse),
        (status = 403, description = "Forbidden – owner permission required"),
        (status = 404, description = "Package not found"),
        (status = 409, description = "Package is not disabled"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn restore_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
) -> Result<Json<DeletePackageResponse>, ApiError> {
    let uid = user.sub().map_err(|_| ApiError::UNAUTHORIZED)?;

    crate::ensure_wasm_permission!(state, &uid, &id, WasmPackagePermission::Owner);

    let pkg = wasm_package::Entity::find_by_id(&id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", id)))?;

    if pkg.status != WasmPackageStatus::Disabled {
        return Err(ApiError::conflict("Package is not disabled"));
    }

    let model = wasm_package::ActiveModel {
        id: Set(id.clone()),
        status: Set(WasmPackageStatus::Active),
        updated_at: Set(chrono::Utc::now().fixed_offset()),
        ..Default::default()
    };
    model
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to restore package: {}", e)))?;

    Ok(Json(DeletePackageResponse {
        message: "Package restored and active again.".to_string(),
    }))
}
