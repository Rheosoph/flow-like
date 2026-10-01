use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine as _;
use futures_util::StreamExt as _;
use md5::{Digest as _, Md5};
use tokio::io::AsyncWriteExt as _;

use crate::BrowserError;
use crate::launch::profile::process_alive;
use crate::launch::{Executable, ExecutableSource, Flavor};
use crate::transport::tls::{self, HttpProxy};

pub const CFT_PINNED: &str = "154.0.8037.92";
pub const CFT_ENDPOINT: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";

const DOWNLOAD_BASE: &str = "https://storage.googleapis.com/chrome-for-testing-public";
pub(crate) const CACHE_SUBDIR: &str = "chrome-for-testing";
const COMPLETE_MARKER: &str = ".complete";
pub(crate) const IN_USE_PREFIX: &str = ".in-use-";
const PARTIAL_DIR_SUFFIX: &str = ".partial";
const PARTIAL_ZIP_SUFFIX: &str = ".partial.zip";
const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(4 * 60 * 60);
const STALL_TIMEOUT: Duration = Duration::from_secs(60);
const PROGRESS_STEP: u64 = 1 << 20;
const RENAME_ATTEMPTS: u32 = if cfg!(windows) { 20 } else { 1 };
const RENAME_RETRY_DELAY: Duration = Duration::from_millis(250);
const MAC_APP_EXECUTABLE: [&str; 4] = [
    "Google Chrome for Testing.app",
    "Contents",
    "MacOS",
    "Google Chrome for Testing",
];

type Version = [u32; 4];

static INSTALL_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Clone, Debug)]
pub enum CftVersion {
    Pinned,
    Stable,
}

#[derive(Clone, Debug)]
pub struct CftRelease {
    pub version: String,
    pub url: String,
    pub platform: String,
}

#[derive(Clone, Debug)]
pub enum CftProgress {
    Resolving,
    Downloading { received: u64, total: Option<u64> },
    Verifying,
    Extracting,
    Configuring,
    Done { version: String },
}

pub fn platform() -> Option<&'static str> {
    platform_for(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn download_url(version: &str, platform: &str) -> String {
    format!("{DOWNLOAD_BASE}/{version}/{platform}/chrome-{platform}.zip")
}

pub fn find_cached(cache_dir: &Path) -> Option<Executable> {
    let platform = platform()?;
    scan_installs(&cache_dir.join(CACHE_SUBDIR))
        .into_iter()
        .filter(|install| {
            install.platform == platform
                && install.version >= pinned_version()
                && install.is_complete()
        })
        .filter_map(|install| {
            let executable = install_executable(&install.dir, platform, &install.version_text)?;
            executable
                .path
                .is_file()
                .then_some((install.version, executable))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, executable)| executable)
}

pub fn parse_last_known_good(json: &str, platform: &str) -> crate::Result<CftRelease> {
    let document: LastKnownGood = serde_json::from_str(json).map_err(|error| {
        install_error(format!(
            "The Chrome for Testing version list is not valid JSON: {error}"
        ))
    })?;
    let stable = document.channels.get("Stable").ok_or_else(|| {
        install_error("The Chrome for Testing version list has no Stable channel")
    })?;
    let version = parse_version(&stable.version).ok_or_else(|| {
        install_error(format!(
            "The Chrome for Testing Stable version '{}' is not a version number",
            stable.version
        ))
    })?;
    if version < pinned_version() {
        return Ok(pinned_release(platform));
    }
    let url = stable_download_url(stable, platform)?;
    Ok(CftRelease {
        version: stable.version.clone(),
        url,
        platform: platform.to_owned(),
    })
}

pub async fn resolve(version: &CftVersion) -> crate::Result<CftRelease> {
    let platform = platform().ok_or_else(|| {
        install_error(unavailable_message(
            std::env::consts::OS,
            std::env::consts::ARCH,
        ))
    })?;
    match version {
        CftVersion::Pinned => Ok(pinned_release(platform)),
        CftVersion::Stable => parse_last_known_good(&fetch_last_known_good().await?, platform),
    }
}

pub async fn install(
    version: CftVersion,
    cache_dir: &Path,
    progress: impl Fn(CftProgress) + Send + Sync,
) -> crate::Result<Executable> {
    progress(CftProgress::Resolving);
    let release = resolve(&version).await?;
    let executable = install_release(&release, cache_dir, &progress).await?;
    let root = cache_dir.join(CACHE_SUBDIR);
    if let Err(error) =
        tokio::task::spawn_blocking(move || remove_older_installs(&root, &release)).await
    {
        tracing::warn!(%error, "removing old Chrome for Testing builds did not finish");
    }
    Ok(executable)
}

pub async fn install_release(
    release: &CftRelease,
    cache_dir: &Path,
    progress: impl Fn(CftProgress) + Send + Sync,
) -> crate::Result<Executable> {
    let layout = InstallLayout::new(release, cache_dir)?;
    let _exclusive = INSTALL_LOCK.lock().await;
    if let Some(executable) = layout.installed() {
        progress(CftProgress::Done {
            version: release.version.clone(),
        });
        return Ok(executable);
    }
    layout.remove_partials().await;
    let result = download_and_unpack(release, &layout, &progress).await;
    if result.is_err() {
        layout.remove_partials().await;
    }
    let executable = result?;
    progress(CftProgress::Done {
        version: release.version.clone(),
    });
    Ok(executable)
}

pub fn remove_all(cache_dir: &Path) -> std::io::Result<()> {
    let root = cache_dir.join(CACHE_SUBDIR);
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(path_error("list", &root, error)),
    };
    let mut kept_in_use = false;
    let mut failures = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                failures.push(path_error("list", &root, error));
                break;
            }
        };
        match remove_unless_in_use(&entry) {
            Ok(kept) => kept_in_use |= kept,
            Err(error) => failures.push(error),
        }
    }
    if let Some(first) = failures.first() {
        let messages: Vec<String> = failures.iter().map(ToString::to_string).collect();
        return Err(std::io::Error::new(first.kind(), messages.join("; ")));
    }
    if kept_in_use {
        return Ok(());
    }
    std::fs::remove_dir(&root).map_err(|error| path_error("remove", &root, error))
}

