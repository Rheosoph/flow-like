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
    sea_orm_active_enums::{WasmCompilationStatus, WasmPackageStatus, WasmPackageVisibility},
    wasm_package, wasm_package_version,
};
use crate::routes::registry::server::{ServerRegistry, executor_target_platform};
use crate::state::AppState;
use flow_like_types::dispatch::WasmPackageRef;
use flow_like_wasm_schema::runtime::{current_artifact_platform, is_previous_artifact_platform};
use futures::{StreamExt, stream};
use sea_orm::{ActiveValue::Set, ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect};
use std::collections::HashMap;

/// Packages signed at once. Each one costs two presigns and a checksum read.
const SIGN_CONCURRENCY: usize = 8;
pub(crate) const UPGRADE_MARKER: &str = "artifact-upgrade:";

fn automatic_upgrade_backend(backend: Option<&crate::compilation::CompilationBackend>) -> bool {
    use crate::compilation::CompilationBackend;
    matches!(
        backend,
        Some(
            CompilationBackend::LambdaInvoke
                | CompilationBackend::Sqs
                | CompilationBackend::AzureQueue
                | CompilationBackend::PubSub
                | CompilationBackend::Kafka
                | CompilationBackend::Redis
                | CompilationBackend::KubernetesJob
        )
    )
}

pub(crate) fn selected_artifact_platform<'a>(
    status: &WasmCompilationStatus,
    platforms: &'a [String],
    target: &str,
) -> Option<&'a str> {
    if !matches!(
        status,
        WasmCompilationStatus::Compiled | WasmCompilationStatus::Pending
    ) {
        return None;
    }
    platforms
        .iter()
        .find(|platform| platform.as_str() == target)
        .or_else(|| {
            platforms
                .iter()
                .find(|platform| is_previous_artifact_platform(platform, target))
        })
        .map(String::as_str)
}

fn upgrade_due(status: &WasmCompilationStatus, error: Option<&str>, now: i64) -> bool {
    let started = error
        .and_then(|value| value.strip_prefix(UPGRADE_MARKER))
        .and_then(|value| value.split(':').next())
        .and_then(|value| value.parse::<i64>().ok());
    match status {
        WasmCompilationStatus::Pending => {
            started.is_some_and(|started| now.saturating_sub(started) >= 1800)
        }
        WasmCompilationStatus::Compiled => {
            started.is_none_or(|started| now.saturating_sub(started) >= 60)
        }
        WasmCompilationStatus::LocalOnly => false,
    }
}

fn artifact_upgrade_claim(
    record: &wasm_package_version::Model,
    marker: &str,
) -> flow_like_types::Result<sea_orm::UpdateMany<wasm_package_version::Entity>> {
    let mut claim = wasm_package_version::Entity::update_many()
        .set(wasm_package_version::ActiveModel {
            compilation_status: Set(WasmCompilationStatus::Pending),
            compilation_error: Set(Some(marker.to_owned())),
            ..Default::default()
        })
        .filter(wasm_package_version::Column::Id.eq(&record.id))
        .filter(wasm_package_version::Column::WasmHash.eq(&record.wasm_hash))
        .filter(
            wasm_package_version::Column::CompiledPlatforms
                .eq(serde_json::to_value(&record.compiled_platforms)?),
        )
        .filter(
            wasm_package_version::Column::CompilationStatus.eq(record.compilation_status.clone()),
        );
    claim = match &record.compilation_error {
        Some(value) => claim.filter(wasm_package_version::Column::CompilationError.eq(value)),
        None => claim.filter(wasm_package_version::Column::CompilationError.is_null()),
    };
    claim = match &record.compiled_artifact_generation {
        Some(value) => {
            claim.filter(wasm_package_version::Column::CompiledArtifactGeneration.eq(value))
        }
        None => claim.filter(wasm_package_version::Column::CompiledArtifactGeneration.is_null()),
    };
    Ok(claim)
}

