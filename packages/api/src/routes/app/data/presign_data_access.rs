//! Presign data access endpoint
//!
//! Provides scoped storage credentials for direct client access to app data under
//! `apps/{app_id}/upload` with optional subpath.

use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    ensure_in_project, ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::data::paths,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_storage::Path as FlowPath;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PresignDataAccessRequest {
    /// Optional subpath inside apps/{app_id}/upload
    #[serde(default)]
    pub prefix: Option<String>,
    /// Access mode: "read" or "write"
    #[serde(default = "default_access_mode")]
    pub access_mode: String,
}

fn default_access_mode() -> String {
    "read".to_string()
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PresignDataAccessResponse {
    /// Shared credentials for direct storage access
    pub shared_credentials: serde_json::Value,
    /// Resolved path within the bucket/container
    pub path: String,
    /// Access mode granted
    pub access_mode: String,
    /// Expiration time (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/presign",
    tag = "data",
    description = "Get shared credentials for direct file access.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = PresignDataAccessRequest,
    responses(
        (status = 200, description = "Presigned data access credentials", body = PresignDataAccessResponse),
        (status = 400, description = "Bad request - invalid access mode"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - insufficient permissions"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/data/presign",
    skip(state, user, payload),
    fields(app_id = %app_id)
)]
pub async fn presign_data_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<PresignDataAccessRequest>,
) -> Result<Json<PresignDataAccessResponse>, ApiError> {
    let access_mode = payload.access_mode.to_lowercase();
    if access_mode != "read" && access_mode != "write" {
        return Err(ApiError::bad_request(
            "access_mode must be either 'read' or 'write'".to_string(),
        ));
    }

    let (required_permission, credentials_access) = if access_mode == "write" {
        (RolePermissions::WriteFiles, CredentialsAccess::EditUser)
    } else {
        (RolePermissions::ReadFiles, CredentialsAccess::ReadUser)
    };

    let permission = ensure_permission!(user, &app_id, &state, required_permission);
    let sub = permission.sub()?;

    let scoped_credentials = RuntimeCredentials::scoped(&sub, &app_id, &state, credentials_access)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate scoped credentials: {}", e);
            ApiError::internal("Failed to generate data access credentials")
        })?;

    let upload_path = build_upload_path(&app_id, payload.prefix.as_deref());
    let path_str = upload_path.to_string();

    let shared_credentials = serde_json::to_value(
        scoped_credentials.clone().into_shared_credentials(),
    )
    .map_err(|e| {
        tracing::error!("Failed to serialize shared credentials: {}", e);
        ApiError::internal("Failed to serialize shared credentials")
    })?;

    let expiration = scoped_credentials.expiration();

    Ok(Json(PresignDataAccessResponse {
        shared_credentials,
        path: path_str,
        access_mode,
        expiration,
    }))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/user/presign",
    tag = "data",
    description = "Get shared credentials for direct access to your private app files.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = PresignDataAccessRequest,
    responses(
        (status = 200, description = "Presigned data access credentials", body = PresignDataAccessResponse),
        (status = 400, description = "Bad request - invalid access mode"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - insufficient permissions"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/data/user/presign",
    skip(state, user, payload),
    fields(app_id = %app_id)
)]
pub async fn presign_user_data_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<PresignDataAccessRequest>,
) -> Result<Json<PresignDataAccessResponse>, ApiError> {
    let access_mode = payload.access_mode.to_lowercase();
    if access_mode != "read" && access_mode != "write" {
        return Err(ApiError::bad_request(
            "access_mode must be either 'read' or 'write'".to_string(),
        ));
    }

    let credentials_access = if access_mode == "write" {
        CredentialsAccess::EditUser
    } else {
        CredentialsAccess::ReadUser
    };

    let permission = ensure_in_project!(user, &app_id, &state);
    let sub = permission.sub()?;

    let scoped_credentials = RuntimeCredentials::scoped(&sub, &app_id, &state, credentials_access)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate scoped credentials: {}", e);
            ApiError::internal("Failed to generate data access credentials")
        })?;

    let upload_path = build_user_upload_path(&sub, &app_id, payload.prefix.as_deref());
    let path_str = upload_path.to_string();

    let shared_credentials = serde_json::to_value(
        scoped_credentials.clone().into_shared_credentials(),
    )
    .map_err(|e| {
        tracing::error!("Failed to serialize shared credentials: {}", e);
        ApiError::internal("Failed to serialize shared credentials")
    })?;

    let expiration = scoped_credentials.expiration();

    Ok(Json(PresignDataAccessResponse {
        shared_credentials,
        path: path_str,
        access_mode,
        expiration,
    }))
}

fn build_upload_path(app_id: &str, prefix: Option<&str>) -> FlowPath {
    match prefix {
        Some(prefix) => paths::resolve_app_upload(app_id, prefix),
        None => paths::app_upload_base(app_id),
    }
}

fn build_user_upload_path(sub: &str, app_id: &str, prefix: Option<&str>) -> FlowPath {
    match prefix {
        Some(prefix) => paths::resolve_user_upload(sub, app_id, prefix),
        None => paths::user_upload_base(sub, app_id),
    }
}
