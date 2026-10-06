use crate::{ModelAssetDescriptor, ProtocolError, Result};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use unicode_normalization::UnicodeNormalization;

pub const PROJECT_ARTIFACT_CHUNK_BYTES: usize = 8192;
pub const PROJECT_ARTIFACT_MANIFEST_BYTES: u64 = 2 * 1024 * 1024;
pub const PROJECT_ARTIFACT_MAX_FILES: usize = 8192;
pub const PROJECT_ARTIFACT_MAX_FILE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
pub const PROJECT_ARTIFACT_MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const PROJECT_ARTIFACT_TTL_SECONDS: i64 = 86_400;
pub const PROJECT_ARTIFACT_PRUNE_REVISIONS: usize = 64;

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
    /// Storage in use next to its budgets: the device's, and with a project also that
    /// project's and its retained revisions after revision `after`.
    Usage {
        project_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
    },
    /// Removes exactly the listed revisions of one project.
    Prune {
        project_id: String,
        revisions: Vec<String>,
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
    /// The project a request acts on; none when it asks for the device's storage only.
    pub fn project_id(&self) -> Option<&str> {
        match self {
            Self::Begin { descriptor } => Some(&descriptor.project_id),
            Self::Chunk { project_id, .. }
            | Self::Status { project_id, .. }
            | Self::Commit { project_id, .. }
            | Self::Abort { project_id, .. }
            | Self::PrepareOnline { project_id }
            | Self::Describe { project_id, .. }
            | Self::Prune { project_id, .. } => Some(project_id),
            Self::Usage { project_id, .. } => project_id.as_deref(),
        }
    }
    pub fn journaled(&self) -> bool {
        matches!(
            self,
            Self::Begin { .. }
                | Self::Commit { .. }
                | Self::Abort { .. }
                | Self::PrepareOnline { .. }
                | Self::Prune { .. }
        )
    }
}

