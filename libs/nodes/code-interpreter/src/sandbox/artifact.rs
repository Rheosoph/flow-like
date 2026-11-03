use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeArtifact {
    pub id: &'static str,
    pub version: &'static str,
    pub file_name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
    pub urls: &'static [&'static str],
}

pub const PYTHON_ARTIFACT: RuntimeArtifact = RuntimeArtifact {
    id: "python",
    version: "3.12.0-20231211-040d5a6",
    file_name: "python.wasm",
    sha256: "e5dc5a398b07b54ea8fdb503bf68fb583d533f10ec3f930963e02b9505f7a763",
    size: 26_267_204,
    urls: &[
        "https://cdn.flow-like.com/runtimes/python/3.12.0-20231211-040d5a6/python.wasm",
        "https://github.com/vmware-labs/webassembly-language-runtimes/releases/download/python%2F3.12.0%2B20231211-040d5a6/python-3.12.0.wasm",
    ],
};

pub const QUICKJS_ARTIFACT: RuntimeArtifact = RuntimeArtifact {
    id: "quickjs",
    version: "0.18.0",
    file_name: "qjs-wasi.wasm",
    sha256: "2c218412cb8b17e46ee91bea7556ec38f5b9b6bd6312fddd3ceeed14aa71eaaa",
    size: 1_571_399,
    urls: &[
        "https://cdn.flow-like.com/runtimes/quickjs/0.18.0/qjs-wasi.wasm",
        "https://github.com/quickjs-ng/quickjs/releases/download/v0.18.0/qjs-wasi.wasm",
    ],
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundleMember {
    pub path_in_archive: &'static str,
    pub file_name: &'static str,
    pub sha256: &'static str,
    pub size: u64,
}

/// An archive pinned by sha256 whose listed members are extracted, each pinned again, into
/// `<root>/<id>/<version>/<file_name>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeBundle {
    pub id: &'static str,
    pub version: &'static str,
    pub archive: RuntimeArtifact,
    pub members: &'static [BundleMember],
}

pub const SCIENTIFIC_PYTHON_BUNDLE: RuntimeBundle = RuntimeBundle {
    id: "python-scientific",
    version: "0.1.2",
    archive: RuntimeArtifact {
        id: "python-scientific-wheel",
        version: "0.1.2",
        file_name: "wasi_python-0.1.2-py3-none-any.whl",
        sha256: "bc56ff7a3dfba12f68ee2a5f9893637af55cff4df4b41fc4fc852fa86aa2e6d0",
        size: 98_385_354,
        urls: &[
            "https://cdn.flow-like.com/runtimes/python-scientific/0.1.2/wasi_python-0.1.2-py3-none-any.whl",
            "https://files.pythonhosted.org/packages/5c/88/17bb6e7050b5630108d0cea47c7dc5d95aa881cfad18020866db79495dcb/wasi_python-0.1.2-py3-none-any.whl",
        ],
    },
    members: &[
        BundleMember {
            path_in_archive: "wasi_python/artifacts/python.wasm",
            file_name: "python.wasm",
            sha256: "915306e6c191f8b24ae6c038975d36f2b67ff1aaf005584fb7e19ea0fc760682",
            size: 99_431_714,
        },
        BundleMember {
            path_in_archive: "wasi_python/artifacts/python-stdlib.zip",
            file_name: "lib/python315.zip",
            sha256: "a7b8f7cbd4799ac8c81223cbc47f8cea503adac934a04826778b66e5a5e9b847",
            size: 53_915_154,
        },
    ],
};

