use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::event_sink::http::{
    flow_path_value, sanitize_request_file_name, sanitize_store_path_segment,
};
use crate::functions::TauriFunctionError;
use crate::state::TauriFlowLikeState;
use flow_like::flow_like_storage::Path as StorePath;
use flow_like::flow_like_storage::files::store::FlowLikeStore;
use flow_like::flow_like_storage::object_store::ObjectStore;
use flow_like::flow_like_storage::object_store::buffered::BufWriter;
use flow_like::utils::hash::hash_file;
use flow_like_types::tokio::io::{AsyncWriteExt, BufReader};
use flow_like_types::{Value, create_id, tokio};
use futures::{StreamExt, TryStreamExt};
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// Where the frontend writes a picked form file before `stage_form_file` takes it.
const FORM_UPLOADS_DIR: &str = "form-uploads";
/// A form file goes into the store in parts of this size, so memory stays bounded whatever
/// the file's size; a smaller file goes up in a single put.
const FORM_FILE_PART_BYTES: usize = 8 * 1024 * 1024;
const FORM_FILE_PARTS_IN_FLIGHT: usize = 2;
const FORM_FILE_READ_BYTES: usize = 1024 * 1024;
/// Form picks and HTTP sink uploads live below
/// `tmp/global/apps/<app>/events/<event>/requests/`.
const REQUEST_FILES_PREFIX: &str = "tmp/global/apps";
const REQUEST_FILES_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);
const REQUEST_FILES_SWEEP_DELAY: Duration = Duration::from_secs(20);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedFormFile {
    flow_path: Value,
}

#[tauri::command(async)]
pub async fn post_process_local_file(file: String) -> Result<String, TauriFunctionError> {
    let path = PathBuf::from(file);
    let hash = hash_file(&path);
    if hash.is_empty() {
        return Err(TauriFunctionError::new(
            "File does not exist or is not a file",
        ));
    }

    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{}", ext))
        .unwrap_or_default();

    let new_file_name = path.with_file_name(format!("{}{}", hash, extension));
    std::fs::rename(path, new_file_name.clone())
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    Ok(new_file_name
        .canonicalize()
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?
        .to_string_lossy()
        .to_string())
}

/// Moves a form file from the cache into the temporary store, laid out like an HTTP sink
/// request, so a run on this device reads it through the returned FlowPath.
#[tauri::command(async)]
pub async fn stage_form_file(
    app_handle: AppHandle,
    app_id: String,
    event_id: String,
    source: String,
    file_name: String,
) -> Result<StagedFormFile, TauriFunctionError> {
    let uploads_dir = form_uploads_dir(&app_handle).map_err(|error| {
        TauriFunctionError::new(&format!(
            "Form file {file_name} cannot be staged: the app cache directory is unavailable ({error})"
        ))
    })?;
    let store = temporary_store(&app_handle).await?;
    let path = form_file_store_path(&app_id, &event_id, &create_id(), &file_name);
    take_form_upload(&uploads_dir, &source, &file_name, store.as_generic(), &path).await?;

    Ok(StagedFormFile {
        flow_path: flow_path_value(&path),
    })
}

fn form_uploads_dir(app_handle: &AppHandle) -> tauri::Result<PathBuf> {
    Ok(app_handle.path().app_cache_dir()?.join(FORM_UPLOADS_DIR))
}

/// Streams a form file the frontend wrote into the store at `path` and removes it from the
/// cache, staged or not. A source outside `uploads_dir` is refused and left alone.
async fn take_form_upload(
    uploads_dir: &Path,
    source: &str,
    file_name: &str,
    store: Arc<dyn ObjectStore>,
    path: &str,
) -> Result<u64, TauriFunctionError> {
    let source = form_upload_source(uploads_dir, source)?;
    let copied = copy_into_store(&source, store, path).await;
    if let Err(error) = tokio::fs::remove_file(&source).await {
        tracing::warn!(
            source = %source.display(),
            error = %error,
            "Failed to remove a staged form file from the cache"
        );
    }
    copied.map_err(|reason| {
        TauriFunctionError::new(&format!(
            "Form file {file_name} could not be staged at {path}: {reason}"
        ))
    })
}

