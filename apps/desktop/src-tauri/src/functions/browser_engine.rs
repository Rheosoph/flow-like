use std::collections::HashSet;
use std::path::Path;
use std::time::Duration;

use flow_like_browser::launch::cft::{self, CftProgress, CftVersion};
use flow_like_browser::launch::{self, BrowserKind, Executable, Flavor, discovery};
use serde::Serialize;
use tauri::AppHandle;

use super::TauriFunctionError;
use crate::utils::{UiEmitTarget, emit_throttled, emit_to_ui};

const PROGRESS_EVENT: &str = "browser-engine-progress";
const DOWNLOAD_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
const CFT_DOWNLOAD_SIZE_MB: u32 = 200;
const CFT_SOURCE: &str = "storage.googleapis.com (Chrome for Testing)";

static CFT_OPERATION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Serialize)]
pub struct InstalledBrowser {
    pub kind: String,
    pub flavor: String,
    pub path: String,
    pub version: Option<String>,
}

#[derive(Serialize)]
pub struct BrowserEngineStatus {
    pub installed: Vec<InstalledBrowser>,
    pub cft_version: Option<String>,
    pub cft_path: Option<String>,
    pub cft_supported: bool,
    pub cft_pinned: String,
    pub cache_dir: String,
    pub download_size_mb: u32,
    pub source: String,
}

#[derive(Clone, Serialize)]
struct EngineProgress {
    stage: &'static str,
    received: Option<u64>,
    total: Option<u64>,
}

impl EngineProgress {
    fn stage(stage: &'static str) -> Self {
        Self {
            stage,
            received: None,
            total: None,
        }
    }
}

impl From<CftProgress> for EngineProgress {
    fn from(progress: CftProgress) -> Self {
        match progress {
            CftProgress::Resolving => Self::stage("resolving"),
            CftProgress::Downloading { received, total } => Self {
                stage: "downloading",
                received: Some(received),
                total,
            },
            CftProgress::Verifying => Self::stage("verifying"),
            CftProgress::Extracting => Self::stage("extracting"),
            CftProgress::Configuring => Self::stage("configuring"),
            CftProgress::Done { .. } => Self::stage("done"),
        }
    }
}

#[tauri::command(async)]
pub async fn browser_engine_status() -> Result<BrowserEngineStatus, TauriFunctionError> {
    tokio::task::spawn_blocking(read_status)
        .await
        .map_err(|error| {
            TauriFunctionError::new(&format!(
                "Reading the browser engine status stopped unexpectedly: {error}"
            ))
        })
}

#[tauri::command(async)]
pub async fn browser_engine_install_cft(app: AppHandle) -> Result<String, TauriFunctionError> {
    let _exclusive = CFT_OPERATION
        .try_lock()
        .map_err(|_| cft_operation_busy("install"))?;
    let cache_dir = launch::default_cache_dir();
    let executable = cft::install(CftVersion::Stable, &cache_dir, |progress| {
        emit_progress(&app, progress)
    })
    .await
    .map_err(|error| {
        TauriFunctionError::new(&format!(
            "Installing Chrome for Testing into {} failed: {error}",
            cache_dir.display()
        ))
    })?;
    installed_version(executable)
}

#[tauri::command(async)]
pub async fn browser_engine_latest_cft() -> Result<Option<String>, TauriFunctionError> {
    if cft::platform().is_none() {
        return Ok(None);
    }
    cft::resolve(&CftVersion::Stable)
        .await
        .map(|release| Some(release.version))
        .map_err(|error| {
            TauriFunctionError::new(&format!(
                "Looking up the latest Chrome for Testing version failed: {error}"
            ))
        })
}

#[tauri::command(async)]
pub async fn browser_engine_remove_cft() -> Result<(), TauriFunctionError> {
    let _exclusive = CFT_OPERATION
        .try_lock()
        .map_err(|_| cft_operation_busy("remove"))?;
    let cache_dir = launch::default_cache_dir();
    tokio::task::spawn_blocking(move || {
        cft::remove_all(&cache_dir).map_err(|error| {
            TauriFunctionError::new(&format!(
                "Removing Chrome for Testing from {} failed: {error}",
                cache_dir.display()
            ))
        })
    })
    .await
    .map_err(|error| {
        TauriFunctionError::new(&format!(
            "Removing Chrome for Testing stopped unexpectedly: {error}"
        ))
    })?
}

fn cft_operation_busy(operation: &str) -> TauriFunctionError {
    TauriFunctionError::new(&format!(
        "Cannot {operation} Chrome for Testing while another install or removal of it is still running"
    ))
}

