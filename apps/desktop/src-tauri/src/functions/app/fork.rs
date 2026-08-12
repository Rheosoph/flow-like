//! Apply an offline-bundle fork to the local store.
//!
//! Server side: `POST /apps/{src}/fork/offline/begin` returns:
//!   - `meta_blobs`: remapped + secret-stripped manifest, boards,
//!     events, widgets, templates, pages, plus translated DB-backed
//!     metadata files (base64-encoded compressed bytes) and `media/...`
//!     raw bytes
//!   - `source_content_prefix` + `shared_credentials`: scoped read
//!     access to the *source's* content store prefix (metadata/,
//!     upload/, storage/)
//!
//! This command:
//!   1. Decodes each inline blob. Meta artifacts are written to the
//!      desktop's local **meta** store; `metadata/...` blobs are
//!      written to the local **content** store because local metadata
//!      readers resolve there. `media/...` blobs are also content.
//!   2. Builds an `object_store` client from the scoped credentials
//!      and lists the source content prefix; for each file, copies
//!      to the desktop's local **content** store, translating any
//!      `metadata/{widgets|templates|pages}/{src_id}/...` segment
//!      via `id_map`. Paths under `content_exclude_prefixes` — the
//!      owner's fork policy — are skipped in both stages.
//!   3. Recreates `db_table_schemas` as empty tables in the local
//!      project database, for schema-only database forks.
//!
//! The desktop's destination app id is **distinct** from the
//! server-generated `new_app_id` returned by the begin endpoint; the
//! caller passes whichever they want here. Per-file failures are
//! non-fatal — collected into the response so the UI can show
//! "12 files copied, 1 failed".

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use flow_like::credentials::SharedCredentials;
use flow_like::flow_like_storage::{
    Path,
    arrow_schema::Schema,
    databases::vector::lancedb::LanceDBVectorStore,
    lancedb::{Connection, table::WriteOptions},
    object_store::ObjectStore,
};
use flow_like::profile::ProfileApp;
use flow_like_types::anyhow;
use flow_like_types::base64::{self, Engine as _};
use futures::TryStreamExt;
use serde::Deserialize;
use tauri::AppHandle;

