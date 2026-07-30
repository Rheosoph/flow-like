//! Presign LanceDB access endpoint
//!
//! This endpoint provides presigned access to the scoped LanceDB database root, allowing clients
//! to directly query the database without proxying through the API. This is useful for
//! performance-sensitive operations and reducing server load.
//!
//! The endpoint supports both read-only and read-write access based on user permissions.

use crate::{
    credentials::{CredentialsAccess, RuntimeCredentials},
    ensure_in_project,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct PresignDbAccessRequest {
    /// Requested table name for client convenience.
    ///
    /// Credentials returned by this endpoint are scoped to the user/app database path,
    /// not to an individual table.
    pub table_name: String,
    /// Access mode: "read" or "write"
    #[serde(default = "default_access_mode")]
    pub access_mode: String,
}

fn default_access_mode() -> String {
    "read".to_string()
}

fn scoped_db_path(sub: &str, app_id: &str) -> String {
    format!("users/{}/apps/{}/db", sub, app_id)
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PresignDbAccessResponse {
    /// Shared credentials for direct storage access
    pub shared_credentials: serde_json::Value,
    /// Base database path for the scoped user database (users/{sub}/apps/{app_id}/db)
    pub db_path: String,
    /// Requested table name echoed back to the client
    pub table_name: String,
    /// Access mode granted
    pub access_mode: String,
    /// Expiration time (if available)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiration: Option<chrono::DateTime<chrono::Utc>>,
}

/// Presign access to a scoped LanceDB database
///
/// This endpoint generates presigned credentials for direct client-side access to LanceDB.
/// The credentials are scoped to the specific app and user database path, with permissions
/// based on the requested access mode and user's role permissions.
///
/// # Access Modes
/// - `read`: Read-only access to the user's own database
/// - `write`: Read-write access to the user's own database
///
/// # Security
/// - Only requires project membership (user-scoped data, not project-scoped)
/// - Credentials are temporary and scoped to the user's database path
/// - Different storage providers (S3, Azure, GCP, R2) use appropriate presigning mechanisms
///
/// # Example Response (AWS/R2)
/// ```json
/// {
///   "provider": "aws",
///   "uri": "s3://bucket/users/user-id/apps/app-id/db",
///   "storage_options": {
///     "aws_access_key_id": "ASIA...",
///     "aws_secret_access_key": "...",
///     "aws_session_token": "...",
///     "aws_region": "us-east-1"
///   },
///   "table_name": "my_table",
///   "access_mode": "read",
///   "expiration": "2026-02-06T12:00:00Z"
/// }
/// ```
///
/// # Example Response (Azure)
/// ```json
/// {
///   "provider": "azure",
///   "uri": "az://container/users/user-id/apps/app-id/db",
///   "storage_options": {
///     "azure_storage_account_name": "account",
///     "azure_storage_sas_token": "..."
///   },
///   "table_name": "my_table",
///   "access_mode": "read",
///   "expiration": "2026-02-06T12:00:00Z"
/// }
/// ```
#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/presign",
    tag = "database",
    description = "Get shared credentials for direct LanceDB access to the scoped user/app database.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = PresignDbAccessRequest,
    responses(
        (status = 200, description = "Presigned database access credentials", body = PresignDbAccessResponse),
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
    name = "POST /apps/{app_id}/db/presign",
    skip(state, user, payload),
    fields(app_id = %app_id)
)]
pub async fn presign_db_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<PresignDbAccessRequest>,
) -> Result<Json<PresignDbAccessResponse>, ApiError> {
    // Validate access mode
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

    // Get scoped credentials for the user
    let scoped_credentials = RuntimeCredentials::scoped(&sub, &app_id, &state, credentials_access)
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate scoped credentials: {}", e);
            ApiError::internal("Failed to generate database access credentials")
        })?;

    let shared_credentials = serde_json::to_value(
        scoped_credentials.clone().into_shared_credentials(),
    )
    .map_err(|e| {
        tracing::error!("Failed to serialize shared credentials: {}", e);
        ApiError::internal("Failed to serialize shared credentials")
    })?;

    let db_path = scoped_db_path(&sub, &app_id);

    // Get expiration time if available
    let expiration = get_credentials_expiration(&scoped_credentials);

    Ok(Json(PresignDbAccessResponse {
        shared_credentials,
        db_path,
        table_name: payload.table_name,
        access_mode,
        expiration,
    }))
}

