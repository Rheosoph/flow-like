//! Widget bundle fallback asset route
//!
//! Serves a single entry of a package version's widget bundle. The primary
//! serving path for the web app is the unpacked `widget-assets/` CDN prefix;
//! this route is the guaranteed fallback during the post-publish
//! eventual-consistency window (and for deployments without a CDN). Access
//! control mirrors `GET /registry/package/{id}` so store previews work
//! pre-install.

use crate::entity::sea_orm_active_enums::WasmPackageVisibility;
use crate::entity::wasm_package;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::Extension;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use flow_like_wasm_schema::widget_bundle::WidgetBundleReader;
use sea_orm::EntityTrait;

const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

fn content_type_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json",
        "webp" => "image/webp",
        "png" => "image/png",
        "svg" => "image/svg+xml",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

fn is_safe_asset_path(path: &str) -> bool {
    !path.is_empty()
        && !path.starts_with('/')
        && !path.contains('\\')
        && !path
            .split('/')
            .any(|seg| seg.is_empty() || seg == "." || seg == "..")
}

/// GET /registry/package/{package_id}/widget-asset/{version}/{path}
/// Serve one widget file of a published package version.
#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/widget-asset/{version}/{path}",
    tag = "registry",
    description = "Serve a single widget file (document, chunk, contract, or image) of a package version — used to render live widget previews before installing the package.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("version" = String, Path, description = "Package version"),
        ("path" = String, Path, description = "Entry path inside the widget bundle, e.g. widgets/{widget_id}/index.html")
    ),
    responses(
        (status = 200, description = "The widget asset bytes, immutable-cached"),
        (status = 403, description = "No access to this package"),
        (status = 404, description = "Package, version, or asset not found"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get_widget_asset(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((package_id, version, path)): Path<(String, String, String)>,
) -> Result<Response, ApiError> {
    let sub = user.sub().ok();

    if !state.platform_config.features.unauthorized_read && sub.is_none() {
        return Err(ApiError::FORBIDDEN);
    }

    if !is_safe_asset_path(&path) {
        return Err(ApiError::not_found("Widget asset not found"));
    }

    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    // Access control mirrors GET /registry/package/{id}
    let package = wasm_package::Entity::find_by_id(&package_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_id)))?;

    if package.status != crate::entity::sea_orm_active_enums::WasmPackageStatus::Active {
        if let Some(ref uid) = sub {
            let access = crate::check_wasm_access!(state, uid, &package_id);
            if access.is_none() {
                return Err(ApiError::not_found(format!(
                    "Package '{}' not found",
                    package_id
                )));
            }
        } else {
            return Err(ApiError::not_found(format!(
                "Package '{}' not found",
                package_id
            )));
        }
    }

    if package.visibility == WasmPackageVisibility::Private {
        let uid = sub.clone().ok_or(ApiError::FORBIDDEN)?;
        let access = crate::check_wasm_access!(state, &uid, &package_id);
        if access.is_none() {
            return Err(ApiError::FORBIDDEN);
        }
    }

    // Version-level visibility: the version must be visible to this viewer
    let entry = registry
        .get_package_as_viewer(&package_id, sub.as_deref())
        .await?
        .ok_or_else(|| ApiError::not_found(format!("Package '{}' not found", package_id)))?;
    let version_entry = entry
        .get_version(&version)
        .ok_or_else(|| ApiError::not_found(format!("Version '{}' not found", version)))?;
    if version_entry
        .widget_bundle_hash
        .as_deref()
        .is_none_or(|h| h.is_empty())
    {
        return Err(ApiError::not_found("This package version ships no widgets"));
    }

    // Prefer the unpacked CDN object; fall back to reading the stored bundle.
    let bytes = match registry
        .get_widget_asset_object(&package_id, &version, &path)
        .await
    {
        Ok(Some(bytes)) => bytes,
        _ => {
            let bundle_bytes = registry
                .get_widget_bundle_bytes(&package_id, &version)
                .await
                .map_err(|_| ApiError::not_found("Widget bundle not found for this version"))?;
            let entry_path = path.clone();
            flow_like_types::tokio::task::spawn_blocking(
                move || -> flow_like_types::Result<Vec<u8>> {
                    let mut reader = WidgetBundleReader::from_bytes(bundle_bytes)?;
                    // read_entry rejects unsafe paths and verifies declared entry hashes
                    reader.read_entry(&entry_path)
                },
            )
            .await
            .map_err(|e| ApiError::internal(format!("Failed to read widget asset: {}", e)))?
            .map_err(|_| ApiError::not_found(format!("Widget asset not found: {}", path)))?
        }
    };

    Ok((
        [
            (header::CONTENT_TYPE, content_type_for(&path)),
            (header::CACHE_CONTROL, IMMUTABLE_CACHE),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_content_types() {
        assert_eq!(
            content_type_for("widgets/sales-chart/index.html"),
            "text/html; charset=utf-8"
        );
        assert_eq!(
            content_type_for("shared/react-abc.js"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type_for("bundle.json"), "application/json");
        assert_eq!(content_type_for("widgets/x/thumbnail.webp"), "image/webp");
        assert_eq!(content_type_for("font.woff2"), "font/woff2");
        assert_eq!(content_type_for("unknown.bin"), "application/octet-stream");
    }

    #[test]
    fn test_safe_asset_paths() {
        assert!(is_safe_asset_path("widgets/sales-chart/index.html"));
        assert!(is_safe_asset_path("bundle.json"));
        assert!(!is_safe_asset_path("../secrets"));
        assert!(!is_safe_asset_path("widgets/../../etc/passwd"));
        assert!(!is_safe_asset_path("/absolute"));
        assert!(!is_safe_asset_path("widgets//double"));
        assert!(!is_safe_asset_path("windows\\path"));
        assert!(!is_safe_asset_path(""));
    }
}
