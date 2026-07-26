use std::time::Duration;

use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{Value, create_id, json};
use utoipa::ToSchema;

const MAX_PREFIXES: usize = 100;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct DownloadFilesPayload {
    pub prefixes: Vec<String>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/download",
    tag = "data",
    description = "Create signed download URLs for file prefixes.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = DownloadFilesPayload,
    responses(
        (status = 200, description = "Signed download URLs", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/data/download", skip(state, user))]
pub async fn download_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<DownloadFilesPayload>,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let sub = user.sub()?;

    // Get scoped credentials first to check the provider type
    let scoped_creds = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::ReadApp,
        )
        .await?;

    // Azure SAS tokens cannot generate new signed URLs, so use master credentials for Azure
    let project_dir = if scoped_creds.as_ref().is_azure() {
        state.master_credentials().await?.to_store(false).await?
    } else {
        scoped_creds.to_store(false).await?
    };

    let mut urls = Vec::with_capacity(payload.prefixes.len());

    for prefix in payload.prefixes.iter().take(MAX_PREFIXES) {
        // Sanitize the path to prevent accessing other apps' files
        // Handle both full paths (apps/{app_id}/upload/...) and relative paths (boards/...)
        let download_path = if prefix.starts_with("apps/") {
            // Full path: extract segments after apps/{any_app_id}/upload/ and reconstruct with actual app_id
            // This prevents users from accessing other apps' files by manipulating the path
            let segments: Vec<&str> = prefix.split('/').collect();
            if segments.len() > 3 {
                // Skip "apps", the (potentially malicious) app_id, and "upload", keep the rest
                let relative_segments = &segments[3..];
                let mut path = flow_like_storage::Path::from("apps")
                    .child(app_id.as_str())
                    .child("upload");
                for segment in relative_segments {
                    if !segment.is_empty() {
                        path = path.child(*segment);
                    }
                }
                path
            } else {
                // Malformed full path, construct safe default
                flow_like_storage::Path::from("apps")
                    .child(app_id.as_str())
                    .child("upload")
            }
        } else {
            // Relative path: construct full path with app_id/upload prefix
            let mut path = flow_like_storage::Path::from("apps")
                .child(app_id.as_str())
                .child("upload");
            for segment in prefix.split('/') {
                if !segment.is_empty() {
                    path = path.child(segment);
                }
            }
            path
        };

        let signed_url = match project_dir
            .sign_cached("GET", &download_path, Duration::from_secs(60 * 60 * 24))
            .await
        {
            Ok(url) => url,
            Err(e) => {
                let id = create_id();
                tracing::error!(
                    "[{}] Failed to sign URL for prefix '{}': {:?} [sent by {} for project {}]",
                    id,
                    prefix,
                    e,
                    sub,
                    app_id
                );
                urls.push(json::json!({
                    "prefix": prefix,
                    "error": format!("Failed to create signed URL, reference ID: {}", id),
                }));
                continue;
            }
        };

        urls.push(json::json!({
            "prefix": prefix,
            "url": signed_url.to_string(),
        }));
    }

    Ok(Json(urls))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/user/download",
    tag = "data",
    description = "Create signed download URLs for your private app files.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = DownloadFilesPayload,
    responses(
        (status = 200, description = "Signed download URLs", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/data/user/download", skip(state, user))]
pub async fn download_user_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<DownloadFilesPayload>,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let sub = user.sub()?;

    let scoped_creds = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::ReadUser,
        )
        .await?;

    let project_dir = if scoped_creds.as_ref().is_azure() {
        state.master_credentials().await?.to_store(false).await?
    } else {
        scoped_creds.to_store(false).await?
    };

    let mut urls = Vec::with_capacity(payload.prefixes.len());

    for prefix in payload.prefixes.iter().take(MAX_PREFIXES) {
        let download_path = if prefix.starts_with("users/") {
            let segments: Vec<&str> = prefix.split('/').collect();
            if segments.len() > 4 {
                let relative_segments = &segments[4..];
                let mut path = flow_like_storage::Path::from("users")
                    .child(sub.as_str())
                    .child("apps")
                    .child(app_id.as_str());
                for segment in relative_segments {
                    if !segment.is_empty() {
                        path = path.child(*segment);
                    }
                }
                path
            } else {
                flow_like_storage::Path::from("users")
                    .child(sub.as_str())
                    .child("apps")
                    .child(app_id.as_str())
            }
        } else {
            let mut path = flow_like_storage::Path::from("users")
                .child(sub.as_str())
                .child("apps")
                .child(app_id.as_str());
            for segment in prefix.split('/') {
                if !segment.is_empty() {
                    path = path.child(segment);
                }
            }
            path
        };

        let signed_url = match project_dir
            .sign_cached("GET", &download_path, Duration::from_secs(60 * 60 * 24))
            .await
        {
            Ok(url) => url,
            Err(e) => {
                let id = create_id();
                tracing::error!(
                    "[{}] Failed to sign user URL for prefix '{}': {:?} [sent by {} for project {}]",
                    id,
                    prefix,
                    e,
                    sub,
                    app_id
                );
                urls.push(json::json!({
                    "prefix": prefix,
                    "error": format!("Failed to create signed URL, reference ID: {}", id),
                }));
                continue;
            }
        };

        urls.push(json::json!({
            "prefix": prefix,
            "url": signed_url.to_string(),
        }));
    }

    Ok(Json(urls))
}
