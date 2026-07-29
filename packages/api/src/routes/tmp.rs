use crate::error::ApiError;
use crate::state::AppState;
use crate::{auth::AppUser, ensure_permission, permission::role_permission::RolePermissions};
use axum::{
    Extension, Json, Router,
    extract::{Query, State},
    routing::get,
};
use chrono::{Duration as ChronoDuration, Utc};
use flow_like_storage::Path as FLPath;
use flow_like_types::dispatch::REQUEST_FILES_STORE_REF;
use flow_like_types::tokio::try_join;
use flow_like_types::{
    create_id,
    mime_guess::{self, mime},
};
use mime::Mime;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

const MAX_DOWNLOAD_TTL_SECS: u64 = 60 * 60 * 24 * 31;
const DEFAULT_DOWNLOAD_TTL_SECS: u64 = 60 * 60 * 24 * 7;
const UPLOAD_TTL_SECS: u64 = 60 * 15;
// Optional soft client hint (not enforced by PUT presign; enforce on POST policies or server finalize step)
const DEFAULT_SIZE_LIMIT_BYTES: Option<u64> = Some(1024 * 1024 * 35); // 35 MB

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
    pub head_url: String,
    pub delete_url: String,
    pub size_limit_bytes: Option<u64>,
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
    /// Optional original filename. Appended as a query param on the download URL so consumers can recover it.
    pub filename: Option<String>,
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(get_temporary_upload))
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

    let id = create_id();
    let ext = sanitize_ext(params.extension.as_deref()).unwrap_or_else(|| "bin".to_string());

    let now_utc = Utc::now();
    let date_prefix = now_utc.format("%Y/%m/%d").to_string();
    let file_name = format!("{id}.{ext}");
    let key = if let Some(app_id) = params.app_id.as_deref() {
        ensure_permission!(user, app_id, &state, RolePermissions::ExecuteEvents);
        let app_id = sanitize_path_segment(app_id, "app");
        let sub = sanitize_path_segment(&sub, "user");
        format!("tmp/user/{sub}/apps/{app_id}/{date_prefix}/{file_name}")
    } else {
        let sub = sanitize_path_segment(&sub, "user");
        format!("tmp/user/{sub}/{date_prefix}/{file_name}")
    };

    let content_type: Mime = params
        .content_type
        .as_deref()
        .and_then(|s| s.parse::<Mime>().ok())
        .or_else(|| mime_guess::from_ext(&ext).first())
        .unwrap_or(mime::APPLICATION_OCTET_STREAM);

    let download_ttl = params
        .download_ttl_secs
        .unwrap_or(DEFAULT_DOWNLOAD_TTL_SECS)
        .min(MAX_DOWNLOAD_TTL_SECS);
    let upload_ttl = UPLOAD_TTL_SECS;

    let master = state.master_credentials().await?;
    let store = master.to_store(false).await?;
    let path = FLPath::from(key.clone());

    let (download_url, upload_url) = try_join!(
        store.sign("GET", &path, Duration::from_secs(download_ttl)),
        store.sign("PUT", &path, Duration::from_secs(upload_ttl)),
    )?;
    let (head_url, delete_url) = try_join!(
        store.sign("HEAD", &path, Duration::from_secs(60 * 60)),
        store.sign("DELETE", &path, Duration::from_secs(60 * 60)),
    )?;

    let download_expires_at = (now_utc + ChronoDuration::seconds(download_ttl as i64)).to_rfc3339();
    let upload_expires_at = (now_utc + ChronoDuration::seconds(upload_ttl as i64)).to_rfc3339();

    let response = TemporaryFileResponse {
        flow_path: params.app_id.as_ref().map(|_| TemporaryFlowPath {
            path: key.clone(),
            store_ref: REQUEST_FILES_STORE_REF.to_string(),
            cache_store_ref: None,
        }),
        key,
        content_type: content_type.to_string(),
        upload_url: upload_url.to_string(),
        upload_expires_at,
        download_url: download_url.to_string(),
        download_expires_at: download_expires_at.clone(),
        head_url: head_url.to_string(),
        delete_url: delete_url.to_string(),
        size_limit_bytes: DEFAULT_SIZE_LIMIT_BYTES,
    };

    Ok(Json(response))
}

fn sanitize_ext(input: Option<&str>) -> Option<String> {
    let mut s = input?.trim().trim_start_matches('.').to_ascii_lowercase();
    if s.is_empty() || s.len() > 16 || !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(std::mem::take(&mut s))
}

fn sanitize_path_segment(input: &str, fallback: &str) -> String {
    let mut s = String::with_capacity(input.len().min(80));
    for ch in input.chars().take(80) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            s.push(ch);
        } else {
            s.push('_');
        }
    }

    let s = s.trim_matches(|ch| ch == '.' || ch == '_');
    if s.is_empty() {
        fallback.to_string()
    } else {
        s.to_string()
    }
}
