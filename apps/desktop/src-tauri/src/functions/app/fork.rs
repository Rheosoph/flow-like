//! Fork files between this device and a hub, moved only through signed URLs.
//!
//! The hub never hands out a storage credential. It signs one request per
//! file, picks every object key, and keeps whatever a fork produces on the
//! way under `tmp/`, which expires on its own.
//!
//! Online → offline (`POST /apps/{src}/fork/offline/begin`):
//!   1. [`apply_fork_bundle`] downloads the re-keyed, secret-stripped meta
//!      artifacts (manifest, boards, events, widgets, templates, pages,
//!      DB-backed metadata, app media) into the local stores: meta
//!      artifacts into the **meta** store, `metadata/...` and `media/...`
//!      into the **content** store, where local readers resolve them. It
//!      then recreates `db_table_schemas` as empty tables for schema-only
//!      database forks.
//!   2. [`download_fork_files`] writes each page of content files from
//!      `GET /apps/{src}/fork/offline/files` into the content store. The hub
//!      already applied the owner's fork policy and the destination paths.
//!
//! Offline → online (`POST /apps/fork/online/begin`):
//!   [`list_local_app_content`] names the files to upload, the hub signs
//!   uploads into its staging area, and [`upload_local_app_files`] puts
//!   them. The hub moves them into the app and re-keys them.
//!
//! Per-file failures are non-fatal: they are collected into the response so
//! the UI can show "12 files copied, 1 failed".

use flow_like::flow_like_storage::object_store::ObjectStoreExt;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

use flow_like::flow_like_storage::{
    Path,
    arrow_schema::Schema,
    databases::vector::lancedb::LanceDBVectorStore,
    join_object_path,
    lancedb::{Connection, table::WriteOptions},
    normalize_object_path,
    object_store::ObjectStore,
};
use flow_like_types::anyhow;
use flow_like_types::storage_paths::is_legacy_training_ledger_path;
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use tauri::AppHandle;

use super::sharing::add_app_to_profile;
use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

const TRANSFER_CONCURRENCY: usize = 8;