// Same predicate as the private `discovery::is_executable_file` that `launch::find` filters with.
fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

fn read_status() -> BrowserEngineStatus {
    let cache_dir = launch::default_cache_dir();
    let cached = cft::find_cached(&cache_dir);
    BrowserEngineStatus {
        installed: installed_browsers(),
        cft_version: cached.as_ref().and_then(|cached| cached.version.clone()),
        cft_path: cached.map(|cached| cached.path.display().to_string()),
        cft_supported: cft::platform().is_some(),
        cft_pinned: cft::CFT_PINNED.to_owned(),
        cache_dir: cache_dir.display().to_string(),
        download_size_mb: CFT_DOWNLOAD_SIZE_MB,
        source: CFT_SOURCE.to_owned(),
    }
}

fn installed_browsers() -> Vec<InstalledBrowser> {
    let mut seen = HashSet::new();
    [BrowserKind::Chrome, BrowserKind::Edge]
        .into_iter()
        .flat_map(|kind| {
            discovery::candidates(kind)
                .into_iter()
                .map(move |candidate| (kind, candidate))
        })
        .filter(|(_, candidate)| {
            is_executable_file(&candidate.path) && seen.insert(candidate.path.clone())
        })
        .map(|(kind, candidate)| InstalledBrowser {
            kind: kind_name(kind).to_owned(),
            flavor: flavor_name(candidate.flavor).to_owned(),
            version: discovery::executable_version(&candidate.path, candidate.flavor),
            path: candidate.path.display().to_string(),
        })
        .collect()
}

fn emit_progress(app: &AppHandle, progress: CftProgress) {
    let payload = EngineProgress::from(progress);
    if payload.stage == "downloading" {
        emit_throttled(
            app,
            UiEmitTarget::All,
            PROGRESS_EVENT,
            payload,
            DOWNLOAD_PROGRESS_INTERVAL,
        );
    } else {
        emit_to_ui(app, PROGRESS_EVENT, payload);
    }
}

fn installed_version(executable: Executable) -> Result<String, TauriFunctionError> {
    executable.version.ok_or_else(|| {
        TauriFunctionError::new(&format!(
            "Chrome for Testing was installed at {} but its version is unknown",
            executable.path.display()
        ))
    })
}

fn kind_name(kind: BrowserKind) -> &'static str {
    match kind {
        BrowserKind::Chrome => "chrome",
        BrowserKind::Edge => "edge",
    }
}

fn flavor_name(flavor: Flavor) -> &'static str {
    match flavor {
        Flavor::Chrome => "chrome",
        Flavor::Edge => "edge",
        Flavor::Chromium => "chromium",
        Flavor::SnapChromium => "snap_chromium",
        Flavor::ChromeForTesting => "chrome_for_testing",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn progress_stages_match_the_install_stages_of_the_settings_page() {
        let stages = [
            (CftProgress::Resolving, "resolving"),
            (
                CftProgress::Downloading {
                    received: 1,
                    total: Some(2),
                },
                "downloading",
            ),
            (CftProgress::Verifying, "verifying"),
            (CftProgress::Extracting, "extracting"),
            (CftProgress::Configuring, "configuring"),
            (
                CftProgress::Done {
                    version: "140.0.7339.0".to_owned(),
                },
                "done",
            ),
        ];
        for (progress, stage) in stages {
            assert_eq!(EngineProgress::from(progress).stage, stage);
        }
    }

    #[test]
    fn download_progress_carries_the_byte_counts() {
        let progress = EngineProgress::from(CftProgress::Downloading {
            received: 512,
            total: None,
        });
        assert_eq!((progress.received, progress.total), (Some(512), None));
    }

    #[test]
    fn names_match_the_browser_types_of_the_settings_page() {
        assert_eq!(kind_name(BrowserKind::Chrome), "chrome");
        assert_eq!(kind_name(BrowserKind::Edge), "edge");
        let flavors = [
            (Flavor::Chrome, "chrome"),
            (Flavor::Edge, "edge"),
            (Flavor::Chromium, "chromium"),
            (Flavor::SnapChromium, "snap_chromium"),
            (Flavor::ChromeForTesting, "chrome_for_testing"),
        ];
        for (flavor, name) in flavors {
            assert_eq!(flavor_name(flavor), name);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_file_without_an_exec_bit_is_not_a_detected_browser() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("browser-engine-exec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chrome");
        std::fs::write(&path, b"").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!is_executable_file(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(is_executable_file(&path));
        assert!(!is_executable_file(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
