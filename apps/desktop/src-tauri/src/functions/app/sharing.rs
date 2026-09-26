use std::{
    path::PathBuf,
    sync::{Arc, LazyLock},
};

use flow_like::{
    app::{
        App,
        sharing::{
            ArchiveInfo, ArchiveObserver, ArchiveProgress, ExportOptions, ExportPreflight,
            ExportReport, ImportMode, ImportOptions, ImportReport,
        },
    },
    profile::ProfileApp,
};
use flow_like_types::{sync::DashMap, tokio_util::sync::CancellationToken};
use serde::Serialize;

use tauri::AppHandle;
#[cfg(target_os = "ios")]
use tauri::Manager;
use tauri_plugin_dialog::{DialogExt, FilePath};
use tracing::{info, warn};
use urlencoding::decode;

use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

static ARCHIVE_OPERATIONS: LazyLock<DashMap<String, CancellationToken>> =
    LazyLock::new(DashMap::new);

/// Dismissing the save panel is a normal outcome, not a failure. The frontend
/// recognises it through the same "cancelled" marker as an in-flight cancel.
const PICKER_DISMISSED: &str = "Export target selection cancelled";

#[derive(Clone, Serialize)]
struct ArchiveProgressEvent {
    operation_id: Option<String>,
    kind: &'static str,
    progress: ArchiveProgress,
}

struct ArchiveOperation {
    id: Option<String>,
    token: CancellationToken,
}

impl ArchiveOperation {
    fn register(id: Option<String>) -> Self {
        let token = CancellationToken::new();
        if let Some(id) = &id {
            ARCHIVE_OPERATIONS.insert(id.clone(), token.clone());
        }
        Self { id, token }
    }

    fn observer(&self, app_handle: &AppHandle, kind: &'static str) -> ArchiveObserver {
        let app_handle = app_handle.clone();
        let operation_id = self.id.clone();
        let progress: Arc<dyn Fn(ArchiveProgress) + Send + Sync> = Arc::new(move |progress| {
            crate::utils::emit_to_ui(
                &app_handle,
                "archive:progress",
                ArchiveProgressEvent {
                    operation_id: operation_id.clone(),
                    kind,
                    progress,
                },
            );
        });
        ArchiveObserver {
            progress: Some(progress),
            cancel: Some(self.token.clone()),
        }
    }
}

impl Drop for ArchiveOperation {
    fn drop(&mut self) {
        if let Some(id) = &self.id {
            ARCHIVE_OPERATIONS.remove(id);
        }
    }
}

fn sanitize_file_name(name: &str) -> String {
    let mut sanitized = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' => c,
            ' ' | '-' | '_' => c,
            _ => '-',
        })
        .collect::<String>();

    // Collapse consecutive separators and trim from edges for nicer defaults.
    while sanitized.contains("--") {
        sanitized = sanitized.replace("--", "-");
    }
    let sanitized = sanitized
        .trim_matches(|c: char| matches!(c, ' ' | '-' | '_'))
        .to_string();

    if sanitized.is_empty() {
        String::from("flow-like-app")
    } else {
        sanitized
    }
}

/// An archive the import pipeline can open with `File::open`. On Android the
/// picker hands back a Storage Access Framework `content://` URI, which has no
/// filesystem path, so it is copied into a temp file that is removed on drop.
struct StagedArchive {
    path: PathBuf,
    temporary: bool,
}

impl Drop for StagedArchive {
    fn drop(&mut self) {
        if self.temporary
            && let Err(err) = std::fs::remove_file(&self.path)
        {
            warn!(
                target: "import",
                path = %self.path.display(),
                error = %err,
                "Failed to remove staged import copy"
            );
        }
    }
}

#[cfg(target_os = "android")]
fn unique_temp_name(prefix: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map(|since| since.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("{prefix}-{}-{nanos}.flow-app", std::process::id()))
}

fn stage_archive_path(
    app_handle: &AppHandle,
    path: PathBuf,
) -> Result<StagedArchive, TauriFunctionError> {
    let _ = app_handle;
    #[cfg(target_os = "android")]
    {
        let raw = path.to_string_lossy().to_string();
        if !raw.starts_with("file://") && raw.contains("://") {
            return stage_content_uri(app_handle, &raw);
        }
    }
    Ok(StagedArchive {
        path: normalize_import_path(path)?,
        temporary: false,
    })
}

