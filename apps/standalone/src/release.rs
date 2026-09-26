use crate::{enrollment::unix_time, supervisor, vault};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    Ed25519PublicKey, MAX_COMPACT_JWS_BYTES, ReleaseTarget, StandaloneArtifact, StandaloneRelease,
    validate_release_url, verify_standalone_release,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

pub mod update;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseTrust {
    pub manifest_url: String,
    pub public_keys: Vec<String>,
    #[serde(default)]
    pub minimum_sequence: u64,
}

impl ReleaseTrust {
    pub fn load(path: &Path) -> Result<Self> {
        let value: Self = serde_json::from_slice(&vault::read_private(path)?)?;
        value.keys()?;
        Ok(value)
    }
    pub fn keys(&self) -> Result<Vec<Ed25519PublicKey>> {
        validate_release_url(&self.manifest_url)?;
        ensure!(
            !self.public_keys.is_empty()
                && self.public_keys.len() <= 8
                && self.minimum_sequence <= 9_007_199_254_740_991,
            "Configure trusted release keys and a valid minimum sequence"
        );
        let mut distinct = std::collections::HashSet::new();
        self.public_keys
            .iter()
            .map(|value| {
                let decoded = URL_SAFE_NO_PAD.decode(value)?;
                ensure!(
                    URL_SAFE_NO_PAD.encode(&decoded) == *value && distinct.insert(value),
                    "Invalid or duplicate release public key"
                );
                Ok(Ed25519PublicKey::from_bytes(decoded.try_into().map_err(
                    |_| anyhow::anyhow!("Release public keys must contain 32 bytes"),
                )?)?)
            })
            .collect()
    }
}

pub struct VerifiedRelease {
    manifest: StandaloneRelease,
    compact: String,
    trust: ReleaseTrust,
}
impl VerifiedRelease {
    pub fn verify(compact: String, trust: &ReleaseTrust) -> Result<Self> {
        let manifest = verify_standalone_release(
            &compact,
            &trust.keys()?,
            trust.minimum_sequence,
            unix_time()?,
        )?;
        Ok(Self {
            manifest,
            compact,
            trust: trust.clone(),
        })
    }
    pub fn manifest(&self) -> &StandaloneRelease {
        &self.manifest
    }
    pub fn compact(&self) -> &str {
        &self.compact
    }
    pub fn artifact(&self, target: ReleaseTarget) -> Result<&StandaloneArtifact> {
        self.manifest
            .artifacts
            .iter()
            .find(|artifact| artifact.target == target)
            .context("The signed release has no artifact for the selected target")
    }
    pub fn check_fresh(&self) -> Result<()> {
        verify_standalone_release(
            &self.compact,
            &self.trust.keys()?,
            self.trust.minimum_sequence,
            unix_time()?,
        )?;
        Ok(())
    }
}

/// Copy only verified release authority from an intact onboarding package, never replace local trust.
pub fn install_package_trust(state_dir: &Path, package_dir: &Path) -> Result<()> {
    let trust_path = package_dir.join("release-trust.json");
    if !trust_path.try_exists()? {
        return Ok(());
    }
    let trust = ReleaseTrust::load(&trust_path)?;
    let compact =
        String::from_utf8(vault::read_private(&package_dir.join("release.jws"))?.to_vec())?;
    let release = VerifiedRelease::verify(compact, &trust)?;
    let target = ReleaseTarget::current()?;
    // Docker-only packages verify the running image's embedded binary against the same artifact.
    let binary = if package_dir.join("flow-like-standalone").try_exists()? {
        package_dir.join("flow-like-standalone")
    } else {
        std::env::current_exe()?
    };
    verify_artifact_file(&binary, release.artifact(target)?)?;
    verify_running_artifact(release.artifact(target)?)?;
    let state_dir = supervisor::prepare_state_dir(state_dir)?;
    let _lock = supervisor::lock_file(&state_dir.join("release.lock"))?;
    for (name, bytes) in [
        ("release-trust.json", serde_json::to_vec_pretty(&trust)?),
        ("active-release.jws", release.compact.as_bytes().to_vec()),
    ] {
        let path = state_dir.join(name);
        if path.try_exists()? {
            ensure!(
                &**vault::read_private(&path)? == bytes.as_slice(),
                "Installed release authority differs; change it explicitly before enrollment"
            );
        } else {
            vault::write_new_private(&path, &bytes)?;
        }
    }
    Ok(())
}

fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(300))
        .build()?)
}

pub async fn fetch_release(trust: &ReleaseTrust) -> Result<VerifiedRelease> {
    trust.keys()?;
    let mut response = client()?
        .get(&trust.manifest_url)
        .send()
        .await
        .context("Download signed release metadata")?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Release metadata download failed without following redirects"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= MAX_COMPACT_JWS_BYTES as u64),
        "Release manifest exceeds its size limit"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            bytes.len() + chunk.len() <= MAX_COMPACT_JWS_BYTES,
            "Release manifest exceeds its size limit"
        );
        bytes.extend_from_slice(&chunk);
    }
    VerifiedRelease::verify(String::from_utf8(bytes)?, trust)
}

