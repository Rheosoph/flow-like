//! Apply an offline-bundle fork to the local store.
//!
//! Server side: `POST /apps/{src}/fork/offline/begin` returns:
//!   - `meta_blobs`: remapped + secret-stripped manifest, boards,
//!     events, widgets, templates, pages (base64-encoded compressed
//!     bytes)
//!   - `source_content_prefix` + `shared_credentials`: scoped read
//!     access to the *source's* content store prefix (metadata/,
//!     upload/, storage/)
//!
//! This command:
//!   1. Decodes each meta blob and writes it to the desktop's local
//!      **meta** store at `apps/{app_id}/{relative_path}`.
//!   2. Builds an `object_store` client from the scoped credentials
//!      and lists the source content prefix; for each file, copies
//!      to the desktop's local **content** store, translating any
//!      `metadata/{widgets|templates|pages}/{src_id}/...` segment
//!      via `id_map`.
//!
//! The desktop's destination app id is **distinct** from the
//! server-generated `new_app_id` returned by the begin endpoint; the
//! caller passes whichever they want here. Per-file failures are
//! non-fatal — collected into the response so the UI can show
//! "12 files copied, 1 failed".

use std::collections::HashMap;
use std::sync::Arc;

use flow_like::credentials::SharedCredentials;
use flow_like::flow_like_storage::{Path, object_store::ObjectStore};
use flow_like_types::anyhow;
use flow_like_types::base64::{self, Engine as _};
use futures::TryStreamExt;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::{functions::TauriFunctionError, state::TauriFlowLikeState};