#[cfg(target_os = "android")]
fn stage_content_uri(
    app_handle: &AppHandle,
    uri: &str,
) -> Result<StagedArchive, TauriFunctionError> {
    use std::io::Write;
    use tauri_plugin_fs::{FsExt, OpenOptions};

    let url = tauri::Url::parse(uri)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to parse content uri: {}", e)))?;
    let mut open_options = OpenOptions::new();
    open_options.read(true);
    let mut source = app_handle
        .fs()
        .open(FilePath::Url(url), open_options)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to open {}: {}", uri, e)))?;

    let staged = StagedArchive {
        path: unique_temp_name("flow-like-import"),
        temporary: true,
    };
    info!(target: "import", path = %staged.path.display(), "Staging content uri import");
    (|| -> std::io::Result<()> {
        let mut target = std::fs::File::create(&staged.path)?;
        std::io::copy(&mut source, &mut target)?;
        target.flush()
    })()
    .map_err(|e| TauriFunctionError::new(&format!("Failed to stage {}: {}", uri, e)))?;
    Ok(staged)
}

fn normalize_import_path(path: PathBuf) -> Result<PathBuf, TauriFunctionError> {
    let path_str = path.to_string_lossy();
    if !path_str.starts_with("file://") {
        return Ok(path);
    }

    let stripped = path_str.trim_start_matches("file://");
    let stripped = stripped.strip_prefix("localhost/").unwrap_or(stripped);

    let decoded = decode(stripped)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to decode file URI: {}", e)))?;

    #[cfg(target_os = "windows")]
    {
        let without_leading_slash = decoded.trim_start_matches('/');
        Ok(PathBuf::from(without_leading_slash))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let normalized = if decoded.starts_with('/') {
            decoded.to_string()
        } else {
            format!("/{}", decoded)
        };
        Ok(PathBuf::from(normalized))
    }
}

#[cfg(target_os = "ios")]
fn decode_file_url(uri: &str) -> Option<PathBuf> {
    let stripped = uri.strip_prefix("file://")?;
    let stripped = stripped.strip_prefix("localhost/").unwrap_or(stripped);
    let decoded = decode(stripped).ok()?;

    #[cfg(target_os = "windows")]
    {
        let without_leading_slash = decoded.trim_start_matches('/');
        Some(PathBuf::from(without_leading_slash))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let normalized = if decoded.starts_with('/') {
            decoded.to_string()
        } else {
            format!("/{}", decoded)
        };
        Some(PathBuf::from(normalized))
    }
}

#[cfg(target_os = "ios")]
fn dialog_response_into_path(response: FilePath) -> Result<PathBuf, TauriFunctionError> {
    match response {
        FilePath::Path(path) => {
            info!(target: "export", ?path, "Dialog returned filesystem path");
            Ok(path)
        }
        FilePath::Url(url) => {
            info!(target: "export", uri = %url, "Dialog returned URI, attempting to decode");

            if let Ok(path) = url.to_file_path() {
                info!(target: "export", ?path, "Converted URI into filesystem path via to_file_path");
                return Ok(path);
            }

            if let Some(path) = decode_file_url(url.as_str()) {
                info!(target: "export", ?path, "Converted URI into filesystem path via manual decoding");
                return Ok(path);
            }

            warn!(target: "export", uri = %url, "Failed to convert URI into filesystem path");
            Err(TauriFunctionError::new(
                "Dialog response missing both path and decodable uri",
            ))
        }
    }
}

#[cfg(not(target_os = "ios"))]
fn dialog_response_into_path(response: FilePath) -> Result<PathBuf, TauriFunctionError> {
    response
        .into_path()
        .map_err(|e| TauriFunctionError::new(&format!("Failed to convert file path: {}", e)))
}

async fn load_app(app_handle: &AppHandle, app_id: &str) -> Result<App, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(app_handle).await?;
    App::load(app_id.to_string(), flow_like_state)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to load app {}: {}", app_id, e)))
}

async fn export_with_options(
    app: &App,
    options: ExportOptions,
    target: PathBuf,
    observer: ArchiveObserver,
) -> Result<ExportReport, TauriFunctionError> {
    app.export_archive_with(options, target, observer)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to export app: {}", e)))
}