pub struct DownloadedArtifact {
    path: PathBuf,
}
impl DownloadedArtifact {
    pub fn path(&self) -> &Path {
        &self.path
    }
}
impl Drop for DownloadedArtifact {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub async fn download_artifact(
    release: &VerifiedRelease,
    target: ReleaseTarget,
    temporary_directory: &Path,
) -> Result<DownloadedArtifact> {
    release.check_fresh()?;
    let artifact = release.artifact(target)?;
    let directory = supervisor::prepare_state_dir(temporary_directory)?;
    let path = directory.join(format!(".standalone-release-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700).custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(&path)?;
    let downloaded = DownloadedArtifact { path };
    let mut file = tokio::fs::File::from_std(file);
    let mut response = client()?
        .get(&artifact.url)
        .send()
        .await
        .context("Download standalone artifact")?;
    ensure!(
        response.status() == reqwest::StatusCode::OK,
        "Artifact download failed without following redirects"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size == artifact.size),
        "Artifact content length differs from the signed release"
    );
    let mut size = 0u64;
    let mut digest = Sha256::new();
    while let Some(chunk) = response.chunk().await? {
        size = size
            .checked_add(chunk.len() as u64)
            .context("Artifact size overflow")?;
        ensure!(
            size <= artifact.size,
            "Artifact download exceeds its signed size"
        );
        digest.update(&chunk);
        file.write_all(&chunk).await?;
    }
    ensure!(
        size == artifact.size && format!("{:x}", digest.finalize()) == artifact.sha256,
        "Artifact size or SHA256 differs from the signed release"
    );
    file.sync_all().await?;
    release.check_fresh()?;
    Ok(downloaded)
}

/// Verify a local file again before publishing or activating it.
pub fn verify_artifact_file(path: &Path, artifact: &StandaloneArtifact) -> Result<()> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path)?;
    verify_artifact_handle(file, artifact)
}

pub(crate) fn verify_running_artifact(artifact: &StandaloneArtifact) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        verify_artifact_handle(File::open("/proc/self/exe")?, artifact)
    }
    #[cfg(not(target_os = "linux"))]
    {
        verify_artifact_file(&std::env::current_exe()?, artifact)
    }
}

fn verify_artifact_handle(mut file: File, artifact: &StandaloneArtifact) -> Result<()> {
    ensure!(
        file.metadata()?.is_file() && file.metadata()?.len() == artifact.size,
        "Artifact file size differs from the signed release"
    );
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        size += read as u64;
        ensure!(size <= artifact.size, "Artifact changed while being read");
        hash.update(&buffer[..read]);
    }
    ensure!(
        size == artifact.size && format!("{:x}", hash.finalize()) == artifact.sha256,
        "Artifact SHA256 differs from the signed release"
    );
    Ok(())
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub enum PackageMode {
    Binary,
    Docker,
    Both,
}

pub struct PreparedPackage {
    release: VerifiedRelease,
    target: ReleaseTarget,
    mode: PackageMode,
    binary: Option<DownloadedArtifact>,
}

impl PreparedPackage {
    pub async fn download(
        trust: &ReleaseTrust,
        target: ReleaseTarget,
        mode: PackageMode,
        temporary_directory: &Path,
    ) -> Result<Self> {
        let release = fetch_release(trust).await?;
        release.artifact(target)?;
        if !matches!(mode, PackageMode::Binary) {
            let platform = target
                .docker_platform()
                .context("Containers require a Linux target")?;
            ensure!(
                release
                    .manifest
                    .container
                    .as_ref()
                    .is_some_and(|container| container
                        .platforms
                        .iter()
                        .any(|value| value == platform)),
                "The signed release has no container for this target"
            );
        }
        let binary = if matches!(mode, PackageMode::Docker) {
            None
        } else {
            Some(download_artifact(&release, target, temporary_directory).await?)
        };
        Ok(Self {
            release,
            target,
            mode,
            binary,
        })
    }
    pub fn target(&self) -> ReleaseTarget {
        self.target
    }
    pub fn manifest(&self) -> &StandaloneRelease {
        self.release.manifest()
    }
    pub(crate) fn write_payload(&self, directory: &Path) -> Result<()> {
        self.release.check_fresh()?;
        if let Some(binary) = &self.binary {
            verify_artifact_file(binary.path(), self.release.artifact(self.target)?)?;
            let mut input = File::open(binary.path())?;
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o700).custom_flags(libc::O_NOFOLLOW);
            }
            let mut output = options.open(directory.join("flow-like-standalone"))?;
            std::io::copy(&mut input, &mut output)?;
            output.sync_all()?;
            verify_artifact_file(
                &directory.join("flow-like-standalone"),
                self.release.artifact(self.target)?,
            )?;
            script(directory, "start.sh", BINARY_START)?;
        }
        vault::write_new_private(
            &directory.join("release.jws"),
            self.release.compact.as_bytes(),
        )?;
        let mut trust = self.release.trust.clone();
        trust.minimum_sequence = trust.minimum_sequence.max(self.release.manifest.sequence);
        vault::write_new_private(
            &directory.join("release-trust.json"),
            &serde_json::to_vec_pretty(&trust)?,
        )?;
        vault::write_new_private(
            &directory.join("platform.json"),
            &serde_json::to_vec_pretty(
                &serde_json::json!({"target":self.target,"version":self.release.manifest.release_version,"release_sequence":self.release.manifest.sequence}),
            )?,
        )?;
        if !matches!(self.mode, PackageMode::Binary) {
            let container = self
                .release
                .manifest
                .container
                .as_ref()
                .context("Missing pinned container")?;
            let platform = self
                .target
                .docker_platform()
                .context("Missing container platform")?;
            let compose = format!(
                "services:\n  agent:\n    image: {}\n    platform: {platform}\n    restart: unless-stopped\n    user: \"${{FLOW_LIKE_DEVICE_UID:?Run start-docker.sh}}:${{FLOW_LIKE_DEVICE_GID:?Run start-docker.sh}}\"\n    working_dir: /package\n    env_file: .env\n    ports:\n      - \"${{FLOW_LIKE_PUBLISH_HOST:-127.0.0.1}}:${{FLOW_LIKE_SERVICE_PORT:-8080}}:${{FLOW_LIKE_SERVICE_PORT:-8080}}\"\n    volumes:\n      - ./:/package\n    entrypoint: [\"/bin/sh\", \"/package/container-start.sh\"]\n    stop_grace_period: 45s\n",
                container.image
            );
            vault::write_new_private(&directory.join("compose.yaml"), compose.as_bytes())?;
            script(directory, "container-start.sh", CONTAINER_START)?;
            script(directory, "start-docker.sh", DOCKER_START)?;
        }
        Ok(())
    }
}

