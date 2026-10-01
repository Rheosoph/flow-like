use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use flow_like_browser::BrowserError;
use flow_like_browser::launch::cft::{self, CFT_PINNED, CftProgress, CftRelease};
use flow_like_browser::launch::{ExecutableSource, Flavor};
use md5::{Digest as _, Md5};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use zip::write::SimpleFileOptions;

const FIXTURE: &str = include_str!("fixtures/cft/last-known-good-versions-with-downloads.json");
const PLATFORMS: [&str; 6] = [
    "mac-arm64",
    "mac-x64",
    "linux64",
    "linux-arm64",
    "win64",
    "win32",
];
const VERSION: &str = "155.0.8059.12";
const LINUX_TOP: &str = "chrome-linux64";

struct Reply {
    status: &'static str,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    declared_length: usize,
}

impl Reply {
    fn new(body: Vec<u8>, headers: Vec<(&'static str, String)>) -> Self {
        Self {
            status: "200 OK",
            headers,
            declared_length: body.len(),
            body,
        }
    }

    fn storage(body: Vec<u8>) -> Self {
        let md5 = base64::engine::general_purpose::STANDARD.encode(Md5::digest(&body));
        Self::new(
            body,
            vec![
                ("x-goog-hash", "crc32c=n03x6A==".to_owned()),
                ("x-goog-hash", format!("md5={md5}")),
            ],
        )
    }
}

struct Server {
    url: String,
    hits: Arc<AtomicUsize>,
}

async fn serve(reply: Reply) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            counter.fetch_add(1, Ordering::SeqCst);
            let reply = reply.clone();
            tokio::spawn(async move { respond(stream, &reply).await });
        }
    });
    Server {
        url: format!("http://{address}/chrome-for-testing-public/{VERSION}/chrome.zip"),
        hits,
    }
}

async fn respond(mut stream: TcpStream, reply: &Reply) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let mut head = format!(
        "HTTP/1.1 {}\r\ncontent-length: {}\r\nconnection: close\r\n",
        reply.status, reply.declared_length
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).await?;
    stream.write_all(&reply.body).await?;
    stream.shutdown().await
}

fn archive(top: &str, executable: &str) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let deflated =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    writer
        .add_directory(format!("{top}/"), deflated.unix_permissions(0o755))
        .unwrap();
    writer
        .start_file(
            format!("{top}/{executable}"),
            deflated.unix_permissions(0o755),
        )
        .unwrap();
    writer.write_all(b"#!/bin/sh\nexit 0\n").unwrap();
    writer
        .start_file(
            format!("{top}/Framework/Versions/{VERSION}/Resources/about.txt"),
            deflated.unix_permissions(0o644),
        )
        .unwrap();
    writer.write_all(b"about").unwrap();
    if cfg!(unix) {
        writer
            .add_symlink(
                format!("{top}/Framework/Versions/Current"),
                VERSION,
                SimpleFileOptions::default(),
            )
            .unwrap();
        writer
            .add_symlink(
                format!("{top}/Framework/Resources"),
                "Versions/Current/Resources",
                SimpleFileOptions::default(),
            )
            .unwrap();
    }
    writer.finish().unwrap().into_inner()
}

fn release(server: &Server, platform: &str) -> CftRelease {
    CftRelease {
        version: VERSION.to_owned(),
        url: server.url.clone(),
        platform: platform.to_owned(),
    }
}

type Events = Arc<Mutex<Vec<CftProgress>>>;

fn recorder() -> (Events, impl Fn(CftProgress) + Send + Sync) {
    let events: Events = Arc::default();
    let sink = events.clone();
    (events, move |event| sink.lock().unwrap().push(event))
}

fn cache_root(cache: &Path) -> PathBuf {
    cache.join("chrome-for-testing")
}

fn entries(dir: &Path) -> Vec<String> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn install_message(result: flow_like_browser::Result<impl std::fmt::Debug>) -> String {
    match result {
        Err(BrowserError::Install { message }) => message,
        other => panic!("expected an Install error, got {other:?}"),
    }
}

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

fn complete_install(root: &Path, name: &str, owner: Option<u32>) {
    let dir = root.join(name);
    std::fs::create_dir_all(dir.join(LINUX_TOP)).unwrap();
    std::fs::write(dir.join(".complete"), name).unwrap();
    std::fs::write(dir.join(LINUX_TOP).join("chrome"), "binary").unwrap();
    if let Some(pid) = owner {
        std::fs::write(dir.join(format!(".in-use-{pid}")), "").unwrap();
    }
}

fn stage(event: &CftProgress) -> Option<String> {
    match event {
        CftProgress::Downloading { .. } => None,
        CftProgress::Done { version } => Some(format!("done {version}")),
        other => Some(format!("{other:?}").to_lowercase()),
    }
}

