use crate::entity::sea_orm_active_enums::WasmCompilationStatus;
use crate::entity::wasm_package_version;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::extract::State;
use axum::{Extension, Json};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct RecompileRequest {
    pub package_id: String,
    pub version: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RecompileResponse {
    pub success: bool,
    pub message: String,
}

/// POST /registry/recompile
/// Trigger server-side recompilation of a WASM package version.
#[utoipa::path(
    post,
    path = "/registry/recompile",
    tag = "registry",
    request_body = RecompileRequest,
    responses(
        (status = 200, description = "Recompilation triggered", body = RecompileResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Insufficient permissions"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn recompile(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<RecompileRequest>,
) -> Result<Json<RecompileResponse>, ApiError> {
    let sub = user.sub()?;

    state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    crate::ensure_wasm_permission!(state, &sub, &request.package_id, WasmPackagePermission::Maintainer);

    let version_record = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(&request.package_id))
        .filter(wasm_package_version::Column::Version.eq(&request.version))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::bad_request(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::bad_request("Version not found"))?;

    if version_record.yanked {
        return Err(ApiError::bad_request("Cannot recompile a yanked version"));
    }

    if version_record.compilation_status == WasmCompilationStatus::Compiled {
        return Err(ApiError::bad_request("Already compiled"));
    }

    if version_record.compilation_status == WasmCompilationStatus::Pending {
        return Err(ApiError::bad_request("Compilation already in progress"));
    }

    let mut active: wasm_package_version::ActiveModel = version_record.into();
    active.compilation_status = Set(WasmCompilationStatus::Pending);
    active.compilation_error = Set(None);
    active
        .update(&state.db)
        .await
        .map_err(|e| ApiError::bad_request(format!("Update failed: {}", e)))?;

    let registry = state.wasm_registry.clone().unwrap();

    match registry
        .recompile_version(sub, &request.package_id, &request.version)
        .await
    {
        Ok(()) => {
            tracing::info!(
                pkg = %request.package_id,
                ver = %request.version,
                "Recompilation succeeded / dispatched"
            );
        }
        Err(e) => {
            tracing::warn!(
                pkg = %request.package_id,
                ver = %request.version,
                err = %e,
                "Recompilation failed"
            );
            let record = wasm_package_version::Entity::find()
                .filter(wasm_package_version::Column::PackageId.eq(&request.package_id))
                .filter(wasm_package_version::Column::Version.eq(&request.version))
                .one(&state.db)
                .await;
            if let Ok(Some(rec)) = record {
                let mut am: wasm_package_version::ActiveModel = rec.into();
                am.compilation_status = Set(WasmCompilationStatus::LocalOnly);
                am.compilation_error = Set(Some(e.to_string()));
                let _ = am.update(&state.db).await;
            }
            return Err(ApiError::bad_request(format!("Recompilation failed: {e}")));
        }
    }

    Ok(Json(RecompileResponse {
        success: true,
        message: "Recompilation completed".to_string(),
    }))
}