const BINARY_START: &str = "#!/bin/sh\nset -eu\ncd -- \"$(dirname -- \"$0\")\"\nchmod 700 . ./flow-like-standalone\nchmod 600 .env onboarding.json release-trust.json release.jws 2>/dev/null || true\nif [ -f onboarding.json ]; then ./flow-like-standalone --state-dir ./state enroll .; fi\nif [ -f ./state/agent.env ]; then chmod 600 ./state/agent.env; fi\nexec ./flow-like-standalone --state-dir ./state run\n";
const CONTAINER_START: &str = "#!/bin/sh\nset -eu\nif [ -f /package/onboarding.json ]; then flow-like-standalone --state-dir /package/state enroll /package; fi\nexec flow-like-standalone --state-dir /package/state run\n";
const DOCKER_START: &str = "#!/bin/sh\nset -eu\ncd -- \"$(dirname -- \"$0\")\"\nchmod 700 .\nchmod 600 .env onboarding.json release-trust.json release.jws 2>/dev/null || true\nif [ -f ./state/agent.env ]; then chmod 600 ./state/agent.env; fi\nexport FLOW_LIKE_DEVICE_UID=\"$(id -u)\"\nexport FLOW_LIKE_DEVICE_GID=\"$(id -g)\"\nexec docker compose up -d\n";
fn script(directory: &Path, name: &str, text: &str) -> Result<()> {
    vault::write_new_private(&directory.join(name), text.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(directory.join(name), std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{SigningKey, sign_standalone_release};
    #[test]
    fn pinned_release_and_file_check_reject_changed_and_linked_artifacts() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let binary = directory.path().join("binary");
        std::fs::write(&binary, b"binary")?;
        let key = SigningKey::generate();
        let now = unix_time()?;
        let manifest = StandaloneRelease {
            version: 1,
            state_schema_version: crate::state::SCHEMA_VERSION.try_into().unwrap(),
            sequence: 1,
            release_version: "1.2.3".into(),
            issued_at: now,
            expires_at: now + 300,
            artifacts: vec![StandaloneArtifact {
                target: ReleaseTarget::LinuxX86_64,
                url: "https://releases.example/binary".into(),
                size: 6,
                sha256: format!("{:x}", Sha256::digest(b"binary")),
            }],
            container: None,
        };
        let trust = ReleaseTrust {
            manifest_url: "https://releases.example/release.jws".into(),
            public_keys: vec![URL_SAFE_NO_PAD.encode(key.public_key().to_bytes()?)],
            minimum_sequence: 1,
        };
        let release = VerifiedRelease::verify(sign_standalone_release(&manifest, &key)?, &trust)?;
        verify_artifact_file(&binary, release.artifact(ReleaseTarget::LinuxX86_64)?)?;
        std::fs::write(&binary, b"tamper")?;
        assert!(
            verify_artifact_file(&binary, release.artifact(ReleaseTarget::LinuxX86_64)?).is_err()
        );
        #[cfg(unix)]
        {
            let linked = directory.path().join("linked");
            std::os::unix::fs::symlink(&binary, &linked)?;
            assert!(
                verify_artifact_file(&linked, release.artifact(ReleaseTarget::LinuxX86_64)?)
                    .is_err()
            );
        }
        assert!(
            ReleaseTrust {
                public_keys: vec![],
                ..trust
            }
            .keys()
            .is_err()
        );
        Ok(())
    }
}
