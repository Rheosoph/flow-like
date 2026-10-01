//! Windows only: installs Chrome for Testing from an archive without `setup.exe`, so
//! `cft::install_release` has to give the browser sandbox access to the folder with its icacls
//! fallback (spec §2.25, §6.4 item 21).

use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use flow_like_browser::launch::cft::{self, CFT_PINNED, CftRelease, CftVersion};
use flow_like_browser::launch::{Executable, Flavor};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};

pub const INSTALL_BUDGET: Duration = Duration::from_secs(10 * 60);
const PLATFORM: &str = "win64";
const TOP_DIR: &str = "chrome-win64";
const SETUP: &str = "setup.exe";
/// ALL APPLICATION PACKAGES, the SID the icacls fallback grants `(OI)(CI)(RX)`.
const APP_CONTAINER_SID: &str = "S-1-15-2-1";
const ACL_PATH_ENV: &str = "FLOW_LIKE_E2E_ACL_PATH";
const ACL_SCRIPT: &str = "$ErrorActionPreference = 'Stop'; \
    (Get-Acl -LiteralPath $env:FLOW_LIKE_E2E_ACL_PATH).GetAccessRules($true, $true, \
    [System.Security.Principal.SecurityIdentifier]) | ForEach-Object { '{0}|{1}|{2}|{3}' -f \
    $_.IdentityReference.Value, $_.IsInherited, $_.FileSystemRights, $_.InheritanceFlags }";
const MAX_REQUEST_HEAD: usize = 64 * 1024;
const REMOVE_ATTEMPTS: u32 = 60;
const REMOVE_RETRY: Duration = Duration::from_millis(500);

#[derive(Debug)]
pub struct AclRule {
    inherited: bool,
    rights: String,
    inheritance: String,
}

impl AclRule {
    fn parse(line: &str) -> Option<(String, AclRule)> {
        let mut fields = line.trim().split('|');
        let sid = fields.next()?.to_owned();
        let rule = AclRule {
            inherited: fields.next()?.eq_ignore_ascii_case("true"),
            rights: fields.next()?.to_owned(),
            inheritance: fields.next()?.to_owned(),
        };
        Some((sid, rule))
    }

    fn reads_and_executes(&self) -> bool {
        self.rights.contains("ReadAndExecute") || self.rights.contains("FullControl")
    }

    fn reaches_files_and_folders(&self) -> bool {
        self.inheritance.contains("ContainerInherit") && self.inheritance.contains("ObjectInherit")
    }
}

/// A folder under %LOCALAPPDATA%, where a desktop install goes and the sandbox has no access.
pub struct Workspace {
    dir: tempfile::TempDir,
}

impl Workspace {
    pub fn create() -> Workspace {
        let local = std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .expect("LOCALAPPDATA is set on Windows");
        let dir = tempfile::Builder::new()
            .prefix("flow-like-e2e-icacls-")
            .tempdir_in(&local)
            .unwrap_or_else(|error| panic!("creating a folder in {}: {error}", local.display()));
        Workspace { dir }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub async fn install_without_setup(&self, e2e_executable: &Executable) -> Executable {
        let top = self.source_top_dir(e2e_executable).await;
        let archive = self.path().join(format!("{TOP_DIR}.zip"));
        let written = archive.clone();
        tokio::task::spawn_blocking(move || zip_without_setup(&top, &written))
            .await
            .expect("the archive task finished")
            .unwrap_or_else(|error| panic!("writing {}: {error}", archive.display()));
        let server = ArchiveServer::start(archive.clone()).await;
        let release = CftRelease {
            version: CFT_PINNED.to_owned(),
            url: server.url.clone(),
            platform: PLATFORM.to_owned(),
        };
        let cache = self.path().join("cache");
        let installed = cft::install_release(&release, &cache, |_| {}).await;
        server.task.abort();
        if let Err(error) = std::fs::remove_file(&archive) {
            eprintln!("could not remove {}: {error}", archive.display());
        }
        installed.unwrap_or_else(|error| {
            panic!(
                "installing Chrome for Testing without {SETUP} into {} failed: {error}",
                cache.display()
            )
        })
    }

    /// The provisioned Chrome for Testing when the e2e run uses one, otherwise a fresh pinned install.
    async fn source_top_dir(&self, e2e_executable: &Executable) -> PathBuf {
        if let Some(top) = cft_top_dir(e2e_executable) {
            return top;
        }
        let release = cft::resolve(&CftVersion::Pinned)
            .await
            .expect("the pinned Chrome for Testing release resolves");
        let cache = self.path().join("source");
        let source = cft::install_release(&release, &cache, |_| {})
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "installing the pinned Chrome for Testing into {} failed: {error}",
                    cache.display()
                )
            });
        cft_top_dir(&source).expect("a Chrome for Testing install lives in chrome-win64")
    }

    pub async fn remove(self) {
        let path = self.path().to_path_buf();
        let mut last_error = None;
        for _ in 0..REMOVE_ATTEMPTS {
            match std::fs::remove_dir_all(&path) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) => last_error = Some(error),
            }
            tokio::time::sleep(REMOVE_RETRY).await;
        }
        panic!(
            "{} is still there after the browser closed: {last_error:?}",
            path.display()
        );
    }
}