fn remove_unless_in_use(entry: &std::fs::DirEntry) -> std::io::Result<bool> {
    let path = entry.path();
    let removed = match entry.file_type() {
        Ok(kind) if !kind.is_dir() => std::fs::remove_file(&path),
        Ok(_) if has_live_marker(&path) => return Ok(true),
        Ok(_) => remove_install(&path),
        Err(error) => Err(error),
    };
    removed
        .map(|()| false)
        .map_err(|error| path_error("remove", &path, error))
}

// Marker first, so a removal that stops halfway never leaves a build that counts as installed.
fn remove_install(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(dir.join(COMPLETE_MARKER)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    std::fs::remove_dir_all(dir)
}

#[derive(serde::Deserialize)]
struct LastKnownGood {
    #[serde(default)]
    channels: HashMap<String, Channel>,
}

#[derive(serde::Deserialize)]
struct Channel {
    version: String,
    #[serde(default)]
    downloads: Downloads,
}

#[derive(Default, serde::Deserialize)]
struct Downloads {
    #[serde(default)]
    chrome: Vec<Download>,
}

#[derive(serde::Deserialize)]
struct Download {
    platform: String,
    url: String,
}

struct InstalledBuild {
    dir: PathBuf,
    platform: String,
    version: Version,
    version_text: String,
}

impl InstalledBuild {
    fn is_complete(&self) -> bool {
        self.dir.join(COMPLETE_MARKER).is_file()
    }
}

struct InstallLayout {
    dir: PathBuf,
    partial_dir: PathBuf,
    partial_zip: PathBuf,
    root: PathBuf,
    executable: Executable,
    relative_executable: PathBuf,
}

impl InstallLayout {
    fn new(release: &CftRelease, cache_dir: &Path) -> crate::Result<Self> {
        let relative_executable = executable_relative_path(&release.platform).ok_or_else(|| {
            install_error(format!(
                "Unknown Chrome for Testing platform '{}'; expected one of mac-arm64, mac-x64, linux64, linux-arm64, win64, win32",
                release.platform
            ))
        })?;
        if parse_version(&release.version).is_none() {
            return Err(install_error(format!(
                "Chrome for Testing version '{}' is not a version number",
                release.version
            )));
        }
        let root = cache_dir.join(CACHE_SUBDIR);
        let name = format!("{}-{}", release.platform, release.version);
        let dir = root.join(&name);
        let executable = cached_executable(dir.join(&relative_executable), &release.version);
        Ok(Self {
            partial_dir: root.join(format!("{name}{PARTIAL_DIR_SUFFIX}")),
            partial_zip: root.join(format!("{name}{PARTIAL_ZIP_SUFFIX}")),
            dir,
            root,
            executable,
            relative_executable,
        })
    }

    fn installed(&self) -> Option<Executable> {
        (self.dir.join(COMPLETE_MARKER).is_file() && self.executable.path.is_file())
            .then(|| self.executable.clone())
    }

    async fn remove_partials(&self) {
        remove_quietly(&self.partial_zip).await;
        remove_quietly(&self.partial_dir).await;
    }
}

fn platform_for(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("linux", "x86_64") => Some("linux64"),
        ("linux", "aarch64") => Some("linux-arm64"),
        ("windows", "x86_64") => Some("win64"),
        ("windows", "x86") => Some("win32"),
        _ => None,
    }
}

fn unavailable_message(os: &str, arch: &str) -> String {
    if os == "windows" && arch == "aarch64" {
        return "Chrome for Testing is not available for Windows on ARM; install Microsoft Edge or Google Chrome".to_owned();
    }
    format!(
        "Chrome for Testing is not available for {os} on {arch}; install Google Chrome or Microsoft Edge"
    )
}

fn top_dir_name(platform: &str) -> String {
    format!("chrome-{platform}")
}

fn executable_relative_path(platform: &str) -> Option<PathBuf> {
    let top = PathBuf::from(top_dir_name(platform));
    match platform {
        "mac-arm64" | "mac-x64" => Some(
            MAC_APP_EXECUTABLE
                .iter()
                .fold(top, |path, part| path.join(part)),
        ),
        "linux64" | "linux-arm64" => Some(top.join("chrome")),
        "win64" | "win32" => Some(top.join("chrome.exe")),
        _ => None,
    }
}

