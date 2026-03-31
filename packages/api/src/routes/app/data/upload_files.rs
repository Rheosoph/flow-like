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
pub struct UploadFilesPayload {
    pub prefixes: Vec<String>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/data",
    tag = "data",
    description = "Create signed upload URLs for file prefixes.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UploadFilesPayload,
    responses(
        (status = 200, description = "Signed upload URLs", body = String, content_type = "application/json"),
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
#[tracing::instrument(name = "PUT /apps/{app_id}/data", skip(state, user))]
pub async fn upload_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<UploadFilesPayload>,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);

    let sub = user.sub()?;

    // Get scoped credentials first to check the provider type
    let scoped_creds = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::EditApp,
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
        let upload_dir = project_dir.construct_upload(&app_id, prefix).await?;
        let signed_url = match project_dir
            .sign("PUT", &upload_dir, Duration::from_secs(60 * 60 * 24))
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
    put,
    path = "/apps/{app_id}/data/user",
    tag = "data",
    description = "Create signed upload URLs for your private app files.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UploadFilesPayload,
    responses(
        (status = 200, description = "Signed upload URLs", body = String, content_type = "application/json"),
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
#[tracing::instrument(name = "PUT /apps/{app_id}/data/user", skip(state, user))]
pub async fn upload_user_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<UploadFilesPayload>,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);

    let sub = user.sub()?;

    let scoped_creds = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::EditUser,
        )
        .await?;

    let project_dir = if scoped_creds.as_ref().is_azure() {
        state.master_credentials().await?.to_store(false).await?
    } else {
        scoped_creds.to_store(false).await?
    };

    let mut urls = Vec::with_capacity(payload.prefixes.len());

    for prefix in payload.prefixes.iter().take(MAX_PREFIXES) {
        let upload_dir = project_dir
            .construct_user_upload(&sub, &app_id, prefix)
            .await?;
        let signed_url = match project_dir
            .sign("PUT", &upload_dir, Duration::from_secs(60 * 60 * 24))
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