#[cfg(target_os = "ios")]
async fn perform_ios_export(
    app_handle: &AppHandle,
    app: &App,
    file_name: &str,
    options: ExportOptions,
    observer: ArchiveObserver,
) -> Result<ExportReport, TauriFunctionError> {
    use flow_like_types::tokio::sync::oneshot;

    let documents_dir = app_handle.path().document_dir().map_err(|e| {
        TauriFunctionError::new(&format!("Failed to resolve documents directory: {}", e))
    })?;

    let staging_target = documents_dir.join(file_name);
    info!(
        target: "export",
        path = %staging_target.display(),
        "Preparing iOS staging export file"
    );

    let report = export_with_options(app, options, staging_target, observer).await?;
    let staged_file = report.path.clone();
    // The engine normalises the archive suffix, so the dialog must offer the
    // name of the file that actually exists, not the requested one.
    let dialog_name = staged_file
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(file_name)
        .to_string();

    let (tx, rx) = oneshot::channel();
    app_handle
        .dialog()
        .file()
        .set_title("Export App")
        .set_file_name(dialog_name)
        .save_file(move |response| {
            let _ = tx.send(response);
        });

    let selection = rx
        .await
        .map_err(|_| TauriFunctionError::new("Failed to receive selected file"))?;

    let target_file = match selection {
        Some(file) => file,
        None => {
            if let Err(err) = std::fs::remove_file(&staged_file) {
                warn!(
                    target: "export",
                    path = %staged_file.display(),
                    error = %err,
                    "Failed to remove iOS staging export file after cancellation"
                );
            }
            return Err(TauriFunctionError::new(PICKER_DISMISSED));
        }
    };

    let path_buf = dialog_response_into_path(target_file)?;

    info!(
        target: "export",
        path = %path_buf.display(),
        "Resolved export target path"
    );

    // The picker returns the destination it just created, never the staging
    // source, so only the staging copy may be removed here.
    if path_buf.canonicalize().ok() != staged_file.canonicalize().ok()
        && let Err(err) = std::fs::remove_file(&staged_file)
    {
        warn!(
            target: "export",
            path = %staged_file.display(),
            error = %err,
            "Failed to remove iOS staging export file"
        );
    }

    let mut report = report;
    report.path = path_buf;

    info!(target: "export", "Export completed successfully");
    Ok(report)
}

#[cfg(target_os = "android")]
async fn perform_content_uri_export(
    app_handle: &AppHandle,
    app: &App,
    file_name: &str,
    options: ExportOptions,
    observer: ArchiveObserver,
    destination: tauri::Url,
) -> Result<ExportReport, TauriFunctionError> {
    use std::io::Write;
    use tauri_plugin_fs::{FsExt, OpenOptions};

    let staging_target = std::env::temp_dir().join(file_name);
    info!(
        target: "export",
        path = %staging_target.display(),
        destination = %destination,
        "Preparing Android staging export file"
    );

    let mut report = export_with_options(app, options, staging_target, observer).await?;
    let staged = StagedArchive {
        path: report.path.clone(),
        temporary: true,
    };

    (|| -> std::io::Result<()> {
        let mut source = std::fs::File::open(&staged.path)?;
        let mut open_options = OpenOptions::new();
        open_options.write(true).truncate(true);
        let mut target = app_handle
            .fs()
            .open(FilePath::Url(destination.clone()), open_options)?;
        std::io::copy(&mut source, &mut target)?;
        target.flush()
    })()
    .map_err(|e| {
        TauriFunctionError::new(&format!("Failed to write export to {}: {}", destination, e))
    })?;

    report.path = PathBuf::from(destination.to_string());
    info!(target: "export", "Export completed successfully");
    Ok(report)
}

#[cfg(not(target_os = "ios"))]
async fn perform_standard_export(
    app_handle: &AppHandle,
    app: &App,
    file_name: &str,
    options: ExportOptions,
    observer: ArchiveObserver,
) -> Result<ExportReport, TauriFunctionError> {
    let target_file = app_handle
        .dialog()
        .file()
        .set_title("Export App")
        .set_file_name(file_name.to_string())
        .blocking_save_file()
        .ok_or_else(|| TauriFunctionError::new(PICKER_DISMISSED))?;

    #[cfg(target_os = "android")]
    let target_file = match target_file {
        FilePath::Url(url) if url.scheme() != "file" => {
            return perform_content_uri_export(app_handle, app, file_name, options, observer, url)
                .await;
        }
        other => other,
    };

    let path_buf = dialog_response_into_path(target_file)?;

    info!(
        target: "export",
        path = %path_buf.display(),
        "Resolved export target path"
    );

    let report = export_with_options(app, options, path_buf, observer).await?;

    info!(target: "export", "Export completed successfully");
    Ok(report)
}

