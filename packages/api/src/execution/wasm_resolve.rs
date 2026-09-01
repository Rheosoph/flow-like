//! Resolves WASM packages for execution dispatch.
//!
//! Queries AppPackage records for an app, looks up the compiled artifact paths,
//! and generates presigned download URLs so the executor can fetch them.
//!
//! Package authority is read from the shared database for every dispatch.
//! Process-local caches cannot be invalidated across stateless API instances,
//! so using one here could dispatch a package after its app pin was changed or
//! removed. The returned URLs are fresh transport credentials for that exact
//! database snapshot.

use crate::entity::{
    app_package, sea_orm_active_enums::WasmCompilationStatus, wasm_package_version,
};
use crate::routes::registry::server::executor_target_platform;
use crate::state::AppState;
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};
use std::collections::HashMap;

/// Resolve all WASM packages for an app, returning presigned download URLs.
///
/// Returns `None` if the app has no WASM packages or if the registry is not enabled.
pub async fn resolve_wasm_packages(
    state: &AppState,
    app_id: &str,
) -> Option<HashMap<String, flow_like_types::dispatch::WasmPackageRef>> {
    let registry = state.wasm_registry.as_ref()?;
    let target = executor_target_platform();

    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(app_package::Column::Stale.eq(false))
        .all(&state.db)
        .await
        .ok()?;

    if packages.is_empty() {
        return None;
    }

    // One batched lookup instead of a query per pinned package.
    let version_filter = packages.iter().fold(Condition::any(), |filter, pkg| {
        filter.add(
            Condition::all()
                .add(wasm_package_version::Column::PackageId.eq(&pkg.package_id))
                .add(wasm_package_version::Column::Version.eq(&pkg.version)),
        )
    });
    let mut version_records: HashMap<(String, String), wasm_package_version::Model> =
        wasm_package_version::Entity::find()
            .filter(version_filter)
            .all(&state.db)
            .await
            .ok()?
            .into_iter()
            .map(|record| ((record.package_id.clone(), record.version.clone()), record))
            .collect();

    let mut result: HashMap<String, flow_like_types::dispatch::WasmPackageRef> = HashMap::new();
    let mut had_errors = false;

    for pkg in &packages {
        let version_record = version_records.remove(&(pkg.package_id.clone(), pkg.version.clone()));

        let version_record = match version_record {
            Some(v) => v,
            None => {
                had_errors = true;
                tracing::warn!(
                    package_id = %pkg.package_id,
                    version = %pkg.version,
                    "WASM package version not found — skipping"
                );
                continue;
            }
        };

        let compiled_for_target = version_record.compilation_status
            == WasmCompilationStatus::Compiled
            && version_record
                .compiled_platforms
                .as_deref()
                .unwrap_or_default()
                .iter()
                .any(|platform| platform == &target);

        if !compiled_for_target {
            had_errors = true;
            tracing::warn!(
                package_id = %pkg.package_id,
                version = %pkg.version,
                target = %target,
                status = ?version_record.compilation_status,
                compiled_platforms = ?version_record.compiled_platforms,
                "WASM package is not compiled for executor target — skipping"
            );
            continue;
        }

        let wasm_url = match registry
            .get_wasm_url(&pkg.package_id, Some(&pkg.version))
            .await
        {
            Ok((download_url, _, _)) => download_url,
            Err(e) => {
                had_errors = true;
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
                had_errors = true;
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
        if had_errors {
            tracing::warn!(
                app_id = %app_id,
                resolved = result.len(),
                total = packages.len(),
                "Resolved WASM packages with skipped entries"
            );
        }
        Some(result)
    }
}
