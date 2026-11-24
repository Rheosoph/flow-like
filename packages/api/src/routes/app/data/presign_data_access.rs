//! Presign data access endpoint
//!
//! Provides scoped storage credentials for direct client access to app data under
//! `apps/{app_id}/upload`. An optional subpath selects the returned path;
//! the credentials cover the full upload directory.

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
    /// Optional subpath in the upload directory. Selects the returned path only;
    /// it does not further restrict the credentials.
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
    /// Resolved path within the bucket/container. Append `/` when listing it.
    /// Without the slash the prefix also matches sibling names, which the
    /// credentials exclude.
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
    let write = wants_write(&payload.access_mode)?;
    let permission = ensure_permission!(user, &app_id, &state, app_file_permission(write));
    let sub = permission.sub()?;
    presign(&state, &user, &app_id, &sub, &payload, write, false).await
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
    let write = wants_write(&payload.access_mode)?;
    let permission = ensure_in_project!(user, &app_id, &state);
    let sub = permission.sub()?;
    presign(&state, &user, &app_id, &sub, &payload, write, true).await
}

async fn presign(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    sub: &str,
    payload: &PresignDataAccessRequest,
    write: bool,
    personal: bool,
) -> Result<Json<PresignDataAccessResponse>, ApiError> {
    let (access, path) = file_grant(sub, app_id, payload.prefix.as_deref(), write, personal);
    if write {
        crate::capacity::check_storage_write(state, app_id, sub, 0).await?;
    }

    let scoped_credentials = RuntimeCredentials::scoped(sub, app_id, state, access.clone())
        .await
        .map_err(|e| {
            tracing::error!("Failed to generate scoped credentials: {}", e);
            ApiError::internal("Failed to generate data access credentials")
        })?;
    let shared_credentials = serde_json::to_value(scoped_credentials.into_shared_credentials())
        .map_err(|e| {
            tracing::error!("Failed to serialize shared credentials: {}", e);
            ApiError::internal("Failed to serialize shared credentials")
        })?;

    crate::audit::storage_access::authorize_credentials(
        state,
        user,
        app_id,
        sub,
        &access,
        if personal { "user" } else { "app" },
        &scoped_credentials,
    )
    .await?;

    Ok(Json(PresignDataAccessResponse {
        shared_credentials,
        path: path.to_string(),
        access_mode: if write { "write" } else { "read" }.to_string(),
        expiration: scoped_credentials.expiration(),
    }))
}

fn wants_write(access_mode: &str) -> Result<bool, ApiError> {
    match access_mode.to_lowercase().as_str() {
        "read" => Ok(false),
        "write" => Ok(true),
        _ => Err(ApiError::bad_request(
            "access_mode must be either 'read' or 'write'",
        )),
    }
}

fn app_file_permission(write: bool) -> RolePermissions {
    if write {
        RolePermissions::WriteFiles
    } else {
        RolePermissions::ReadFiles
    }
}

/// The storage scope an endpoint issues and the path it returns. Both come
/// from one branch, so the credentials always cover the returned path.
fn file_grant(
    sub: &str,
    app_id: &str,
    prefix: Option<&str>,
    write: bool,
    personal: bool,
) -> (CredentialsAccess, FlowPath) {
    if personal {
        let access = if write {
            CredentialsAccess::EditUser
        } else {
            CredentialsAccess::ReadUser
        };
        (access, build_user_upload_path(sub, app_id, prefix))
    } else {
        let access = if write {
            CredentialsAccess::EditAppFiles
        } else {
            CredentialsAccess::ReadAppFiles
        };
        (access, build_upload_path(app_id, prefix))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    const APP: &str = "demo";
    const SUB: &str = "alice";
    /// What a client may send: nothing, a folder, a key copied from another
    /// app, and an attempt to climb out of the upload directory.
    const PREFIXES: [Option<&str>; 5] = [
        None,
        Some(""),
        Some("reports"),
        Some("apps/other/upload/reports"),
        Some("../../storage/db"),
    ];

    #[test]
    fn access_modes_ignore_case_and_reject_anything_else() {
        assert!(!wants_write("read").unwrap());
        assert!(wants_write("Write").unwrap());
        for invalid in ["", "delete", "read write"] {
            assert!(wants_write(invalid).is_err(), "{invalid:?}");
        }
        assert_eq!(app_file_permission(false), RolePermissions::ReadFiles);
        assert_eq!(app_file_permission(true), RolePermissions::WriteFiles);
    }

    #[test]
    fn each_endpoint_issues_the_scope_of_the_directory_it_returns() {
        let app_base = paths::app_upload_base(APP);
        let user_base = paths::user_upload_base(SUB, APP);
        assert_eq!(app_base.as_ref(), "apps/demo/upload");
        assert_eq!(user_base.as_ref(), "users/alice/apps/demo");
        for (personal, write, scope, base) in [
            (false, false, "read_app_files", &app_base),
            (false, true, "edit_app_files", &app_base),
            (true, false, "read_user", &user_base),
            (true, true, "edit_user", &user_base),
        ] {
            for prefix in PREFIXES {
                let (access, path) = file_grant(SUB, APP, prefix, write, personal);
                assert_eq!(access.to_string(), scope);
                assert!(path.prefix_matches(base), "{scope} returned {path}");
            }
        }
    }

    /// The app endpoint once returned `apps/{app}/upload/...` together with
    /// the caller's private-directory credentials, which S3 answers with
    /// AccessDenied. Each endpoint's own session policy has to reach the path
    /// it returns, and neither may reach the other's.
    #[cfg(feature = "aws")]
    #[test]
    fn each_endpoint_returns_a_path_its_credentials_reach() {
        use crate::credentials::aws_credentials::session_scope::{CONTENT_BUCKET, SessionScope};

        for write in [false, true] {
            for prefix in PREFIXES {
                let app = file_grant(SUB, APP, prefix, write, false);
                let user = file_grant(SUB, APP, prefix, write, true);
                for ((access, own), (_, foreign)) in [(&app, &user), (&user, &app)] {
                    let scope = SessionScope::of(SUB, APP, access);
                    assert!(scope.lists(CONTENT_BUCKET, &format!("{own}/")), "{own}");
                    assert!(
                        !scope.lists(CONTENT_BUCKET, &format!("{foreign}/")),
                        "{access} must not list {foreign}"
                    );
                    for action in ["s3:GetObject", "s3:PutObject", "s3:DeleteObject"] {
                        assert_eq!(
                            scope.allows(action, CONTENT_BUCKET, &format!("{own}/report.csv")),
                            write || action == "s3:GetObject",
                            "{access}: {action} in {own}"
                        );
                        assert!(
                            !scope.allows(action, CONTENT_BUCKET, &format!("{foreign}/report.csv")),
                            "{access}: {action} in {foreign}"
                        );
                    }
                }
            }
        }
    }
}
