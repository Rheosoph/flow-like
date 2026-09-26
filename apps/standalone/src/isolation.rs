//! Kernel-enforced placement boundaries. The default process profile is for a
//! dedicated, trusted account; requesting the Linux sandbox never falls back.

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const OPERATOR_ENV_TEMPLATE: &str = "# Operator-owned agent configuration. Restart the agent after changes.\n# Use required when project owners must not access the agent or sibling projects.\nFLOW_LIKE_DEVICE_ISOLATION_POLICY=compatible\n# Strict Linux placements also need delegated CPU/memory/PID budgets and ext4 project quotas.\n# FLOW_LIKE_DEVICE_CGROUP_ROOT=/sys/fs/cgroup/flow-like-workloads\n# Artifact admission includes committed revisions and in-flight uploads, plus metadata.\n# FILES counts files and directories; each in-flight file reserves 34 entries until commit.\n# Default per-project FILES admits about 1900 files per upload with no retained revisions.\n# Limits do not remove revisions; lowering them blocks new uploads until usage fits.\nFLOW_LIKE_DEVICE_ARTIFACT_BYTES=68719476736\nFLOW_LIKE_DEVICE_ARTIFACT_FILES=262144\nFLOW_LIKE_DEVICE_ARTIFACT_REVISIONS=1024\nFLOW_LIKE_PROJECT_ARTIFACT_BYTES=17179869184\nFLOW_LIKE_PROJECT_ARTIFACT_FILES=65536\nFLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=128\n";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IsolationProfile {
    TrustedProcess,
    LinuxSandbox,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PlacementResources {
    pub profile: IsolationProfile,
    pub cpu_millis: Option<u32>,
    pub memory_bytes: Option<u64>,
    pub max_processes: Option<u32>,
    /// Aggregate persistent placement data, shared by its replicas. The host
    /// must provision an enforced ext4 project quota before deployment.
    pub disk_bytes: Option<u64>,
}

impl PlacementResources {
    pub fn validate(&self) -> Result<()> {
        match self.profile {
            IsolationProfile::TrustedProcess => ensure!(
                self.cpu_millis.is_none()
                    && self.memory_bytes.is_none()
                    && self.max_processes.is_none()
                    && self.disk_bytes.is_none(),
                "Resource limits require the linux_sandbox profile; trusted_process provides no isolation"
            ),
            IsolationProfile::LinuxSandbox => {
                ensure!(
                    self.cpu_millis
                        .is_some_and(|v| (1..=1_024_000).contains(&v)),
                    "Sandbox CPU limit must be 1..1024000 millicores"
                );
                ensure!(
                    self.memory_bytes
                        .is_some_and(|v| (64 * 1024 * 1024..=16 * 1024_u64.pow(4)).contains(&v)),
                    "Sandbox memory limit must be 64 MiB..16 TiB"
                );
                ensure!(
                    self.max_processes
                        .is_some_and(|v| (16..=65536).contains(&v)),
                    "Sandbox process/thread limit must be 16..65536"
                );
                ensure!(
                    self.disk_bytes
                        .is_some_and(|v| (16 * 1024 * 1024..=1024_u64.pow(5)).contains(&v)),
                    "Sandbox disk limit must be 16 MiB..1 PiB"
                );
            }
        }
        Ok(())
    }
}

#[derive(Debug, Serialize)]
pub struct Capabilities {
    pub platform: &'static str,
    pub landlock_abi: Option<i32>,
    pub bubblewrap: Option<std::path::PathBuf>,
    pub cgroup_root: Option<std::path::PathBuf>,
    pub sandbox_available: bool,
    pub require_isolation: bool,
    /// Namespace policies and the placement's disk quota are checked at launch.
    pub placement_preflight_required: bool,
    pub network_boundary: &'static str,
    pub reason: Option<String>,
    pub disk_requirement: &'static str,
}

pub fn capabilities(state_dir: &Path) -> Capabilities {
    #[cfg(target_os = "linux")]
    let mut capabilities = linux::capabilities(state_dir);
    #[cfg(not(target_os = "linux"))]
    let mut capabilities = Capabilities { platform: std::env::consts::OS, landlock_abi: None, bubblewrap: None, cgroup_root: None, sandbox_available: false, require_isolation: false, placement_preflight_required: true, network_boundary: "shared host network; local services must authenticate callers", reason: Some("linux_sandbox requires Linux; use trusted_process only for a dedicated trusted account".into()), disk_requirement: "enforced ext4 project quota" };
    match host_configuration(state_dir) {
        Ok(config) => capabilities.require_isolation = config.required,
        Err(error) => {
            capabilities.require_isolation = true;
            capabilities.sandbox_available = false;
            capabilities.reason = Some(error.to_string());
        }
    }
    capabilities
}

fn parse_host_policy(value: Option<&std::ffi::OsStr>) -> Result<bool> {
    match value.and_then(std::ffi::OsStr::to_str) {
        None if value.is_none() => Ok(false),
        Some("compatible") => Ok(false),
        Some("required") => Ok(true),
        _ => anyhow::bail!("FLOW_LIKE_DEVICE_ISOLATION_POLICY must be compatible or required"),
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ArtifactLimits {
    pub bytes: u64,
    /// Files and directories, including reserved upload path components.
    pub files: u64,
    pub revisions: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ArtifactBudgets {
    pub device: ArtifactLimits,
    pub project: ArtifactLimits,
}

impl Default for ArtifactBudgets {
    fn default() -> Self {
        Self {
            device: ArtifactLimits {
                bytes: 64 * 1024_u64.pow(3),
                files: 262144,
                revisions: 1024,
            },
            project: ArtifactLimits {
                bytes: 16 * 1024_u64.pow(3),
                files: 65536,
                revisions: 128,
            },
        }
    }
}

pub(crate) fn artifact_budgets(state_dir: &Path) -> Result<ArtifactBudgets> {
    Ok(host_configuration(state_dir)?.artifact_budgets)
}

struct HostConfiguration {
    required: bool,
    artifact_budgets: ArtifactBudgets,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    cgroup_root: Option<PathBuf>,
}

fn host_configuration(state_dir: &Path) -> Result<HostConfiguration> {
    let path = state_dir.join("agent.env");
    let mut policy = None;
    let mut cgroup = None;
    let mut artifact_budgets = ArtifactBudgets::default();
    let mut seen_artifact_keys = std::collections::HashSet::new();
    match std::fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(error.into()),
        Ok(_) => {
            let bytes = crate::vault::read_private(&path)?;
            ensure!(
                bytes.len() <= 4096,
                "Agent configuration exceeds 4096 bytes"
            );
            for entry in dotenvy::from_read_iter(bytes.as_slice()) {
                let (key, value) =
                    entry.map_err(|_| anyhow::anyhow!("Invalid agent.env syntax"))?;
                match key.as_str() {
                    "FLOW_LIKE_DEVICE_ISOLATION_POLICY" => {
                        ensure!(policy.is_none(), "Repeated agent isolation policy");
                        policy = Some(value);
                    }
                    "FLOW_LIKE_DEVICE_CGROUP_ROOT" => {
                        ensure!(cgroup.is_none(), "Repeated agent cgroup root");
                        cgroup = Some(PathBuf::from(value));
                    }
                    key if key.starts_with("FLOW_LIKE_DEVICE_ARTIFACT_")
                        || key.starts_with("FLOW_LIKE_PROJECT_ARTIFACT_") =>
                    {
                        ensure!(
                            seen_artifact_keys.insert(key.to_owned()),
                            "Repeated artifact storage limit"
                        );
                        let (limits, suffix) =
                            if let Some(suffix) = key.strip_prefix("FLOW_LIKE_DEVICE_ARTIFACT_") {
                                (&mut artifact_budgets.device, suffix)
                            } else {
                                (
                                    &mut artifact_budgets.project,
                                    key.strip_prefix("FLOW_LIKE_PROJECT_ARTIFACT_").unwrap(),
                                )
                            };
                        let number: u64 = value.parse().map_err(|_| {
                            anyhow::anyhow!("Artifact storage limits must be positive integers")
                        })?;
                        let (target, maximum) = match suffix {
                            "BYTES" => (&mut limits.bytes, 1024_u64.pow(5)),
                            "FILES" => (&mut limits.files, 1_000_000),
                            "REVISIONS" => (&mut limits.revisions, 16_384),
                            _ => anyhow::bail!("Unknown artifact storage limit {key}"),
                        };
                        ensure!(
                            (1..=maximum).contains(&number),
                            "Artifact storage limit {key} must be 1..{maximum}"
                        );
                        *target = number;
                    }
                    _ => (),
                }
            }
        }
    }
    let required = parse_host_policy(policy.as_deref().map(std::ffi::OsStr::new))?;
    let environment_required =
        parse_host_policy(std::env::var_os("FLOW_LIKE_DEVICE_ISOLATION_POLICY").as_deref())?;
    Ok(HostConfiguration {
        // An ambient compatible setting must never weaken the durable policy.
        required: required || environment_required,
        artifact_budgets,
        cgroup_root: std::env::var_os("FLOW_LIKE_DEVICE_CGROUP_ROOT")
            .map(PathBuf::from)
            .or(cgroup),
    })
}

pub(crate) fn enforce_host_policy(
    config: &crate::config::PlacementConfig,
    state_dir: &Path,
) -> Result<()> {
    ensure!(
        !host_configuration(state_dir)?.required || sandboxed(config),
        "This device requires linux_sandbox for every placement; a project cannot disable the host's isolation policy"
    );
    Ok(())
}

#[cfg(target_os = "linux")]
mod linux;

pub struct IsolationLease {
    #[cfg(target_os = "linux")]
    sandbox: Option<linux::Sandbox>,
}

/// The supervisor attaches inherited descriptors before attaching this lease.
/// The pre-exec child joins its cgroup and Landlock scope; Bubblewrap then builds
/// the namespaces and installs seccomp before starting the worker.
pub(crate) fn command(
    config: &crate::config::PlacementConfig,
    state_dir: &Path,
    data_root: &Path,
    program: &Path,
    slot: u8,
) -> Result<(tokio::process::Command, IsolationLease)> {
    enforce_host_policy(config, state_dir)?;
    let Some(resources) = &config.resources else {
        return Ok((
            tokio::process::Command::new(program),
            IsolationLease {
                #[cfg(target_os = "linux")]
                sandbox: None,
            },
        ));
    };
    resources.validate()?;
    if resources.profile == IsolationProfile::TrustedProcess {
        return Ok((
            tokio::process::Command::new(program),
            IsolationLease {
                #[cfg(target_os = "linux")]
                sandbox: None,
            },
        ));
    }
    #[cfg(target_os = "linux")]
    {
        let (command, sandbox) = linux::command(config, state_dir, data_root, program, slot)?;
        Ok((
            command,
            IsolationLease {
                sandbox: Some(sandbox),
            },
        ))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (state_dir, data_root, program, slot);
        anyhow::bail!(
            "Requested linux_sandbox is unavailable on this platform; no workload was started"
        )
    }
}

impl IsolationLease {
    pub(crate) fn attach(&self, command: &mut tokio::process::Command) -> Result<()> {
        #[cfg(target_os = "linux")]
        if let Some(sandbox) = &self.sandbox {
            sandbox.attach(command)?;
        }
        #[cfg(not(target_os = "linux"))]
        let _ = command;
        Ok(())
    }
    pub(crate) fn signal(&self, force: bool) -> Result<bool> {
        #[cfg(target_os = "linux")]
        if let Some(sandbox) = &self.sandbox {
            sandbox.signal(force)?;
            return Ok(true);
        }
        let _ = force;
        Ok(false)
    }
}

pub fn sandboxed(config: &crate::config::PlacementConfig) -> bool {
    config
        .resources
        .as_ref()
        .is_some_and(|resources| resources.profile == IsolationProfile::LinuxSandbox)
}

pub(crate) struct ResourceSample {
    pub generation: u64,
    pub cpu_microseconds: u64,
    pub memory_bytes: u64,
    pub processes_and_threads: u64,
    pub io_bytes: Option<(u64, u64)>,
}

#[derive(serde::Serialize)]
pub(crate) struct DiskSample {
    pub used_bytes: u64,
    pub limit_bytes: u64,
    pub used_inodes: u64,
    pub limit_inodes: u64,
}

pub(crate) fn disk_sample(state_dir: &Path, placement: &str) -> Result<DiskSample> {
    flow_like_device_protocol::validate_management_id(placement)?;
    #[cfg(target_os = "linux")]
    {
        linux::disk_sample(
            &state_dir
                .join("placement-data")
                .join(placement)
                .join("current/store"),
        )
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = state_dir;
        anyhow::bail!("Project disk accounting is unavailable on this platform")
    }
}

/// Read the kernel accounting for the monitor's exact placement cgroup. This
/// includes descendants; sampling Bubblewrap's own PID would omit the worker.
pub(crate) fn sample(
    pid: u32,
    placement: &str,
    slot: u8,
    state_dir: &Path,
) -> Result<ResourceSample> {
    #[cfg(target_os = "linux")]
    {
        linux::sample(pid, placement, slot, state_dir)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (pid, placement, slot, state_dir);
        anyhow::bail!("Linux isolation accounting is unavailable")
    }
}

/// The worker's inherited broker authenticates its host supervisor. In a PID
/// namespace that supervisor is invisible; bubblewrap's parent-death handling
/// and the cgroup own lifecycle enforcement instead of polling a host PID.
pub fn verify_child(config: &crate::config::PlacementConfig) -> Result<()> {
    if !sandboxed(config) {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        ensure!(
            unsafe { libc::getppid() } == 1
                && unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) } == 1
                && unsafe { libc::prctl(libc::PR_GET_SECCOMP) } == 2,
            "Isolated worker is missing its PID namespace, privilege restriction, or seccomp filter"
        );
        Ok(())
    }
    #[cfg(not(target_os = "linux"))]
    anyhow::bail!("Isolated worker is unsupported on this platform")
}

/// Check host support and this placement's quota without starting a process.
pub fn preflight(
    config: &crate::config::PlacementConfig,
    state_dir: &Path,
    data_root: &Path,
) -> Result<()> {
    enforce_host_policy(config, state_dir)?;
    if let Some(resources) = &config.resources {
        resources.validate()?;
        if resources.profile == IsolationProfile::LinuxSandbox {
            #[cfg(target_os = "linux")]
            {
                linux::preflight(resources, state_dir, data_root)?;
            }
            #[cfg(not(target_os = "linux"))]
            {
                let _ = data_root;
                anyhow::bail!("Requested linux_sandbox is unavailable on this platform");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trusted_profile_never_claims_resource_enforcement() {
        let resources = PlacementResources {
            profile: IsolationProfile::TrustedProcess,
            cpu_millis: Some(1000),
            memory_bytes: None,
            max_processes: None,
            disk_bytes: None,
        };
        assert!(resources.validate().is_err());
    }
    #[test]
    fn host_policy_defaults_to_compatibility_and_rejects_unknown_settings() {
        assert!(!parse_host_policy(None).unwrap());
        assert!(parse_host_policy(Some(std::ffi::OsStr::new("required"))).unwrap());
        assert!(parse_host_policy(Some(std::ffi::OsStr::new("false"))).is_err());
    }
    #[test]
    fn artifact_limits_are_private_positive_bounded_and_reject_duplicates() {
        let root = tempfile::tempdir().unwrap();
        let defaults = artifact_budgets(root.path()).unwrap();
        assert_eq!(defaults.device.bytes, 64 * 1024_u64.pow(3));
        let path = root.path().join("agent.env");
        crate::vault::write_new_private(&path, b"FLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=7\n")
            .unwrap();
        assert_eq!(artifact_budgets(root.path()).unwrap().project.revisions, 7);
        for invalid in [
            "FLOW_LIKE_DEVICE_ARTIFACT_BYTES=0\n",
            "FLOW_LIKE_DEVICE_ARTIFACT_FILES=1000001\n",
            "FLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=1\nFLOW_LIKE_PROJECT_ARTIFACT_REVISIONS=2\n",
            "FLOW_LIKE_DEVICE_ARTIFACT_BYETS=1024\n",
        ] {
            std::fs::write(&path, invalid).unwrap();
            assert!(artifact_budgets(root.path()).is_err(), "{invalid}");
        }
    }
    #[test]
    fn sandbox_requires_every_bound_and_rejects_overflowing_admission_values() {
        let mut resources = PlacementResources {
            profile: IsolationProfile::LinuxSandbox,
            cpu_millis: Some(1000),
            memory_bytes: Some(1024 * 1024 * 1024),
            max_processes: Some(128),
            disk_bytes: Some(1024 * 1024 * 1024),
        };
        resources.validate().unwrap();
        resources.disk_bytes = None;
        assert!(resources.validate().is_err());
        resources.disk_bytes = Some(u64::MAX);
        assert!(resources.validate().is_err());
    }
}
