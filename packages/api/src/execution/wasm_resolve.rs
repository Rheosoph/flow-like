//! Resolves WASM packages for execution dispatch.
//!
//! Queries AppPackage records for an app, looks up the compiled artifact paths,
//! and generates presigned download URLs so the executor can fetch them.
//!
//! Results are cached on `AppState::wasm_resolve_cache` keyed by `app_id` —
//! the per-app bundle is small and changes only when the package list is
//! mutated, so caching cuts the AWS-SDK round-trips out of the hot dispatch
//! path. The cache TTL is set below the signed-URL TTL so callers always
//! receive a signature with safe remaining lifetime.

use crate::entity::{app_package, wasm_package_version};
use crate::routes::registry::server::executor_target_platform;
use crate::state::AppState;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use std::collections::HashMap;
use std::sync::Arc;

/// Resolve all WASM packages for an app, returning presigned download URLs.
///
/// Returns `None` if the app has no WASM packages or if the registry is not enabled.
pub async fn resolve_wasm_packages(
    state: &AppState,
    app_id: &str,
) -> Option<HashMap<String, flow_like_types::dispatch::WasmPackageRef>> {
    if let Some(cached) = state.wasm_resolve_cache.get(app_id) {
        return Some((*cached).clone());
    }

    let registry = state.wasm_registry.as_ref()?;
    let target = executor_target_platform();

    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .all(&state.db)
        .await
        .ok()?;

    if packages.is_empty() {
        return None;
    }

    let mut result: HashMap<String, flow_like_types::dispatch::WasmPackageRef> = HashMap::new();

    for pkg in &packages {
        let version_record = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(&pkg.package_id))
            .filter(wasm_package_version::Column::Version.eq(&pkg.version))
            .one(&state.db)
            .await
            .ok()
            .flatten();

        let version_record = match version_record {
            Some(v) => v,
            None => {
                tracing::warn!(
                    package_id = %pkg.package_id,
                    version = %pkg.version,
                    "WASM package version not found — skipping"
                );
                continue;
            }
        };

        let wasm_url = match registry
            .get_wasm_url(&pkg.package_id, Some(&pkg.version))
            .await
        {
            Ok((download_url, _, _)) => download_url,
            Err(e) => {
                tracing::warn!(
                    package_id = %pkg.package_id,
                    version = %pkg.version,
                    error = %e,
                    "Failed to generate raw WASM download URL — skipping"
                );
                continue;
            }
        };

        match registry
            .sign_cwasm_url(&pkg.package_id, &pkg.version, &target)
            .await
        {
            Ok((cwasm_url, cwasm_checksum)) => {
                let resolved = flow_like_types::dispatch::WasmPackageRef {
                    version: pkg.version.clone(),
                    wasm_hash: version_record.wasm_hash.clone(),
                    wasm_url,
                    cwasm_url,
                    cwasm_checksum,
                };
                result.insert(pkg.package_id.clone(), resolved);
            }
            Err(e) => {
                tracing::warn!(
                    package_id = %pkg.package_id,
                    version = %pkg.version,
                    error = %e,
                    "Failed to generate presigned URLs — skipping"
                );
            }
        }
    }

    if result.is_empty() {
        None
    } else {
        let arc = Arc::new(result.clone());
        state.wasm_resolve_cache.insert(app_id.to_string(), arc);
        Some(result)
    }
}
