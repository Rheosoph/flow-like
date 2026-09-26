use std::time::Duration;

use crate::{
    audit_branch, ensure_permission,
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
const UPLOAD_URL_TTL: Duration = Duration::from_secs(15 * 60);

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct UploadFilesPayload {
    pub prefixes: Vec<String>,
    /// Exact file sizes in the same order as prefixes. Required for bounded upload grants.
    #[serde(default)]
    pub sizes: Vec<u64>,
}

/// Signs byte-bound S3 forms or provider upload URLs, preserving request order.
///
/// A prefix that cannot be signed yields an `error` entry instead of failing
/// the batch, so one bad path does not cost the other ninety-nine.
async fn sign_uploads(
    state: &AppState,
    store: &FlowLikeStore,
    credentials: &crate::credentials::RuntimeCredentials,
    entries: Vec<(String, flow_like_storage::Path, u64)>,
    ttl: Duration,
    sub: &str,
    app_id: &str,
) -> Result<Vec<Value>, ApiError> {
    let mut reservations = Vec::new();
    for (_, path, bytes) in &entries {
        if let Some(form) =
            super::upload_policy::bounded_form(credentials, &path.to_string(), *bytes, ttl)?
        {
            reservations.push((form.bucket, path.to_string(), *bytes));
        }
    }
    let expires = chrono::Utc::now().fixed_offset()
        + chrono::Duration::from_std(ttl)
            .map_err(|_| ApiError::internal("invalid upload lifetime"))?;
    if reservations.is_empty() {
        let requested = entries.iter().map(|(_, _, bytes)| *bytes as i64).sum();
        crate::capacity::check_storage_write(state, app_id, sub, requested).await?;
    } else {
        crate::capacity::reserve_uploads(state, app_id, sub, reservations, expires).await?;
    }
    Ok(stream::iter(entries)
        .map(|(prefix, upload_path, bytes)| async move {
            match super::upload_policy::bounded_form(credentials, &upload_path.to_string(), bytes, ttl) {
                Ok(Some(form)) => return json::json!({ "prefix": prefix, "url": form.url, "method": form.method, "fields": form.fields }),
                Err(error) => return json::json!({ "prefix": prefix, "error": error.public_message().unwrap_or("Unable to authorize this upload.") }),
                Ok(None) => {},
            }
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
        .await)
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
    validate_sizes(&payload)?;

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
    for (prefix, bytes) in payload.prefixes.iter().zip(&payload.sizes) {
        let upload_path = project_dir.construct_upload(&app_id, prefix).await?;
        entries.push((prefix.clone(), upload_path, *bytes));
    }

    let uploads = sign_uploads(
        &state,
        &project_dir,
        signing_creds.as_ref(),
        entries,
        ttl,
        &sub,
        &app_id,
    )
    .await?;
    audit_upload_grants(&state, &user, &app_id, &uploads, "app", ttl).await;
    Ok(Json(uploads))
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
    validate_sizes(&payload)?;

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
    for (prefix, bytes) in payload.prefixes.iter().zip(&payload.sizes) {
        let upload_path = project_dir
            .construct_user_upload(&sub, &app_id, prefix)
            .await?;
        entries.push((prefix.clone(), upload_path, *bytes));
    }

    let uploads = sign_uploads(
        &state,
        &project_dir,
        signing_creds.as_ref(),
        entries,
        ttl,
        &sub,
        &app_id,
    )
    .await?;
    audit_upload_grants(&state, &user, &app_id, &uploads, "user", ttl).await;
    Ok(Json(uploads))
}

async fn audit_upload_grants(
    state: &AppState,
    user: &AppUser,
    app_id: &str,
    uploads: &[Value],
    scope: &str,
    ttl: Duration,
) {
    let granted_count = uploads
        .iter()
        .filter(|entry| entry.get("url").is_some())
        .count();
    if granted_count == 0 {
        return;
    }

    // A signed URL grants access; the subsequent storage write bypasses this API.
    audit_branch!(
        state,
        user,
        app_id,
        "file.upload.authorize",
        "Storage",
        app_id,
        serde_json::json!({
            "scope": scope,
            "granted_count": granted_count,
            "failed_count": uploads.len() - granted_count,
            "expires_in_seconds": ttl.as_secs(),
        })
    );
}

fn validate_sizes(payload: &UploadFilesPayload) -> Result<i64, ApiError> {
    if payload.prefixes.len() != payload.sizes.len() {
        return Err(ApiError::bad_request(
            "Each upload must declare its file size. Update Studio or reload the web app before uploading.",
        ));
    }
    payload.sizes.iter().try_fold(0_i64, |total, bytes| {
        let bytes = i64::try_from(*bytes)
            .map_err(|_| ApiError::bad_request("Upload size is too large."))?;
        total
            .checked_add(bytes)
            .ok_or_else(|| ApiError::bad_request("Upload batch is too large."))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sizes_are_required_and_checked_without_wrapping() {
        assert!(
            validate_sizes(&UploadFilesPayload {
                prefixes: vec!["file".into()],
                sizes: vec![]
            })
            .is_err()
        );
        assert!(
            validate_sizes(&UploadFilesPayload {
                prefixes: vec!["file".into()],
                sizes: vec![u64::MAX]
            })
            .is_err()
        );
        assert_eq!(
            validate_sizes(&UploadFilesPayload {
                prefixes: vec!["empty".into(), "data".into()],
                sizes: vec![0, 123]
            })
            .unwrap(),
            123
        );
    }
}