/// Claim a replacement once across API replicas. Keep the previous publication
/// available while the compiler works; its raw-WASM fallback still executes.
async fn queue_artifact_upgrade(
    state: &AppState,
    registry: &ServerRegistry,
    record: &wasm_package_version::Model,
) -> flow_like_types::Result<()> {
    // HTTP waits for compilation itself. Those deployments use the admin
    // artifact rebuild action while the previous publication keeps running.
    if !automatic_upgrade_backend(registry.compilation_backend()) {
        return Ok(());
    }
    let now = chrono::Utc::now().timestamp();
    if !upgrade_due(
        &record.compilation_status,
        record.compilation_error.as_deref(),
        now,
    ) {
        return Ok(());
    }
    let marker = format!("{UPGRADE_MARKER}{now}:{}", flow_like_types::create_id());
    if artifact_upgrade_claim(record, &marker)?
        .exec(&state.db)
        .await?
        .rows_affected
        == 0
    {
        return Ok(());
    }
    if let Err(error) = registry
        .recompile_version(
            "artifact-compatibility-upgrade".into(),
            &record.package_id,
            &record.version,
        )
        .await
    {
        // The marker allows a bounded retry after a dispatch failure. A later
        // successful callback cannot be overwritten by this cleanup.
        wasm_package_version::Entity::update_many()
            .set(wasm_package_version::ActiveModel {
                compilation_status: Set(WasmCompilationStatus::Compiled),
                ..Default::default()
            })
            .filter(wasm_package_version::Column::Id.eq(&record.id))
            .filter(
                wasm_package_version::Column::CompilationStatus.eq(WasmCompilationStatus::Pending),
            )
            .filter(wasm_package_version::Column::CompilationError.eq(marker))
            .exec(&state.db)
            .await?;
        return Err(error);
    }
    Ok(())
}

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
/// Versionless OS/architecture targets use this API build's artifact version.
pub async fn resolve_wasm_packages_for_platform(
    state: &AppState,
    app_id: &str,
    target: &str,
) -> Option<HashMap<String, flow_like_types::dispatch::WasmPackageRef>> {
    let registry = state.wasm_registry.as_ref()?;
    let target = current_artifact_platform(target);

    // Lapsed pins keep running through the licence grace period; expired ones do not.
    let packages = crate::package_license::usable_app_pins(&state.db, app_id)
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
    // A slow queue must not hold execution indefinitely. A cancelled claim
    // remains Pending and can be recovered by the bounded retry policy.
    let upgrade_deadline =
        flow_like_types::tokio::time::Instant::now() + std::time::Duration::from_secs(2);

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

        let selected_platform = selected_artifact_platform(
            &version_record.compilation_status,
            version_record
                .compiled_platforms
                .as_deref()
                .map(Vec::as_slice)
                .unwrap_or_default(),
            &target,
        )
        .map(str::to_owned);

        let Some(selected_platform) = selected_platform else {
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
        };

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

        if selected_platform != target
            && flow_like_types::tokio::time::Instant::now() < upgrade_deadline
        {
            match flow_like_types::tokio::time::timeout_at(
                upgrade_deadline,
                queue_artifact_upgrade(state, registry, &version_record),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(package_id = %pkg.package_id, version = %pkg.version, %error,
                    "Could not queue portable WASM artifact upgrade; retaining previous artifact")
                }
                Err(_) => tracing::warn!(package_id = %pkg.package_id, version = %pkg.version,
                    "Portable WASM artifact upgrade dispatch timed out; retaining previous artifact"),
            }
        }
        candidates.push((version_record, selected_platform));
    }

    let attempted = candidates.len();
    let storage_scope = state.storage_identity.meta.cache_scope();
    let storage_scope = storage_scope.as_deref();
    let signed: Vec<Option<(String, WasmPackageRef)>> = stream::iter(candidates.into_iter().map(
        |(record, selected_platform)| async move {
            sign_package(
                registry,
                &selected_platform,
                record,
                &state.cache,
                storage_scope,
            )
            .await
        },
    ))
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
    cache: &crate::cache::CacheBackendHandle,
    storage_scope: Option<&str>,
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
        .sign_cwasm_url(
            &record.package_id,
            &record.version,
            target,
            record.compiled_artifact_generation.as_deref(),
            cache,
            storage_scope,
        )
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_wasm_schema::runtime::{WASMTIME_MAJOR_VERSION, artifact_platform_key};

    #[test]
    fn versionless_dispatch_targets_select_the_matching_artifact_architecture() {
        let published = vec![
            artifact_platform_key("linux", "x86_64"),
            artifact_platform_key("linux", "aarch64"),
        ];
        for (target, expected) in [
            ("linux-x86_64", &published[0]),
            ("linux-aarch64", &published[1]),
        ] {
            assert_eq!(
                selected_artifact_platform(
                    &WasmCompilationStatus::Compiled,
                    &published,
                    &current_artifact_platform(target),
                ),
                Some(expected.as_str())
            );
        }
    }

    #[test]
    fn automatic_upgrade_only_uses_asynchronous_backends() {
        use crate::compilation::CompilationBackend::*;
        assert!(!automatic_upgrade_backend(None));
        assert!(!automatic_upgrade_backend(Some(&Http)));
        for backend in [
            LambdaInvoke,
            Sqs,
            AzureQueue,
            PubSub,
            Kafka,
            Redis,
            KubernetesJob,
        ] {
            assert!(automatic_upgrade_backend(Some(&backend)));
        }
    }

    #[test]
    fn upgrade_keeps_legacy_execution_available_until_portable_publication() {
        let current = artifact_platform_key("linux", "x86_64");
        let legacy = format!("linux-x86_64-wt{WASMTIME_MAJOR_VERSION}");
        let previous = vec![legacy.clone()];
        for status in [
            WasmCompilationStatus::Compiled,
            WasmCompilationStatus::Pending,
        ] {
            assert_eq!(
                selected_artifact_platform(&status, &previous, &current),
                Some(legacy.as_str())
            );
            let published = vec![legacy.clone(), current.clone()];
            assert_eq!(
                selected_artifact_platform(&status, &published, &current),
                Some(current.as_str())
            );
        }
        assert_eq!(
            selected_artifact_platform(&WasmCompilationStatus::Pending, &[], &current),
            None
        );
        assert_eq!(
            selected_artifact_platform(&WasmCompilationStatus::LocalOnly, &previous, &current),
            None
        );
        assert_eq!(
            selected_artifact_platform(
                &WasmCompilationStatus::Compiled,
                &previous,
                &artifact_platform_key("linux", "aarch64")
            ),
            None
        );
    }

    #[test]
    fn upgrade_retries_failed_dispatches_and_abandoned_claims_without_duplicate_jobs() {
        let marker = format!("{UPGRADE_MARKER}1000:claim");
        assert!(upgrade_due(&WasmCompilationStatus::Compiled, None, 1000));
        assert!(!upgrade_due(
            &WasmCompilationStatus::Pending,
            Some(&marker),
            1059
        ));
        assert!(!upgrade_due(
            &WasmCompilationStatus::Compiled,
            Some(&marker),
            1059
        ));
        assert!(upgrade_due(
            &WasmCompilationStatus::Compiled,
            Some(&marker),
            1060
        ));
        assert!(!upgrade_due(
            &WasmCompilationStatus::Pending,
            Some(&marker),
            2799
        ));
        assert!(upgrade_due(
            &WasmCompilationStatus::Pending,
            Some(&marker),
            2800
        ));
        assert!(!upgrade_due(&WasmCompilationStatus::Pending, None, 9999));
    }

    #[flow_like_types::tokio::test]
    async fn only_one_api_replica_claims_an_upgrade_and_old_snapshots_cannot_reclaim_publication() {
        use sea_orm::{ConnectionTrait, Database};
        let legacy = format!("linux-x86_64-wt{WASMTIME_MAJOR_VERSION}");
        let snapshot: wasm_package_version::Model = serde_json::from_value(serde_json::json!({
            "id": "version", "package_id": "test", "version": "1.0.0",
            "wasm_path": "node.wasm", "wasm_hash": "hash", "wasm_size": 8,
            "yanked": false, "published_at": "2026-01-01T00:00:00Z", "duplicate_flagged": false,
            "nodes": [], "widgets": [], "status": "Active", "compilation_status": "Compiled",
            "compiled_platforms": [legacy],
        }))
        .unwrap();
        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute_unprepared("ATTACH DATABASE ':memory:' AS public")
            .await
            .unwrap();
        db.execute_unprepared(
            r#"CREATE TABLE public."WasmPackageVersion" (
            id TEXT PRIMARY KEY, "wasmHash" TEXT, "compilationStatus" TEXT,
            "compilationError" TEXT, "compiledPlatforms" TEXT, "compiledArtifactGeneration" TEXT
        )"#,
        )
        .await
        .unwrap();
        wasm_package_version::Entity::insert(wasm_package_version::ActiveModel {
            id: Set(snapshot.id.clone()),
            wasm_hash: Set(snapshot.wasm_hash.clone()),
            compilation_status: Set(snapshot.compilation_status.clone()),
            compilation_error: Set(None),
            compiled_platforms: Set(snapshot.compiled_platforms.clone()),
            compiled_artifact_generation: Set(None),
            ..Default::default()
        })
        .exec(&db)
        .await
        .unwrap();
        assert_eq!(
            artifact_upgrade_claim(&snapshot, "artifact-upgrade:1000:first")
                .unwrap()
                .exec(&db)
                .await
                .unwrap()
                .rows_affected,
            1
        );
        assert_eq!(
            artifact_upgrade_claim(&snapshot, "artifact-upgrade:1000:second")
                .unwrap()
                .exec(&db)
                .await
                .unwrap()
                .rows_affected,
            0
        );

        // A legacy callback can publish without assigning a generation. Check
        // the platform list too so a pre-publication reader cannot reclaim it.
        wasm_package_version::Entity::update_many()
            .set(wasm_package_version::ActiveModel {
                compilation_status: Set(WasmCompilationStatus::Compiled),
                compilation_error: Set(None),
                compiled_platforms: Set(Some(
                    vec![artifact_platform_key("linux", "x86_64")].into(),
                )),
                ..Default::default()
            })
            .filter(wasm_package_version::Column::Id.eq(&snapshot.id))
            .exec(&db)
            .await
            .unwrap();
        assert_eq!(
            artifact_upgrade_claim(&snapshot, "artifact-upgrade:1000:stale")
                .unwrap()
                .exec(&db)
                .await
                .unwrap()
                .rows_affected,
            0
        );
    }
}
