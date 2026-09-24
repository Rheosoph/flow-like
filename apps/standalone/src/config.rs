use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PlacementConfig {
    pub id: String,
    pub project_id: String,
    pub deployment_id: String,
    pub revision: String,
    pub source: ProjectSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online_metadata_sha256: Option<String>,
    pub project_path: PathBuf,
    pub events: Vec<EventBinding>,
    #[serde(default)]
    pub artifact_pins: Vec<ArtifactPin>,
    #[serde(default)]
    pub package_pins: Vec<flow_like_device_protocol::ProjectPackagePin>,
    #[serde(default)]
    pub bit_pins: Vec<flow_like_device_protocol::ProjectBitPin>,
    #[serde(default)]
    pub hosting: Option<HostingConfig>,
    #[serde(default = "default_max_replicas")]
    pub max_replicas: u8,
    #[serde(default)]
    pub variables: BTreeMap<String, Value>,
    #[serde(default)]
    pub secret_overrides: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_grant: Option<ResourceGrantRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offline_writes: Option<crate::outbox::BufferingConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resources: Option<crate::isolation::PlacementResources>,
    #[serde(default)]
    pub restart: RestartPolicy,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ResourceGrantRef {
    pub grant_id: String,
    pub authz_version: u64,
    #[serde(default)]
    pub billing_grant_id: Option<String>,
    #[serde(default)]
    pub billing_authz_version: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct WorkloadIdentity {
    pub instance_id: String,
    pub device_id: String,
    pub device_auth_epoch: u64,
    pub key_epoch: u64,
}

impl WorkloadIdentity {
    pub fn validate(&self) -> Result<()> {
        validate_id("instance", &self.instance_id)?;
        validate_id("device", &self.device_id)?;
        ensure!(
            self.device_auth_epoch > 0 && self.key_epoch > 0,
            "Invalid workload identity epoch"
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSource {
    Offline,
    Online,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventBinding {
    pub event_id: String,
    pub event_version: [u32; 3],
    pub board_version: [u32; 3],
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Widget,
    Template,
}
impl ArtifactKind {
    pub fn path(self) -> &'static str {
        match self {
            Self::Widget => "widgets",
            Self::Template => "templates",
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactPin {
    pub kind: ArtifactKind,
    pub id: String,
    pub version: [u32; 3],
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HostingConfig {
    pub host: std::net::IpAddr,
    pub port: u16,
    pub max_in_flight: u16,
    pub request_timeout_secs: u32,
    pub auth_secret: String,
}
impl HostingConfig {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.port > 0
                && (1..=1024).contains(&self.max_in_flight)
                && (1..=3600).contains(&self.request_timeout_secs),
            "Invalid service listener limits"
        );
        validate_id("service authentication secret", &self.auth_secret)?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct RestartPolicy {
    pub initial_backoff_secs: u64,
    pub max_backoff_secs: u64,
    /// Automatic restarts after consecutive failures, before an explicit start is required.
    pub max_restarts: u32,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_backoff_secs: 2,
            max_backoff_secs: 60,
            max_restarts: 5,
        }
    }
}

fn default_max_replicas() -> u8 {
    1
}

impl PlacementConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let path = path
            .canonicalize()
            .context("Cannot resolve placement manifest")?;
        let file = std::fs::File::open(&path)?;
        ensure!(
            file.metadata()?.len() <= MAX_MANIFEST_BYTES,
            "Placement manifest exceeds 1 MiB"
        );
        use std::io::Read;
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST_BYTES + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_MANIFEST_BYTES,
            "Placement manifest exceeds 1 MiB"
        );
        let mut config: Self =
            serde_json::from_slice(&bytes).context("Invalid placement manifest")?;
        if config.project_path.is_relative() {
            config.project_path = path
                .parent()
                .context("Manifest has no parent")?
                .join(&config.project_path);
        }
        config.project_path = config
            .project_path
            .canonicalize()
            .context("Project path does not exist")?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        if let Some(resources) = &self.resources {
            resources.validate()?;
        }
        ensure!(
            (1..=32).contains(&self.max_replicas),
            "Replica limit must be between 1 and 32"
        );
        ensure!(
            self.max_replicas == 1 || self.hosting.is_some(),
            "Multiple replicas require the native HTTP service host"
        );
        if let Some(hosting) = &self.hosting {
            hosting.validate()?;
        }
        for (label, value) in [
            ("placement", &self.id),
            ("project", &self.project_id),
            ("deployment", &self.deployment_id),
        ] {
            validate_id(label, value)?;
        }
        ensure!(
            self.id != "device",
            "The device telemetry scope is reserved"
        );
        ensure!(
            !self.revision.trim().is_empty() && self.revision.len() <= 256,
            "A bounded revision identifier is required"
        );
        if self.source == ProjectSource::Online {
            ensure!(
                self.resource_grant.is_some(),
                "Online placements require a project resource grant"
            );
            let digest = self.online_metadata_sha256.as_deref().context(
                "Online deployment requires controller-approved executable metadata. Prepare and deploy this project again",
            )?;
            flow_like_device_protocol::validate_artifact_digest(digest)?;
        }
        if let Some(buffering) = &self.offline_writes {
            ensure!(
                self.source == ProjectSource::Online,
                "Offline write buffering applies only to online project sources"
            );
            ensure!(
                self.max_replicas == 1,
                "Offline write buffering currently requires one placement replica"
            );
            buffering.validate()?;
        }
        ensure!(
            self.project_path.is_absolute() && self.project_path.is_dir(),
            "Project path must be an existing absolute directory"
        );
        ensure!(
            !self.events.is_empty() && self.events.len() <= 64,
            "A placement requires 1 to 64 pinned events"
        );
        let mut ids = std::collections::HashSet::new();
        for event in &self.events {
            validate_id("event", &event.event_id)?;
            ensure!(
                ids.insert(&event.event_id),
                "Duplicate event: {}",
                event.event_id
            );
            ensure!(
                !event.event_version.contains(&u32::MAX)
                    && !event.board_version.contains(&u32::MAX),
                "Events and boards must be pinned to concrete versions"
            );
        }
        ensure!(
            self.artifact_pins.len() <= 256,
            "Too many pinned project artifacts"
        );
        let mut artifacts = std::collections::HashSet::new();
        for pin in &self.artifact_pins {
            validate_id("artifact", &pin.id)?;
            ensure!(
                !pin.version.contains(&u32::MAX) && artifacts.insert((pin.kind, pin.id.as_str())),
                "Project artifacts require unique identities and concrete versions"
            );
        }
        ensure!(
            self.package_pins.len() <= 64,
            "Too many pinned WASM packages"
        );
        let mut packages = std::collections::HashSet::new();
        for pin in &self.package_pins {
            pin.validate()?;
            ensure!(packages.insert(&pin.package_id), "Duplicate package pin");
        }
        ensure!(self.bit_pins.len() <= 256, "Too many pinned Bits");
        let mut bits = std::collections::HashSet::new();
        for pin in &self.bit_pins {
            pin.validate()?;
            ensure!(bits.insert(&pin.bit_id), "Duplicate Bit pin");
        }
        ensure!(
            self.variables.len() + self.secret_overrides.len() <= 1024,
            "Too many placement variable overrides"
        );
        for (variable, name) in &self.secret_overrides {
            validate_id("variable", variable)?;
            validate_id("secret", name)?;
            ensure!(
                !self.variables.contains_key(variable),
                "Variable has both a plaintext and secret override"
            );
        }
        if let Some(grant) = &self.resource_grant {
            validate_id("resource grant", &grant.grant_id)?;
            if let Some(id) = &grant.billing_grant_id {
                validate_id("billing grant", id)?;
            }
            ensure!(
                grant.authz_version > 0
                    && grant.billing_grant_id.is_some() == grant.billing_authz_version.is_some()
                    && grant.billing_authz_version != Some(0),
                "Resource and billing grants require positive authorization versions"
            );
        }
        ensure!(
            self.restart.initial_backoff_secs > 0
                && self.restart.max_backoff_secs >= self.restart.initial_backoff_secs
                && self.restart.max_backoff_secs <= 3600,
            "Restart backoff must be between 1 second and 1 hour"
        );
        ensure!(
            self.restart.max_restarts <= 100,
            "Restart limit cannot exceed 100"
        );
        Ok(())
    }
}

pub(crate) fn private_secret_path(config: &PlacementConfig, name: &str) -> Result<PathBuf> {
    validate_id("secret", name)?;
    let mut path = config.project_path.clone();
    for component in [".secrets", config.id.as_str()] {
        ensure_unaliased_child(&path, component)?;
        path.push(component);
        let metadata =
            std::fs::symlink_metadata(&path).context("Placement secret directory is missing")?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Placement secret directory must not be a symlink"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            ensure!(
                metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
                "Placement secrets must be private and owned by this user"
            );
        }
    }
    let file_name = format!("{name}.secret");
    ensure_unaliased_child(&path, &file_name)?;
    Ok(path.join(file_name))
}

/// Identifiers remain case-sensitive even on filesystems which fold their names.
pub(crate) fn ensure_unaliased_child(parent: &Path, name: &str) -> Result<()> {
    ensure!(
        !name.is_empty()
            && name.is_ascii()
            && !name.contains(['/', '\\', '\0'])
            && name != "."
            && name != "..",
        "Invalid filesystem identity"
    );
    let mut exact = false;
    for entry in std::fs::read_dir(parent)? {
        let entry = entry?;
        let entry_name = entry.file_name();
        if let Some(entry_name) = entry_name.to_str() {
            ensure!(
                !entry_name.eq_ignore_ascii_case(name) || entry_name == name,
                "Filesystem identity differs only by case"
            );
            exact |= entry_name == name;
        }
    }
    ensure!(
        !parent.join(name).try_exists()? || exact,
        "Filesystem resolved a different identity spelling"
    );
    Ok(())
}

pub(crate) fn validate_id(label: &str, id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 128
            && id != "."
            && id != ".."
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'),
        "Invalid {label} identifier"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_project_and_requires_online_authorization() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = dir.path().join("placement.json");
        let value = serde_json::json!({"id":"service","project_id":"app","deployment_id":"deploy","revision":"v1","source":"offline","project_path":".","events":[{"event_id":"rest","event_version":[1,0,0],"board_version":[1,0,0]}]});
        std::fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        let mut config = PlacementConfig::load(&manifest).unwrap();
        assert_eq!(config.project_path, dir.path().canonicalize().unwrap());
        config.source = ProjectSource::Online;
        assert!(
            config
                .validate()
                .unwrap_err()
                .to_string()
                .contains("project resource grant")
        );
        config.source = ProjectSource::Offline;
        config.events.push(config.events[0].clone());
        assert!(config.validate().is_err());
    }

    #[test]
    fn rejects_path_identifiers() {
        for id in ["", "..", "../app", "a/b", "a\\b"] {
            assert!(validate_id("placement", id).is_err());
        }
    }
}
