use crate::entity::json_types::StringList;
use crate::entity::sea_orm_active_enums::{WasmCompilationStatus, WasmPackageVisibility};
use crate::entity::{wasm_package, wasm_package_user, wasm_package_version};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect};
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

/// The columns of a pinned version row this check reads.
struct PinnedVersion {
    yanked: bool,
    compilation_status: WasmCompilationStatus,
    compiled_platforms: Option<StringList>,
}

/// Whether the user holds any permission on each package, keyed by package id.
/// One authoritative query keeps revocations effective across API replicas.
async fn load_user_access(
    state: &AppState,
    user_id: &str,
    package_ids: Vec<String>,
) -> Result<HashMap<String, bool>, ApiError> {
    let mut access = HashMap::with_capacity(package_ids.len());
    if package_ids.is_empty() {
        return Ok(access);
    }

    let granted: HashMap<String, i64> = wasm_package_user::Entity::find()
        .select_only()
        .column(wasm_package_user::Column::PackageId)
        .column(wasm_package_user::Column::Permission)
        .filter(wasm_package_user::Column::UserId.eq(user_id))
        .filter(wasm_package_user::Column::PackageId.is_in(package_ids.clone()))
        .into_tuple::<(String, i64)>()
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .into_iter()
        .collect();

    for package_id in package_ids {
        let permission = granted
            .get(&package_id)
            .map(|bits| WasmPackagePermission::from_bits_truncate(*bits))
            .unwrap_or(WasmPackagePermission::empty());
        access.insert(package_id, !permission.is_empty());
    }

    Ok(access)
}

fn resolve_compilation(
    version_record: Option<&PinnedVersion>,
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
            let compiled = crate::execution::wasm_resolve::selected_artifact_platform(
                &v.compilation_status,
                v.compiled_platforms
                    .as_deref()
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
                platform_key,
            )
            .is_some();
            (Some(status_str.to_string()), compiled)
        }
        None => (None, false),
    }
}

fn resolve_status(
    version_record: Option<&PinnedVersion>,
    has_user_access: bool,
) -> PackageAccessStatus {
    let is_unavailable = version_record.is_none() || version_record.is_some_and(|v| v.yanked);
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

    let platform_key = super::server::executor_target_platform();

    if request.packages.is_empty() {
        return Ok(Json(PrerunCheckResponse {
            packages: Vec::new(),
            all_accessible: true,
            has_remote_only: false,
            has_unavailable: false,
        }));
    }

    let package_ids: Vec<String> = request.packages.keys().cloned().collect();

    // One round-trip for all packages instead of N.
    let pkg_by_id: HashMap<String, (String, WasmPackageVisibility)> = wasm_package::Entity::find()
        .select_only()
        .column(wasm_package::Column::Id)
        .column(wasm_package::Column::Name)
        .column(wasm_package::Column::Visibility)
        .filter(wasm_package::Column::Id.is_in(package_ids))
        .into_tuple::<(String, String, WasmPackageVisibility)>()
        .all(&state.db)
        .await
        .map_err(|e| ApiError::bad_request(format!("DB error: {}", e)))?
        .into_iter()
        .map(|(id, name, visibility)| (id, (name, visibility)))
        .collect();

    // One round-trip for exactly the pinned (package, version) pairs.
    let mut pinned = Condition::any();
    for (package_id, version) in &request.packages {
        pinned = pinned.add(
            Condition::all()
                .add(wasm_package_version::Column::PackageId.eq(package_id))
                .add(wasm_package_version::Column::Version.eq(version)),
        );
    }
    let version_by_pair: HashMap<(String, String), PinnedVersion> =
        wasm_package_version::Entity::find()
            .select_only()
            .column(wasm_package_version::Column::PackageId)
            .column(wasm_package_version::Column::Version)
            .column(wasm_package_version::Column::Yanked)
            .column(wasm_package_version::Column::CompilationStatus)
            .column(wasm_package_version::Column::CompiledPlatforms)
            .filter(pinned)
            .into_tuple::<(
                String,
                String,
                bool,
                WasmCompilationStatus,
                Option<StringList>,
            )>()
            .all(&state.db)
            .await
            .map_err(|e| ApiError::bad_request(format!("DB error: {}", e)))?
            .into_iter()
            .map(
                |(package_id, version, yanked, compilation_status, compiled_platforms)| {
                    (
                        (package_id, version),
                        PinnedVersion {
                            yanked,
                            compilation_status,
                            compiled_platforms,
                        },
                    )
                },
            )
            .collect();

    let restricted_ids: Vec<String> = pkg_by_id
        .iter()
        .filter(|(_, (_, visibility))| *visibility != WasmPackageVisibility::Public)
        .map(|(id, _)| id.clone())
        .collect();
    let user_access = load_user_access(&state, &sub, restricted_ids).await?;

    let mut results = Vec::with_capacity(request.packages.len());
    for (package_id, version) in &request.packages {
        let Some((package_name, visibility)) = pkg_by_id.get(package_id) else {
            results.push(unavailable_info(package_id));
            continue;
        };

        let is_public = *visibility == WasmPackageVisibility::Public;
        let has_user_access = is_public || user_access.get(package_id).copied().unwrap_or(false);

        let version_record = version_by_pair.get(&(package_id.clone(), version.clone()));
        let (compilation_status, server_compiled) =
            resolve_compilation(version_record, &platform_key);
        let status = resolve_status(version_record, has_user_access);

        results.push(PackageAccessInfo {
            package_id: package_id.clone(),
            package_name: Some(package_name.clone()),
            status,
            has_user_access,
            is_public,
            compilation_status,
            server_compiled,
        });
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

#[cfg(test)]
mod artifact_availability_tests {
    use super::*;
    use flow_like_wasm_schema::runtime::{WASMTIME_MAJOR_VERSION, artifact_platform_key};

    #[test]
    fn pending_upgrade_keeps_published_artifacts_available() {
        let target = artifact_platform_key("linux", "x86_64");
        let mut version = PinnedVersion {
            yanked: false,
            compilation_status: WasmCompilationStatus::Pending,
            compiled_platforms: Some(
                vec![format!("linux-x86_64-wt{WASMTIME_MAJOR_VERSION}")].into(),
            ),
        };
        assert_eq!(
            resolve_compilation(Some(&version), &target),
            (Some("pending".into()), true)
        );
        version.compiled_platforms = Some(Vec::<String>::new().into());
        assert_eq!(
            resolve_compilation(Some(&version), &target),
            (Some("pending".into()), false)
        );
        version.compiled_platforms = Some(vec![target.clone()].into());
        version.yanked = true;
        assert_eq!(
            resolve_compilation(Some(&version), &target),
            (Some("yanked".into()), false)
        );
    }
}
