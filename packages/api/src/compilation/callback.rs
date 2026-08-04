use crate::compilation::jwt;
use crate::entity::sea_orm_active_enums::{
    WasmCompilationStatus, WasmPackageStatus, WasmPackageVisibility,
};
use crate::entity::{wasm_package, wasm_package_version};
use crate::routes::registry::server::with_current_wasmtime_version;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use flow_like_types::dispatch::{CompilationResult, CompilationStatus};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
};
use serde::Serialize;
use std::sync::Arc;

#[derive(Debug, Serialize)]
pub struct CallbackResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub async fn handle_compilation_callback(
    State(db): State<Arc<DatabaseConnection>>,
    headers: axum::http::HeaderMap,
    Json(result): Json<CompilationResult>,
) -> Result<Json<CallbackResponse>, (StatusCode, Json<CallbackResponse>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| {
            (
                StatusCode::UNAUTHORIZED,
                Json(CallbackResponse {
                    ok: false,
                    error: Some("Missing or invalid Authorization header".to_string()),
                }),
            )
        })?;

    let claims = jwt::verify(token).map_err(|e| {
        (
            StatusCode::UNAUTHORIZED,
            Json(CallbackResponse {
                ok: false,
                error: Some(format!("JWT verification failed: {e}")),
            }),
        )
    })?;

    if claims.job_id != result.job_id
        || claims.package_id != result.package_id
        || claims.version != result.version
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(CallbackResponse {
                ok: false,
                error: Some("JWT claims do not match result payload".to_string()),
            }),
        ));
    }

    let version_record = wasm_package_version::Entity::find()
        .filter(wasm_package_version::Column::PackageId.eq(&result.package_id))
        .filter(wasm_package_version::Column::Version.eq(&result.version))
        .one(db.as_ref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CallbackResponse {
                    ok: false,
                    error: Some(format!("DB error: {e}")),
                }),
            )
        })?;

    let version_record = version_record.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(CallbackResponse {
                ok: false,
                error: Some("Package version not found".to_string()),
            }),
        )
    })?;

    let nodes = result.nodes;

    tracing::info!(
        job_id = %result.job_id,
        has_nodes = nodes.is_some(),
        platforms_count = result.compiled_platforms.len(),
        "Compilation callback received"
    );

    let (compilation_status, platforms, error) = match result.status {
        CompilationStatus::Compiled => (
            WasmCompilationStatus::Compiled,
            Some(result.compiled_platforms),
            None,
        ),
        CompilationStatus::Failed => (
            WasmCompilationStatus::LocalOnly,
            Some(Vec::new()),
            result.error,
        ),
    };

    let compiled_ok = compilation_status == WasmCompilationStatus::Compiled;
    let supported_wasmtime_versions = version_record.supported_wasmtime_versions.clone();

    let mut update = wasm_package_version::ActiveModel {
        id: Set(version_record.id),
        compilation_status: Set(compilation_status),
        compiled_platforms: Set(platforms),
        compilation_error: Set(error),
        ..Default::default()
    };

    if let Some(ref nodes) = nodes {
        update.nodes = Set(nodes.clone());
    }

    if compiled_ok {
        update.supported_wasmtime_versions = Set(Some(with_current_wasmtime_version(
            supported_wasmtime_versions,
        )));
    }

    // Auto-approve private packages on successful compilation
    let package = wasm_package::Entity::find_by_id(&result.package_id)
        .one(db.as_ref())
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(CallbackResponse {
                    ok: false,
                    error: Some(format!("DB error: {e}")),
                }),
            )
        })?;

    let auto_approve = compiled_ok
        && package
            .as_ref()
            .is_some_and(|p| p.visibility == WasmPackageVisibility::Private);

    if auto_approve {
        let now = chrono::Utc::now().naive_utc();
        update.status = Set(WasmPackageStatus::Active);
        update.approved_at = Set(Some(now));
    }

    update.update(db.as_ref()).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(CallbackResponse {
                ok: false,
                error: Some(format!("Failed to update version: {e}")),
            }),
        )
    })?;

    // Promote version data to parent package for private auto-approved packages
    if auto_approve && let Some(pkg) = &package {
        let now = chrono::Utc::now().naive_utc();
        let mut pkg_update = wasm_package::ActiveModel {
            id: Set(pkg.id.clone()),
            version: Set(result.version.clone()),
            wasm_path: Set(version_record.wasm_path.clone()),
            wasm_hash: Set(version_record.wasm_hash.clone()),
            wasm_size: Set(version_record.wasm_size),
            widgets: Set(version_record.widgets.clone()),
            widget_bundle_hash: Set(version_record.widget_bundle_hash.clone()),
            widget_bundle_size: Set(version_record.widget_bundle_size),
            updated_at: Set(now),
            ..Default::default()
        };
        if let Some(ref nodes) = nodes {
            pkg_update.nodes = Set(nodes.clone());
        }
        if let Err(e) = pkg_update.update(db.as_ref()).await {
            tracing::warn!(
                package_id = %result.package_id,
                error = %e,
                "Failed to promote version to parent package"
            );
        }
    }

    tracing::info!(
        job_id = %result.job_id,
        package_id = %result.package_id,
        version = %result.version,
        auto_approved = auto_approve,
        "Compilation callback processed"
    );

    Ok(Json(CallbackResponse {
        ok: true,
        error: None,
    }))
}
