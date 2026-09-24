use crate::{ProtocolError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use unicode_normalization::UnicodeNormalization;

pub const PROJECT_ARTIFACT_CHUNK_BYTES: usize = 8192;
pub const PROJECT_ARTIFACT_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
pub const PROJECT_ARTIFACT_MAX_FILES: usize = 8192;
pub const PROJECT_ARTIFACT_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const PROJECT_ARTIFACT_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const PROJECT_ARTIFACT_TTL_SECONDS: i64 = 86_400;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectBitPin {
    pub bit_id: String,
    pub metadata_sha256: String,
}

impl ProjectBitPin {
    pub fn validate(&self) -> Result<()> {
        validate_artifact_project_id(&self.bit_id)?;
        validate_artifact_digest(&self.metadata_sha256)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectArtifactFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectArtifactManifest {
    pub version: u32,
    pub project_id: String,
    #[serde(default, skip_serializing_if = "ProjectArtifactSource::is_offline")]
    pub source: ProjectArtifactSource,
    pub files: Vec<ProjectArtifactFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bit_pins: Vec<ProjectBitPin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub package_pins: Vec<ProjectPackagePin>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectArtifactDescriptor {
    pub project_id: String,
    #[serde(default, skip_serializing_if = "ProjectArtifactSource::is_offline")]
    pub source: ProjectArtifactSource,
    pub manifest_sha256: String,
    pub manifest_size: u64,
    pub file_count: u32,
    pub total_bytes: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectArtifactSource {
    #[default]
    Offline,
    Online,
}
impl ProjectArtifactSource {
    fn is_offline(&self) -> bool {
        *self == Self::Offline
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ArtifactRequest {
    Begin {
        descriptor: ProjectArtifactDescriptor,
    },
    Chunk {
        project_id: String,
        transfer_id: String,
        /// None selects the manifest; Some indexes its sorted files.
        file_index: Option<u32>,
        offset: u64,
        data: String,
    },
    Status {
        project_id: String,
        transfer_id: String,
        file_index: Option<u32>,
    },
    Commit {
        project_id: String,
        transfer_id: String,
    },
    Abort {
        project_id: String,
        transfer_id: String,
    },
    PrepareOnline {
        project_id: String,
    },
    Describe {
        project_id: String,
        revision: String,
        event_id: Option<String>,
        after: Option<String>,
    },
}
impl std::fmt::Debug for ArtifactRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArtifactRequest")
            .field("project_id", &self.project_id())
            .field("payload", &"[REDACTED]")
            .finish()
    }
}
impl ArtifactRequest {
    pub fn project_id(&self) -> &str {
        match self {
            Self::Begin { descriptor } => &descriptor.project_id,
            Self::Chunk { project_id, .. }
            | Self::Status { project_id, .. }
            | Self::Commit { project_id, .. }
            | Self::Abort { project_id, .. }
            | Self::PrepareOnline { project_id }
            | Self::Describe { project_id, .. } => project_id,
        }
    }
    pub fn journaled(&self) -> bool {
        matches!(
            self,
            Self::Begin { .. }
                | Self::Commit { .. }
                | Self::Abort { .. }
                | Self::PrepareOnline { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactTransferState {
    Receiving,
    Committed,
    Aborted,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactTransferStatus {
    pub transfer_id: String,
    pub descriptor: ProjectArtifactDescriptor,
    pub state: ArtifactTransferState,
    pub expires_at: i64,
    pub manifest_ready: bool,
    pub file_index: Option<u32>,
    pub offset: u64,
    pub complete: bool,
    pub project_path: Option<String>,
}

pub fn validate_artifact_project_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 128
        || id == "."
        || id == ".."
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(ProtocolError::Invalid("artifact project ID"));
    }
    Ok(())
}
pub fn artifact_sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn validate_artifact_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(ProtocolError::Invalid("artifact SHA256"));
    }
    Ok(())
}
pub fn normalize_artifact_path(project: &str, path: &str) -> Result<String> {
    let normalized = path.nfc().collect::<String>();
    validate_artifact_path(project, &normalized)?;
    Ok(normalized)
}
pub fn validate_artifact_path(project: &str, path: &str) -> Result<()> {
    validate_artifact_project_id(project)?;
    let prefix = format!("apps/{project}/");
    if !path.starts_with(&prefix) {
        return Err(ProtocolError::Invalid("project store artifact path"));
    }
    validate_artifact_relative_path(path)
}
pub fn validate_artifact_relative_path(path: &str) -> Result<()> {
    if path.len() > 1024 || path.nfc().collect::<String>() != path {
        return Err(ProtocolError::Invalid("project store artifact path"));
    }
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() > 32 {
        return Err(ProtocolError::Invalid("artifact path depth"));
    }
    for part in parts {
        let lower = part.to_lowercase();
        let stem = lower.split('.').next().unwrap_or_default();
        let reserved = ["con", "prn", "aux", "nul"].contains(&stem)
            || (stem.len() == 4
                && (stem.starts_with("com") || stem.starts_with("lpt"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'));
        if part.is_empty()
            || part.len() > 255
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' ']) || part.chars().any(|c| {
            c.is_control()
                || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
                || matches!(c as u32, 0x200b..=0x200f | 0x202a..=0x202e | 0x2060..=0x206f | 0xfeff)
        }) || reserved || lower == ".secrets" || lower == ".env"
            || lower.ends_with(".secret")
            || [
                "management.sqlite",
                "secrets.key",
                "device-keys.json",
                "onboarding.json",
                "release-trust.json",
            ]
            .contains(&lower.as_str())
        {
            return Err(ProtocolError::Invalid("artifact path component"));
        }
    }
    Ok(())
}
impl ProjectArtifactDescriptor {
    pub fn validate(&self) -> Result<()> {
        validate_artifact_project_id(&self.project_id)?;
        validate_artifact_digest(&self.manifest_sha256)?;
        if self.manifest_size == 0
            || self.manifest_size > PROJECT_ARTIFACT_MANIFEST_BYTES
            || self.file_count == 0
            || self.file_count as usize > PROJECT_ARTIFACT_MAX_FILES
            || self.total_bytes > PROJECT_ARTIFACT_MAX_BYTES
        {
            return Err(ProtocolError::Invalid("artifact transfer bounds"));
        }
        Ok(())
    }
}
impl ProjectArtifactManifest {
    pub fn validate_file_path(&self, path: &str) -> Result<()> {
        validate_artifact_relative_path(path)?;
        if path.starts_with(&format!("apps/{}/", self.project_id)) {
            return Ok(());
        }
        if self
            .bit_pins
            .iter()
            .any(|pin| path == format!("bits/metadata/{}.json", pin.bit_id))
        {
            return Ok(());
        }
        let parts = path.split('/').collect::<Vec<_>>();
        if !self.bit_pins.is_empty()
            && parts.len() >= 3
            && parts[0] == "bits"
            && parts[1] != "metadata"
            && parts[1] != "deps-cache"
        {
            return Ok(());
        }
        if self.package_pins.iter().any(|pin| {
            path == format!("packages/{}/{}/manifest.json", pin.package_id, pin.version)
                || path == format!("packages/{}/{}/module.wasm", pin.package_id, pin.version)
        }) {
            return Ok(());
        }
        Err(ProtocolError::Invalid("unselected project artifact path"))
    }
    /// Fixed struct field order and sorted file paths define the exact manifest bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)
            .map_err(|_| ProtocolError::Invalid("artifact manifest JSON"))?;
        if bytes.len() as u64 > PROJECT_ARTIFACT_MANIFEST_BYTES {
            return Err(ProtocolError::Invalid("artifact manifest size"));
        }
        Ok(bytes)
    }
    pub fn descriptor(&self) -> Result<ProjectArtifactDescriptor> {
        let bytes = self.canonical_bytes()?;
        Ok(ProjectArtifactDescriptor {
            project_id: self.project_id.clone(),
            source: self.source,
            manifest_sha256: artifact_sha256(&bytes),
            manifest_size: bytes.len() as u64,
            file_count: self.files.len() as u32,
            total_bytes: self.files.iter().map(|f| f.size).sum(),
        })
    }
    pub fn validate(&self) -> Result<()> {
        validate_artifact_project_id(&self.project_id)?;
        if self.version != 1
            || self.files.is_empty()
            || self.files.len() > PROJECT_ARTIFACT_MAX_FILES
        {
            return Err(ProtocolError::Invalid("artifact manifest shape"));
        }
        if self.bit_pins.len() > 256 || self.package_pins.len() > 256 {
            return Err(ProtocolError::Invalid("artifact asset selection bound"));
        }
        let mut selected_bits = HashSet::new();
        for pin in &self.bit_pins {
            pin.validate()?;
            if !selected_bits.insert(pin.bit_id.as_str()) {
                return Err(ProtocolError::Invalid("duplicate artifact Bit selection"));
            }
        }
        let mut selected_packages = HashSet::new();
        for pin in &self.package_pins {
            pin.validate()?;
            if !selected_packages.insert((&pin.package_id, &pin.version)) {
                return Err(ProtocolError::Invalid(
                    "duplicate artifact package selection",
                ));
            }
        }
        let mut previous = "";
        let mut paths = HashSet::new();
        let mut total = 0u64;
        for file in &self.files {
            self.validate_file_path(&file.path)?;
            if self.source == ProjectArtifactSource::Online
                && file.path.starts_with("apps/")
                && file.path != format!("apps/{}/online-source.json", self.project_id)
            {
                return Err(ProtocolError::Invalid(
                    "online artifact may only contain its source marker and dependencies",
                ));
            }
            validate_artifact_digest(&file.sha256)?;
            if file.path.as_str() <= previous || file.size > PROJECT_ARTIFACT_MAX_FILE_BYTES {
                return Err(ProtocolError::Invalid("artifact file order or size"));
            }
            let lower = file.path.to_lowercase().nfc().collect::<String>();
            if !paths.insert(lower) {
                return Err(ProtocolError::Invalid("artifact case collision"));
            }
            previous = &file.path;
            total = total
                .checked_add(file.size)
                .ok_or(ProtocolError::Invalid("artifact size overflow"))?;
        }
        for path in &paths {
            for (offset, _) in path.match_indices('/') {
                if paths.contains(&path[..offset]) {
                    return Err(ProtocolError::Invalid("artifact file/directory collision"));
                }
            }
        }
        for pin in &self.bit_pins {
            let path = format!("bits/metadata/{}.json", pin.bit_id);
            if !self.files.iter().any(|f| {
                f.path == path
                    && f.sha256 == pin.metadata_sha256
                    && f.size > 0
                    && f.size <= 16 * 1024 * 1024
            }) {
                return Err(ProtocolError::Invalid(
                    "selected Bit metadata is missing or differs",
                ));
            }
        }
        for pin in &self.package_pins {
            for (name, digest) in [
                ("manifest.json", &pin.manifest_sha256),
                ("module.wasm", &pin.wasm_sha256),
            ] {
                let path = format!("packages/{}/{}/{name}", pin.package_id, pin.version);
                if !self
                    .files
                    .iter()
                    .any(|f| f.path == path && &f.sha256 == digest && f.size > 0)
                {
                    return Err(ProtocolError::Invalid(
                        "selected WASM package artifact is missing or differs",
                    ));
                }
            }
        }
        if total > PROJECT_ARTIFACT_MAX_BYTES {
            return Err(ProtocolError::Invalid("artifact total size"));
        }
        if !self.files.iter().any(|f| {
            f.path
                == format!(
                    "apps/{}/{}",
                    self.project_id,
                    if self.source == ProjectArtifactSource::Online {
                        "online-source.json"
                    } else {
                        "manifest.app"
                    }
                )
                && f.size > 0
        }) {
            return Err(ProtocolError::Invalid(
                "project source manifest is required",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn manifest() -> ProjectArtifactManifest {
        ProjectArtifactManifest {
            version: 1,
            source: ProjectArtifactSource::Offline,
            project_id: "project".into(),
            bit_pins: vec![],
            package_pins: vec![],
            files: vec![ProjectArtifactFile {
                path: "apps/project/manifest.app".into(),
                size: 1,
                sha256: artifact_sha256(b"x"),
            }],
        }
    }
    #[test]
    fn online_dependencies_are_bound_to_the_source_kind() {
        let mut online = manifest();
        online.source = ProjectArtifactSource::Online;
        assert!(online.validate().is_err());
        online.files[0].path = "apps/project/online-source.json".into();
        assert!(online.validate().is_ok());
        assert_eq!(
            online.descriptor().unwrap().source,
            ProjectArtifactSource::Online
        );
        let digest = online.descriptor().unwrap().manifest_sha256;
        online.source = ProjectArtifactSource::Offline;
        assert!(online.validate().is_err());
        online.files[0].path = "apps/project/manifest.app".into();
        assert_ne!(online.descriptor().unwrap().manifest_sha256, digest);
        assert!(
            !String::from_utf8(online.canonical_bytes().unwrap())
                .unwrap()
                .contains("\"source\"")
        );
    }
    #[test]
    fn paths_reject_cross_project_secrets_traversal_and_aliases() {
        for path in [
            "apps/other/manifest.app",
            "apps/project/../file",
            "apps/project/.secrets/key",
            "apps/project/CON.txt",
            "apps/project/x\\y",
            "apps/project/file.",
            "apps/project/re\u{301}sume",
            "apps/project//x",
        ] {
            assert!(validate_artifact_path("project", path).is_err(), "{path}");
        }
        assert!(validate_artifact_path("project", "apps/project/storage/files/résumé.pdf").is_ok());
        let mut value = manifest();
        value.files.push(ProjectArtifactFile {
            path: "apps/project/MANIFEST.app".into(),
            ..value.files[0].clone()
        });
        value.files.sort_by(|a, b| a.path.cmp(&b.path));
        assert!(value.validate().is_err());
    }
    #[test]
    fn descriptor_binds_exact_bounded_manifest() {
        let value = manifest();
        let descriptor = value.descriptor().unwrap();
        descriptor.validate().unwrap();
        assert_eq!(
            descriptor.manifest_size,
            value.canonical_bytes().unwrap().len() as u64
        );
        assert_eq!(descriptor.total_bytes, 1);
        let mut changed = value;
        changed.files[0].size = PROJECT_ARTIFACT_MAX_FILE_BYTES + 1;
        assert!(changed.descriptor().is_err());
    }

    #[test]
    fn selected_bit_assets_allow_nested_files_without_path_aliases() {
        let mut value = manifest();
        let nested = "bits/model-hash/vision_encoder/config.json";
        assert!(value.validate_file_path(nested).is_err());
        value.bit_pins.push(ProjectBitPin {
            bit_id: "model".into(),
            metadata_sha256: artifact_sha256(b"metadata"),
        });
        assert!(value.validate_file_path(nested).is_ok());
        for path in [
            "bits/model-hash/vision_encoder/../config.json",
            "bits/model-hash/vision_encoder//config.json",
            "bits/model-hash/vision_encoder/.secrets/key",
            "bits/model-hash/vision_encoder\\config.json",
            "bits/deps-cache/nested/cache.bin",
            "bits/metadata/nested/model.json",
        ] {
            assert!(value.validate_file_path(path).is_err(), "{path}");
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPackagePin {
    pub package_id: String,
    pub version: String,
    pub wasm_sha256: String,
    pub manifest_sha256: String,
}
impl ProjectPackagePin {
    pub fn validate(&self) -> Result<()> {
        validate_artifact_project_id(&self.package_id)?;
        validate_artifact_project_id(&self.version)?;
        validate_artifact_digest(&self.wasm_sha256)?;
        validate_artifact_digest(&self.manifest_sha256)?;
        Ok(())
    }
}
