use crate::{
    Ed25519PublicKey, ProtocolError, Result, SigningKey,
    proof::{sign_pinned, verify_pinned},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

pub const STANDALONE_RELEASE_JWS_TYPE: &str = "flow-like-standalone-release+jws";
pub const MAX_STANDALONE_ARTIFACT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
const MAX_RELEASE_LIFETIME: i64 = 30 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
pub enum ReleaseTarget {
    #[serde(rename = "x86_64-unknown-linux-gnu")]
    LinuxX86_64,
    #[serde(rename = "aarch64-unknown-linux-gnu")]
    LinuxAarch64,
    #[serde(rename = "x86_64-apple-darwin")]
    MacosX86_64,
    #[serde(rename = "aarch64-apple-darwin")]
    MacosAarch64,
}

impl ReleaseTarget {
    pub fn triple(self) -> &'static str {
        match self {
            Self::LinuxX86_64 => "x86_64-unknown-linux-gnu",
            Self::LinuxAarch64 => "aarch64-unknown-linux-gnu",
            Self::MacosX86_64 => "x86_64-apple-darwin",
            Self::MacosAarch64 => "aarch64-apple-darwin",
        }
    }
    pub fn docker_platform(self) -> Option<&'static str> {
        match self {
            Self::LinuxX86_64 => Some("linux/amd64"),
            Self::LinuxAarch64 => Some("linux/arm64"),
            _ => None,
        }
    }
    pub fn current() -> Result<Self> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("linux", "x86_64") => Ok(Self::LinuxX86_64),
            ("linux", "aarch64") => Ok(Self::LinuxAarch64),
            ("macos", "x86_64") => Ok(Self::MacosX86_64),
            ("macos", "aarch64") => Ok(Self::MacosAarch64),
            _ => Err(ProtocolError::Invalid(
                "unsupported standalone release target",
            )),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StandaloneArtifact {
    pub target: ReleaseTarget,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StandaloneContainer {
    pub image: String,
    pub platforms: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct StandaloneRelease {
    pub version: u32,
    pub state_schema_version: u32,
    pub sequence: u64,
    pub release_version: String,
    pub issued_at: i64,
    pub expires_at: i64,
    pub artifacts: Vec<StandaloneArtifact>,
    pub container: Option<StandaloneContainer>,
}

pub fn validate_release_url(value: &str) -> Result<()> {
    let url = url::Url::parse(value).map_err(|_| ProtocolError::Invalid("release URL"))?;
    if value.len() > 1024
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.as_str() != value
    {
        return Err(ProtocolError::Invalid("canonical HTTPS release URL"));
    }
    Ok(())
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}
fn version(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }
    let mut metadata = value.split('+');
    let Some(main) = metadata.next() else {
        return false;
    };
    if let Some(build) = metadata.next() {
        if !identifiers(build, false) {
            return false;
        }
    }
    if metadata.next().is_some() {
        return false;
    }
    let (core, pre) = main
        .split_once('-')
        .map_or((main, None), |(core, pre)| (core, Some(pre)));
    let numbers = core.split('.').collect::<Vec<_>>();
    numbers.len() == 3
        && numbers.iter().all(|part| {
            !part.is_empty()
                && (part.len() == 1 || !part.starts_with('0'))
                && part.bytes().all(|c| c.is_ascii_digit())
                && part.parse::<u64>().is_ok()
        })
        && pre.is_none_or(|pre| identifiers(pre, true))
}
fn identifiers(value: &str, numeric_rule: bool) -> bool {
    value.split('.').all(|part| {
        !part.is_empty()
            && part.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
            && !(numeric_rule
                && part.len() > 1
                && part.starts_with('0')
                && part.bytes().all(|c| c.is_ascii_digit()))
    })
}

pub fn validate_standalone_release(value: &StandaloneRelease) -> Result<()> {
    if value.version != 1
        || value.state_schema_version == 0
        || value.sequence == 0
        || value.sequence > MAX_SAFE_INTEGER
        || !version(&value.release_version)
        || value.issued_at < 0
        || value.expires_at > MAX_SAFE_INTEGER as i64
        || value.expires_at <= value.issued_at
        || value.expires_at - value.issued_at > MAX_RELEASE_LIFETIME
        || value.artifacts.is_empty()
        || value.artifacts.len() > 4
    {
        return Err(ProtocolError::Invalid("standalone release shape"));
    }
    let mut targets = HashSet::new();
    for artifact in &value.artifacts {
        if !targets.insert(artifact.target)
            || artifact.size == 0
            || artifact.size > MAX_STANDALONE_ARTIFACT_BYTES
            || !digest(&artifact.sha256)
        {
            return Err(ProtocolError::Invalid("standalone release artifact"));
        }
        validate_release_url(&artifact.url)?;
    }
    if let Some(container) = &value.container {
        let Some((image, sha256)) = container.image.split_once("@sha256:") else {
            return Err(ProtocolError::Invalid("digest-pinned container image"));
        };
        if image.len() > 255
            || !image.contains('/')
            || !image
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
            || !image
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"/._:-".contains(&c))
            || !digest(sha256)
            || container.platforms.is_empty()
            || container.platforms.len() > 2
        {
            return Err(ProtocolError::Invalid("standalone container"));
        }
        let mut platforms = HashSet::new();
        if !container.platforms.iter().all(|platform| {
            matches!(platform.as_str(), "linux/amd64" | "linux/arm64") && platforms.insert(platform)
        }) {
            return Err(ProtocolError::Invalid("standalone container platforms"));
        }
    }
    Ok(())
}