fn cft_top_dir(executable: &Executable) -> Option<PathBuf> {
    let top = executable.path.parent()?;
    (executable.flavor == Flavor::ChromeForTesting && top.file_name()? == TOP_DIR)
        .then(|| top.to_path_buf())
}

pub async fn assert_icacls_grant(executable: &Executable) {
    let top = executable
        .path
        .parent()
        .expect("chrome.exe lives in chrome-win64");
    assert!(
        !top.join(SETUP).exists(),
        "{} exists, so the installer could have run it instead of icacls",
        top.join(SETUP).display()
    );
    let folder = app_container_rules(top).await;
    assert!(
        folder.iter().any(|rule| !rule.inherited
            && rule.reads_and_executes()
            && rule.reaches_files_and_folders()),
        "{} has no explicit (OI)(CI)(RX) rule for {APP_CONTAINER_SID} after the install without {SETUP}: {folder:?}",
        top.display()
    );
    let file = app_container_rules(&executable.path).await;
    assert!(
        file.iter()
            .any(|rule| rule.inherited && rule.reads_and_executes()),
        "{} did not inherit read and execute for {APP_CONTAINER_SID}: {file:?}",
        executable.path.display()
    );
}

pub async fn app_container_rules(path: &Path) -> Vec<AclRule> {
    let output = tokio::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            ACL_SCRIPT,
        ])
        .env(ACL_PATH_ENV, path)
        .output()
        .await
        .unwrap_or_else(|error| {
            panic!(
                "running PowerShell to read the ACL of {}: {error}",
                path.display()
            )
        });
    assert!(
        output.status.success(),
        "reading the ACL of {} failed ({}): {}",
        path.display(),
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(AclRule::parse)
        .filter(|(sid, _)| sid == APP_CONTAINER_SID)
        .map(|(_, rule)| rule)
        .collect()
}

type ArchiveWriter = zip::ZipWriter<std::io::BufWriter<std::fs::File>>;

fn zip_without_setup(top: &Path, archive: &Path) -> std::io::Result<()> {
    let file = std::fs::File::create(archive)?;
    let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
    for path in archive_entries(top)? {
        add_entry(&mut zip, &entry_name(top, &path), &path)?;
    }
    zip.finish()?;
    Ok(())
}

fn add_entry(zip: &mut ArchiveWriter, name: &str, path: &Path) -> std::io::Result<()> {
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    if path.is_dir() {
        zip.add_directory(format!("{name}/"), options)?;
        return Ok(());
    }
    zip.start_file(name, options)?;
    std::io::copy(&mut std::fs::File::open(path)?, zip)?;
    Ok(())
}

/// Every folder and file under `top` except the top-level setup.exe.
fn archive_entries(top: &Path) -> std::io::Result<Vec<PathBuf>> {
    let skipped = top.join(SETUP);
    let mut entries = Vec::new();
    let mut pending = vec![top.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            if path != skipped {
                entries.push(path);
            }
        }
    }
    Ok(entries)
}

fn entry_name(top: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(top)
        .expect("walked paths stay inside the install");
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    format!("{TOP_DIR}/{}", parts.join("/"))
}

struct ArchiveServer {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl ArchiveServer {
    async fn start(archive: PathBuf) -> ArchiveServer {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .expect("bind the archive server");
        let port = listener
            .local_addr()
            .expect("the archive server has an address")
            .port();
        let task = tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(send_archive(stream, archive.clone()));
            }
        });
        ArchiveServer {
            url: format!("http://127.0.0.1:{port}/{TOP_DIR}.zip"),
            task,
        }
    }
}

async fn send_archive(mut stream: TcpStream, archive: PathBuf) {
    if let Err(error) = try_send_archive(&mut stream, &archive).await {
        eprintln!("serving {} failed: {error}", archive.display());
    }
}

async fn try_send_archive(stream: &mut TcpStream, archive: &Path) -> std::io::Result<()> {
    read_request_head(stream).await?;
    let mut file = tokio::fs::File::open(archive).await?;
    let length = file.metadata().await?.len();
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(response.as_bytes()).await?;
    tokio::io::copy(&mut file, stream).await?;
    stream.shutdown().await
}

async fn read_request_head(stream: &mut TcpStream) -> std::io::Result<()> {
    let mut head = Vec::new();
    let mut chunk = [0_u8; 4096];
    while !head.windows(4).any(|window| window == b"\r\n\r\n") {
        let read = stream.read(&mut chunk).await?;
        if read == 0 || head.len() > MAX_REQUEST_HEAD {
            return Err(std::io::Error::other("the request head did not finish"));
        }
        head.extend_from_slice(&chunk[..read]);
    }
    Ok(())
}