fn install_executable(install_dir: &Path, platform: &str, version: &str) -> Option<Executable> {
    let relative = executable_relative_path(platform)?;
    Some(cached_executable(install_dir.join(relative), version))
}

fn cached_executable(path: PathBuf, version: &str) -> Executable {
    Executable {
        path,
        flavor: Flavor::ChromeForTesting,
        version: Some(version.to_owned()),
        source: ExecutableSource::CachedCft,
    }
}

fn parse_version(text: &str) -> Option<Version> {
    let mut parts = text.split('.');
    let mut version = [0; 4];
    for slot in &mut version {
        let part = parts.next()?;
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        *slot = part.parse().ok()?;
    }
    parts.next().is_none().then_some(version)
}

fn pinned_version() -> Version {
    parse_version(CFT_PINNED).unwrap_or_default()
}

fn pinned_release(platform: &str) -> CftRelease {
    CftRelease {
        version: CFT_PINNED.to_owned(),
        url: download_url(CFT_PINNED, platform),
        platform: platform.to_owned(),
    }
}

fn stable_download_url(stable: &Channel, platform: &str) -> crate::Result<String> {
    let download = stable
        .downloads
        .chrome
        .iter()
        .find(|download| download.platform == platform)
        .ok_or_else(|| {
            install_error(format!(
                "Chrome for Testing {} has no download for {platform}",
                stable.version
            ))
        })?;
    match url::Url::parse(&download.url) {
        Ok(url) if url.scheme() == "https" => Ok(download.url.clone()),
        _ => Err(install_error(format!(
            "The Chrome for Testing download for {platform} is not an https URL: '{}'",
            download.url
        ))),
    }
}

async fn fetch_last_known_good() -> crate::Result<String> {
    let failed = |error: &dyn std::fmt::Display| {
        install_error(format!(
            "Could not fetch the Chrome for Testing version list from {CFT_ENDPOINT}: {error}"
        ))
    };
    let client = tls::http_client(&[], RESOLVE_TIMEOUT, HttpProxy::System)
        .map_err(|error| failed(&error))?;
    let response = client
        .get(CFT_ENDPOINT)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| failed(&error))?;
    response.text().await.map_err(|error| failed(&error))
}

async fn download_and_unpack(
    release: &CftRelease,
    layout: &InstallLayout,
    progress: &(dyn Fn(CftProgress) + Send + Sync),
) -> crate::Result<Executable> {
    tokio::fs::create_dir_all(&layout.root)
        .await
        .map_err(|error| io_error("create the Chrome for Testing cache", &layout.root, &error))?;
    download(&release.url, &layout.partial_zip, progress).await?;
    progress(CftProgress::Extracting);
    extract(&layout.partial_zip, &layout.partial_dir).await?;
    let unpacked = layout.partial_dir.join(&layout.relative_executable);
    if !unpacked.is_file() {
        return Err(install_error(format!(
            "The Chrome for Testing archive from {} has no {}",
            release.url,
            layout.relative_executable.display()
        )));
    }
    progress(CftProgress::Configuring);
    configure(&layout.partial_dir.join(top_dir_name(&release.platform))).await?;
    commit(layout, &release.version).await?;
    Ok(layout.executable.clone())
}

async fn commit(layout: &InstallLayout, version: &str) -> crate::Result<()> {
    let marker = layout.partial_dir.join(COMPLETE_MARKER);
    tokio::fs::write(&marker, version)
        .await
        .map_err(|error| io_error("write the install marker", &marker, &error))?;
    remove_quietly(&layout.dir).await;
    rename_with_retry(&layout.partial_dir, &layout.dir)
        .await
        .map_err(|error| io_error("move the finished install to", &layout.dir, &error))?;
    remove_quietly(&layout.partial_zip).await;
    Ok(())
}

// Windows virus scanners hold handles on freshly extracted executables for a moment,
// which makes renaming their parent directory fail with access denied.
async fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 1;
    loop {
        match tokio::fs::rename(from, to).await {
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && attempt < RENAME_ATTEMPTS =>
            {
                attempt += 1;
                tokio::time::sleep(RENAME_RETRY_DELAY).await;
            }
            result => return result,
        }
    }
}