/// Copies `source` into the store in parts of `FORM_FILE_PART_BYTES`, at most
/// `FORM_FILE_PARTS_IN_FLIGHT` of them in memory at once.
async fn copy_into_store(
    source: &Path,
    store: Arc<dyn ObjectStore>,
    path: &str,
) -> Result<u64, String> {
    let file = tokio::fs::File::open(source)
        .await
        .map_err(|error| format!("{} could not be opened ({error})", source.display()))?;
    let mut reader = BufReader::with_capacity(FORM_FILE_READ_BYTES, file);
    let mut writer = BufWriter::with_capacity(store, StorePath::from(path), FORM_FILE_PART_BYTES)
        .with_max_concurrency(FORM_FILE_PARTS_IN_FLIGHT);
    let copied = match tokio::io::copy_buf(&mut reader, &mut writer).await {
        Ok(copied) => copied,
        Err(error) => {
            if let Err(abort) = writer.abort().await {
                tracing::warn!(path = %path, error = %abort, "Failed to abort a partial form file upload");
            }
            return Err(format!("copying {} failed ({error})", source.display()));
        }
    };
    writer.shutdown().await.map_err(|error| {
        format!(
            "{copied} bytes of {} could not be written ({error})",
            source.display()
        )
    })?;
    Ok(copied)
}

/// Deletes form picks and HTTP sink uploads older than `REQUEST_FILES_MAX_AGE`, off the main
/// thread once startup settled. A run reads its request files while it runs and a form keeps
/// only their names, so nothing else ever removes them.
pub(crate) fn spawn_request_files_sweep(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(REQUEST_FILES_SWEEP_DELAY).await;
        let Some(cutoff) = SystemTime::now().checked_sub(REQUEST_FILES_MAX_AGE) else {
            return;
        };
        match temporary_store(&app_handle).await {
            Ok(store) => {
                let deleted = sweep_stale_request_files(&store.as_generic(), cutoff).await;
                if deleted > 0 {
                    tracing::info!(
                        deleted,
                        "Swept stale request files from the temporary store"
                    );
                }
            }
            Err(error) => tracing::debug!(error = %error, "Request file sweep skipped"),
        }
        if let Ok(uploads_dir) = form_uploads_dir(&app_handle) {
            let removed = sweep_stale_form_uploads(&uploads_dir, cutoff).await;
            if removed > 0 {
                tracing::info!(removed, "Swept form files the cache never handed over");
            }
        }
    });
}

async fn sweep_stale_request_files(store: &Arc<dyn ObjectStore>, cutoff: SystemTime) -> usize {
    let cutoff = chrono::DateTime::<chrono::Utc>::from(cutoff);
    let stale = store
        .list(Some(&StorePath::from(REQUEST_FILES_PREFIX)))
        .try_filter(move |meta| {
            futures::future::ready(meta.last_modified < cutoff && is_request_file(&meta.location))
        })
        .map_ok(|meta| meta.location)
        .boxed();

    let mut deleted = 0;
    let mut failed = 0;
    let mut results = store.delete_stream(stale);
    while let Some(result) = results.next().await {
        match result {
            Ok(_) => deleted += 1,
            Err(error) => {
                failed += 1;
                tracing::debug!(error = %error, "A stale request file could not be swept");
            }
        }
    }
    if failed > 0 {
        tracing::warn!(
            failed,
            deleted,
            "Some stale request files could not be swept"
        );
    }
    deleted
}

/// `tmp/global/apps/<app>/events/<event>/requests/…`
fn is_request_file(path: &StorePath) -> bool {
    let parts: Vec<&str> = path.as_ref().split('/').collect();
    parts.len() >= 8
        && parts[..3] == ["tmp", "global", "apps"]
        && parts[4] == "events"
        && parts[6] == "requests"
}

/// Removes cache files a form wrote but never handed over (the app quit in between).
async fn sweep_stale_form_uploads(uploads_dir: &Path, cutoff: SystemTime) -> usize {
    let Ok(mut entries) = tokio::fs::read_dir(uploads_dir).await else {
        return 0;
    };
    let mut removed = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let stale = entry
            .metadata()
            .await
            .ok()
            .filter(|metadata| metadata.is_file())
            .and_then(|metadata| metadata.modified().ok())
            .is_some_and(|modified| modified < cutoff);
        if stale && tokio::fs::remove_file(entry.path()).await.is_ok() {
            removed += 1;
        }
    }
    removed
}

async fn temporary_store(app_handle: &AppHandle) -> Result<FlowLikeStore, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let store = state.config.read().await.stores.temporary_store.clone();
    store.ok_or_else(|| {
        TauriFunctionError::new("Form files cannot be staged: no temporary store is configured")
    })
}

fn form_file_store_path(app_id: &str, event_id: &str, request_id: &str, file_name: &str) -> String {
    format!(
        "tmp/global/apps/{}/events/{}/requests/{request_id}/0001-{}",
        sanitize_store_path_segment(app_id, "app"),
        sanitize_store_path_segment(event_id, "event"),
        sanitize_request_file_name(Some(file_name), 1),
    )
}

