//! Ensure active package versions have current Linux AOT artifacts.

use crate::audit;
use crate::entity::sea_orm_active_enums::{WasmCompilationStatus, WasmPackageStatus};
use crate::entity::{wasm_package, wasm_package_version};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::registry::server::{WASM_COMPILED_PATH, with_current_wasmtime_version};
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use flow_like_storage::object_store::Error as ObjectStoreError;
use flow_like_storage::object_store::path::Path;
use flow_like_wasm_schema::runtime::WASMTIME_MAJOR_VERSION;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use utoipa::ToSchema;

const LINUX_X86_64_OS: &str = "linux";
const LINUX_X86_64_ARCH: &str = "x86_64";

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnsureWasmArtifactsResponse {
    pub target_platform: String,
    pub wasmtime_version: String,
    pub active_packages: usize,
    pub checked_versions: usize,
    pub skipped_versions: usize,
    pub already_available: usize,
    pub already_pending: usize,
    pub jobs_started: usize,
    pub failed: usize,
    pub failures: Vec<EnsureWasmArtifactsFailure>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct EnsureWasmArtifactsFailure {
    pub package_id: String,
    pub version: String,
    pub message: String,
}

fn current_linux_x86_64_platform() -> String {
    format!(
        "{}-{}-wt{}",
        LINUX_X86_64_OS, LINUX_X86_64_ARCH, WASMTIME_MAJOR_VERSION
    )
}

fn artifact_paths(package_id: &str, version: &str, target_platform: &str) -> (Path, Path) {
    let base = Path::from(WASM_COMPILED_PATH)
        .child(package_id)
        .child(version);
    (
        base.child(format!("{}.cwasm", target_platform)),
        base.child(format!("{}.cwasm.b3", target_platform)),
    )
}

async fn object_exists(state: &AppState, path: &Path) -> Result<bool, ApiError> {
    match state.meta_bucket.as_generic().head(path).await {
        Ok(_) => Ok(true),
        Err(ObjectStoreError::NotFound { .. }) => Ok(false),
        Err(err) => Err(ApiError::internal(format!(
            "Failed to check object {}: {}",
            path, err
        ))),
    }
}

async fn artifacts_exist(
    state: &AppState,
    package_id: &str,
    version: &str,
    target_platform: &str,
) -> Result<bool, ApiError> {
    let (cwasm_path, checksum_path) = artifact_paths(package_id, version, target_platform);
    let cwasm_exists = object_exists(state, &cwasm_path).await?;
    let checksum_exists = object_exists(state, &checksum_path).await?;
    Ok(cwasm_exists && checksum_exists)
}

fn add_target_platform(platforms: Option<Vec<String>>, target_platform: &str) -> Vec<String> {
    let mut platforms = platforms.unwrap_or_default();
    if !platforms.iter().any(|platform| platform == target_platform) {
        platforms.push(target_platform.to_string());
    }
    platforms
}

fn remove_target_platform(platforms: Option<Vec<String>>, target_platform: &str) -> Vec<String> {
    let mut platforms = platforms.unwrap_or_default();
    platforms.retain(|platform| platform != target_platform);
    platforms
}

/// POST /admin/packages/ensure-wasm-artifacts
/// Check active packages' active current versions for current Linux cwasm artifacts.
#[utoipa::path(
    post,
    path = "/admin/packages/ensure-wasm-artifacts",
    tag = "admin",
    responses(
        (status = 200, description = "WASM artifact audit completed", body = EnsureWasmArtifactsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "WASM registry not configured")
    )
)]
pub async fn ensure_wasm_artifacts(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<EnsureWasmArtifactsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ManagePackages)
        .await?;

    let sub = user.sub()?;
    let registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let target_platform = current_linux_x86_64_platform();
    let active_packages = wasm_package::Entity::find()
        .filter(wasm_package::Column::Status.eq(WasmPackageStatus::Active))
        .all(&state.db)
        .await?;

    let mut checked_versions = 0;
    let mut skipped_versions = 0;
    let mut already_available = 0;
    let mut already_pending = 0;
    let mut jobs_started = 0;
    let mut failures = Vec::new();

    for package in &active_packages {
        let version_record = wasm_package_version::Entity::find()
            .filter(wasm_package_version::Column::PackageId.eq(&package.id))
            .filter(wasm_package_version::Column::Version.eq(&package.version))
            .filter(wasm_package_version::Column::Status.eq(WasmPackageStatus::Active))
            .filter(wasm_package_version::Column::Yanked.eq(false))
            .one(&state.db)
            .await?;

        let Some(version_record) = version_record else {
            skipped_versions += 1;
            continue;
        };

        checked_versions += 1;

        if artifacts_exist(&state, &package.id, &package.version, &target_platform).await? {
            let supported_wasmtime_versions = version_record.supported_wasmtime_versions.clone();
            let compiled_platforms = version_record.compiled_platforms.clone();
            let mut active: wasm_package_version::ActiveModel = version_record.into();
            active.compilation_status = Set(WasmCompilationStatus::Compiled);
            active.compiled_platforms = Set(Some(
                add_target_platform(compiled_platforms.map(Into::into), &target_platform).into(),
            ));
            active.supported_wasmtime_versions = Set(Some(
                with_current_wasmtime_version(supported_wasmtime_versions.map(Into::into)).into(),
            ));
            active.compilation_error = Set(None);
            active.update(&state.db).await?;
            already_available += 1;
            continue;
        }

        if version_record.compilation_status == WasmCompilationStatus::Pending {
            already_pending += 1;
            continue;
        }

        let version_id = version_record.id.clone();
        let next_platforms = remove_target_platform(
            version_record.compiled_platforms.clone().map(Into::into),
            &target_platform,
        );
        let mut active: wasm_package_version::ActiveModel = version_record.into();
        active.compilation_status = Set(WasmCompilationStatus::Pending);
        active.compiled_platforms = Set(Some(next_platforms.into()));
        active.compilation_error = Set(None);
        active.update(&state.db).await?;

        match registry
            .recompile_version(sub.clone(), &package.id, &package.version)
            .await
        {
            Ok(()) => {
                jobs_started += 1;
            }
            Err(err) => {
                failures.push(EnsureWasmArtifactsFailure {
                    package_id: package.id.clone(),
                    version: package.version.clone(),
                    message: err.to_string(),
                });

                let _ = wasm_package_version::ActiveModel {
                    id: Set(version_id),
                    compilation_status: Set(WasmCompilationStatus::LocalOnly),
                    compilation_error: Set(Some(err.to_string())),
                    ..Default::default()
                }
                .update(&state.db)
                .await;
            }
        }
    }

    audit!(
        state,
        user,
        "admin.package.ensure_wasm_artifacts",
        "WasmPackageVersion",
        target_platform.clone(),
        format!(
            "Checked {} active package versions and started {} compilation jobs",
            checked_versions, jobs_started
        )
    );

    Ok(Json(EnsureWasmArtifactsResponse {
        target_platform,
        wasmtime_version: WASMTIME_MAJOR_VERSION.to_string(),
        active_packages: active_packages.len(),
        checked_versions,
        skipped_versions,
        already_available,
        already_pending,
        jobs_started,
        failed: failures.len(),
        failures,
    }))
}