async fn download(
    url: &str,
    target: &Path,
    progress: &(dyn Fn(CftProgress) + Send + Sync),
) -> crate::Result<()> {
    let failed = |error: &dyn std::fmt::Display| {
        install_error(format!(
            "Could not download Chrome for Testing from {url}: {error}"
        ))
    };
    let client = tls::http_client(&[], DOWNLOAD_TIMEOUT, HttpProxy::System)
        .map_err(|error| failed(&error))?;
    let response = tokio::time::timeout(STALL_TIMEOUT, client.get(url).send())
        .await
        .map_err(|_| {
            failed(&format_args!(
                "no response within {} s",
                STALL_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|error| failed(&error))?;
    let status = response.status();
    if !status.is_success() {
        return Err(failed(&format_args!("HTTP {status}")));
    }
    let expected_length = response.content_length();
    let expected_md5 = expected_md5(response.headers());
    let (received, digest) = stream_to_file(response, target, expected_length, progress).await?;
    progress(CftProgress::Verifying);
    verify_length(url, expected_length, received)?;
    verify_md5(url, expected_md5, digest)
}

async fn stream_to_file(
    response: reqwest::Response,
    target: &Path,
    total: Option<u64>,
    progress: &(dyn Fn(CftProgress) + Send + Sync),
) -> crate::Result<(u64, [u8; 16])> {
    let mut file = tokio::fs::File::create(target)
        .await
        .map_err(|error| io_error("create the download file", target, &error))?;
    let mut stream = std::pin::pin!(response.bytes_stream());
    let mut hasher = Md5::new();
    let mut received = 0u64;
    let mut reported = 0u64;
    progress(CftProgress::Downloading { received, total });
    while let Some(chunk) = next_chunk(&mut stream, received).await? {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .map_err(|error| io_error("write the download file", target, &error))?;
        received += chunk.len() as u64;
        if received - reported >= PROGRESS_STEP {
            reported = received;
            progress(CftProgress::Downloading { received, total });
        }
    }
    file.sync_all()
        .await
        .map_err(|error| io_error("flush the download file", target, &error))?;
    if reported != received {
        progress(CftProgress::Downloading { received, total });
    }
    Ok((received, hasher.finalize().into()))
}

async fn next_chunk<C>(
    stream: &mut (impl futures_util::Stream<Item = reqwest::Result<C>> + Unpin),
    received: u64,
) -> crate::Result<Option<C>> {
    match tokio::time::timeout(STALL_TIMEOUT, stream.next()).await {
        Err(_) => Err(install_error(format!(
            "The Chrome for Testing download stalled: no data for {} s after {received} bytes",
            STALL_TIMEOUT.as_secs()
        ))),
        Ok(None) => Ok(None),
        Ok(Some(Ok(chunk))) => Ok(Some(chunk)),
        Ok(Some(Err(error))) => Err(install_error(format!(
            "The Chrome for Testing download failed after {received} bytes: {error}"
        ))),
    }
}

fn verify_length(url: &str, expected: Option<u64>, received: u64) -> crate::Result<()> {
    match expected {
        Some(length) if length != received => Err(install_error(format!(
            "The Chrome for Testing download from {url} is incomplete: received {received} of {length} bytes"
        ))),
        _ => Ok(()),
    }
}

fn verify_md5(url: &str, expected: Option<[u8; 16]>, actual: [u8; 16]) -> crate::Result<()> {
    match expected {
        Some(expected) if expected != actual => Err(install_error(format!(
            "The Chrome for Testing download from {url} is corrupt: MD5 {} does not match the server's {}",
            to_hex(&actual),
            to_hex(&expected)
        ))),
        Some(_) => Ok(()),
        None => {
            tracing::warn!(
                url,
                "the server sent no MD5 for the Chrome for Testing download; only its length was checked"
            );
            Ok(())
        }
    }
}

fn expected_md5(headers: &reqwest::header::HeaderMap) -> Option<[u8; 16]> {
    goog_hash_md5(headers).or_else(|| etag_md5(headers))
}

fn goog_hash_md5(headers: &reqwest::header::HeaderMap) -> Option<[u8; 16]> {
    let encoded = headers
        .get_all("x-goog-hash")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .find_map(|part| part.trim().strip_prefix("md5="))?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .ok()?;
    bytes.try_into().ok()
}

fn etag_md5(headers: &reqwest::header::HeaderMap) -> Option<[u8; 16]> {
    let etag = headers.get(reqwest::header::ETAG)?.to_str().ok()?;
    decode_hex_md5(etag.strip_prefix('"')?.strip_suffix('"')?)
}

fn decode_hex_md5(hex: &str) -> Option<[u8; 16]> {
    if hex.len() != 32 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0u8; 16];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(digest)
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn extract(archive: &Path, target: &Path) -> crate::Result<()> {
    let archive = archive.to_path_buf();
    let target = target.to_path_buf();
    tokio::task::spawn_blocking(move || extract_blocking(&archive, &target))
        .await
        .map_err(|error| {
            install_error(format!(
                "The Chrome for Testing extraction task failed: {error}"
            ))
        })?
}

fn extract_blocking(archive: &Path, target: &Path) -> crate::Result<()> {
    let file = std::fs::File::open(archive)
        .map_err(|error| io_error("open the downloaded archive", archive, &error))?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|error| {
        install_error(format!(
            "The Chrome for Testing archive {} is not a valid zip file: {error}",
            archive.display()
        ))
    })?;
    zip.extract(target).map_err(|error| {
        install_error(format!(
            "Could not extract Chrome for Testing into {}: {error}",
            target.display()
        ))
    })
}

#[cfg(target_os = "macos")]
async fn configure(top_dir: &Path) -> crate::Result<()> {
    let top_dir = top_dir.to_path_buf();
    tokio::task::spawn_blocking(move || quarantine::strip(&top_dir))
        .await
        .map_err(|error| {
            install_error(format!(
                "Removing the macOS quarantine from Chrome for Testing did not finish: {error}"
            ))
        })?
}

#[cfg(windows)]
async fn configure(top_dir: &Path) -> crate::Result<()> {
    if windows_sandbox::run_setup(top_dir).await {
        return Ok(());
    }
    windows_sandbox::grant_app_container_access(top_dir).await
}

#[cfg(not(any(target_os = "macos", windows)))]
async fn configure(_top_dir: &Path) -> crate::Result<()> {
    Ok(())
}

#[cfg(target_os = "macos")]
mod quarantine {
    use std::ffi::{CStr, CString};
    use std::os::unix::ffi::OsStrExt as _;
    use std::os::unix::fs::PermissionsExt as _;
    use std::path::{Path, PathBuf};

    const ATTRIBUTE: &CStr = c"com.apple.quarantine";

    pub(super) fn strip(root: &Path) -> crate::Result<()> {
        let mut pending = vec![root.to_path_buf()];
        while let Some(path) = pending.pop() {
            let metadata = std::fs::symlink_metadata(&path)
                .map_err(|error| failure("inspect", &path, &error))?;
            remove_from(&path, &metadata)?;
            if metadata.is_dir() {
                pending.extend(children(&path)?);
            }
        }
        Ok(())
    }

    fn children(dir: &Path) -> crate::Result<Vec<PathBuf>> {
        std::fs::read_dir(dir)
            .and_then(|entries| {
                entries
                    .map(|entry| entry.map(|entry| entry.path()))
                    .collect()
            })
            .map_err(|error| failure("list", dir, &error))
    }

    fn remove_from(path: &Path, metadata: &std::fs::Metadata) -> crate::Result<()> {
        match remove_attribute(path) {
            Err(error) if error.raw_os_error() == Some(libc::EACCES) && !metadata.is_symlink() => {
                remove_with_write_access(path, metadata)
            }
            result => absent_is_fine(result).map_err(|error| failure("clear", path, &error)),
        }
    }

    fn remove_with_write_access(path: &Path, metadata: &std::fs::Metadata) -> crate::Result<()> {
        let original = metadata.permissions();
        let writable = std::fs::Permissions::from_mode(original.mode() | 0o200);
        std::fs::set_permissions(path, writable)
            .map_err(|error| failure("make writable", path, &error))?;
        let removed = absent_is_fine(remove_attribute(path));
        std::fs::set_permissions(path, original)
            .map_err(|error| failure("restore the mode of", path, &error))?;
        removed.map_err(|error| failure("clear", path, &error))
    }

    fn absent_is_fine(result: std::io::Result<()>) -> std::io::Result<()> {
        match result {
            Err(error) if error.raw_os_error() == Some(libc::ENOATTR) => Ok(()),
            other => other,
        }
    }

    fn remove_attribute(path: &Path) -> std::io::Result<()> {
        let path = CString::new(path.as_os_str().as_bytes())
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
        let result =
            unsafe { libc::removexattr(path.as_ptr(), ATTRIBUTE.as_ptr(), libc::XATTR_NOFOLLOW) };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn failure(action: &str, path: &Path, error: &std::io::Error) -> crate::BrowserError {
        crate::BrowserError::io(
            format!(
                "Could not {action} {} while removing the macOS quarantine",
                path.display()
            ),
            error,
        )
    }
}

#[cfg(windows)]
mod windows_sandbox {
    use std::ffi::OsString;
    use std::path::Path;
    use std::process::Stdio;
    use std::time::Duration;

    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    const SETUP_TIMEOUT: Duration = Duration::from_secs(60);
    const APP_CONTAINER_GRANT: &str = "*S-1-15-2-1:(OI)(CI)(RX)";

    pub(super) async fn run_setup(top_dir: &Path) -> bool {
        let setup = top_dir.join("setup.exe");
        if !setup.is_file() {
            return false;
        }
        let mut argument = OsString::from("--configure-browser-in-directory=");
        argument.push(top_dir);
        let status =
            tokio::time::timeout(SETUP_TIMEOUT, hidden_command(&setup).arg(argument).status())
                .await;
        match status {
            Ok(Ok(status)) if status.success() => true,
            Ok(Ok(status)) => {
                tracing::debug!(%status, "setup.exe did not configure the Chrome for Testing sandbox; granting access with icacls");
                false
            }
            Ok(Err(error)) => {
                tracing::debug!(%error, "setup.exe could not run; granting access with icacls");
                false
            }
            Err(_) => {
                tracing::debug!("setup.exe timed out; granting access with icacls");
                false
            }
        }
    }

    pub(super) async fn grant_app_container_access(top_dir: &Path) -> crate::Result<()> {
        let status = hidden_command(Path::new("icacls"))
            .arg(top_dir)
            .args(["/grant", APP_CONTAINER_GRANT])
            .status()
            .await
            .map_err(|error| {
                super::install_error(format!(
                    "Could not run icacls to give the Chrome sandbox access to {}: {error}",
                    top_dir.display()
                ))
            })?;
        if status.success() {
            return Ok(());
        }
        Err(super::install_error(format!(
            "icacls could not give the Chrome sandbox access to {} ({status})",
            top_dir.display()
        )))
    }

    fn hidden_command(program: &Path) -> tokio::process::Command {
        let mut command = tokio::process::Command::new(program);
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .creation_flags(CREATE_NO_WINDOW);
        command
    }
}

fn scan_installs(root: &Path) -> Vec<InstalledBuild> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let (platform, version_text) = name.rsplit_once('-')?;
            Some(InstalledBuild {
                version: parse_version(version_text)?,
                platform: platform.to_owned(),
                version_text: version_text.to_owned(),
                dir: entry.path(),
            })
        })
        .collect()
}