#[test]
fn parse_last_known_good_raises_an_older_stable_to_the_pinned_floor() {
    for platform in PLATFORMS {
        let release = cft::parse_last_known_good(FIXTURE, platform).unwrap();
        assert_eq!(release.version, CFT_PINNED, "{platform}");
        assert_eq!(release.url, cft::download_url(CFT_PINNED, platform));
        assert_eq!(release.platform, platform);
    }
}

#[test]
fn parse_last_known_good_takes_the_chrome_download_of_a_newer_stable() {
    let fixture: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let mut document = fixture.clone();
    document["channels"]["Stable"] = fixture["channels"]["Beta"].clone();
    document["channels"]["Stable"]["channel"] = "Stable".into();
    document["channels"]["Stable"]["downloads"]["chrome-headless-shell"] =
        fixture["channels"]["Stable"]["downloads"]["chrome-headless-shell"].clone();
    let json = document.to_string();

    for platform in PLATFORMS {
        let release = cft::parse_last_known_good(&json, platform).unwrap();
        assert_eq!(release.version, VERSION);
        assert_eq!(release.url, cft::download_url(VERSION, platform));
        assert_eq!(release.platform, platform);
    }
}

#[test]
fn parse_last_known_good_rejects_unusable_documents() {
    let stable = |version: &str, platform: &str, url: &str| {
        serde_json::json!({
            "channels": { "Stable": {
                "version": version,
                "downloads": { "chrome": [{ "platform": platform, "url": url }] }
            }}
        })
        .to_string()
    };
    let https = cft::download_url(VERSION, "linux64");
    let cases = [
        ("{".to_owned(), "not valid JSON"),
        (
            r#"{"channels":{"Beta":{"version":"155.0.8059.12"}}}"#.to_owned(),
            "no Stable channel",
        ),
        (
            stable("latest", "linux64", &https),
            "'latest' is not a version number",
        ),
        (
            stable(VERSION, "win64", &https),
            "155.0.8059.12 has no download for linux64",
        ),
        (
            stable(
                VERSION,
                "linux64",
                "http://storage.googleapis.com/chrome.zip",
            ),
            "not an https URL",
        ),
    ];
    for (json, expected) in cases {
        let message = install_message(cft::parse_last_known_good(&json, "linux64"));
        assert!(message.contains(expected), "{message:?} lacks {expected:?}");
    }
}

#[test]
fn download_url_follows_the_storage_layout() {
    assert_eq!(
        cft::download_url(CFT_PINNED, "mac-arm64"),
        "https://storage.googleapis.com/chrome-for-testing-public/154.0.8037.92/mac-arm64/chrome-mac-arm64.zip"
    );
}

#[test]
fn platform_is_a_cft_build_except_on_windows_on_arm() {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => assert_eq!(cft::platform(), None),
        _ => assert!(PLATFORMS.contains(&cft::platform().unwrap())),
    }
}

#[tokio::test]
async fn install_release_unpacks_modes_symlinks_and_the_complete_marker() {
    let body = archive(LINUX_TOP, "chrome");
    let length = body.len() as u64;
    let server = serve(Reply::storage(body)).await;
    let cache = tempfile::tempdir().unwrap();
    let (events, progress) = recorder();

    let executable = cft::install_release(&release(&server, "linux64"), cache.path(), progress)
        .await
        .unwrap();

    let install = cache_root(cache.path()).join(format!("linux64-{VERSION}"));
    assert_eq!(executable.path, install.join(LINUX_TOP).join("chrome"));
    assert_eq!(executable.flavor, Flavor::ChromeForTesting);
    assert_eq!(executable.source, ExecutableSource::CachedCft);
    assert_eq!(executable.version.as_deref(), Some(VERSION));
    assert!(executable.path.is_file());
    assert_eq!(
        std::fs::read_to_string(install.join(".complete")).unwrap(),
        VERSION
    );
    assert_eq!(
        entries(&cache_root(cache.path())),
        [format!("linux64-{VERSION}")]
    );

    let events = events.lock().unwrap();
    assert!(matches!(
        events.first(),
        Some(CftProgress::Downloading { received: 0, total: Some(total) }) if *total == length
    ));
    assert!(events.iter().any(|event| matches!(
        event,
        CftProgress::Downloading { received, .. } if *received == length
    )));
    let stages: Vec<String> = events.iter().filter_map(stage).collect();
    assert_eq!(
        stages,
        [
            "verifying",
            "extracting",
            "configuring",
            &format!("done {VERSION}")
        ]
    );
}

