use crate::error::ApiError;
use crate::state::AppState;
use crate::{auth::AppUser, ensure_permission, permission::role_permission::RolePermissions};
use axum::{
    Extension, Json, Router,
    extract::{Query, State},
    routing::{get, post},
};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use flow_like_storage::{Path as FLPath, files::store::FlowLikeStore};
use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
use flow_like_types::tokio::try_join;
use flow_like_types::{
    create_id,
    mime_guess::{self, mime},
};
use futures::future::try_join_all;
use mime::Mime;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

const MAX_DOWNLOAD_TTL_SECS: u64 = 60 * 60 * 24 * 31;
const DEFAULT_DOWNLOAD_TTL_SECS: u64 = 60 * 60 * 24 * 7;
const UPLOAD_TTL_SECS: u64 = 60 * 15;
// Optional soft client hint (not enforced by PUT presign; enforce on POST policies or server finalize step)
const DEFAULT_SIZE_LIMIT_BYTES: Option<u64> = Some(1024 * 1024 * 35); // 35 MB
/// Upper bound on one batch presign request. Folder uploads chunk client-side to this size.
const MAX_BATCH_FILES: usize = 100;

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryFileResponse {
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_path: Option<TemporaryFlowPath>,
    pub content_type: String,
    pub upload_url: String,
    pub upload_expires_at: String,
    pub download_url: String,
    pub download_expires_at: String,
    /// Only signed for single-file requests; batch requests omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_url: Option<String>,
    /// Only signed for single-file requests; batch requests omit it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_url: Option<String>,
    pub size_limit_bytes: Option<u64>,
}

/// One file in a batch presign request.
#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryFileBatchItem {
    /// Optional file extension (e.g. "png"). Will be sanitized (alnum only).
    pub extension: Option<String>,
    /// Optional explicit content-type; falls back to extension mapping or octet-stream.
    pub content_type: Option<String>,
}

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryFileBatchRequest {
    /// Optional app id. When set, uploads are created under the app-scoped temporary invoke prefix.
    pub app_id: Option<String>,
    /// Optional custom download TTL in seconds (capped at 31 days).
    pub download_ttl_secs: Option<u64>,
    /// Files to presign, at most 100 per request.
    pub files: Vec<TemporaryFileBatchItem>,
}

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryFileBatchResponse {
    /// Presigned uploads in the same order as the request.
    pub files: Vec<TemporaryFileResponse>,
}

#[derive(Clone, Deserialize, Serialize, Debug, ToSchema)]
pub struct TemporaryFlowPath {
    pub path: String,
    pub store_ref: String,
    pub cache_store_ref: Option<String>,
}

#[derive(Deserialize, Debug, ToSchema, utoipa::IntoParams)]
pub struct ExtensionParams {
    /// Optional app id. When set, the upload is created under the app-scoped temporary invoke prefix.
    #[serde(default, alias = "appId")]
    pub app_id: Option<String>,
    /// Optional file extension (e.g. "png"). Will be sanitized (alnum only).
    pub extension: Option<String>,
    /// Optional explicit content-type; falls back to extension mapping or octet-stream.
    pub content_type: Option<String>,
    /// Optional custom download TTL in seconds (capped at 31 days).
    pub download_ttl_secs: Option<u64>,
    /// Optional original filename. Fills in the extension - and through it the
    /// content type - when `extension` is not given. It cannot be echoed on the
    /// download URL: that URL is presigned, and any query parameter added after
    /// signing invalidates the signature. The name itself survives on the object
    /// via the `Content-Disposition` the client sets on the upload.
    pub filename: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(get_temporary_upload))
        .route("/batch", post(create_temporary_uploads))
}