/// One file and the signed request that moves it, as the hub hands it out.
#[derive(Clone, Debug, Deserialize)]
pub struct SignedFileInput {
    /// Path relative to the app root.
    pub path: String,
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ApplyForkBundleArgs {
    /// Local destination app id. The desktop typically reuses the
    /// server's `new_app_id` so paths line up across the boundary
    /// (saves a remap pass).
    pub app_id: String,
    /// Signed downloads of the re-keyed meta artifacts, each rooted at
    /// `apps/{app_id}/`.
    #[serde(default)]
    pub meta_files: Vec<SignedFileInput>,
    /// Arrow schemas of the source's user tables on a schema-only
    /// database fork. Their `.lance` directories never arrive; each table
    /// is recreated empty locally instead.
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
    pub meta_files_written: u64,
    pub db_tables_created: u64,
    pub failures: Vec<BundleFileFailure>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DownloadForkFilesArgs {
    pub app_id: String,
    pub files: Vec<SignedFileInput>,
    /// Paths a meta artifact already wrote; the staged DB metadata wins
    /// over a content file with the same path.
    #[serde(default)]
    pub skip_paths: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UploadLocalAppFilesArgs {
    pub source_app_id: String,
    pub files: Vec<SignedFileInput>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ForkFilesResponse {
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub failures: Vec<BundleFileFailure>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BundleFileFailure {
    /// `meta:{path}`, `content:{path}` or `db:{table}` so the UI can group
    /// failures by what failed.
    pub kind_and_path: String,
    pub reason: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct LocalAppBundleSummary {
    pub total_size_bytes: u64,
    pub total_object_count: u64,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct LocalContentFile {
    pub path: String,
    pub size: u64,
}

/// Summarizes exactly what an upload pushes, so the server's cap check at
/// `/fork/online/begin` and its re-measurement at finalize agree. Meta
/// artifacts are excluded (they travel through the app-edit endpoints), as
/// is the device-local training ledger; each relative path is counted once
/// even though the desktop's meta and content stores share a root, and
/// project-database objects are left out of the object count.
#[tauri::command(async)]
pub async fn summarize_local_app_bundle(
    app_handle: AppHandle,
    app_id: String,
) -> Result<LocalAppBundleSummary, TauriFunctionError> {
    if app_id.is_empty() {
        return Err(TauriFunctionError::new("app_id is empty"));
    }

    let prefix = Path::from("apps").join(app_id);
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
            if !is_uploaded_content_path(&relative_path) {
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

/// The content files an upload of `app_id` pushes. The desktop keeps meta and
/// content under one root, so meta artifacts (`manifest.app`, `*.board`,
/// `events/**`, `versions/**`) are left out: they go up through the app-edit
/// endpoints.
#[tauri::command(async)]
pub async fn list_local_app_content(
    app_handle: AppHandle,
    app_id: String,
) -> Result<Vec<LocalContentFile>, TauriFunctionError> {
    if app_id.is_empty() {
        return Err(TauriFunctionError::new("app_id is empty"));
    }
    let content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let entries = summarize_store_prefix(&content_store, &Path::from("apps").join(app_id))
        .await
        .map_err(|e| TauriFunctionError::new(&format!("list local content: {e}")))?;
    Ok(entries
        .into_iter()
        .filter(|(path, _)| is_uploaded_content_path(path))
        .map(|(path, size)| LocalContentFile { path, size })
        .collect())
}

/// Put local content files through the hub's signed upload URLs.
#[tauri::command(async)]
pub async fn upload_local_app_files(
    app_handle: AppHandle,
    args: UploadLocalAppFilesArgs,
) -> Result<ForkFilesResponse, TauriFunctionError> {
    if args.source_app_id.is_empty() {
        return Err(TauriFunctionError::new("source_app_id is empty"));
    }
    let content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let app_root = Path::from("apps").join(args.source_app_id);
    let client = transfer_client()?;

    let outcomes: Vec<(String, flow_like_types::Result<u64>)> = futures::stream::iter(args.files)
        .map(|file| {
            let content_store = content_store.clone();
            let app_root = app_root.clone();
            let client = client.clone();
            async move {
                let outcome = upload_from(&client, &content_store, &app_root, &file).await;
                (file.path, outcome)
            }
        })
        .buffer_unordered(TRANSFER_CONCURRENCY)
        .collect()
        .await;
    Ok(tally("content", outcomes))
}

/// Read `file` from under `root` and PUT it.
async fn upload_from(
    client: &reqwest::Client,
    store: &Arc<dyn ObjectStore>,
    root: &Path,
    file: &SignedFileInput,
) -> flow_like_types::Result<u64> {
    if !is_uploaded_content_path(&file.path) || !is_plain_relative(&file.path) {
        return Err(anyhow!("not a content file of this app"));
    }
    let bytes = store
        .get(&join_object_path(root, &file.path))
        .await
        .map_err(|e| anyhow!("read local file: {e}"))?
        .bytes()
        .await
        .map_err(|e| anyhow!("read local file: {e}"))?;
    let size = bytes.len() as u64;
    send_signed(client, reqwest::Method::PUT, file, Some(bytes.to_vec())).await?;
    Ok(size)
}

/// Write the re-keyed meta artifacts of an offline fork into the local
/// stores, recreate schema-only tables and list the app in the profile.
#[tauri::command(async)]
pub async fn apply_fork_bundle(
    app_handle: AppHandle,
    args: ApplyForkBundleArgs,
) -> Result<ApplyForkBundleResponse, TauriFunctionError> {
    if args.app_id.is_empty() {
        return Err(TauriFunctionError::new("app_id is empty"));
    }

    let profile_id = TauriSettingsState::current_profile(&app_handle)
        .await?
        .hub_profile
        .id;
    let dst_meta_store = TauriFlowLikeState::get_project_meta_store(&app_handle).await?;
    let dst_content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let dst_app_prefix = Path::from("apps").join(args.app_id.clone());
    let client = transfer_client()?;

    let manifest_listed = args
        .meta_files
        .iter()
        .any(|file| file.path.trim_matches('/') == "manifest.app");
    let outcomes: Vec<(String, flow_like_types::Result<u64>)> =
        futures::stream::iter(args.meta_files)
            .map(|file| {
                let store = if is_content_blob_path(file.path.trim_matches('/')) {
                    dst_content_store.clone()
                } else {
                    dst_meta_store.clone()
                };
                let dst_app_prefix = dst_app_prefix.clone();
                let client = client.clone();
                async move {
                    let outcome = download_into(&client, &store, &dst_app_prefix, &file).await;
                    (file.path, outcome)
                }
            })
            .buffer_unordered(TRANSFER_CONCURRENCY)
            .collect()
            .await;
    let manifest_written = outcomes
        .iter()
        .any(|(path, outcome)| path.trim_matches('/') == "manifest.app" && outcome.is_ok());
    let written = tally("meta", outcomes);
    if !manifest_listed || !manifest_written {
        return Err(TauriFunctionError::new(
            "fork bundle did not write a local manifest.app",
        ));
    }

    let mut failures = written.failures;
    let db_tables_created = create_empty_project_tables(
        &app_handle,
        &args.app_id,
        &args.db_table_schemas,
        &mut failures,
    )
    .await;

    add_app_to_profile(&app_handle, &profile_id, &args.app_id).await?;

    Ok(ApplyForkBundleResponse {
        app_id: args.app_id,
        meta_files_written: written.files_copied,
        db_tables_created,
        failures,
    })
}

/// Write one page of an offline fork's content files into the local content
/// store.
#[tauri::command(async)]
pub async fn download_fork_files(
    app_handle: AppHandle,
    args: DownloadForkFilesArgs,
) -> Result<ForkFilesResponse, TauriFunctionError> {
    if args.app_id.is_empty() {
        return Err(TauriFunctionError::new("app_id is empty"));
    }
    let content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let app_root = Path::from("apps").join(args.app_id);
    let skip: HashSet<String> = args
        .skip_paths
        .iter()
        .map(|path| path.trim_matches('/').to_string())
        .collect();
    let client = transfer_client()?;

    let outcomes: Vec<(String, flow_like_types::Result<u64>)> = futures::stream::iter(
        args.files
            .into_iter()
            .filter(|file| !skip.contains(file.path.trim_matches('/'))),
    )
    .map(|file| {
        let content_store = content_store.clone();
        let app_root = app_root.clone();
        let client = client.clone();
        async move {
            let outcome = download_into(&client, &content_store, &app_root, &file).await;
            (file.path, outcome)
        }
    })
    .buffer_unordered(TRANSFER_CONCURRENCY)
    .collect()
    .await;
    Ok(tally("content", outcomes))
}

fn transfer_client() -> Result<reqwest::Client, TauriFunctionError> {
    reqwest::Client::builder()
        .build()
        .map_err(|e| TauriFunctionError::new(&format!("build transfer client: {e}")))
}

/// GET `file` and write it under `root`.
async fn download_into(
    client: &reqwest::Client,
    store: &Arc<dyn ObjectStore>,
    root: &Path,
    file: &SignedFileInput,
) -> flow_like_types::Result<u64> {
    let path = file.path.trim_matches('/');
    if !is_plain_relative(path) || is_device_local_path(path) {
        return Err(anyhow!("refused a path outside the app"));
    }
    let bytes = send_signed(client, reqwest::Method::GET, file, None).await?;
    let size = bytes.len() as u64;
    store
        .put(&join_object_path(root, path), bytes.into())
        .await
        .map_err(|e| anyhow!("write local file: {e}"))?;
    Ok(size)
}

/// Send the one request `file` was signed for and return the response body.
async fn send_signed(
    client: &reqwest::Client,
    method: reqwest::Method,
    file: &SignedFileInput,
    body: Option<Vec<u8>>,
) -> flow_like_types::Result<Vec<u8>> {
    let mut request = client.request(method.clone(), &file.url);
    for (name, value) in &file.headers {
        request = request.header(name, value);
    }
    if let Some(body) = body {
        request = request.body(body);
    }
    let response = request
        .send()
        .await
        .map_err(|e| anyhow!("{method} request: {e}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(anyhow!("{method} answered HTTP {status}"));
    }
    Ok(response
        .bytes()
        .await
        .map_err(|e| anyhow!("read {method} response: {e}"))?
        .to_vec())
}

fn tally(kind: &str, outcomes: Vec<(String, flow_like_types::Result<u64>)>) -> ForkFilesResponse {
    let mut response = ForkFilesResponse {
        files_copied: 0,
        bytes_copied: 0,
        failures: Vec::new(),
    };
    for (path, outcome) in outcomes {
        match outcome {
            Ok(bytes) => {
                response.files_copied = response.files_copied.saturating_add(1);
                response.bytes_copied = response.bytes_copied.saturating_add(bytes);
            }
            Err(e) => response.failures.push(BundleFileFailure {
                kind_and_path: format!("{kind}:{path}"),
                reason: e.to_string(),
            }),
        }
    }
    response
}

/// A path that stays inside the app root it is joined to.
fn is_plain_relative(path: &str) -> bool {
    !path.is_empty()
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Recreates the source's user tables as **empty** tables in the local
/// project database. A schema-only fork never ships a `{table}.lance/`
/// directory, so without this the destination has the Data Studio
/// configuration but no tables behind it. Per-table failures are collected —
/// a fork with one unusable schema still applies.
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

    let db_dir = Path::from("apps").join(app_id).join("storage").join("db");
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

/// Lists `(relative_path, size)` for every object below `prefix`.
pub(crate) async fn summarize_store_prefix(
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

/// What an offline → online fork uploads. Shared by the summary and the
/// upload so the server's cap check measures exactly what arrives.
pub(crate) fn is_uploaded_content_path(relative_path: &str) -> bool {
    is_content_blob_path(relative_path) && !is_device_local_path(relative_path)
}

/// The legacy training ledger under `storage/inspection/`. Training state
/// never leaves this device implicitly, and never lands on it from a fork.
fn is_device_local_path(relative_path: &str) -> bool {
    is_legacy_training_ledger_path(relative_path)
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

fn is_missing_prefix_error(error: &impl std::fmt::Display) -> bool {
    let message = error.to_string();
    message.contains("not found") || message.contains("No such file")
}

/// The content prefix the API handed us is already an object-store key, so it
/// is normalized rather than encoded a second time.
pub(crate) fn parse_storage_path(raw: &str) -> Path {
    normalize_object_path(raw)
}

fn relative_to_prefix(path: &str, prefix: &str) -> Option<String> {
    let suffix = path.strip_prefix(prefix)?;
    if !suffix.is_empty() && !suffix.starts_with('/') {
        return None;
    }
    Some(suffix.trim_start_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_legacy_training_ledger_is_device_local() {
        for path in [
            "storage/inspection/jobs.sqlite",
            "storage/inspection/jobs.sqlite-wal",
            "storage/inspection/jobs.artifacts/x.blob",
            "storage/inspection/.training-locks/a.lock",
        ] {
            assert!(is_device_local_path(path), "{path}");
            assert!(!is_uploaded_content_path(path), "{path}");
        }
        for path in [
            "storage/inspection/pki/cert.der",
            "storage/models/artifacts/a/weights.bin",
            "storage/db/t.lance/data/a.lance",
            "upload/jobs.sqlite",
        ] {
            assert!(!is_device_local_path(path), "{path}");
            assert!(is_uploaded_content_path(path), "{path}");
        }
        assert!(!is_uploaded_content_path("manifest.app"));
    }

    #[test]
    fn hub_paths_never_leave_the_app_root() {
        for path in ["manifest.app", "upload/a b/c.csv", "storage/db/t.lance/x"] {
            assert!(is_plain_relative(path), "{path}");
        }
        for path in [
            "",
            "../other/manifest.app",
            "upload/../../x",
            "upload//x",
            "./x",
        ] {
            assert!(!is_plain_relative(path), "{path}");
        }
    }
}