/// Revisions are named one by one: an empty list never stands for every revision.
pub fn validate_artifact_prune(revisions: &[String]) -> Result<()> {
    if revisions.is_empty() || revisions.len() > PROJECT_ARTIFACT_PRUNE_REVISIONS {
        return Err(ProtocolError::Invalid("artifact revision selection"));
    }
    let mut selected = HashSet::new();
    for revision in revisions {
        validate_artifact_digest(revision)?;
        if !selected.insert(revision) {
            return Err(ProtocolError::Invalid("duplicate artifact revision"));
        }
    }
    Ok(())
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
                && file.path != format!("apps/{}/online-metadata.json", self.project_id)
            {
                return Err(ProtocolError::Invalid(
                    "online artifact may only contain its approved metadata, source marker and dependencies",
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
    fn capacity_requests_round_trip_with_their_wire_names() {
        let digest = artifact_sha256(b"revision");
        for (wire, project, journaled) in [
            (
                serde_json::json!({"kind":"usage","project_id":null}),
                None,
                false,
            ),
            (
                serde_json::json!({"kind":"usage","project_id":"project","after":digest}),
                Some("project"),
                false,
            ),
            (
                serde_json::json!({"kind":"prune","project_id":"project","revisions":[digest]}),
                Some("project"),
                true,
            ),
        ] {
            let request: ArtifactRequest = serde_json::from_value(wire.clone()).unwrap();
            assert_eq!(request.project_id(), project, "{wire}");
            assert_eq!(request.journaled(), journaled, "{wire}");
            assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        }
        assert!(matches!(
            serde_json::from_value::<ArtifactRequest>(serde_json::json!({"kind":"usage"})).unwrap(),
            ArtifactRequest::Usage {
                project_id: None,
                after: None
            }
        ));
        for wire in [
            serde_json::json!({"kind":"prune","project_id":"project"}),
            serde_json::json!({"kind":"prune","project_id":"project","revisions":[digest],"all":true}),
            serde_json::json!({"kind":"usage","project_id":null,"limit":8}),
        ] {
            assert!(
                serde_json::from_value::<ArtifactRequest>(wire.clone()).is_err(),
                "{wire}"
            );
        }
    }

    #[test]
    fn revisions_to_remove_are_named_one_by_one() {
        let digest = |index: usize| artifact_sha256(index.to_string().as_bytes());
        let listed = |count: usize| (0..count).map(digest).collect::<Vec<_>>();
        assert!(validate_artifact_prune(&listed(1)).is_ok());
        assert!(validate_artifact_prune(&listed(PROJECT_ARTIFACT_PRUNE_REVISIONS)).is_ok());
        for refused in [
            Vec::new(),
            listed(PROJECT_ARTIFACT_PRUNE_REVISIONS + 1),
            vec![digest(0), digest(0)],
            vec!["*".into()],
            vec![digest(0).to_uppercase()],
        ] {
            assert!(validate_artifact_prune(&refused).is_err(), "{refused:?}");
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

pub const PACKAGED_BIT_METADATA_MAX_BYTES: u64 = 16 * 1024 * 1024;
pub const PACKAGED_BIT_MAX_DEPENDENCIES: usize = 2048;
/// Artifact files and model-store assets of one packaged Bit together.
pub const PACKAGED_BIT_MAX_FILES: usize = 2048;

/// One model-store file of a packaged Bit. A Bit with several entries lists its parts in load
/// order, and the first carries the Bit's own `file_name`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagedBitAsset {
    pub bit_id: String,
    pub descriptor: ModelAssetDescriptor,
}

/// The `"version": 2` of Bit metadata v2.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "u32", into = "u32")]
pub struct PackagedBitMetadataVersion2;

impl TryFrom<u32> for PackagedBitMetadataVersion2 {
    type Error = ProtocolError;

    fn try_from(version: u32) -> Result<Self> {
        if version != 2 {
            return Err(ProtocolError::Invalid("packaged Bit metadata version"));
        }
        Ok(Self)
    }
}

impl From<PackagedBitMetadataVersion2> for u32 {
    fn from(_: PackagedBitMetadataVersion2) -> Self {
        2
    }
}

/// Every Bit file travels in the artifact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagedBitMetadataV1 {
    pub bit: Value,
    pub dependencies: Vec<Value>,
    pub artifacts: Vec<ProjectArtifactFile>,
}

/// The device acquires `assets` into its model store itself; small files may still travel in
/// the artifact as `artifacts`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackagedBitMetadataV2 {
    pub version: PackagedBitMetadataVersion2,
    pub bit: Value,
    #[serde(default)]
    pub dependencies: Vec<Value>,
    pub assets: Vec<PackagedBitAsset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<ProjectArtifactFile>,
}

/// `bits/metadata/<bit_id>.json` of a project artifact, pinned by
/// `ProjectBitPin.metadata_sha256`. v1 has no `version` field. Bits stay JSON here, and every
/// Bit with a `file_name` has one file location, `bits/<hash>/<file_name>`, whose bytes are an
/// artifact file or, in v2, model-store assets.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(try_from = "Map<String, Value>")]
pub enum PackagedBitMetadata {
    V1(PackagedBitMetadataV1),
    V2(PackagedBitMetadataV2),
}

impl TryFrom<Map<String, Value>> for PackagedBitMetadata {
    type Error = serde_json::Error;

    fn try_from(fields: Map<String, Value>) -> std::result::Result<Self, Self::Error> {
        if fields.contains_key("version") {
            serde_json::from_value(Value::Object(fields)).map(Self::V2)
        } else {
            serde_json::from_value(Value::Object(fields)).map(Self::V1)
        }
    }
}

impl Serialize for PackagedBitMetadata {
    fn serialize<S: Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::V1(metadata) => metadata.serialize(serializer),
            Self::V2(metadata) => metadata.serialize(serializer),
        }
    }
}

/// The file location a packaged Bit names.
struct BitFile {
    id: Option<String>,
    path: String,
    file_name: String,
    size: Option<u64>,
}

fn bit_file(bit: &Value) -> Result<Option<BitFile>> {
    let Some(name) = bit.get("file_name").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let file_name = name
        .as_str()
        .ok_or(ProtocolError::Invalid("packaged Bit file name"))?;
    let path = format!("bits/{}/{file_name}", bit_hash(bit)?);
    validate_artifact_relative_path(&path)?;
    Ok(Some(BitFile {
        id: bit.get("id").and_then(Value::as_str).map(str::to_owned),
        path,
        file_name: file_name.to_owned(),
        size: bit_size(bit)?,
    }))
}

/// The directory of a Bit's file; `metadata` and `deps-cache` belong to the Bit store.
fn bit_hash(bit: &Value) -> Result<&str> {
    let hash = bit
        .get("hash")
        .and_then(Value::as_str)
        .ok_or(ProtocolError::Invalid("packaged Bit file hash"))?;
    if hash.contains('/') || hash == "metadata" || hash == "deps-cache" {
        return Err(ProtocolError::Invalid("packaged Bit file path"));
    }
    Ok(hash)
}

fn bit_size(bit: &Value) -> Result<Option<u64>> {
    bit.get("size")
        .filter(|value| !value.is_null())
        .map(|size| {
            size.as_u64()
                .ok_or(ProtocolError::Invalid("packaged Bit file size"))
        })
        .transpose()
}

/// The Bit each asset belongs to, by id; an id that two file-backed Bits share owns nothing.
fn asset_owners(files: &[BitFile]) -> HashMap<&str, Option<&BitFile>> {
    let mut owners = HashMap::new();
    for file in files {
        if let Some(id) = &file.id {
            owners
                .entry(id.as_str())
                .and_modify(|owner| *owner = None)
                .or_insert(Some(file));
        }
    }
    owners
}

fn validate_parts(file: &BitFile, parts: &[&ModelAssetDescriptor]) -> Result<()> {
    if parts
        .first()
        .is_some_and(|first| first.file_name != file.file_name)
    {
        return Err(ProtocolError::Invalid(
            "first model asset of a packaged Bit has another file name",
        ));
    }
    for (index, part) in parts.iter().enumerate() {
        if parts[..index]
            .iter()
            .any(|earlier| earlier.file_name == part.file_name || earlier.digest == part.digest)
        {
            return Err(ProtocolError::Invalid(
                "duplicate model asset of a packaged Bit",
            ));
        }
    }
    if let [single] = parts
        && file.size.is_some_and(|size| size != single.size)
    {
        return Err(ProtocolError::Invalid(
            "model asset size differs from its packaged Bit",
        ));
    }
    Ok(())
}

/// The ids of the Bits whose bytes are model-store assets.
fn asset_backed(assets: &[PackagedBitAsset], files: &[BitFile]) -> Result<HashSet<String>> {
    let owners = asset_owners(files);
    let mut parts: HashMap<&str, Vec<&ModelAssetDescriptor>> = HashMap::new();
    for asset in assets {
        asset.descriptor.validate()?;
        if !matches!(owners.get(asset.bit_id.as_str()), Some(Some(_))) {
            return Err(ProtocolError::Invalid(
                "model asset of an unknown or ambiguous packaged Bit",
            ));
        }
        parts
            .entry(asset.bit_id.as_str())
            .or_default()
            .push(&asset.descriptor);
    }
    for (id, descriptors) in &parts {
        if let Some(Some(file)) = owners.get(id) {
            validate_parts(file, descriptors)?;
        }
    }
    Ok(parts.into_keys().map(str::to_owned).collect())
}

/// Locations whose bytes travel as artifact files, with the size their Bits declare.
fn artifact_locations(
    files: &[BitFile],
    stored: &HashSet<String>,
) -> Result<BTreeMap<String, Option<u64>>> {
    let mut expected = BTreeMap::new();
    let mut store_paths = HashSet::new();
    for file in files {
        if file.id.as_ref().is_some_and(|id| stored.contains(id)) {
            store_paths.insert(file.path.as_str());
            continue;
        }
        if let Some(previous) = expected.insert(file.path.clone(), file.size)
            && previous != file.size
        {
            return Err(ProtocolError::Invalid(
                "conflicting packaged Bit file sizes",
            ));
        }
    }
    if expected
        .keys()
        .any(|path| store_paths.contains(path.as_str()))
    {
        return Err(ProtocolError::Invalid(
            "packaged Bit file listed as an artifact and as model assets",
        ));
    }
    Ok(expected)
}

fn validate_packaged_artifacts(
    artifacts: &[ProjectArtifactFile],
    expected: &BTreeMap<String, Option<u64>>,
) -> Result<()> {
    if artifacts.len() != expected.len() {
        return Err(ProtocolError::Invalid(
            "packaged Bit artifacts differ from its files",
        ));
    }
    let mut seen = HashSet::new();
    for file in artifacts {
        let size = expected
            .get(&file.path)
            .ok_or(ProtocolError::Invalid("unselected packaged Bit artifact"))?;
        validate_artifact_digest(&file.sha256)?;
        if size.is_some_and(|size| size != file.size)
            || file.size > PROJECT_ARTIFACT_MAX_FILE_BYTES
            || !seen.insert(file.path.as_str())
        {
            return Err(ProtocolError::Invalid(
                "packaged Bit artifact size or identity",
            ));
        }
    }
    Ok(())
}

impl PackagedBitMetadata {
    pub fn bit(&self) -> &Value {
        match self {
            Self::V1(metadata) => &metadata.bit,
            Self::V2(metadata) => &metadata.bit,
        }
    }

    pub fn dependencies(&self) -> &[Value] {
        match self {
            Self::V1(metadata) => &metadata.dependencies,
            Self::V2(metadata) => &metadata.dependencies,
        }
    }

    /// Files whose bytes travel in the artifact.
    pub fn artifacts(&self) -> &[ProjectArtifactFile] {
        match self {
            Self::V1(metadata) => &metadata.artifacts,
            Self::V2(metadata) => &metadata.artifacts,
        }
    }

    /// Files the device acquires into its model store; v1 names none.
    pub fn assets(&self) -> &[PackagedBitAsset] {
        match self {
            Self::V1(_) => &[],
            Self::V2(metadata) => &metadata.assets,
        }
    }

    /// The root Bit is `bit_id`, and every Bit file is listed exactly once: as one artifact
    /// file, or as one or more model-store assets.
    pub fn validate(&self, bit_id: &str) -> Result<()> {
        if self.bit().get("id").and_then(Value::as_str) != Some(bit_id) {
            return Err(ProtocolError::Invalid("packaged Bit identity"));
        }
        if self.dependencies().len() > PACKAGED_BIT_MAX_DEPENDENCIES {
            return Err(ProtocolError::Invalid("packaged Bit dependency count"));
        }
        if self.artifacts().len() + self.assets().len() > PACKAGED_BIT_MAX_FILES {
            return Err(ProtocolError::Invalid("packaged Bit file count"));
        }
        let mut files = Vec::new();
        for bit in std::iter::once(self.bit()).chain(self.dependencies()) {
            files.extend(bit_file(bit)?);
        }
        let stored = asset_backed(self.assets(), &files)?;
        validate_packaged_artifacts(self.artifacts(), &artifact_locations(&files, &stored)?)
    }
}

#[cfg(test)]
mod packaged_bit_tests {
    use super::*;
    use crate::{DigestAlgorithm, ModelAssetDigest};
    use serde_json::json;

    fn descriptor(fill: char, file_name: &str, size: u64) -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Blake3,
                hex: fill.to_string().repeat(64),
            },
            size,
            file_name: file_name.into(),
            sources: vec!["https://cdn.flow-like.com/bits/model".into()],
        }
    }

    fn asset(bit_id: &str, fill: char, file_name: &str, size: u64) -> PackagedBitAsset {
        PackagedBitAsset {
            bit_id: bit_id.into(),
            descriptor: descriptor(fill, file_name, size),
        }
    }

    fn artifact(path: &str, size: u64) -> ProjectArtifactFile {
        ProjectArtifactFile {
            path: path.into(),
            size,
            sha256: artifact_sha256(path.as_bytes()),
        }
    }

    /// A model whose weights are a model-store asset and whose tokenizer is an artifact file.
    fn v2() -> PackagedBitMetadataV2 {
        PackagedBitMetadataV2 {
            version: PackagedBitMetadataVersion2,
            bit: json!({"id": "model", "hash": "weights-hash", "file_name": "model.gguf",
                        "size": 4096, "dependencies": ["tokenizer"]}),
            dependencies: vec![json!({"id": "tokenizer", "hash": "tokenizer-hash",
                                      "file_name": "tokenizer.json", "size": 12})],
            assets: vec![asset("model", 'a', "model.gguf", 4096)],
            artifacts: vec![artifact("bits/tokenizer-hash/tokenizer.json", 12)],
        }
    }

    fn checked(metadata: PackagedBitMetadataV2) -> Result<()> {
        PackagedBitMetadata::V2(metadata).validate("model")
    }

    #[test]
    fn versions_are_told_apart_by_the_version_field_and_round_trip() {
        let v1 = json!({"bit": {"id": "model", "hash": "h", "file_name": "model.bin", "size": 7},
                        "dependencies": [],
                        "artifacts": [{"path": "bits/h/model.bin", "size": 7,
                                       "sha256": artifact_sha256(b"weights")}]});
        let parsed: PackagedBitMetadata = serde_json::from_value(v1.clone()).unwrap();
        assert!(matches!(parsed, PackagedBitMetadata::V1(_)));
        assert!(parsed.assets().is_empty());
        assert_eq!(serde_json::to_value(&parsed).unwrap(), v1);
        parsed.validate("model").unwrap();

        let wire = serde_json::to_value(PackagedBitMetadata::V2(v2())).unwrap();
        assert_eq!(wire["version"], 2);
        let parsed: PackagedBitMetadata = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(parsed, PackagedBitMetadata::V2(v2()));
        assert_eq!(parsed.assets().len(), 1);
        parsed.validate("model").unwrap();

        let mut only_assets = v2();
        only_assets.artifacts.clear();
        only_assets.dependencies.clear();
        let mut wire = serde_json::to_value(PackagedBitMetadata::V2(only_assets)).unwrap();
        assert!(wire.get("artifacts").is_none());
        assert!(serde_json::from_value::<PackagedBitMetadata>(wire.clone()).is_ok());
        wire.as_object_mut().unwrap().remove("dependencies");
        let without_dependencies: PackagedBitMetadata = serde_json::from_value(wire).unwrap();
        without_dependencies.validate("model").unwrap();

        for refused in [
            json!({"version": 1, "bit": {}, "dependencies": [], "assets": []}),
            json!({"version": 3, "bit": {}, "dependencies": [], "assets": []}),
            json!({"version": "2", "bit": {}, "dependencies": [], "assets": []}),
            json!({"version": 2, "bit": {}, "dependencies": []}),
            json!({"version": 2, "bit": {}, "dependencies": [], "assets": [], "extra": 1}),
            json!({"bit": {}, "dependencies": [], "artifacts": [], "assets": []}),
            json!({"bit": {}, "dependencies": []}),
            json!({"dependencies": [], "artifacts": []}),
        ] {
            assert!(
                serde_json::from_value::<PackagedBitMetadata>(refused.clone()).is_err(),
                "{refused}"
            );
        }
    }

    #[test]
    fn v1_lists_every_file_as_an_artifact_exactly_once() {
        let metadata = |artifacts: Value| {
            serde_json::from_value::<PackagedBitMetadata>(json!({
                "bit": {"id": "model", "hash": "h", "file_name": "model.bin", "size": 7},
                "dependencies": [{"id": "projector", "hash": "p", "file_name": "mmproj.gguf"}],
                "artifacts": artifacts,
            }))
            .unwrap()
        };
        let weights =
            json!({"path": "bits/h/model.bin", "size": 7, "sha256": artifact_sha256(b"w")});
        let projector =
            json!({"path": "bits/p/mmproj.gguf", "size": 3, "sha256": artifact_sha256(b"p")});
        metadata(json!([weights, projector]))
            .validate("model")
            .unwrap();
        assert!(
            metadata(json!([weights, projector]))
                .validate("other")
                .is_err()
        );
        assert!(metadata(json!([weights])).validate("model").is_err());
        assert!(
            metadata(json!([weights, weights]))
                .validate("model")
                .is_err()
        );
        let mut resized = weights.clone();
        resized["size"] = json!(8);
        assert!(
            metadata(json!([resized, projector]))
                .validate("model")
                .is_err()
        );
        let mut elsewhere = projector.clone();
        elsewhere["path"] = json!("bits/other/mmproj.gguf");
        assert!(
            metadata(json!([weights, elsewhere]))
                .validate("model")
                .is_err()
        );
    }

    #[test]
    fn v2_names_store_assets_and_small_artifacts_for_every_bit_file() {
        checked(v2()).unwrap();

        let mut stored_tokenizer = v2();
        stored_tokenizer.artifacts.clear();
        stored_tokenizer
            .assets
            .push(asset("tokenizer", 'c', "tokenizer.json", 12));
        checked(stored_tokenizer).unwrap();

        let mut split = v2();
        split.bit["file_name"] = json!("qwen-00001-of-00002.gguf");
        split.bit["size"] = Value::Null;
        split.assets = vec![
            asset("model", 'a', "qwen-00001-of-00002.gguf", 4096),
            asset("model", 'b', "qwen-00002-of-00002.gguf", 2048),
        ];
        checked(split.clone()).unwrap();
        split.assets.swap(0, 1);
        assert!(checked(split.clone()).is_err(), "first part names the Bit");
        split.assets.swap(0, 1);
        split.assets[1].descriptor.digest = split.assets[0].descriptor.digest.clone();
        assert!(checked(split).is_err(), "parts are distinct");
    }

    #[test]
    fn v2_refuses_files_listed_twice_nowhere_or_unsafely() {
        let mut resized = v2();
        resized.assets[0].descriptor.size = 4097;
        assert!(checked(resized).is_err());

        let mut unknown = v2();
        unknown
            .assets
            .push(asset("missing", 'd', "model.gguf", 4096));
        assert!(checked(unknown).is_err());

        let mut unlisted = v2();
        unlisted.artifacts.clear();
        assert!(
            checked(unlisted).is_err(),
            "the tokenizer is listed nowhere"
        );

        let mut twice = v2();
        twice
            .artifacts
            .push(artifact("bits/weights-hash/model.gguf", 4096));
        assert!(
            checked(twice).is_err(),
            "weights are an asset and an artifact"
        );

        let mut ambiguous = v2();
        ambiguous
            .dependencies
            .push(json!({"id": "model", "hash": "x", "file_name": "model.gguf"}));
        assert!(checked(ambiguous).is_err());

        let mut insecure = v2();
        insecure.assets[0].descriptor.sources = vec!["http://cdn.flow-like.com/model".into()];
        assert!(checked(insecure).is_err());

        let mut traversal = v2();
        traversal.bit["file_name"] = json!("../model.gguf");
        traversal.assets[0].descriptor.file_name = "../model.gguf".into();
        assert!(checked(traversal).is_err());

        let mut renamed = v2();
        renamed.bit["id"] = json!("other");
        assert!(checked(renamed).is_err());
    }
}