/// Presign access to the project-scoped LanceDB database
///
/// Like `presign_db_access`, but for the **project** database
/// (`apps/{app_id}/storage/db`) instead of the per-user database. Access is
/// gated by `ReadFiles` or `ReadDatabase` (read) / `WriteFiles` or
/// `WriteDatabase` (write) on the app — this also covers connected apps
/// calling with an app-connection token, whose permissions come from the
/// role assigned to the connection.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/presign/project",
    tag = "database",
    description = "Get shared credentials for direct LanceDB access to the project app database.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = PresignDbAccessRequest,
    responses(
        (status = 200, description = "Presigned database access credentials", body = PresignDbAccessResponse),
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
    name = "POST /apps/{app_id}/db/presign/project",
    skip(state, user, payload),
    fields(app_id = %app_id)
)]
pub async fn presign_project_db_access(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<PresignDbAccessRequest>,
) -> Result<Json<PresignDbAccessResponse>, ApiError> {
    let access_mode = payload.access_mode.to_lowercase();
    if access_mode != "read" && access_mode != "write" {
        return Err(ApiError::bad_request(
            "access_mode must be either 'read' or 'write'".to_string(),
        ));
    }

    use crate::permission::role_permission::RolePermissions;

    let (file_permission, db_permission, credentials_access) = if access_mode == "write" {
        (
            RolePermissions::WriteFiles,
            RolePermissions::WriteDatabase,
            CredentialsAccess::EditAppDb,
        )
    } else {
        (
            RolePermissions::ReadFiles,
            RolePermissions::ReadDatabase,
            CredentialsAccess::ReadAppDb,
        )
    };

    let permission =
        crate::ensure_any_permission!(user, &app_id, &state, file_permission, db_permission);
    let identifier = permission.identifier();

    let scoped_credentials =
        RuntimeCredentials::scoped(&identifier, &app_id, &state, credentials_access)
            .await
            .map_err(|e| {
                tracing::error!("Failed to generate scoped credentials: {}", e);
                ApiError::internal("Failed to generate database access credentials")
            })?;

    let shared_credentials = serde_json::to_value(
        scoped_credentials.clone().into_shared_credentials(),
    )
    .map_err(|e| {
        tracing::error!("Failed to serialize shared credentials: {}", e);
        ApiError::internal("Failed to serialize shared credentials")
    })?;

    let db_path = format!("apps/{}/storage/db", app_id);
    let expiration = get_credentials_expiration(&scoped_credentials);

    Ok(Json(PresignDbAccessResponse {
        shared_credentials,
        db_path,
        table_name: payload.table_name,
        access_mode,
        expiration,
    }))
}

/// Get expiration time from credentials if available
fn get_credentials_expiration(
    credentials: &RuntimeCredentials,
) -> Option<chrono::DateTime<chrono::Utc>> {
    match credentials {
        #[cfg(feature = "aws")]
        RuntimeCredentials::Aws(aws) => aws.expiration,
        #[cfg(feature = "azure")]
        RuntimeCredentials::Azure(azure) => azure.expiration,
        #[cfg(feature = "gcp")]
        RuntimeCredentials::Gcp(gcp) => gcp.expiration,
        #[cfg(feature = "r2")]
        RuntimeCredentials::R2(r2) => r2.expiration,
        RuntimeCredentials::Mixed(mixed) => get_credentials_expiration(&mixed.content),
    }
}
