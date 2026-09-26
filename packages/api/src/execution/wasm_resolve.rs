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
    app_package,
    sea_orm_active_enums::{WasmCompilationStatus, WasmPackageStatus, WasmPackageVisibility},
    wasm_package, wasm_package_version,
};
use crate::routes::registry::server::{ServerRegistry, executor_target_platform};
use crate::state::AppState;
use flow_like_types::dispatch::WasmPackageRef;
use futures::{StreamExt, stream};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect};
use std::collections::HashMap;

/// Packages signed at once. Each one costs two presigns and a checksum read.
const SIGN_CONCURRENCY: usize = 8;

/// Resolve all WASM packages for an app, returning presigned download URLs.
///
/// Returns `None` if the app has no WASM packages or if the registry is not enabled.
pub async fn resolve_wasm_packages(
    state: &AppState,
    app_id: &str,
) -> Option<HashMap<String, flow_like_types::dispatch::WasmPackageRef>> {
    let target = executor_target_platform();
    resolve_wasm_packages_for_platform(state, app_id, &target).await
}

/// Resolve packages for the executor receiving this run. Background and live
/// executors can use different CPU architectures in the same deployment.
pub async fn resolve_wasm_packages_for_platform(
    state: &AppState,
    app_id: &str,
    target: &str,
) -> Option<HashMap<String, flow_like_types::dispatch::WasmPackageRef>> {
    let registry = state.wasm_registry.as_ref()?;

    // Lapsed pins keep running through the licence grace period; expired ones do not.
    let packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.eq(app_id))
        .filter(crate::package_license::usable_pins(chrono::Utc::now()))
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
    let package_ids: Vec<String> = packages.iter().map(|pkg| pkg.package_id.clone()).collect();

    // The package and version status rules of `ServerRegistry::get_wasm_url`,
    // answered for every pin by two queries instead of five per package.
    let (version_rows, active_package_rows) = flow_like_types::tokio::join!(
        wasm_package_version::Entity::find()
            .filter(version_filter)
            .all(&state.db),
        wasm_package::Entity::find()
            .select_only()
            .column(wasm_package::Column::Id)
            .column(wasm_package::Column::Visibility)
            .filter(wasm_package::Column::Id.is_in(package_ids))
            .filter(wasm_package::Column::Status.eq(WasmPackageStatus::Active))
            .into_tuple::<(String, WasmPackageVisibility)>()
            .all(&state.db),
    );
    let mut version_records: HashMap<(String, String), wasm_package_version::Model> = version_rows
        .ok()?
        .into_iter()
        .map(|record| ((record.package_id.clone(), record.version.clone()), record))
        .collect();
    let active_packages: HashMap<String, WasmPackageVisibility> =
        active_package_rows.ok()?.into_iter().collect();

    let mut candidates = Vec::with_capacity(packages.len());
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
                .map(Vec::as_slice)
                .unwrap_or_default()
                .iter()
                .any(|platform| platform == target);

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

        // A private package runs any of its versions; every other package
        // only the approved ones.
        let unavailable = match active_packages.get(&pkg.package_id) {
            None => Some("Package not found"),
            Some(visibility)
                if *visibility != WasmPackageVisibility::Private
                    && version_record.status != WasmPackageStatus::Active =>
            {
                Some("Version not found")
            }
            Some(_) => None,
        };
        if let Some(reason) = unavailable {
            had_errors = true;
            tracing::warn!(
                package_id = %pkg.package_id,
                version = %pkg.version,
                error = reason,
                "Failed to generate raw WASM download URL — skipping"
            );
            continue;
        }

        candidates.push(version_record);
    }

    let attempted = candidates.len();
    let signed: Vec<Option<(String, WasmPackageRef)>> = stream::iter(
        candidates
            .into_iter()
            .map(|record| sign_package(registry, target, record)),
    )
    .buffer_unordered(SIGN_CONCURRENCY)
    .collect()
    .await;
    let result: HashMap<String, WasmPackageRef> = signed.into_iter().flatten().collect();
    had_errors |= result.len() < attempted;

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

/// Fresh transport credentials for one pinned version whose status the caller
/// already checked. `None` means the package is skipped, and says why.
async fn sign_package(
    registry: &ServerRegistry,
    target: &str,
    record: wasm_package_version::Model,
) -> Option<(String, WasmPackageRef)> {
    let wasm_url = match registry.sign_wasm_path(&record.wasm_path).await {
        Ok(url) => url,
        Err(e) => {
            tracing::warn!(
                package_id = %record.package_id,
                version = %record.version,
                error = %e,
                "Failed to generate raw WASM download URL — skipping"
            );
            return None;
        }
    };

    match registry
        .sign_cwasm_url(&record.package_id, &record.version, target)
        .await
    {
        Ok((cwasm_url, cwasm_checksum)) => Some((
            record.package_id,
            WasmPackageRef {
                version: record.version,
                wasm_hash: record.wasm_hash,
                wasm_url,
                cwasm_url,
                cwasm_checksum,
            },
        )),
        Err(e) => {
            tracing::warn!(
                package_id = %record.package_id,
                version = %record.version,
                error = %e,
                "Failed to generate presigned URLs — skipping"
            );
            None
        }
    }
}