/// Only files directly inside `uploads_dir` are accepted, so the command never reads
/// anything else on disk.
fn form_upload_source(uploads_dir: &Path, source: &str) -> Result<PathBuf, TauriFunctionError> {
    let rejected = |reason: String| {
        TauriFunctionError::new(&format!("Form file {source} was rejected: {reason}"))
    };
    let root = uploads_dir.canonicalize().map_err(|error| {
        rejected(format!(
            "{} is unavailable ({error})",
            uploads_dir.display()
        ))
    })?;
    let file = Path::new(source)
        .canonicalize()
        .map_err(|error| rejected(error.to_string()))?;
    if file.parent() != Some(root.as_path()) || !file.is_file() {
        return Err(rejected(format!(
            "only files in {} can be staged",
            root.display()
        )));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow_like_storage::files::store::local_store::LocalObjectStore;
    use flow_like::flow_like_storage::object_store::{ObjectStoreExt, PutPayload};

    struct UploadsFixture(PathBuf);

    impl UploadsFixture {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("flow-like-form-file-{}", create_id()));
            std::fs::create_dir_all(root.join(FORM_UPLOADS_DIR)).unwrap();
            Self(root)
        }

        fn uploads(&self) -> PathBuf {
            self.0.join(FORM_UPLOADS_DIR)
        }
    }

    impl Drop for UploadsFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn rejection(uploads: &Path, source: &Path) -> String {
        let source = source.to_string_lossy();
        let error = form_upload_source(uploads, &source)
            .unwrap_err()
            .to_string();
        assert!(error.contains(&*source), "{error}");
        error
    }

    #[test]
    fn form_file_store_path_matches_the_http_sink_layout() {
        assert_eq!(
            form_file_store_path("app-1", "event-1", "req1", "Invoice 03.pdf"),
            "tmp/global/apps/app-1/events/event-1/requests/req1/0001-Invoice_03.pdf"
        );
        assert_eq!(
            form_file_store_path("../app", "ev/../x", "req1", "../../etc/passwd"),
            "tmp/global/apps/app/events/ev_.._x/requests/req1/0001-passwd"
        );
        assert_eq!(
            form_file_store_path("", "", "req1", ""),
            "tmp/global/apps/app/events/event/requests/req1/0001-file"
        );
    }

    #[test]
    fn form_file_staged_flow_path_is_camel_outside_and_snake_inside() {
        let staged = StagedFormFile {
            flow_path: flow_path_value("tmp/global/apps/a/events/e/requests/r/0001-a.txt"),
        };
        assert_eq!(
            serde_json::to_value(staged).unwrap(),
            serde_json::json!({
                "flowPath": {
                    "path": "tmp/global/apps/a/events/e/requests/r/0001-a.txt",
                    "store_ref": flow_like_types::dispatch::REQUEST_FILES_STORE_REF,
                    "cache_store_ref": null
                }
            })
        );
    }

    #[test]
    fn form_file_source_in_form_uploads_is_accepted() {
        let fixture = UploadsFixture::new();
        let file = fixture.uploads().join("picked.pdf");
        std::fs::write(&file, b"%PDF").unwrap();
        assert_eq!(
            form_upload_source(&fixture.uploads(), &file.to_string_lossy()).unwrap(),
            file.canonicalize().unwrap()
        );
    }

    #[test]
    fn form_file_sources_outside_form_uploads_are_rejected() {
        let fixture = UploadsFixture::new();
        let uploads = fixture.uploads();
        let secret = fixture.0.join("secret.txt");
        std::fs::write(&secret, b"secret").unwrap();
        let nested = uploads.join("nested");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("deep.txt"), b"deep").unwrap();

        for source in [
            secret.clone(),
            uploads.join("../secret.txt"),
            uploads.join("missing.txt"),
            nested.clone(),
            nested.join("deep.txt"),
        ] {
            rejection(&uploads, &source);
        }

        #[cfg(unix)]
        {
            let link = uploads.join("link.txt");
            std::os::unix::fs::symlink(&secret, &link).unwrap();
            rejection(&uploads, &link);
        }
    }

    #[test]
    fn form_file_rejects_everything_when_form_uploads_is_missing() {
        let fixture = UploadsFixture::new();
        let file = fixture.0.join("picked.pdf");
        std::fs::write(&file, b"%PDF").unwrap();
        let error = rejection(&fixture.0.join("absent"), &file);
        assert!(error.contains("unavailable"), "{error}");
    }

    #[test]
    fn form_file_store_path_keeps_the_extension_of_a_non_ascii_name() {
        assert_eq!(
            form_file_store_path("app-1", "event-1", "req1", "請求書.pdf"),
            "tmp/global/apps/app-1/events/event-1/requests/req1/0001-file-1.pdf"
        );
    }

    fn local_store(root: &Path) -> Arc<dyn ObjectStore> {
        Arc::new(LocalObjectStore::new(root.to_path_buf()).unwrap())
    }

    fn backdate(file: &Path, age: Duration) {
        std::fs::File::options()
            .write(true)
            .open(file)
            .unwrap()
            .set_modified(SystemTime::now() - age)
            .unwrap();
    }

    async fn stored_paths(store: &Arc<dyn ObjectStore>) -> Vec<String> {
        let mut paths: Vec<String> = store
            .list(None)
            .map_ok(|meta| meta.location.to_string())
            .try_collect()
            .await
            .unwrap();
        paths.sort();
        paths
    }

    #[tokio::test]
    async fn form_file_streams_into_the_store_and_leaves_the_cache() {
        let fixture = UploadsFixture::new();
        let store = local_store(&fixture.0.join("store"));
        for size in [0, 5, FORM_FILE_PART_BYTES * 2 + 5] {
            let content: Vec<u8> = (0..size).map(|index| (index % 251) as u8).collect();
            let source = fixture.uploads().join(format!("picked-{size}.bin"));
            std::fs::write(&source, &content).unwrap();
            let path = format!("tmp/global/apps/a/events/e/requests/r{size}/0001-picked.bin");

            let copied = take_form_upload(
                &fixture.uploads(),
                &source.to_string_lossy(),
                "picked.bin",
                store.clone(),
                &path,
            )
            .await
            .unwrap();

            assert_eq!(copied, size as u64);
            assert!(!source.exists(), "{size} bytes: the cache copy stays");
            let stored = store
                .get(&StorePath::from(path))
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap();
            assert!(stored.as_ref() == content.as_slice(), "{size} bytes differ");
        }
    }

    #[tokio::test]
    async fn form_file_refused_source_stays_where_it_is() {
        let fixture = UploadsFixture::new();
        let store = local_store(&fixture.0.join("store"));
        let outside = fixture.0.join("outside.pdf");
        std::fs::write(&outside, b"%PDF").unwrap();

        let error = take_form_upload(
            &fixture.uploads(),
            &outside.to_string_lossy(),
            "outside.pdf",
            store.clone(),
            "tmp/global/apps/a/events/e/requests/r/0001-outside.pdf",
        )
        .await
        .unwrap_err()
        .to_string();

        assert!(error.contains("was rejected"), "{error}");
        assert!(outside.exists());
        assert!(stored_paths(&store).await.is_empty());
    }

    #[tokio::test]
    async fn form_file_sweep_deletes_only_stale_request_files() {
        let fixture = UploadsFixture::new();
        let root = fixture.0.join("store");
        let store = local_store(&root);
        let day = REQUEST_FILES_MAX_AGE;
        let objects = [
            ("tmp/global/apps/a/events/e/requests/r1/0001-a.pdf", true),
            ("tmp/global/apps/b/events/f/requests/r2/0002-b.csv", true),
            ("tmp/global/apps/a/events/e/requests/r3/0001-c.pdf", false),
            ("tmp/global/apps/a/events/e/other/r4/0001-d.pdf", true),
            ("tmp/global/apps/a/compiled/drafts/b/v.bin", true),
            ("tmp/apps/a/events/e/requests/r5/0001-e.pdf", true),
        ];
        for (path, stale) in objects {
            store
                .put(&StorePath::from(path), PutPayload::from_static(b"x"))
                .await
                .unwrap();
            if stale {
                backdate(&root.join(path), day + Duration::from_secs(60));
            }
        }

        let cutoff = SystemTime::now() - day;
        assert_eq!(sweep_stale_request_files(&store, cutoff).await, 2);
        assert_eq!(
            stored_paths(&store).await,
            [
                "tmp/apps/a/events/e/requests/r5/0001-e.pdf",
                "tmp/global/apps/a/compiled/drafts/b/v.bin",
                "tmp/global/apps/a/events/e/other/r4/0001-d.pdf",
                "tmp/global/apps/a/events/e/requests/r3/0001-c.pdf",
            ]
        );
        assert_eq!(sweep_stale_request_files(&store, cutoff).await, 0);
    }

    #[tokio::test]
    async fn form_file_sweep_removes_stale_cache_copies() {
        let fixture = UploadsFixture::new();
        let uploads = fixture.uploads();
        let stale = uploads.join("stale.pdf");
        let fresh = uploads.join("fresh.pdf");
        std::fs::write(&stale, b"%PDF").unwrap();
        std::fs::write(&fresh, b"%PDF").unwrap();
        std::fs::create_dir_all(uploads.join("nested")).unwrap();
        backdate(&stale, REQUEST_FILES_MAX_AGE + Duration::from_secs(60));

        let cutoff = SystemTime::now() - REQUEST_FILES_MAX_AGE;
        assert_eq!(sweep_stale_form_uploads(&uploads, cutoff).await, 1);
        assert!(!stale.exists());
        assert!(fresh.exists());
        assert!(uploads.join("nested").exists());
        assert_eq!(
            sweep_stale_form_uploads(&fixture.0.join("absent"), cutoff).await,
            0
        );
    }
}