use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

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
    /// Source-relative prefixes (rooted at `apps/{src_app_id}/`) the
    /// owner's fork policy excludes. Applied to both the inline blobs
    /// and the mirrored source listing.
    #[serde(default)]
    pub content_exclude_prefixes: Vec<String>,
    /// Arrow schemas of the source's user tables on a schema-only
    /// database fork. Their `.lance` directories are excluded from the
    /// mirror; each table is recreated empty locally instead.
    #[serde(default)]
    pub db_table_schemas: Vec<ForkTableSchemaInput>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ForkTableSchemaInput {
    pub table: String,
    /// serde-serialized `arrow_schema::Schema`. Decoded per table so one
    /// malformed schema doesn't fail the whole apply.
    pub schema: serde_json::Value,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ApplyForkBundleResponse {
    pub app_id: String,
    pub meta_blobs_written: u64,
    pub content_objects_copied: u64,
    pub content_bytes_copied: u64,
    pub content_objects_skipped: u64,
    pub db_tables_created: u64,
    pub failures: Vec<BundleFileFailure>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BundleFileFailure {
    /// Either `meta:{relative_path}` or `content:{relative_path}` so
    /// the UI can group failures by transport.
    pub kind_and_path: String,
    pub reason: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct LocalAppBundleSummary {
    pub total_size_bytes: u64,
    pub total_object_count: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UploadLocalAppContentArgs {
    pub source_app_id: String,
    pub destination_content_prefix: String,
    pub credentials: Arc<SharedCredentials>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct UploadLocalAppContentResponse {
    pub content_objects_copied: u64,
    pub content_bytes_copied: u64,
    pub failures: Vec<BundleFileFailure>,
}

/// Summarizes exactly what [`upload_local_app_content_bundle`] will
/// push, so the server's cap check at `/fork/online/begin` and its
/// re-measurement at finalize agree. Meta artifacts are excluded (they
/// travel through the app-edit endpoints), each relative path is counted
/// once even though the desktop's meta and content stores share a root,
/// and project-database objects are left out of the object count.
#[tauri::command(async)]
pub async fn summarize_local_app_bundle(
    app_handle: AppHandle,
    app_id: String,
) -> Result<LocalAppBundleSummary, TauriFunctionError> {
    if app_id.is_empty() {
        return Err(TauriFunctionError::new("app_id is empty"));
    }

    let prefix = Path::from("apps").child(app_id);
    let meta_store = TauriFlowLikeState::get_project_meta_store(&app_handle).await?;
    let content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;

    let mut seen: HashSet<String> = HashSet::new();
    let mut total_size_bytes: u64 = 0;
    let mut total_object_count: u64 = 0;

    for (label, store) in [("meta", &meta_store), ("content", &content_store)] {
        let entries = summarize_store_prefix(store, &prefix)
            .await
            .map_err(|e| TauriFunctionError::new(&format!("summarize {label} store: {e}")))?;
        for (relative_path, size) in entries {
            if !is_content_blob_path(&relative_path) {
                continue;
            }
            if !seen.insert(relative_path.clone()) {
                continue;
            }
            total_size_bytes = total_size_bytes.saturating_add(size);
            if is_project_db_path(&relative_path) {
                continue;
            }
            total_object_count = total_object_count.saturating_add(1);
        }
    }

    Ok(LocalAppBundleSummary {
        total_size_bytes,
        total_object_count,
    })
}

#[tauri::command(async)]
pub async fn upload_local_app_content_bundle(
    app_handle: AppHandle,
    args: UploadLocalAppContentArgs,
) -> Result<UploadLocalAppContentResponse, TauriFunctionError> {
    if args.source_app_id.is_empty() {
        return Err(TauriFunctionError::new("source_app_id is empty"));
    }
    if args.destination_content_prefix.is_empty() {
        return Err(TauriFunctionError::new(
            "destination_content_prefix is empty",
        ));
    }

    let src_content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let dst_content_store = args
        .credentials
        .to_store(false)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("build dst content store: {e}")))?
        .as_generic();

    let src_prefix = Path::from("apps").child(args.source_app_id);
    let src_prefix_str = src_prefix.as_ref().to_string();
    let dst_prefix = parse_storage_path(&args.destination_content_prefix);

    let mut content_objects_copied: u64 = 0;
    let mut content_bytes_copied: u64 = 0;
    let mut failures: Vec<BundleFileFailure> = Vec::new();
    let mut listing = src_content_store.list(Some(&src_prefix));

    loop {
        let item = match listing.try_next().await {
            Ok(Some(item)) => item,
            Ok(None) => break,
            Err(e) if is_missing_prefix_error(&e) => break,
            Err(e) => return Err(TauriFunctionError::new(&format!("list local content: {e}"))),
        };
        let path_str = item.location.as_ref().to_string();
        let Some(relative_path) = relative_to_prefix(&path_str, &src_prefix_str) else {
            continue;
        };
        if relative_path.is_empty() {
            continue;
        }
        // The desktop keeps meta and content under one root, so an
        // unfiltered walk would push `manifest.app`, `*.board`,
        // `events/**` and `versions/**` into the destination's content
        // bucket — where nothing reads them, while they still count
        // against the fork size cap and trip the server's
        // meta-in-content-store leak detector. Meta artifacts go up
        // through the app-edit endpoints instead.
        if !is_content_blob_path(&relative_path) {
            continue;
        }

        let dst_path = relative_path
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(dst_prefix.clone(), |acc, seg| acc.child(seg));

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

    Ok(UploadLocalAppContentResponse {
        content_objects_copied,
        content_bytes_copied,
        failures,
    })
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
    let mut content_objects_skipped: u64 = 0;
    let mut failures: Vec<BundleFileFailure> = Vec::new();
    let mut manifest_written = false;
    let mut inline_content_paths: HashSet<String> = HashSet::new();

    // ---- Stage 1: inline blobs (decode body → local stores) -----
    for blob in args.meta_blobs {
        let relative_path = blob.relative_path.trim_matches('/').to_string();
        // `manifest.app` is never excludable — the check below this loop
        // hard-errors without it, and no policy prefix can match it.
        if relative_path != "manifest.app"
            && is_excluded(&relative_path, &args.content_exclude_prefixes)
        {
            content_objects_skipped = content_objects_skipped.saturating_add(1);
            continue;
        }
        let dst_path = relative_path
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(dst_app_prefix.clone(), |acc, seg| acc.child(seg));
        let dst_store = if is_content_blob_path(&relative_path) {
            &dst_content_store
        } else {
            &dst_meta_store
        };
        match base64::engine::general_purpose::STANDARD.decode(&blob.data_b64) {
            Ok(bytes) => {
                let payload: Vec<u8> = bytes;
                if let Err(e) = dst_store.put(&dst_path, payload.into()).await {
                    failures.push(BundleFileFailure {
                        kind_and_path: format!("inline:{}", blob.relative_path),
                        reason: format!("local put: {e}"),
                    });
                    continue;
                }
                meta_blobs_written = meta_blobs_written.saturating_add(1);
                if is_content_blob_path(&relative_path) {
                    inline_content_paths.insert(relative_path.clone());
                }
                if relative_path == "manifest.app" {
                    manifest_written = true;
                }
            }
            Err(e) => {
                failures.push(BundleFileFailure {
                    kind_and_path: format!("inline:{}", blob.relative_path),
                    reason: format!("base64 decode: {e}"),
                });
            }
        }
    }
    if !manifest_written {
        return Err(TauriFunctionError::new(
            "fork bundle did not write a local manifest.app",
        ));
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
        let Some(relative_path) = relative_to_prefix(&path_str, &src_prefix_str) else {
            continue;
        };
        if relative_path.is_empty() {
            continue;
        }
        // Matched on the SOURCE-relative path: `translate_content_path`
        // rewrites `metadata/{widgets|templates|pages}/{id}` segments,
        // which the policy prefixes are stated against.
        if is_excluded(&relative_path, &args.content_exclude_prefixes) {
            content_objects_skipped = content_objects_skipped.saturating_add(1);
            continue;
        }

        let translated = translate_content_path(
            &relative_path,
            &args.widget_id_map,
            &args.template_id_map,
            &args.page_id_map,
        );
        if inline_content_paths.contains(&translated) {
            continue;
        }
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

    // ---- Stage 3: schema-only tables (empty local project DB) -----
    let db_tables_created = create_empty_project_tables(
        &app_handle,
        &args.app_id,
        &args.db_table_schemas,
        &mut failures,
    )
    .await;

    register_profile_app(&app_handle, &args.app_id).await?;

    Ok(ApplyForkBundleResponse {
        app_id: args.app_id,
        meta_blobs_written,
        content_objects_copied,
        content_bytes_copied,
        content_objects_skipped,
        db_tables_created,
        failures,
    })
}

/// Recreates the source's user tables as **empty** tables in the local
/// project database. A schema-only fork excludes every `{table}.lance/`
/// directory from the content mirror, so without this the destination has
/// the Data Studio configuration but no tables behind it. Per-table
/// failures are collected — a fork with one unusable schema still applies.
async fn create_empty_project_tables(
    app_handle: &AppHandle,
    app_id: &str,
    schemas: &[ForkTableSchemaInput],
    failures: &mut Vec<BundleFileFailure>,
) -> u64 {
    if schemas.is_empty() {
        return 0;
    }

    let (connection, write_options) = match open_local_project_db(app_handle, app_id).await {
        Ok(opened) => opened,
        Err(e) => {
            for entry in schemas {
                failures.push(BundleFileFailure {
                    kind_and_path: format!("db:{}", entry.table),
                    reason: format!("open local project db: {e}"),
                });
            }
            return 0;
        }
    };

    let mut created: u64 = 0;
    for entry in schemas {
        match create_one_empty_table(&connection, write_options.as_ref(), entry).await {
            Ok(()) => created = created.saturating_add(1),
            Err(e) => failures.push(BundleFileFailure {
                kind_and_path: format!("db:{}", entry.table),
                reason: e.to_string(),
            }),
        }
    }
    created
}

async fn open_local_project_db(
    app_handle: &AppHandle,
    app_id: &str,
) -> flow_like_types::Result<(Connection, Option<WriteOptions>)> {
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let config = state.config.read().await;
    let builder = config
        .callbacks
        .build_project_database
        .clone()
        .ok_or_else(|| anyhow!("No database builder found"))?;
    let write_options = config.callbacks.lance_write_options.clone();
    drop(config);

    let db_dir = Path::from("apps")
        .child(app_id)
        .child("storage")
        .child("db");
    let connection = builder(db_dir)
        .execute()
        .await
        .map_err(|e| anyhow!("connect: {e}"))?;
    Ok((connection, write_options))
}

async fn create_one_empty_table(
    connection: &Connection,
    write_options: Option<&WriteOptions>,
    entry: &ForkTableSchemaInput,
) -> flow_like_types::Result<()> {
    // LanceDB's listing backend panics on an invalid table name while
    // deriving the table URI — never hand it an unvalidated server value.
    LanceDBVectorStore::validate_table_name(&entry.table)?;
    let schema: Schema =
        serde_json::from_value(entry.schema.clone()).map_err(|e| anyhow!("decode schema: {e}"))?;
    let mut store =
        LanceDBVectorStore::from_connection(connection.clone(), entry.table.clone()).await;
    if let Some(opts) = write_options {
        store.set_write_options(opts.clone());
    }
    store
        .create_empty_table(schema, true)
        .await
        .map_err(|e| anyhow!("create empty table: {e}"))?;
    Ok(())
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
        .map_err(|e| anyhow!("put: {e}"))?;
    Ok(len)
}

/// Lists `(relative_path, size)` for every object below `prefix`.
async fn summarize_store_prefix(
    store: &Arc<dyn ObjectStore>,
    prefix: &Path,
) -> flow_like_types::Result<Vec<(String, u64)>> {
    let prefix_str = prefix.as_ref().to_string();
    let mut entries: Vec<(String, u64)> = Vec::new();
    let mut listing = store.list(Some(prefix));

    loop {
        let item = match listing.try_next().await {
            Ok(Some(item)) => item,
            Ok(None) => break,
            Err(e) if is_missing_prefix_error(&e) => break,
            Err(e) => return Err(anyhow!("list prefix: {e}")),
        };
        let path_str = item.location.as_ref().to_string();
        let Some(relative_path) = relative_to_prefix(&path_str, &prefix_str) else {
            continue;
        };
        if relative_path.is_empty() {
            continue;
        }
        entries.push((relative_path, item.size as u64));
    }

    Ok(entries)
}

async fn register_profile_app(
    app_handle: &AppHandle,
    app_id: &str,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(app_handle).await?;
    let mut settings = settings.lock().await;
    let profile_id = settings.get_current_profile()?.hub_profile.id.clone();
    let profile = settings
        .profiles
        .get_mut(&profile_id)
        .ok_or_else(|| TauriFunctionError::new("Profile not found"))?;
    let apps = profile.hub_profile.apps.get_or_insert_with(Vec::new);
    if !apps.iter().any(|app| app.app_id == app_id) {
        apps.push(ProfileApp::new(app_id.to_string()));
    }
    settings.serialize();
    Ok(())
}

fn is_content_blob_path(relative_path: &str) -> bool {
    relative_path == "metadata"
        || relative_path.starts_with("metadata/")
        || relative_path == "media"
        || relative_path.starts_with("media/")
        || relative_path == "upload"
        || relative_path.starts_with("upload/")
        || relative_path == "storage"
        || relative_path.starts_with("storage/")
}

/// The project LanceDB. Excluded from the fork's object count (never
/// from its byte total) because one table fans out into a file per
/// fragment, per index and per commit manifest — a database that is
/// small on disk still reaches five-digit object counts and would trip
/// the deployment's `forking.max_file_count` on every fork. Mirrors
/// `utils::fork::preview::project_db_prefix` on the server.
fn is_project_db_path(relative_path: &str) -> bool {
    relative_path.starts_with("storage/db/")
}

/// Owner fork policy, applied to a source-relative path. The server already
/// stripped what it ships inline; this keeps the mirrored source listing
/// honest. Not a security boundary — the forker holds `ReadFiles` on the
/// source either way.
fn is_excluded(relative_path: &str, excludes: &[String]) -> bool {
    excludes
        .iter()
        .any(|prefix| relative_path.starts_with(prefix.as_str()))
}

fn is_missing_prefix_error(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string();
    message.contains("not found") || message.contains("No such file")
}

fn parse_storage_path(raw: &str) -> Path {
    raw.split('/')
        .filter(|s| !s.is_empty())
        .fold(Path::default(), |acc, seg| acc.child(seg))
}

fn relative_to_prefix(path: &str, prefix: &str) -> Option<String> {
    let suffix = path.strip_prefix(prefix)?;
    if !suffix.is_empty() && !suffix.starts_with('/') {
        return None;
    }
    Some(suffix.trim_start_matches('/').to_string())
}