#[cfg(unix)]
#[tokio::test]
async fn install_release_keeps_unix_modes_and_framework_symlinks() {
    use std::os::unix::fs::PermissionsExt as _;
    const MAC_TOP: &str = "chrome-mac-arm64";
    const MAC_EXECUTABLE: &str =
        "Google Chrome for Testing.app/Contents/MacOS/Google Chrome for Testing";

    let server = serve(Reply::storage(archive(MAC_TOP, MAC_EXECUTABLE))).await;
    let cache = tempfile::tempdir().unwrap();

    let executable = cft::install_release(&release(&server, "mac-arm64"), cache.path(), |_| {})
        .await
        .unwrap();

    let top = cache_root(cache.path())
        .join(format!("mac-arm64-{VERSION}"))
        .join(MAC_TOP);
    assert_eq!(executable.path, top.join(MAC_EXECUTABLE));
    let mode = std::fs::metadata(&executable.path)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o755);
    let about = top.join(format!("Framework/Versions/{VERSION}/Resources/about.txt"));
    assert_eq!(
        std::fs::metadata(&about).unwrap().permissions().mode() & 0o777,
        0o644
    );
    let current = top.join("Framework/Versions/Current");
    assert!(std::fs::symlink_metadata(&current).unwrap().is_symlink());
    assert_eq!(std::fs::read_link(&current).unwrap(), Path::new(VERSION));
    let resources = top.join("Framework/Resources");
    assert!(std::fs::symlink_metadata(&resources).unwrap().is_symlink());
    assert_eq!(
        std::fs::read_to_string(resources.join("about.txt")).unwrap(),
        "about"
    );
}