fn remove_older_installs(root: &Path, installed: &CftRelease) {
    let Some(current) = parse_version(&installed.version) else {
        return;
    };
    let stale = scan_installs(root).into_iter().filter(|install| {
        install.platform == installed.platform
            && install.version < current
            && install.is_complete()
            && !has_live_marker(&install.dir)
    });
    for install in stale {
        if let Err(error) = remove_install(&install.dir) {
            tracing::warn!(dir = %install.dir.display(), %error, "could not remove an old Chrome for Testing build");
        }
    }
}

fn has_live_marker(install_dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(install_dir) else {
        return false;
    };
    entries
        .flatten()
        .filter_map(|entry| marker_pid(entry.file_name().to_str()?))
        .any(process_alive)
}

fn marker_pid(name: &str) -> Option<u32> {
    let pid = name.strip_prefix(IN_USE_PREFIX)?;
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    pid.parse().ok()
}

async fn remove_quietly(path: &Path) {
    let result = match tokio::fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_dir() => tokio::fs::remove_dir_all(path).await,
        Ok(_) => tokio::fs::remove_file(path).await,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = result {
        tracing::warn!(path = %path.display(), %error, "could not remove a Chrome for Testing download leftover");
    }
}

fn install_error(message: impl Into<String>) -> BrowserError {
    BrowserError::Install {
        message: message.into(),
    }
}

