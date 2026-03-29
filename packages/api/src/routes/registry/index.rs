//! Registry index endpoints

use super::types::{PackageVersion, RegistryEntry};
use crate::entity::sea_orm_active_enums::{WasmPackageStatus, WasmPackageVisibility};
use crate::entity::wasm_package;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::state::AppState;
use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::Serialize;
use utoipa::ToSchema;

/// GET /registry/package/{id}
/// Returns full package entry details.
/// - Public packages: accessible to anyone
/// - PublicRequestAccess packages: metadata visible, download gated separately
/// - Private packages: only accessible to users with a permission record
#[utoipa::path(
    get,
    path = "/registry/package/{id}",
    tag = "registry",
    description = "Get package details by ID.",
    params(("id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Package entry"),
        (status = 403, description = "No access"),
        (status = 404, description = "Not found"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_package(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
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

    if package.visibility == WasmPackageVisibility::Private {
        let uid = sub.clone().ok_or(ApiError::FORBIDDEN)?;
        let access = crate::check_wasm_access!(state, &uid, &id);
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    // Access / visibility control is done above; fetch with correct version visibility.
    let mut entry = registry
        .get_package_as_viewer(&id, sub.as_deref())
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", id)))?;

    if let Some(ref uid) = sub {
        let access = crate::check_wasm_access!(state, uid, &id);
        entry.current_user_permission = access.map(|a| a.bits() as i32);
    }

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
            let access = crate::check_wasm_access!(state, uid, &id);
            if access.is_none() {
                return Err(ApiError::not_found(format!("Package '{}' not found", id)));
            }
        } else {
            return Err(ApiError::not_found(format!("Package '{}' not found", id)));
        }
    }

    if package.visibility == WasmPackageVisibility::Private {
        let uid = sub.ok_or(ApiError::FORBIDDEN)?;
        let access = crate::check_wasm_access!(state, &uid, &id);
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    let versions = registry.get_versions_approved(&id).await?;

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

    let mut model: wasm_package::ActiveModel = Default::default();
    model.id = Set(id.clone());
    model.status = Set(WasmPackageStatus::Disabled);
    model.updated_at = Set(chrono::Utc::now().naive_utc());
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

    let mut model: wasm_package::ActiveModel = Default::default();
    model.id = Set(id.clone());
    model.status = Set(WasmPackageStatus::Active);
    model.updated_at = Set(chrono::Utc::now().naive_utc());
    model
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to restore package: {}", e)))?;

    Ok(Json(DeletePackageResponse {
        message: "Package restored and active again.".to_string(),
    }))
}