#[derive(Clone, Debug, Deserialize)]
pub struct MetaBlobInput {
    pub relative_path: String,
    pub data_b64: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApplyForkBundleArgs {
    /// Local destination app id. The desktop typically reuses the
    /// server's `new_app_id` so paths line up across the boundary
    /// (saves a remap pass).
    pub app_id: String,
    /// Remapped + stripped meta artifacts shipped in the begin
    /// response body. Each blob's `relative_path` is rooted at
    /// `apps/{app_id}/`.
    #[serde(default)]
    pub meta_blobs: Vec<MetaBlobInput>,
    /// Bucket-relative source content prefix (e.g.
    /// `apps/{src_app_id}`). Paired with `credentials`.
    pub source_content_prefix: String,
    /// Scoped read credentials for `source_content_prefix`.
    pub credentials: Arc<SharedCredentials>,
    /// `id_map.widgets` / `templates` / `pages` from the begin
    /// response. Used to translate `metadata/{kind}/{src_id}/...`
    /// segments to their destination ids when writing locally.
    /// Other meta paths (upload/, storage/, metadata/{lang}.meta)
    /// are identity-copied.
    #[serde(default)]
    pub widget_id_map: HashMap<String, String>,
    #[serde(default)]
    pub template_id_map: HashMap<String, String>,
    #[serde(default)]
    pub page_id_map: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ApplyForkBundleResponse {
    pub app_id: String,
    pub meta_blobs_written: u64,
    pub content_objects_copied: u64,
    pub content_bytes_copied: u64,
    pub failures: Vec<BundleFileFailure>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BundleFileFailure {
    /// Either `meta:{relative_path}` or `content:{relative_path}` so
    /// the UI can group failures by transport.
    pub kind_and_path: String,
    pub reason: String,
}

#[tauri::command(async)]
pub async fn apply_fork_bundle(
    app_handle: AppHandle,
    args: ApplyForkBundleArgs,
) -> Result<ApplyForkBundleResponse, TauriFunctionError> {
    if args.app_id.is_empty() {
        return Err(TauriFunctionError::new("app_id is empty"));
    }
    if args.source_content_prefix.is_empty() {
        return Err(TauriFunctionError::new("source_content_prefix is empty"));
    }

    let dst_meta_store = TauriFlowLikeState::get_project_meta_store(&app_handle).await?;
    let dst_content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let dst_app_prefix = Path::from("apps").child(args.app_id.clone());

    let mut meta_blobs_written: u64 = 0;
    let mut content_objects_copied: u64 = 0;
    let mut content_bytes_copied: u64 = 0;
    let mut failures: Vec<BundleFileFailure> = Vec::new();

    // ---- Stage 1: META blobs (decode body → local meta store) -----
    for blob in args.meta_blobs {
        let dst_path = blob
            .relative_path
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(dst_app_prefix.clone(), |acc, seg| acc.child(seg));
        match base64::engine::general_purpose::STANDARD.decode(&blob.data_b64) {
            Ok(bytes) => {
                let payload: Vec<u8> = bytes;
                if let Err(e) = dst_meta_store.put(&dst_path, payload.into()).await {
                    failures.push(BundleFileFailure {
                        kind_and_path: format!("meta:{}", blob.relative_path),
                        reason: format!("local put: {e}"),
                    });
                    continue;
                }
                meta_blobs_written = meta_blobs_written.saturating_add(1);
            }
            Err(e) => {
                failures.push(BundleFileFailure {
                    kind_and_path: format!("meta:{}", blob.relative_path),
                    reason: format!("base64 decode: {e}"),
                });
            }
        }
    }

    // ---- Stage 2: CONTENT (signed src prefix → local content) -----
    let src_content_store = args
        .credentials
        .to_store(false)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("build src content store: {e}")))?
        .as_generic();
    let src_prefix = parse_storage_path(&args.source_content_prefix);
    let src_prefix_str = src_prefix.as_ref().to_string();

    let mut listing = src_content_store.list(Some(&src_prefix));
    while let Some(item) = listing
        .try_next()
        .await
        .map_err(|e| TauriFunctionError::new(&format!("list source content: {e}")))?
    {
        let path_str = item.location.as_ref().to_string();
        let relative_path = path_str
            .strip_prefix(src_prefix_str.as_str())
            .map(|s| s.trim_start_matches('/').to_string())
            .unwrap_or_else(|| path_str.clone());

        let translated = translate_content_path(
            &relative_path,
            &args.widget_id_map,
            &args.template_id_map,
            &args.page_id_map,
        );
        let dst_path = translated
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(dst_app_prefix.clone(), |acc, seg| acc.child(seg));

        match copy_one(
            &src_content_store,
            &dst_content_store,
            &item.location,
            &dst_path,
        )
        .await
        {
            Ok(bytes) => {
                content_objects_copied = content_objects_copied.saturating_add(1);
                content_bytes_copied = content_bytes_copied.saturating_add(bytes);
            }
            Err(e) => {
                failures.push(BundleFileFailure {
                    kind_and_path: format!("content:{}", relative_path),
                    reason: e.to_string(),
                });
            }
        }
    }

    Ok(ApplyForkBundleResponse {
        app_id: args.app_id,
        meta_blobs_written,
        content_objects_copied,
        content_bytes_copied,
        failures,
    })
}

/// Translate a content-store relative path. Currently the only
/// segments that need translation are
/// `metadata/{widgets|templates|pages}/{src_id}/...` — everything
/// else (`metadata/{lang}.meta`, `upload/...`, `storage/...`) is
/// identity-copied.
fn translate_content_path(
    relative: &str,
    widget_map: &HashMap<String, String>,
    template_map: &HashMap<String, String>,
    page_map: &HashMap<String, String>,
) -> String {
    let segments: Vec<&str> = relative.split('/').collect();
    if segments.len() < 3 || segments[0] != "metadata" {
        return relative.to_string();
    }
    let map = match segments[1] {
        "widgets" => widget_map,
        "templates" => template_map,
        "pages" => page_map,
        _ => return relative.to_string(),
    };
    let dst_id = map
        .get(segments[2])
        .map(String::as_str)
        .unwrap_or(segments[2]);
    let mut out = format!("metadata/{}/{}", segments[1], dst_id);
    for seg in &segments[3..] {
        out.push('/');
        out.push_str(seg);
    }
    out
}

async fn copy_one(
    src_store: &Arc<dyn ObjectStore>,
    dst_store: &Arc<dyn ObjectStore>,
    src_path: &Path,
    dst_path: &Path,
) -> flow_like_types::Result<u64> {
    let bytes = src_store
        .get(src_path)
        .await
        .map_err(|e| anyhow!("get: {e}"))?
        .bytes()
        .await
        .map_err(|e| anyhow!("read body: {e}"))?;
    let len = bytes.len() as u64;
    dst_store
        .put(dst_path, bytes.into())
        .await
        .map_err(|e| anyhow!("local put: {e}"))?;
    Ok(len)
}

fn parse_storage_path(raw: &str) -> Path {
    raw.split('/')
        .filter(|s| !s.is_empty())
        .fold(Path::default(), |acc, seg| acc.child(seg))
}