pub fn sign_standalone_release(value: &StandaloneRelease, key: &SigningKey) -> Result<String> {
    validate_standalone_release(value)?;
    sign_pinned(value, key, STANDALONE_RELEASE_JWS_TYPE)
}

pub fn verify_standalone_release(
    compact: &str,
    pinned: &[Ed25519PublicKey],
    minimum_sequence: u64,
    now: i64,
) -> Result<StandaloneRelease> {
    if pinned.is_empty() || pinned.len() > 8 || minimum_sequence > MAX_SAFE_INTEGER || now < 0 {
        return Err(ProtocolError::Invalid(
            "standalone release trust configuration",
        ));
    }
    for key in pinned {
        key.validate()?;
    }
    let value: StandaloneRelease = pinned
        .iter()
        .find_map(|key| verify_pinned(compact, key, STANDALONE_RELEASE_JWS_TYPE).ok())
        .ok_or(ProtocolError::InvalidSignature)?;
    validate_standalone_release(&value)?;
    if value.issued_at > now || value.expires_at <= now {
        return Err(ProtocolError::InvalidTime);
    }
    if value.sequence < minimum_sequence {
        return Err(ProtocolError::Invalid("standalone release rollback"));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release() -> StandaloneRelease {
        StandaloneRelease {
            version: 1,
            state_schema_version: 4,
            sequence: 12,
            release_version: "1.2.3-alpha.1+build".into(),
            issued_at: 100,
            expires_at: 200,
            artifacts: vec![StandaloneArtifact {
                target: ReleaseTarget::LinuxX86_64,
                url: "https://releases.example/standalone".into(),
                size: 100,
                sha256: "a".repeat(64),
            }],
            container: Some(StandaloneContainer {
                image: format!("ghcr.io/example/standalone@sha256:{}", "b".repeat(64)),
                platforms: vec!["linux/amd64".into()],
            }),
        }
    }
    #[test]
    fn release_requires_pinned_authority_time_sequence_and_exact_artifact_binding() {
        let key = SigningKey::generate();
        let value = release();
        let signed = sign_standalone_release(&value, &key).unwrap();
        assert_eq!(
            verify_standalone_release(&signed, &[key.public_key()], 12, 100).unwrap(),
            value
        );
        assert!(verify_standalone_release(&signed, &[], 0, 100).is_err());
        assert!(
            verify_standalone_release(&signed, &[SigningKey::generate().public_key()], 0, 100)
                .is_err()
        );
        for (floor, now) in [(13, 100), (0, 99), (0, 200)] {
            assert!(verify_standalone_release(&signed, &[key.public_key()], floor, now).is_err());
        }
        let mut bad = value.clone();
        bad.artifacts.push(bad.artifacts[0].clone());
        assert!(sign_standalone_release(&bad, &key).is_err());
        bad = value.clone();
        bad.artifacts[0].size = MAX_STANDALONE_ARTIFACT_BYTES + 1;
        assert!(sign_standalone_release(&bad, &key).is_err());
        bad = value.clone();
        bad.container.as_mut().unwrap().image = "ghcr.io/example/standalone:latest".into();
        assert!(sign_standalone_release(&bad, &key).is_err());
        bad = value;
        bad.artifacts[0].url = "http://releases.example/file".into();
        assert!(sign_standalone_release(&bad, &key).is_err());
    }
    #[test]
    fn semantic_versions_and_urls_do_not_become_script_inputs() {
        for good in ["0.0.1", "1.2.3-alpha.1", "1.2.3+build.01"] {
            assert!(version(good));
        }
        for bad in [
            "1.2",
            "01.2.3",
            "1.2.3-01",
            "1.2.3+",
            "1.2.3;reboot",
            "1.2.3\n",
        ] {
            assert!(!version(bad));
        }
        for bad in [
            "https://user:secret@releases.example/file",
            "https://releases.example/file?token=secret",
            "https://releases.example/file#hash",
            "https://releases.example/a/../file",
        ] {
            assert!(validate_release_url(bad).is_err());
        }
    }
}