#[tokio::test]
async fn install_release_reuses_a_complete_install_without_downloading() {
    let server = serve(Reply::storage(archive(LINUX_TOP, "chrome"))).await;
    let cache = tempfile::tempdir().unwrap();
    let first = cft::install_release(&release(&server, "linux64"), cache.path(), |_| {})
        .await
        .unwrap();
    let (events, progress) = recorder();

    let second = cft::install_release(&release(&server, "linux64"), cache.path(), progress)
        .await
        .unwrap();

    assert_eq!(second.path, first.path);
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    let stages: Vec<String> = events.lock().unwrap().iter().filter_map(stage).collect();
    assert_eq!(stages, [format!("done {VERSION}")]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_installs_of_one_release_download_once() {
    let server = serve(Reply::storage(archive(LINUX_TOP, "chrome"))).await;
    let cache = tempfile::tempdir().unwrap();
    let release = release(&server, "linux64");

    let (first, second) = tokio::join!(
        cft::install_release(&release, cache.path(), |_| {}),
        cft::install_release(&release, cache.path(), |_| {}),
    );

    assert_eq!(first.unwrap().path, second.unwrap().path);
    assert_eq!(server.hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        entries(&cache_root(cache.path())),
        [format!("linux64-{VERSION}")]
    );
}

#[tokio::test]
async fn install_release_replaces_leftovers_of_an_interrupted_install() {
    let server = serve(Reply::storage(archive(LINUX_TOP, "chrome"))).await;
    let cache = tempfile::tempdir().unwrap();
    let root = cache_root(cache.path());
    let name = format!("linux64-{VERSION}");
    std::fs::create_dir_all(root.join(&name)).unwrap();
    std::fs::write(root.join(&name).join("stale"), "stale").unwrap();
    std::fs::create_dir_all(root.join(format!("{name}.partial"))).unwrap();
    std::fs::write(root.join(format!("{name}.partial.zip")), "truncated").unwrap();

    let executable = cft::install_release(&release(&server, "linux64"), cache.path(), |_| {})
        .await
        .unwrap();

    assert!(executable.path.is_file());
    assert_eq!(entries(&root), std::slice::from_ref(&name));
    assert!(!root.join(&name).join("stale").exists());
    assert!(root.join(&name).join(".complete").is_file());
}

#[tokio::test]
async fn install_release_rejects_an_md5_mismatch_and_cleans_up() {
    let body = archive(LINUX_TOP, "chrome");
    let mut reply = Reply::storage(b"a different archive".to_vec());
    reply.declared_length = body.len();
    reply.body = body;
    let server = serve(reply).await;
    let cache = tempfile::tempdir().unwrap();

    let message = install_message(
        cft::install_release(&release(&server, "linux64"), cache.path(), |_| {}).await,
    );

    assert!(message.contains("is corrupt: MD5"), "{message}");
    assert!(entries(&cache_root(cache.path())).is_empty());
}

#[tokio::test]
async fn install_release_checks_a_hex_etag_when_goog_hash_is_missing() {
    let body = archive(LINUX_TOP, "chrome");
    let hex = |bytes: &[u8]| -> String { bytes.iter().map(|byte| format!("{byte:02x}")).collect() };
    let good = hex(&Md5::digest(&body));
    let bad = hex(&Md5::digest(b"other"));
    let accepted = serve(Reply::new(
        body.clone(),
        vec![("etag", format!("\"{good}\""))],
    ))
    .await;
    let rejected = serve(Reply::new(body, vec![("etag", format!("\"{bad}\""))])).await;
    let good_cache = tempfile::tempdir().unwrap();
    let bad_cache = tempfile::tempdir().unwrap();

    let executable =
        cft::install_release(&release(&accepted, "linux64"), good_cache.path(), |_| {})
            .await
            .unwrap();
    let message = install_message(
        cft::install_release(&release(&rejected, "linux64"), bad_cache.path(), |_| {}).await,
    );

    assert!(executable.path.is_file());
    assert!(
        message.contains(&format!("does not match the server's {bad}")),
        "{message}"
    );
    assert!(entries(&cache_root(bad_cache.path())).is_empty());
}

#[tokio::test]
async fn install_release_cleans_up_a_truncated_download() {
    let mut reply = Reply::storage(archive(LINUX_TOP, "chrome"));
    reply.declared_length += 4096;
    let server = serve(reply).await;
    let cache = tempfile::tempdir().unwrap();

    let message = install_message(
        cft::install_release(&release(&server, "linux64"), cache.path(), |_| {}).await,
    );

    assert!(
        message.contains("download failed after") || message.contains("is incomplete"),
        "{message}"
    );
    assert!(entries(&cache_root(cache.path())).is_empty());
}

#[tokio::test]
async fn install_release_reports_http_errors() {
    let mut reply = Reply::new(b"no such object".to_vec(), Vec::new());
    reply.status = "404 Not Found";
    let server = serve(reply).await;
    let cache = tempfile::tempdir().unwrap();

    let message = install_message(
        cft::install_release(&release(&server, "linux64"), cache.path(), |_| {}).await,
    );

    assert!(message.contains("HTTP 404"), "{message}");
    assert!(message.contains(&server.url), "{message}");
    assert!(entries(&cache_root(cache.path())).is_empty());
}

#[tokio::test]
async fn install_release_rejects_an_archive_without_the_executable() {
    let server = serve(Reply::storage(archive(LINUX_TOP, "chromium"))).await;
    let cache = tempfile::tempdir().unwrap();

    let message = install_message(
        cft::install_release(&release(&server, "linux64"), cache.path(), |_| {}).await,
    );

    assert!(message.contains("has no chrome-linux64"), "{message}");
    assert!(entries(&cache_root(cache.path())).is_empty());
}

#[tokio::test]
async fn install_release_rejects_unsafe_release_fields_before_touching_disk() {
    let cache = tempfile::tempdir().unwrap();
    let url = cft::download_url(VERSION, "linux64");
    let cases = [
        ("../../escape", "linux64", "is not a version number"),
        (
            VERSION,
            "win-arm64",
            "Unknown Chrome for Testing platform 'win-arm64'",
        ),
        (VERSION, "../linux64", "Unknown Chrome for Testing platform"),
    ];
    for (version, platform, expected) in cases {
        let release = CftRelease {
            version: version.to_owned(),
            url: url.clone(),
            platform: platform.to_owned(),
        };
        let message = install_message(cft::install_release(&release, cache.path(), |_| {}).await);
        assert!(message.contains(expected), "{message:?} lacks {expected:?}");
    }
    assert!(entries(cache.path()).is_empty());
}

#[test]
fn remove_all_keeps_installs_used_by_live_processes() {
    let cache = tempfile::tempdir().unwrap();
    let root = cache_root(cache.path());
    complete_install(&root, "linux64-150.0.0.1", Some(std::process::id()));
    complete_install(&root, "linux64-151.0.0.1", Some(exited_pid()));
    complete_install(&root, "linux64-152.0.0.1", None);
    std::fs::create_dir_all(root.join("linux64-153.0.0.1.partial")).unwrap();
    std::fs::write(root.join("linux64-153.0.0.1.partial.zip"), "partial").unwrap();

    cft::remove_all(cache.path()).unwrap();

    assert_eq!(entries(&root), ["linux64-150.0.0.1"]);
    assert!(
        root.join("linux64-150.0.0.1/chrome-linux64/chrome")
            .is_file()
    );
}

#[test]
fn remove_all_removes_the_cache_when_nothing_is_in_use() {
    let cache = tempfile::tempdir().unwrap();
    let root = cache_root(cache.path());
    complete_install(&root, "linux64-150.0.0.1", Some(exited_pid()));
    complete_install(&root, "mac-arm64-151.0.0.1", None);

    cft::remove_all(cache.path()).unwrap();

    assert!(!root.exists());
    cft::remove_all(cache.path()).unwrap();
}
