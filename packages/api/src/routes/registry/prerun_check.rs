use crate::entity::sea_orm_active_enums::{WasmCompilationStatus, WasmPackageVisibility};
use crate::entity::{wasm_package, wasm_package_version};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct PrerunCheckRequest {
    pub packages: HashMap<String, String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PackageAccessInfo {
    pub package_id: String,
    pub package_name: Option<String>,
    pub status: PackageAccessStatus,
    pub has_user_access: bool,
    pub is_public: bool,
    pub compilation_status: Option<String>,
    pub server_compiled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PackageAccessStatus {
    Accessible,
    RemoteOnly,
    Unavailable,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PrerunCheckResponse {
    pub packages: Vec<PackageAccessInfo>,
    pub all_accessible: bool,
    pub has_remote_only: bool,
    pub has_unavailable: bool,
}

fn unavailable_info(package_id: &str) -> PackageAccessInfo {
    PackageAccessInfo {
        package_id: package_id.to_string(),
        package_name: None,
        status: PackageAccessStatus::Unavailable,
        has_user_access: false,
        is_public: false,
        compilation_status: None,
        server_compiled: false,
    }
}

async fn check_user_access(
    state: &AppState,
    package_id: &str,
    user_id: &str,
    is_public: bool,
) -> Result<bool, ApiError> {
    if is_public {
        return Ok(true);
    }
    let access = crate::check_wasm_access!(state, user_id, package_id);
    Ok(access.is_some())
}

fn resolve_compilation(
    version_record: &Option<wasm_package_version::Model>,
    platform_key: &str,
) -> (Option<String>, bool) {
    match version_record {
        Some(v) if v.yanked => (Some("yanked".to_string()), false),
        Some(v) => {
            let status_str = match v.compilation_status {
                WasmCompilationStatus::Compiled => "compiled",
                WasmCompilationStatus::LocalOnly => "local_only",
                WasmCompilationStatus::Pending => "pending",
            };
            let compiled = v.compilation_status == WasmCompilationStatus::Compiled
                && v.compiled_platforms.iter().any(|k| k == platform_key);
            (Some(status_str.to_string()), compiled)
        }
        None => (None, false),
    }
}

fn resolve_status(
    version_record: &Option<wasm_package_version::Model>,
    has_user_access: bool,
) -> PackageAccessStatus {
    let is_unavailable =
        version_record.is_none() || version_record.as_ref().is_some_and(|v| v.yanked);
    if is_unavailable {
        PackageAccessStatus::Unavailable
    } else if has_user_access {
        PackageAccessStatus::Accessible
    } else {
        PackageAccessStatus::RemoteOnly
    }
}

#[utoipa::path(
    post,
    path = "/registry/prerun-check",
    tag = "registry",
    request_body = PrerunCheckRequest,
    responses(
        (status = 200, description = "Access check results", body = PrerunCheckResponse),
        (status = 401, description = "Authentication required"),
    ),
    security(("bearer_auth" = []))
)]
pub async fn prerun_check(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<PrerunCheckRequest>,
) -> Result<Json<PrerunCheckResponse>, ApiError> {
    let sub = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let platform_key = format!("{}-{}-wt40", std::env::consts::OS, std::env::consts::ARCH);
    let mut results = Vec::with_capacity(request.packages.len());

    for (package_id, version) in &request.packages {
        let info = check_single_package(&state, package_id, version, &sub, &platform_key).await?;
        results.push(info);
    }

    let all_accessible = results
        .iter()
        .all(|p| matches!(p.status, PackageAccessStatus::Accessible));
    let has_remote_only = results
        .iter()
        .any(|p| matches!(p.status, PackageAccessStatus::RemoteOnly));
    let has_unavailable = results
        .iter()
        .any(|p| matches!(p.status, PackageAccessStatus::Unavailable));

    Ok(Json(PrerunCheckResponse {
        packages: results,
        all_accessible,
        has_remote_only,
        has_unavailable,
    }))
}

async fn check_single_package(
    state: &AppState,
    package_id: &str,
    version: &str,
    user_id: &str,
    platform_key: &str,
) -> Result<PackageAccessInfo, ApiError> {
    let pkg = wasm_package::Entity::find_by_id(package_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::bad_request(format!("DB error: {}", e)))?;

    let Some(pkg) = pkg else {
        return Ok(unavailable_info(package_id));
    };

    let is_public = pkg.visibility == WasmPackageVisibility::Public;
    let has_user_access = check_user_access(state, package_id, user_id, is_public).await?;

    let version_record = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(package_id))
        .filter(wasm_package_version::Column::Version.eq(version))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::bad_request(format!("DB error: {}", e)))?;

    let (compilation_status, server_compiled) = resolve_compilation(&version_record, platform_key);
    let status = resolve_status(&version_record, has_user_access);

    Ok(PackageAccessInfo {
        package_id: package_id.to_string(),
        package_name: Some(pkg.name),
        status,
        has_user_access,
        is_public,
        compilation_status,
        server_compiled,
    })
}
