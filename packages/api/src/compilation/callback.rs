use crate::compilation::jwt;
use crate::entity::sea_orm_active_enums::WasmCompilationStatus;
use crate::entity::wasm_package_version;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
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

    let (status, platforms, error) = match result.status {
        CompilationStatus::Compiled => (
            WasmCompilationStatus::Compiled,
            result.compiled_platforms,
            None,
        ),
        CompilationStatus::Failed => (
            WasmCompilationStatus::LocalOnly,
            Vec::new(),
            result.error,
        ),
    };

    let mut update = wasm_package_version::ActiveModel {
        id: Set(version_record.id),
        compilation_status: Set(status),
        compiled_platforms: Set(platforms),
        compilation_error: Set(error),
        ..Default::default()
    };

    if let Some(nodes) = nodes {
        update.nodes = Set(nodes);
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

    tracing::info!(
        job_id = %result.job_id,
        package_id = %result.package_id,
        version = %result.version,
        "Compilation callback processed"
    );

    Ok(Json(CallbackResponse {
        ok: true,
        error: None,
    }))
}