#[tauri::command(async)]
pub async fn export_app_to_file(
    app_handle: AppHandle,
    app_id: String,
    password: Option<String>,
    compact: Option<bool>,
    operation_id: Option<String>,
) -> Result<ExportReport, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let app = load_app(&app_handle, &app_id).await?;

    let meta = App::get_meta(app_id, flow_like_state, None, None)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to get app meta: {}", e)))?;

    let file_suffix = if password.is_some() {
        "enc.flow-app"
    } else {
        "flow-app"
    };
    let default_file_name = format!("{}.{}", sanitize_file_name(&meta.name), file_suffix);

    let options = ExportOptions {
        password,
        compact_tables: compact.unwrap_or(false),
    };

    let operation = ArchiveOperation::register(operation_id);
    let observer = operation.observer(&app_handle, "export");

    #[cfg(target_os = "ios")]
    {
        perform_ios_export(&app_handle, &app, &default_file_name, options, observer).await
    }

    #[cfg(not(target_os = "ios"))]
    {
        perform_standard_export(&app_handle, &app, &default_file_name, options, observer).await
    }
}

#[tauri::command(async)]
pub async fn get_app_export_preflight(
    app_handle: AppHandle,
    app_id: String,
) -> Result<ExportPreflight, TauriFunctionError> {
    let app = load_app(&app_handle, &app_id).await?;
    app.export_preflight().await.map_err(|e| {
        TauriFunctionError::new(&format!(
            "Failed to run export preflight for app {}: {}",
            app_id, e
        ))
    })
}

#[tauri::command(async)]
pub async fn import_app_from_file(
    app_handle: AppHandle,
    path: PathBuf,
    password: Option<String>,
    mode: Option<ImportMode>,
    operation_id: Option<String>,
) -> Result<ImportReport, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    let staged = stage_archive_path(&app_handle, path)?;
    let path = staged.path.clone();

    let profile_id = TauriSettingsState::current_profile(&app_handle)
        .await?
        .hub_profile
        .id;

    let options = ImportOptions {
        password,
        mode: mode.unwrap_or_default(),
    };
    let operation = ArchiveOperation::register(operation_id);
    let observer = operation.observer(&app_handle, "import");

    let report = App::import_archive_with(flow_like_state, path, options, observer)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to import app: {}", e)))?;
    drop(operation);

    let app_id = &report.app.id;
    info!(target: "import", app_id = %app_id, mode = ?report.mode, "Imported app");

    add_app_to_profile(&app_handle, &profile_id, app_id).await?;

    Ok(report)
}

/// Adds `app_id` to the profile that was current when a long-running copy
/// started. Applied to the latest stored profile so edits made meanwhile,
/// including Home saves, remain intact.
pub(super) async fn add_app_to_profile(
    app_handle: &AppHandle,
    profile_id: &str,
    app_id: &str,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(app_handle).await?;
    let mut settings = settings.lock().await;
    let profile = settings
        .profiles
        .get_mut(profile_id)
        .ok_or_else(|| TauriFunctionError::new(&format!("Profile {profile_id} not found")))?;
    let apps = profile.hub_profile.apps.get_or_insert_with(Vec::new);

    if !apps.iter().any(|a| a.app_id == app_id) {
        apps.push(ProfileApp::new(app_id.to_string()));
        profile.advance_revision(None);
        settings.try_serialize()?;
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn inspect_app_archive(
    app_handle: AppHandle,
    path: PathBuf,
    password: Option<String>,
) -> Result<ArchiveInfo, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let staged = stage_archive_path(&app_handle, path)?;

    App::inspect_archive(flow_like_state, staged.path.clone(), password)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to inspect archive: {}", e)))
}

#[tauri::command(async)]
pub async fn cancel_archive_operation(operation_id: String) -> Result<bool, TauriFunctionError> {
    let token = ARCHIVE_OPERATIONS
        .get(&operation_id)
        .map(|entry| entry.value().clone());
    match token {
        Some(token) => {
            token.cancel();
            Ok(true)
        }
        None => Ok(false),
    }
}
