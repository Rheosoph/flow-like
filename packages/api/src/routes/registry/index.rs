//! Registry index endpoints

use super::types::{PackageVersion, RegistryEntry};
use crate::entity::sea_orm_active_enums::WasmPackageVisibility;
use crate::entity::wasm_package;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::Extension;
use axum::Json;
use axum::extract::{Path, State};
use sea_orm::EntityTrait;

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

    let mut entry = if non_active {
        registry
            .get_package_any_status(&id)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", id)))?
    } else {
        registry
            .get_package(&id)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", id)))?
    };

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
