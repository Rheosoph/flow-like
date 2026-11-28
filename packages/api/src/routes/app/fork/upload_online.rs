use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
    utils::fork::{
        transfer::{
            MAX_TRANSFER_BATCH, SignedFile, TransferSigner, is_uploadable_content_path,
            upload_staging_prefix,
        },
        upload::{adopt_staged_page, own_waiting_upload},
    },
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_storage::join_object_path;
use futures::future::try_join_all;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct UploadFileRequest {
    /// Path of the file in the local app, relative to its root, below
    /// `metadata/`, `media/`, `upload/` or `storage/`.
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize, ToSchema)]
pub struct UploadUrlsBody {
    pub files: Vec<UploadFileRequest>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct UploadUrlsResponse {
    /// One signed `PUT` per requested file, in request order.
    pub files: Vec<SignedFile>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AdoptUploadResponse {
    /// Files moved into the app by this call.
    pub adopted: u64,
    /// Whether staged files are left; call again until this is `false`.
    pub remaining: bool,
}

/// Hand out signed upload URLs for files of an offline → online fork. The
/// files land in a staging area under `tmp/` that expires on its own; the
/// server moves them into the app with `POST /apps/{app_id}/fork/online/adopt`
/// or at finalize.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/fork/online/upload-urls",
    tag = "forking",
    description = "Get signed upload URLs for the files of a local app you are uploading.",
    params(
        ("app_id" = String, Path, description = "Destination application id (returned by /fork/online/begin)"),
    ),
    request_body = UploadUrlsBody,
    responses(
        (status = 200, description = "One signed upload URL per file", body = UploadUrlsResponse),
        (status = 400, description = "A path is not allowed in an app's content, or too many files at once"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — the upload belongs to someone else"),
        (status = 409, description = "No upload is waiting for files in this app")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/fork/online/upload-urls",
    skip(state, user, body)
)]
pub async fn upload_urls(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<UploadUrlsBody>,
) -> Result<Json<UploadUrlsResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub = permission.sub()?;
    if body.files.len() > MAX_TRANSFER_BATCH {
        return Err(ApiError::bad_request(format!(
            "{} files requested at once, at most {MAX_TRANSFER_BATCH} are allowed",
            body.files.len()
        )));
    }
    if let Some(bad) = body
        .files
        .iter()
        .find(|file| !is_uploadable_content_path(&file.path))
    {
        return Err(ApiError::bad_request(format!(
            "{:?} is not a path an upload may write; files go below metadata/, media/, upload/ or storage/",
            bad.path
        )));
    }
    let job = own_waiting_upload(&state, &app_id, &sub).await?;
    let max_size = state.platform_config.forking.max_size_bytes;
    if let Some(too_big) = body.files.iter().find(|file| file.size > max_size) {
        return Err(ApiError::bad_request(format!(
            "{} has {} bytes, more than the deployment's fork size cap of {max_size} bytes",
            too_big.path, too_big.size
        )));
    }

    let signer = TransferSigner::content(&state).await?;
    let staging = upload_staging_prefix(&job.user_id, &app_id, &job.id);
    let files = try_join_all(body.files.into_iter().map(|file| {
        let key = join_object_path(&staging, &file.path);
        let signer = &signer;
        async move { signer.upload(file.path, &key, Some(file.size)).await }
    }))
    .await?;
    Ok(Json(UploadUrlsResponse {
        files,
        expires_at: signer.expires_at(),
    }))
}

/// Move one page of the files staged for an offline → online fork into the
/// app. Call until `remaining` is `false`; finalize moves whatever is left.
#[utoipa::path(
    post,
    path = "/apps/{app_id}/fork/online/adopt",
    tag = "forking",
    description = "Move uploaded files of a local app into the new online app.",
    params(
        ("app_id" = String, Path, description = "Destination application id (returned by /fork/online/begin)"),
    ),
    responses(
        (status = 200, description = "A page of files moved", body = AdoptUploadResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden — the upload belongs to someone else"),
        (status = 409, description = "No upload is waiting for files in this app")
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/fork/online/adopt", skip(state, user))]
pub async fn adopt_upload(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<AdoptUploadResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub = permission.sub()?;
    let job = own_waiting_upload(&state, &app_id, &sub).await?;
    let (adopted, remaining) = adopt_staged_page(&state, &app_id, &job).await?;
    Ok(Json(AdoptUploadResponse { adopted, remaining }))
}