fn io_error(action: &str, path: &Path, error: &std::io::Error) -> BrowserError {
    BrowserError::io(format!("Could not {action} {}", path.display()), error)
}

fn path_error(action: &str, path: &Path, error: std::io::Error) -> std::io::Error {
    std::io::Error::new(
        error.kind(),
        format!("Could not {action} {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exited_pid() -> u32 {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    fn build(root: &Path, name: &str, complete: bool, owner: Option<u32>) -> PathBuf {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        if complete {
            std::fs::write(dir.join(COMPLETE_MARKER), name).unwrap();
        }
        if let Some(pid) = owner {
            std::fs::write(dir.join(format!("{IN_USE_PREFIX}{pid}")), "").unwrap();
        }
        dir
    }

    fn with_executable(dir: &Path, platform: &str) {
        let path = dir.join(executable_relative_path(platform).unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "binary").unwrap();
    }

    fn names(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn headers(pairs: &[(&str, &str)]) -> reqwest::header::HeaderMap {
        let mut map = reqwest::header::HeaderMap::new();
        for (name, value) in pairs {
            map.append(
                reqwest::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                reqwest::header::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn platform_mapping_covers_every_cft_build_and_rejects_windows_on_arm() {
        let table = [
            ("macos", "aarch64", Some("mac-arm64")),
            ("macos", "x86_64", Some("mac-x64")),
            ("linux", "x86_64", Some("linux64")),
            ("linux", "aarch64", Some("linux-arm64")),
            ("windows", "x86_64", Some("win64")),
            ("windows", "x86", Some("win32")),
            ("windows", "aarch64", None),
            ("linux", "riscv64", None),
            ("freebsd", "x86_64", None),
        ];
        for (os, arch, expected) in table {
            assert_eq!(platform_for(os, arch), expected, "{os}/{arch}");
        }
        assert_eq!(
            platform(),
            platform_for(std::env::consts::OS, std::env::consts::ARCH)
        );
    }

    #[test]
    fn unavailable_platforms_get_an_actionable_message() {
        assert_eq!(
            unavailable_message("windows", "aarch64"),
            "Chrome for Testing is not available for Windows on ARM; install Microsoft Edge or Google Chrome"
        );
        assert_eq!(
            unavailable_message("linux", "riscv64"),
            "Chrome for Testing is not available for linux on riscv64; install Google Chrome or Microsoft Edge"
        );
    }

    #[test]
    fn versions_have_exactly_four_numeric_parts() {
        assert_eq!(parse_version("154.0.8037.92"), Some([154, 0, 8037, 92]));
        assert_eq!(parse_version(CFT_PINNED), Some(pinned_version()));
        for invalid in [
            "",
            "154.0.8037",
            "154.0.8037.92.1",
            "154.0.8037.x",
            "154.0.+8037.92",
            "154..8037.92",
            "../154.0.8037.92",
            "154.0.8037.99999999999",
        ] {
            assert_eq!(parse_version(invalid), None, "{invalid}");
        }
        assert!(parse_version("155.0.8059.12") > parse_version("154.0.8037.92"));
        assert!(parse_version("154.0.8037.100") > parse_version("154.0.8037.92"));
    }

    #[test]
    fn executable_paths_follow_the_cft_zip_layout() {
        let mac = "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";
        let expected = [
            ("mac-arm64", format!("chrome-mac-arm64/{mac}")),
            ("mac-x64", format!("chrome-mac-x64/{mac}")),
            ("linux64", "chrome-linux64/chrome".to_owned()),
            ("linux-arm64", "chrome-linux-arm64/chrome".to_owned()),
            ("win64", "chrome-win64/chrome.exe".to_owned()),
            ("win32", "chrome-win32/chrome.exe".to_owned()),
        ];
        for (platform, path) in expected {
            assert_eq!(
                executable_relative_path(platform),
                Some(PathBuf::from(path)),
                "{platform}"
            );
        }
        assert_eq!(executable_relative_path("win-arm64"), None);
        assert_eq!(executable_relative_path("../linux64"), None);
    }

    #[test]
    fn in_use_markers_name_their_owner_pid() {
        assert_eq!(marker_pid(".in-use-4242"), Some(4242));
        for invalid in [
            ".in-use-",
            ".in-use-4242-5f1c",
            ".in-use-+12",
            ".in-use-abc",
            ".complete",
            "in-use-12",
        ] {
            assert_eq!(marker_pid(invalid), None, "{invalid}");
        }
    }

    #[test]
    fn expected_md5_reads_goog_hash_then_a_hex_etag() {
        let digest: [u8; 16] = Md5::digest(b"chrome").into();
        let encoded = base64::engine::general_purpose::STANDARD.encode(digest);
        let hex = to_hex(&digest);
        let separate = headers(&[
            ("x-goog-hash", "crc32c=n03x6A=="),
            ("x-goog-hash", &format!("md5={encoded}")),
        ]);
        assert_eq!(expected_md5(&separate), Some(digest));
        let combined = headers(&[("x-goog-hash", &format!("crc32c=n03x6A==, md5={encoded}"))]);
        assert_eq!(expected_md5(&combined), Some(digest));
        let etag = headers(&[("etag", &format!("\"{hex}\""))]);
        assert_eq!(expected_md5(&etag), Some(digest));
        let both = headers(&[
            ("x-goog-hash", &format!("md5={encoded}")),
            ("etag", "\"00000000000000000000000000000000\""),
        ]);
        assert_eq!(expected_md5(&both), Some(digest));
        for unusable in [
            headers(&[]),
            headers(&[("etag", &format!("W/\"{hex}\""))]),
            headers(&[("etag", "\"CJjIvqjU/4MDEAE=\"")]),
            headers(&[("etag", &format!("\"{}\"", &hex[..30]))]),
            headers(&[("x-goog-hash", "crc32c=n03x6A==")]),
            headers(&[("x-goog-hash", "md5=AAAA")]),
        ] {
            assert_eq!(expected_md5(&unusable), None, "{unusable:?}");
        }
    }

    #[test]
    fn old_build_gc_keeps_live_newer_incomplete_and_foreign_builds() {
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path();
        let dead = exited_pid();
        build(root, "linux64-150.0.0.1", true, None);
        build(root, "linux64-151.0.0.1", true, Some(std::process::id()));
        build(root, "linux64-152.0.0.1", true, Some(dead));
        build(root, "linux64-153.0.0.1", false, None);
        build(root, "mac-arm64-150.0.0.1", true, None);
        build(root, "linux64-156.0.0.1", true, None);
        build(root, "linux64-155.0.8059.12", true, None);
        std::fs::write(root.join("linux64-149.0.0.1.partial.zip"), "partial").unwrap();

        remove_older_installs(
            root,
            &CftRelease {
                version: "155.0.8059.12".to_owned(),
                url: download_url("155.0.8059.12", "linux64"),
                platform: "linux64".to_owned(),
            },
        );

        assert_eq!(
            names(root),
            [
                "linux64-149.0.0.1.partial.zip",
                "linux64-151.0.0.1",
                "linux64-153.0.0.1",
                "linux64-155.0.8059.12",
                "linux64-156.0.0.1",
                "mac-arm64-150.0.0.1",
            ]
        );
    }

    // Named so that APFS and sorted listings return it before .complete, which is the order
    // in which a removal that deletes the marker last keeps it.
    #[cfg(unix)]
    fn undeletable_dir(install: &Path) -> Option<PathBuf> {
        use std::os::unix::fs::PermissionsExt as _;
        let busy = install.join("-busy");
        std::fs::create_dir_all(&busy).unwrap();
        std::fs::write(busy.join("resources.pak"), "pak").unwrap();
        std::fs::set_permissions(&busy, std::fs::Permissions::from_mode(0o555)).unwrap();
        let permissions_bypassed = std::fs::write(busy.join("probe"), "").is_ok();
        (!permissions_bypassed).then_some(busy)
    }

    #[cfg(unix)]
    fn make_deletable(dir: &Path) {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_partly_removed_old_build_no_longer_counts_as_installed() {
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path().join(CACHE_SUBDIR);
        let old = CftRelease {
            version: "150.0.0.1".to_owned(),
            url: download_url("150.0.0.1", "linux64"),
            platform: "linux64".to_owned(),
        };
        let stuck = build(&root, "linux64-150.0.0.1", true, None);
        with_executable(&stuck, "linux64");
        let Some(busy) = undeletable_dir(&stuck) else {
            return;
        };
        let layout = InstallLayout::new(&old, cache.path()).unwrap();
        assert!(layout.installed().is_some());

        remove_older_installs(&root, &pinned_release("linux64"));
        make_deletable(&busy);

        assert!(busy.join("resources.pak").is_file());
        assert!(layout.installed().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn remove_all_tries_every_entry_and_names_the_one_it_could_not_remove() {
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path().join(CACHE_SUBDIR);
        let stuck = build(&root, "linux64-150.0.0.1", true, None);
        build(&root, "linux64-151.0.0.1", true, None);
        build(&root, "linux64-152.0.0.1", true, None);
        std::fs::write(root.join("linux64-153.0.0.1.partial.zip"), "partial").unwrap();
        let Some(busy) = undeletable_dir(&stuck) else {
            return;
        };

        let error = remove_all(cache.path()).unwrap_err();
        make_deletable(&busy);

        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let message = error.to_string();
        assert!(
            message.starts_with(&format!("Could not remove {}: ", stuck.display())),
            "{message}"
        );
        assert_eq!(names(&root), ["linux64-150.0.0.1"]);
        assert!(!stuck.join(COMPLETE_MARKER).exists());
    }

    #[test]
    fn find_cached_picks_the_newest_complete_build_at_or_above_the_floor() {
        let Some(platform) = platform() else {
            return;
        };
        let cache = tempfile::tempdir().unwrap();
        assert!(find_cached(cache.path()).is_none());
        let root = cache.path().join(CACHE_SUBDIR);
        let foreign = if platform == "linux64" {
            "mac-arm64"
        } else {
            "linux64"
        };
        for (name, complete, executable_platform) in [
            (format!("{platform}-153.0.8010.52"), true, Some(platform)),
            (format!("{platform}-{CFT_PINNED}"), true, Some(platform)),
            (format!("{platform}-155.0.8059.12"), true, Some(platform)),
            (format!("{platform}-156.0.8078.3"), false, Some(platform)),
            (format!("{platform}-157.0.1.1"), true, None),
            (format!("{foreign}-158.0.0.1"), true, Some(foreign)),
        ] {
            let dir = build(&root, &name, complete, None);
            if let Some(executable_platform) = executable_platform {
                with_executable(&dir, executable_platform);
            }
        }

        let found = find_cached(cache.path()).unwrap();

        let install = root.join(format!("{platform}-155.0.8059.12"));
        assert_eq!(
            found.path,
            install.join(executable_relative_path(platform).unwrap())
        );
        assert_eq!(found.version.as_deref(), Some("155.0.8059.12"));
        assert_eq!(found.flavor, Flavor::ChromeForTesting);
        assert_eq!(found.source, ExecutableSource::CachedCft);
    }

    #[test]
    fn find_cached_ignores_builds_below_the_floor() {
        let Some(platform) = platform() else {
            return;
        };
        let cache = tempfile::tempdir().unwrap();
        let root = cache.path().join(CACHE_SUBDIR);
        let dir = build(&root, &format!("{platform}-154.0.8037.57"), true, None);
        with_executable(&dir, platform);
        assert!(find_cached(cache.path()).is_none());
    }

    #[test]
    fn install_futures_are_send() {
        fn assert_send<T: Send>(_: &T) {}
        let cache = Path::new("unused");
        let pending_install = install(CftVersion::Pinned, cache, |_| {});
        assert_send(&pending_install);
        let release = pinned_release("linux64");
        let pending_release = install_release(&release, cache, |_| {});
        assert_send(&pending_release);
    }

    #[cfg(target_os = "macos")]
    fn c_path(path: &Path) -> std::ffi::CString {
        use std::os::unix::ffi::OsStrExt as _;
        std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap()
    }

    #[cfg(target_os = "macos")]
    fn mark_quarantined(path: &Path) {
        let value = b"0081;00000000;Chrome;";
        let result = unsafe {
            libc::setxattr(
                c_path(path).as_ptr(),
                c"com.apple.quarantine".as_ptr(),
                value.as_ptr().cast(),
                value.len(),
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        assert_eq!(result, 0, "{}", std::io::Error::last_os_error());
    }

    #[cfg(target_os = "macos")]
    fn quarantined(path: &Path) -> bool {
        let size = unsafe {
            libc::getxattr(
                c_path(path).as_ptr(),
                c"com.apple.quarantine".as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                libc::XATTR_NOFOLLOW,
            )
        };
        size >= 0
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_is_stripped_from_dirs_files_links_and_read_only_files() {
        use std::os::unix::fs::PermissionsExt as _;

        let temp = tempfile::tempdir().unwrap();
        let top = temp.path().join("chrome-mac-arm64");
        let nested = top.join("Framework/Versions/1.0");
        std::fs::create_dir_all(&nested).unwrap();
        let read_only = nested.join("resources.pak");
        let writable = top.join("chrome");
        let link = top.join("Current");
        std::fs::write(&read_only, "pak").unwrap();
        std::fs::write(&writable, "binary").unwrap();
        std::os::unix::fs::symlink("Framework/Versions/1.0", &link).unwrap();
        let paths = [
            top.clone(),
            nested.clone(),
            read_only.clone(),
            writable,
            link,
        ];
        for path in &paths {
            mark_quarantined(path);
        }
        std::fs::set_permissions(&read_only, std::fs::Permissions::from_mode(0o444)).unwrap();

        quarantine::strip(&top).unwrap();

        for path in &paths {
            assert!(!quarantined(path), "{}", path.display());
        }
        let mode = std::fs::metadata(&read_only).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o444);
    }
}