#[utoipa::path(
    get,
    path = "/tmp",
    tag = "tmp",
    params(ExtensionParams),
    responses(
        (status = 200, description = "Presigned temporary upload URL generated successfully", body = TemporaryFileResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(
    name = "GET /tmp",
    skip(state, user, params),
    fields(user_sub = tracing::field::Empty, key = tracing::field::Empty, ext = tracing::field::Empty)
)]
pub async fn get_temporary_upload(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(params): Query<ExtensionParams>,
) -> Result<Json<TemporaryFileResponse>, ApiError> {
    let sub = user.sub()?;

    if let Some(app_id) = params.app_id.as_deref() {
        ensure_permission!(user, app_id, &state, RolePermissions::ExecuteEvents);
    }

    let now_utc = Utc::now();
    let extension = params
        .extension
        .as_deref()
        .map(str::trim)
        .filter(|extension| !extension.is_empty())
        .or_else(|| filename_extension(params.filename.as_deref()));
    let key = temporary_upload_key(&sub, params.app_id.as_deref(), extension, now_utc);
    let content_type = resolve_content_type(&key, params.content_type.as_deref());

    tracing::Span::current().record("user_sub", sub.as_str());
    tracing::Span::current().record("key", key.as_str());
    tracing::Span::current().record("ext", extension.unwrap_or("bin"));

    let download_ttl = params
        .download_ttl_secs
        .unwrap_or(DEFAULT_DOWNLOAD_TTL_SECS)
        .min(MAX_DOWNLOAD_TTL_SECS);

    let master = state.master_credentials().await?;
    let store = master.to_store(false).await?;

    let response = sign_temporary_upload(
        &store,
        key,
        content_type,
        params.app_id.is_some(),
        download_ttl,
        now_utc,
        true,
    )
    .await?;

    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/tmp/batch",
    tag = "tmp",
    request_body = TemporaryFileBatchRequest,
    responses(
        (status = 200, description = "Presigned temporary upload URLs generated successfully", body = TemporaryFileBatchResponse),
        (status = 400, description = "Too many files requested"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal server error")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(
    name = "POST /tmp/batch",
    skip(state, user, request),
    fields(user_sub = tracing::field::Empty, files = tracing::field::Empty)
)]
pub async fn create_temporary_uploads(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(request): Json<TemporaryFileBatchRequest>,
) -> Result<Json<TemporaryFileBatchResponse>, ApiError> {
    let sub = user.sub()?;
    tracing::Span::current().record("files", request.files.len());

    if request.files.is_empty() {
        return Ok(Json(TemporaryFileBatchResponse { files: Vec::new() }));
    }

    if request.files.len() > MAX_BATCH_FILES {
        return Err(ApiError::bad_request(format!(
            "Batch upload requests are limited to {MAX_BATCH_FILES} files, got {}",
            request.files.len()
        )));
    }

    if let Some(app_id) = request.app_id.as_deref() {
        ensure_permission!(user, app_id, &state, RolePermissions::ExecuteEvents);
    }

    let now_utc = Utc::now();
    let download_ttl = request
        .download_ttl_secs
        .unwrap_or(DEFAULT_DOWNLOAD_TTL_SECS)
        .min(MAX_DOWNLOAD_TTL_SECS);

    let master = state.master_credentials().await?;
    let store = master.to_store(false).await?;
    let is_app_scoped = request.app_id.is_some();

    // Only the upload and download URLs are signed here - head/delete are unused by bulk
    // uploads and would double the signing work for every file in the batch.
    let files = try_join_all(request.files.iter().map(|file| {
        let key = temporary_upload_key(
            &sub,
            request.app_id.as_deref(),
            file.extension.as_deref(),
            now_utc,
        );
        let content_type = resolve_content_type(&key, file.content_type.as_deref());
        sign_temporary_upload(
            &store,
            key,
            content_type,
            is_app_scoped,
            download_ttl,
            now_utc,
            false,
        )
    }))
    .await?;

    Ok(Json(TemporaryFileBatchResponse { files }))
}

fn temporary_upload_key(
    sub: &str,
    app_id: Option<&str>,
    extension: Option<&str>,
    now_utc: DateTime<Utc>,
) -> String {
    let id = create_id();
    let ext = sanitize_ext(extension).unwrap_or_else(|| "bin".to_string());
    let date_prefix = now_utc.format("%Y/%m/%d").to_string();
    let file_name = format!("{id}.{ext}");
    let sub = sanitize_path_segment(sub, "user");

    match app_id {
        Some(app_id) => {
            let app_id = sanitize_path_segment(app_id, "app");
            format!("tmp/user/{sub}/apps/{app_id}/{date_prefix}/{file_name}")
        }
        None => format!("tmp/user/{sub}/{date_prefix}/{file_name}"),
    }
}

fn resolve_content_type(key: &str, requested: Option<&str>) -> Mime {
    let extension = key.rsplit('.').next().unwrap_or_default();
    requested
        .and_then(|value| value.parse::<Mime>().ok())
        .or_else(|| mime_guess::from_ext(extension).first())
        .unwrap_or(mime::APPLICATION_OCTET_STREAM)
}

async fn sign_temporary_upload(
    store: &FlowLikeStore,
    key: String,
    content_type: Mime,
    app_scoped: bool,
    download_ttl: u64,
    now_utc: DateTime<Utc>,
    include_management_urls: bool,
) -> Result<TemporaryFileResponse, ApiError> {
    let path = FLPath::from(key.clone());
    let upload_ttl = UPLOAD_TTL_SECS;

    let (download_url, upload_url) = try_join!(
        store.sign("GET", &path, Duration::from_secs(download_ttl)),
        store.sign("PUT", &path, Duration::from_secs(upload_ttl)),
    )?;

    let (head_url, delete_url) = if include_management_urls {
        let (head_url, delete_url) = try_join!(
            store.sign("HEAD", &path, Duration::from_secs(60 * 60)),
            store.sign("DELETE", &path, Duration::from_secs(60 * 60)),
        )?;
        (Some(head_url.to_string()), Some(delete_url.to_string()))
    } else {
        (None, None)
    };

    let download_expires_at = (now_utc + ChronoDuration::seconds(download_ttl as i64)).to_rfc3339();
    let upload_expires_at = (now_utc + ChronoDuration::seconds(upload_ttl as i64)).to_rfc3339();

    Ok(TemporaryFileResponse {
        flow_path: app_scoped.then(|| TemporaryFlowPath {
            path: key.clone(),
            store_ref: REQUEST_FILES_STORE_REF.to_string(),
            cache_store_ref: None,
        }),
        key,
        content_type: content_type.to_string(),
        upload_url: upload_url.to_string(),
        upload_expires_at,
        download_url: download_url.to_string(),
        download_expires_at,
        head_url,
        delete_url,
        size_limit_bytes: DEFAULT_SIZE_LIMIT_BYTES,
    })
}

/// Extension carried by an original filename, if any. Only the part after the
/// last dot of the last path segment counts, and a leading-dot name such as
/// `.env` has no extension at all.
fn filename_extension(filename: Option<&str>) -> Option<&str> {
    let name = filename?.trim().rsplit(['/', '\\']).next()?;
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty()).then_some(extension)
}

fn sanitize_ext(input: Option<&str>) -> Option<String> {
    let mut s = input?.trim().trim_start_matches('.').to_ascii_lowercase();
    if s.is_empty() || s.len() > 16 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(std::mem::take(&mut s))
}

fn sanitize_path_segment(input: &str, fallback: &str) -> String {
    crate::credentials::storage_path_segment(input, fallback)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filename_extension_reads_the_last_segment() {
        assert_eq!(filename_extension(Some("report.pdf")), Some("pdf"));
        assert_eq!(filename_extension(Some("  report.tar.gz  ")), Some("gz"));
        assert_eq!(filename_extension(Some("a/b/c/report.PNG")), Some("PNG"));
        assert_eq!(
            filename_extension(Some(r"C:\dir\report.docx")),
            Some("docx")
        );
    }

    #[test]
    fn filename_extension_rejects_names_without_one() {
        assert_eq!(filename_extension(None), None);
        assert_eq!(filename_extension(Some("")), None);
        assert_eq!(filename_extension(Some("README")), None);
        assert_eq!(filename_extension(Some(".env")), None);
        assert_eq!(filename_extension(Some("../../etc/passwd")), None);
    }

    #[test]
    fn hostile_filenames_still_sanitize_to_a_safe_extension() {
        // Whatever the filename carries goes through `sanitize_ext` when the key
        // is minted, so path separators and spaces can never reach the key.
        assert_eq!(
            sanitize_ext(filename_extension(Some("x.PDF"))).as_deref(),
            Some("pdf")
        );
        assert_eq!(sanitize_ext(filename_extension(Some("x.p df"))), None);
        assert_eq!(sanitize_ext(filename_extension(Some("x.php%00"))), None);
        assert_eq!(
            sanitize_ext(filename_extension(Some("x.averyverylongextension"))),
            None
        );
    }

    #[test]
    fn content_type_follows_the_key_extension() {
        assert_eq!(
            resolve_content_type("tmp/user/u/2026/08/20/id.pdf", None).to_string(),
            "application/pdf"
        );
        assert_eq!(
            resolve_content_type("tmp/user/u/2026/08/20/id.bin", None).to_string(),
            "application/octet-stream"
        );
    }
}
