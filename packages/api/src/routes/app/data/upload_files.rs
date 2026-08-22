use std::time::Duration;

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::data::batch::{SIGN_CONCURRENCY, validate_batch},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_types::{Value, create_id, json};
use futures::stream::{self, StreamExt};
use utoipa::ToSchema;

/// How long an upload link is worth handing out, before the signing
/// credential's own lifetime is taken into account. See
/// [`RuntimeCredentials::signing_ttl`] — the credential, not this constant, is
/// what decides the deadline the URL actually advertises.
const UPLOAD_URL_TTL: Duration = Duration::from_secs(60 * 60 * 24);

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct UploadFilesPayload {
    pub prefixes: Vec<String>,
}

/// Signs a PUT URL per destination, preserving request order.
///
/// A prefix that cannot be signed yields an `error` entry instead of failing
/// the batch, so one bad path does not cost the other ninety-nine.
async fn sign_uploads(
    store: &FlowLikeStore,
    entries: Vec<(String, flow_like_storage::Path)>,
    ttl: Duration,
    sub: &str,
    app_id: &str,
) -> Vec<Value> {
    stream::iter(entries)
        .map(|(prefix, upload_path)| async move {
            match store.sign("PUT", &upload_path, ttl).await {
                Ok(url) => json::json!({
                    "prefix": prefix,
                    "url": url.to_string(),
                }),
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
                    json::json!({
                        "prefix": prefix,
                        "error": format!("Failed to create signed URL, reference ID: {}", id),
                    })
                }
            }
        })
        .buffered(SIGN_CONCURRENCY)
        .collect()
        .await
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/data",
    tag = "data",
    description = "Create signed upload URLs for file prefixes. Accepts at most 100 prefixes per request; split larger uploads into batches.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UploadFilesPayload,
    responses(
        (status = 200, description = "Signed upload URLs", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request - no prefixes, or more than 100 in one request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/data", skip(state, user, payload))]
pub async fn upload_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<UploadFilesPayload>,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);
    validate_batch(&payload.prefixes)?;

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
    let signing_creds = if scoped_creds.as_ref().is_azure() {
        state.master_credentials().await?
    } else {
        scoped_creds
    };
    let ttl = signing_creds.signing_ttl(UPLOAD_URL_TTL);
    let project_dir = signing_creds.to_store(false).await?;

    let mut entries = Vec::with_capacity(payload.prefixes.len());
    for prefix in &payload.prefixes {
        let upload_path = project_dir.construct_upload(&app_id, prefix).await?;
        entries.push((prefix.clone(), upload_path));
    }

    Ok(Json(
        sign_uploads(&project_dir, entries, ttl, &sub, &app_id).await,
    ))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/data/user",
    tag = "data",
    description = "Create signed upload URLs for your private app files. Accepts at most 100 prefixes per request; split larger uploads into batches.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = UploadFilesPayload,
    responses(
        (status = 200, description = "Signed upload URLs", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request - no prefixes, or more than 100 in one request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/data/user", skip(state, user, payload))]
pub async fn upload_user_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<UploadFilesPayload>,
) -> Result<Json<Vec<Value>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);
    validate_batch(&payload.prefixes)?;

    let sub = user.sub()?;

    let scoped_creds = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::EditUser,
        )
        .await?;

    let signing_creds = if scoped_creds.as_ref().is_azure() {
        state.master_credentials().await?
    } else {
        scoped_creds
    };
    let ttl = signing_creds.signing_ttl(UPLOAD_URL_TTL);
    let project_dir = signing_creds.to_store(false).await?;

    let mut entries = Vec::with_capacity(payload.prefixes.len());
    for prefix in &payload.prefixes {
        let upload_path = project_dir
            .construct_user_upload(&sub, &app_id, prefix)
            .await?;
        entries.push((prefix.clone(), upload_path));
    }

    Ok(Json(
        sign_uploads(&project_dir, entries, ttl, &sub, &app_id).await,
    ))
}
