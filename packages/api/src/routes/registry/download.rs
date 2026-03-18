//! Package download endpoint

use super::types::{DownloadRequest, DownloadResponse, MetaSummary};
use crate::entity::meta;
use crate::entity::sea_orm_active_enums::WasmPackageVisibility;
use crate::entity::wasm_package;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

/// POST /registry/download
/// Get download URL for a package WASM binary.
///
/// Access rules:
/// - Public + free (price <= 0): any authenticated user can download
/// - Public + paid: requires a completed purchase (wasm_package_user record)
/// - PublicRequestAccess: requires a wasm_package_user record (granted via join approval or purchase)
/// - Private: requires a wasm_package_user record
#[utoipa::path(
    post,
    path = "/registry/download",
    tag = "registry",
    request_body = DownloadRequest,
    responses(
        (status = 200, description = "Download URL and package info", body = DownloadResponse),
        (status = 402, description = "Payment required"),
        (status = 403, description = "No access to this package"),
        (status = 404, description = "Package not found"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn download(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<DownloadRequest>,
) -> Result<Json<DownloadResponse>, ApiError> {
    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let package = wasm_package::Entity::find_by_id(&request.package_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Package not found"))?;

    let is_free_public = package.visibility == WasmPackageVisibility::Public && package.price <= 0;

    if !is_free_public {
        let sub = user
            .sub()
            .map_err(|_| ApiError::unauthorized("Authentication required for downloads"))?;

        let access = crate::check_wasm_access!(state, &sub, &request.package_id);
        if access.is_none() {
            return match package.visibility {
                WasmPackageVisibility::Public if package.price > 0 => Err(
                    ApiError::payment_required("Purchase required to download this package"),
                ),
                WasmPackageVisibility::PublicRequestAccess => Err(ApiError::forbidden(
                    "Access request required for this package",
                )),
                _ => Err(ApiError::FORBIDDEN),
            };
        }
    }

    let (download_url, manifest, version) = registry
        .get_wasm_url(&request.package_id, request.version.as_deref())
        .await?;

    let package_id = package.id.clone();
    let _ = registry.increment_downloads(&package_id).await;

    // Fetch metadata (icon, thumbnail, localized name) for the package
    let mut metadata = meta::Entity::find()
        .filter(meta::Column::WasmPackageId.eq(&package_id))
        .all(&state.db)
        .await
        .ok()
        .and_then(|metas| MetaSummary::pick_best(&metas, "en").map(MetaSummary::from_model));

    if let Some(meta) = &mut metadata {
        if let Ok(master_creds) = state.master_credentials().await {
            if let Ok(store) = master_creds.to_store(false).await {
                meta.presign_media(&package_id, &store).await;
            }
        }
    }

    Ok(Json(DownloadResponse {
        package_id,
        version,
        wasm_base64: String::new(),
        download_url: Some(download_url),
        manifest,
        metadata,
    }))
}