#[async_trait]
pub trait ArtifactSource: Send + Sync {
    async fn fetch(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>>;
}

pub struct HttpArtifactSource {
    client: flow_like_types::reqwest::Client,
}

impl Default for HttpArtifactSource {
    fn default() -> Self {
        Self {
            client: flow_like_types::reqwest::Client::builder()
                .user_agent("flow-like-sandbox-runtime/1")
                .connect_timeout(std::time::Duration::from_secs(20))
                .timeout(std::time::Duration::from_secs(600))
                .build()
                .unwrap_or_default(),
        }
    }
}

#[async_trait]
impl ArtifactSource for HttpArtifactSource {
    async fn fetch(&self, url: &str, max_bytes: u64) -> Result<Vec<u8>> {
        let response = self
            .client
            .get(url)
            .send()
            .await
            .with_context(|| format!("request {url}"))?
            .error_for_status()
            .with_context(|| format!("download {url}"))?;
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.with_context(|| format!("read body of {url}"))?;
            if body.len() as u64 + chunk.len() as u64 > max_bytes {
                bail!("{url} is larger than the pinned size of {max_bytes} bytes");
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex_lower(&Sha256::digest(bytes))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn verify_member_bytes(member: &BundleMember, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != member.size {
        bail!(
            "{} has {} bytes, expected {}",
            member.path_in_archive,
            bytes.len(),
            member.size
        );
    }
    let actual = sha256_hex(bytes);
    if actual != member.sha256 {
        bail!(
            "{} failed its integrity check: sha256 {actual}, expected {}",
            member.path_in_archive,
            member.sha256
        );
    }
    Ok(())
}

pub fn verify_artifact_bytes(artifact: &RuntimeArtifact, bytes: &[u8]) -> Result<()> {
    if bytes.len() as u64 != artifact.size {
        bail!(
            "{} {} has {} bytes, expected {}",
            artifact.id,
            artifact.version,
            bytes.len(),
            artifact.size
        );
    }
    let actual = sha256_hex(bytes);
    if actual != artifact.sha256 {
        bail!(
            "{} {} failed its integrity check: sha256 {actual}, expected {}",
            artifact.id,
            artifact.version,
            artifact.sha256
        );
    }
    Ok(())
}

pub struct ArtifactStore {
    root: PathBuf,
    source: Arc<dyn ArtifactSource>,
    lock: Mutex<()>,
}

impl ArtifactStore {
    pub fn new(root: impl Into<PathBuf>, source: Arc<dyn ArtifactSource>) -> Self {
        Self {
            root: root.into(),
            source,
            lock: Mutex::new(()),
        }
    }

    pub fn path_of(&self, artifact: &RuntimeArtifact) -> PathBuf {
        self.root
            .join(artifact.id)
            .join(artifact.version)
            .join(artifact.file_name)
    }

    pub fn is_installed(&self, artifact: &RuntimeArtifact) -> bool {
        std::fs::metadata(self.path_of(artifact))
            .map(|m| m.len() == artifact.size)
            .unwrap_or(false)
    }

    pub async fn ensure(&self, artifact: &RuntimeArtifact) -> Result<PathBuf> {
        let _guard = self.lock.lock().await;
        let path = self.path_of(artifact);

        if let Some(existing) = read_if_present(&path).await {
            match verify_artifact_bytes(artifact, &existing) {
                Ok(()) => return Ok(path),
                Err(e) => {
                    tracing::warn!("discarding cached {}: {e:#}", path.display());
                    let _ = tokio::fs::remove_file(&path).await;
                }
            }
        }

        let bytes = self.download(artifact).await?;
        write_atomic(&path, &bytes).await?;
        Ok(path)
    }

    pub fn bundle_dir(&self, bundle: &RuntimeBundle) -> PathBuf {
        self.root.join(bundle.id).join(bundle.version)
    }

    pub fn is_bundle_installed(&self, bundle: &RuntimeBundle) -> bool {
        let dir = self.bundle_dir(bundle);
        bundle.members.iter().all(|member| {
            std::fs::metadata(dir.join(member.file_name))
                .map(|m| m.len() == member.size)
                .unwrap_or(false)
        })
    }

    /// Returns the directory holding every member, downloading and extracting the archive when
    /// a member is missing or fails its hash.
    pub async fn ensure_bundle(&self, bundle: &RuntimeBundle) -> Result<PathBuf> {
        let _guard = self.lock.lock().await;
        let dir = self.bundle_dir(bundle);
        if bundle_is_intact(&dir, bundle).await {
            return Ok(dir);
        }

        let archive = self.download(&bundle.archive).await?;
        let members = bundle.members;
        let extracted = tokio::task::spawn_blocking(move || extract_members(&archive, members))
            .await
            .context("extract task")??;
        for (member, bytes) in bundle.members.iter().zip(extracted) {
            write_atomic(&dir.join(member.file_name), &bytes).await?;
        }
        Ok(dir)
    }

    async fn download(&self, artifact: &RuntimeArtifact) -> Result<Vec<u8>> {
        let mut failures = Vec::new();
        for url in artifact.urls {
            match self.source.fetch(url, artifact.size).await {
                Ok(bytes) => match verify_artifact_bytes(artifact, &bytes) {
                    Ok(()) => return Ok(bytes),
                    Err(e) => failures.push(format!("{url}: {e:#}")),
                },
                Err(e) => failures.push(format!("{url}: {e:#}")),
            }
        }
        Err(anyhow!(
            "could not download {} {}:\n{}",
            artifact.id,
            artifact.version,
            failures.join("\n")
        ))
    }
}

async fn bundle_is_intact(dir: &Path, bundle: &RuntimeBundle) -> bool {
    for member in bundle.members {
        let path = dir.join(member.file_name);
        let Some(bytes) = read_if_present(&path).await else {
            return false;
        };
        if let Err(e) = verify_member_bytes(member, &bytes) {
            tracing::warn!("discarding cached {}: {e:#}", path.display());
            let _ = tokio::fs::remove_file(&path).await;
            return false;
        }
    }
    true
}

pub fn extract_members(archive: &[u8], members: &[BundleMember]) -> Result<Vec<Vec<u8>>> {
    use std::io::Read;

    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).context("open archive")?;
    members
        .iter()
        .map(|member| {
            let mut entry = zip
                .by_name(member.path_in_archive)
                .with_context(|| format!("{} is not in the archive", member.path_in_archive))?;
            if entry.size() != member.size {
                bail!(
                    "{} is {} bytes in the archive, expected {}",
                    member.path_in_archive,
                    entry.size(),
                    member.size
                );
            }
            let mut bytes = Vec::with_capacity(member.size as usize);
            entry
                .by_ref()
                .take(member.size + 1)
                .read_to_end(&mut bytes)
                .with_context(|| format!("read {}", member.path_in_archive))?;
            verify_member_bytes(member, &bytes)?;
            Ok(bytes)
        })
        .collect()
}

async fn read_if_present(path: &Path) -> Option<Vec<u8>> {
    tokio::fs::read(path).await.ok()
}

async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("artifact path has no parent")?;
    tokio::fs::create_dir_all(parent)
        .await
        .with_context(|| format!("create {}", parent.display()))?;
    let part = path.with_extension("part");
    tokio::fs::write(&part, bytes)
        .await
        .with_context(|| format!("write {}", part.display()))?;
    tokio::fs::rename(&part, path)
        .await
        .with_context(|| format!("install {}", path.display()))
}
