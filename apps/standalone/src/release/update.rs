use super::{
    ReleaseTrust, VerifiedRelease, download_artifact, verify_artifact_file, verify_running_artifact,
};
use crate::{enrollment::unix_time, service, supervisor, vault};
use anyhow::{Context, Result, ensure};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    ReleaseTarget, StandaloneArtifact, StandaloneRelease, verify_standalone_release,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CandidateInfo {
    version: String,
    target: ReleaseTarget,
    state_schema_version: u32,
    runtime: bool,
}
async fn probe_candidate(path: &Path, release: &StandaloneRelease) -> Result<()> {
    let mut child = tokio::process::Command::new(path)
        .arg("release-info")
        .current_dir(path.parent().context("Candidate has no directory")?)
        .kill_on_drop(true)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Run verified update metadata probe")?;
    let mut stdout = child
        .stdout
        .take()
        .context("Update metadata probe has no output")?
        .take(4097);
    let result=tokio::time::timeout(Duration::from_secs(15),async {
        let mut bytes=Vec::new();stdout.read_to_end(&mut bytes).await?;
        ensure!(bytes.len()<=4096,"Update metadata probe exceeded its output limit");
        ensure!(child.wait().await?.success(),"Update metadata probe failed");
        let info:CandidateInfo=serde_json::from_slice(&bytes)?;
        ensure!(info.version==release.release_version && info.target==ReleaseTarget::current()? && info.state_schema_version==release.state_schema_version && info.runtime,"Candidate metadata differs from its signed target, version, schema, or runtime capability");
        Ok::<_,anyhow::Error>(())
    }).await;
    result?
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateTicket {
    pub operation_id: String,
    pub release_version: String,
    pub sequence: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct UpdateOutcome {
    pub operation_id: String,
    pub state: String,
}
pub fn outcome(state_dir: &Path) -> Result<Option<UpdateOutcome>> {
    let Some(operation) = active(state_dir)? else {
        return Ok(None);
    };
    Ok(Some(operation_outcome(state_dir, &operation)?))
}
pub fn operation_outcome(state_dir: &Path, operation_id: &str) -> Result<UpdateOutcome> {
    let journal = load(state_dir, operation_id)?;
    let state = match journal.phase {
        Phase::Staged => "staged",
        Phase::Armed => "armed",
        Phase::Swapped => "swapped",
        Phase::Completed => "completed",
        Phase::RolledBack => "rolled_back",
        Phase::Failed => "failed",
    };
    Ok(UpdateOutcome {
        operation_id: journal.ticket.operation_id,
        state: state.into(),
    })
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum Phase {
    Staged,
    Armed,
    Swapped,
    Completed,
    RolledBack,
    Failed,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    ticket: UpdateTicket,
    phase: Phase,
    created_at: i64,
    executable: PathBuf,
    candidate: PathBuf,
    backup: PathBuf,
    old_release_jws: String,
    new_release_jws: String,
    trust: ReleaseTrust,
    device_id: String,
    boot_id: String,
    previous_run_id: String,
    nonce: String,
    previous_file: Option<FileIdentity>,
    candidate_file: Option<FileIdentity>,
}
#[derive(Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    device: u64,
    inode: u64,
}
fn file_identity(path: &Path) -> Result<Option<FileIdentity>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Update target was replaced with a non-regular file"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(Some(FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        }))
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        anyhow::bail!("Atomic updates require Unix file identity")
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Readiness {
    operation_id: String,
    nonce: String,
    sha256: String,
    device_id: String,
    boot_id: String,
    run_id: String,
}

fn operation_directory(state_dir: &Path, operation_id: &str) -> Result<PathBuf> {
    let id = uuid::Uuid::parse_str(operation_id)?;
    ensure!(
        id.to_string() == operation_id,
        "Update operation ID must be a canonical UUID"
    );
    Ok(state_dir.join("updates").join(operation_id))
}
fn journal_path(state_dir: &Path, operation_id: &str) -> Result<PathBuf> {
    Ok(operation_directory(state_dir, operation_id)?.join("journal.json"))
}
fn atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().context("Update file has no parent")?;
    if path.try_exists()? {
        vault::read_private(path)?;
    }
    let temporary = parent.join(format!(".update-{}.tmp", uuid::Uuid::new_v4()));
    vault::write_new_private(&temporary, bytes)?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(temporary);
        return Err(error.into());
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}
fn save(state_dir: &Path, journal: &Journal) -> Result<()> {
    atomic(
        &journal_path(state_dir, &journal.ticket.operation_id)?,
        &serde_json::to_vec(journal)?,
    )
}
fn load(state_dir: &Path, operation_id: &str) -> Result<Journal> {
    let journal: Journal = serde_json::from_slice(&vault::read_private(&journal_path(
        state_dir,
        operation_id,
    )?)?)?;
    ensure!(
        journal.version == 1 && journal.ticket.operation_id == operation_id,
        "Update journal binding mismatch"
    );
    let directory = operation_directory(state_dir, operation_id)?;
    ensure!(
        journal.backup == directory.join("previous-binary")
            && journal.candidate.parent() == journal.executable.parent()
            && journal.candidate.file_name().and_then(|v| v.to_str())
                == Some(format!(".flow-like-update-{operation_id}").as_str()),
        "Update journal path mismatch"
    );
    Ok(journal)
}
fn active(state_dir: &Path) -> Result<Option<String>> {
    let path = state_dir.join("updates/active.json");
    if !path.try_exists()? {
        return Ok(None);
    }
    Ok(Some(serde_json::from_slice::<String>(
        &vault::read_private(&path)?,
    )?))
}
fn historical(compact: &str, trust: &ReleaseTrust) -> Result<StandaloneRelease> {
    ensure!(
        compact.len() <= flow_like_device_protocol::MAX_COMPACT_JWS_BYTES,
        "Stored release manifest exceeds its limit"
    );
    let payload = compact
        .split('.')
        .nth(1)
        .context("Stored release manifest has no payload")?;
    let value: StandaloneRelease = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload)?)?;
    // Expiry prevents new downloads, while a previously installed release remains a rollback candidate.
    Ok(verify_standalone_release(
        compact,
        &trust.keys()?,
        0,
        value.issued_at,
    )?)
}
fn artifact(release: &StandaloneRelease) -> Result<&StandaloneArtifact> {
    let target = ReleaseTarget::current()?;
    release
        .artifacts
        .iter()
        .find(|value| value.target == target)
        .context("Update does not include this target")
}
fn current_schema(state_dir: &Path) -> Result<u32> {
    let store = crate::state::StateStore::open(&state_dir.join("management.sqlite"))?;
    Ok(store
        .connection
        .pragma_query_value::<u32, _>(None, "user_version", |row| row.get(0))?)
}
fn private_executable(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute() && path.canonicalize()? == path,
        "Executable path must be absolute and contain no symlinks"
    );
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "Update target must be a regular executable"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let uid = unsafe { libc::geteuid() };
        let parent = std::fs::metadata(path.parent().context("Executable has no parent")?)?;
        ensure!(
            (metadata.uid() == uid || metadata.uid() == 0)
                && metadata.mode() & 0o022 == 0
                && metadata.mode() & 0o111 != 0
                && parent.is_dir()
                && (parent.uid() == uid || parent.uid() == 0)
                && parent.mode() & 0o022 == 0,
            "Executable or its directory is not owned and protected for an update"
        );
    }
    Ok(())
}
fn copy_executable(source: &Path, destination: &Path, expected: &StandaloneArtifact) -> Result<()> {
    verify_artifact_file(source, expected)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o700).custom_flags(libc::O_NOFOLLOW);
    }
    let mut output = options.open(destination).context(
        "Cannot write beside the installed executable; an operator must update this installation",
    )?;
    let mut input = File::open(source)?;
    std::io::copy(&mut input, &mut output)?;
    output.sync_all()?;
    verify_artifact_file(destination, expected)?;
    File::open(
        destination
            .parent()
            .context("Update destination has no parent")?,
    )?
    .sync_all()?;
    Ok(())
}

/// Download and verify while workloads continue. Activation is a separate drained-host step.
pub async fn stage(
    state_dir: &Path,
    trust_file: &Path,
    release_jws: &str,
    operation_id: &str,
    device_id: &str,
    boot_id: &str,
    run_id: &str,
) -> Result<UpdateTicket> {
    ensure!(
        cfg!(target_os = "linux"),
        "Automatic binary updates currently require Linux systemd"
    );
    let state_dir = supervisor::prepare_state_dir(state_dir)?;
    let _lock = supervisor::lock_file(&state_dir.join("release.lock"))?;
    ensure!(
        trust_file.canonicalize()? == state_dir.join("release-trust.json").canonicalize()?,
        "Updates use only this device's installed release authority"
    );
    if let Some(existing) = active(&state_dir)? {
        let journal = load(&state_dir, &existing)?;
        if existing == operation_id
            && journal.new_release_jws == release_jws
            && journal.phase == Phase::Staged
        {
            return Ok(journal.ticket);
        }
        ensure!(
            matches!(
                journal.phase,
                Phase::Completed | Phase::RolledBack | Phase::Failed
            ),
            "An update is already staged or awaiting confirmation"
        );
    }
    let trust = ReleaseTrust::load(trust_file)?;
    let release = VerifiedRelease::verify(release_jws.to_owned(), &trust)?;
    let old_compact =
        String::from_utf8(vault::read_private(&state_dir.join("active-release.jws"))?.to_vec())?;
    let old = historical(&old_compact, &trust)?;
    ensure!(
        release.manifest().sequence > old.sequence,
        "Updates must advance the installed release sequence"
    );
    let schema = current_schema(&state_dir)?;
    ensure!(
        release.manifest().state_schema_version == schema && old.state_schema_version == schema,
        "Automatic rollback supports only releases with the current management database schema"
    );
    let executable = std::env::current_exe()?;
    private_executable(&executable)?;
    service::verify_user_service(&executable, &state_dir).await?;
    verify_artifact_file(&executable, artifact(&old)?)?;
    verify_running_artifact(artifact(&old)?)?;
    let directory = supervisor::prepare_state_dir(&operation_directory(&state_dir, operation_id)?)?;
    let candidate = executable
        .parent()
        .context("Executable has no parent")?
        .join(format!(".flow-like-update-{operation_id}"));
    let backup = directory.join("previous-binary");
    ensure!(
        !candidate.try_exists()? && !backup.try_exists()?,
        "An incomplete update with this ID needs operator review before retrying"
    );
    let downloaded = download_artifact(&release, ReleaseTarget::current()?, &directory).await?;
    probe_candidate(downloaded.path(), release.manifest()).await?;
    let mut journal = Journal {
        version: 1,
        ticket: UpdateTicket {
            operation_id: operation_id.into(),
            release_version: release.manifest().release_version.clone(),
            sequence: release.manifest().sequence,
            sha256: artifact(release.manifest())?.sha256.clone(),
        },
        phase: Phase::Failed,
        created_at: unix_time()?,
        executable,
        candidate,
        backup,
        old_release_jws: old_compact,
        new_release_jws: release_jws.into(),
        trust,
        device_id: device_id.into(),
        boot_id: boot_id.into(),
        previous_run_id: run_id.into(),
        nonce: uuid::Uuid::new_v4().to_string(),
        previous_file: file_identity(&std::env::current_exe()?)?,
        candidate_file: None,
    };
    let copied = (|| -> Result<()> {
        copy_executable(&journal.executable, &journal.backup, artifact(&old)?)?;
        copy_executable(
            downloaded.path(),
            &journal.candidate,
            artifact(release.manifest())?,
        )?;
        Ok(())
    })();
    if let Err(error) = copied {
        let _ = std::fs::remove_file(&journal.candidate);
        let _ = std::fs::remove_file(&journal.backup);
        return Err(error);
    }
    journal.phase = Phase::Staged;
    journal.candidate_file = file_identity(&journal.candidate)?;
    save(&state_dir, &journal)?;
    atomic(
        &state_dir.join("updates/active.json"),
        &serde_json::to_vec(operation_id)?,
    )?;
    Ok(journal.ticket)
}

async fn command(program: &str, args: &[std::ffi::OsString]) -> Result<()> {
    let status = tokio::time::timeout(
        Duration::from_secs(15),
        tokio::process::Command::new(program)
            .args(args)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status(),
    )
    .await??;
    ensure!(
        status.success(),
        "Managed update service command was rejected; inspect the user systemd manager"
    );
    Ok(())
}
async fn restart() -> Result<()> {
    command(
        "systemctl",
        &[
            "--user".into(),
            "--no-ask-password".into(),
            "restart".into(),
            service::SYSTEMD_UNIT_NAME.into(),
        ],
    )
    .await
}

/// Arm an independent watchdog before atomically replacing a drained agent's executable.
pub async fn activate(state_dir: &Path, operation_id: &str) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "Automatic binary updates currently require Linux systemd"
    );
    let state_dir = state_dir.canonicalize()?;
    let _lock = supervisor::lock_file(&state_dir.join("release.lock"))?;
    let mut journal = load(&state_dir, operation_id)?;
    ensure!(
        journal.phase == Phase::Staged && unix_time()?.saturating_sub(journal.created_at) <= 600,
        "Update is not staged or its activation window expired"
    );
    service::verify_user_service(&journal.executable, &state_dir).await?;
    private_executable(&journal.executable)?;
    let release = VerifiedRelease::verify(journal.new_release_jws.clone(), &journal.trust)?;
    verify_artifact_file(&journal.candidate, artifact(release.manifest())?)?;
    verify_artifact_file(
        &journal.backup,
        artifact(&historical(&journal.old_release_jws, &journal.trust)?)?,
    )?;
    journal.phase = Phase::Armed;
    save(&state_dir, &journal)?;
    let args = vec![
        "--user".into(),
        "--collect".into(),
        "--quiet".into(),
        "--expand-environment=no".into(),
        format!("--unit=flow-like-standalone-update-{operation_id}").into(),
        "--property=Type=exec".into(),
        "--property=RuntimeMaxSec=120s".into(),
        "--property=KillMode=control-group".into(),
        "--property=UMask=0077".into(),
        journal.backup.as_os_str().to_owned(),
        "--state-dir".into(),
        state_dir.as_os_str().to_owned(),
        "update-guard".into(),
        "--operation-id".into(),
        operation_id.into(),
    ];
    if let Err(error) = command("systemd-run", &args).await {
        journal.phase = Phase::Failed;
        save(&state_dir, &journal)?;
        return Err(error.context("Starting the update watchdog requires systemd 254 or newer"));
    }
    let ready_path = operation_directory(&state_dir, operation_id)?.join("watchdog-ready.json");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        if ready_path.try_exists()? {
            let nonce: String = serde_json::from_slice(&vault::read_private(&ready_path)?)?;
            ensure!(nonce == journal.nonce, "Update watchdog readiness mismatch");
            break;
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Update watchdog did not confirm readiness; current binary remains installed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    // Persist the possible-swap state first. Recovery inspects file hashes after a power loss.
    release.check_fresh()?;
    private_executable(&journal.executable)?;
    ensure!(
        file_identity(&journal.executable)? == journal.previous_file,
        "Installed binary changed after the update was staged"
    );
    verify_artifact_file(
        &journal.executable,
        artifact(&historical(&journal.old_release_jws, &journal.trust)?)?,
    )?;
    verify_artifact_file(&journal.candidate, artifact(release.manifest())?)?;
    journal.phase = Phase::Swapped;
    save(&state_dir, &journal)?;
    std::fs::rename(&journal.candidate, &journal.executable)?;
    File::open(
        journal
            .executable
            .parent()
            .context("Executable has no parent")?,
    )?
    .sync_all()?;
    restart().await
}

fn readiness_matches(journal: &Journal, ready: &Readiness) -> bool {
    ready.operation_id == journal.ticket.operation_id
        && ready.nonce == journal.nonce
        && ready.sha256 == journal.ticket.sha256
        && ready.device_id == journal.device_id
        && ready.boot_id == journal.boot_id
        && !ready.run_id.is_empty()
        && ready.run_id != journal.previous_run_id
}

/// Called by the newly started agent after management DB and supervisor initialization.
pub fn confirm_ready(state_dir: &Path, device_id: &str, boot_id: &str, run_id: &str) -> Result<()> {
    let Some(operation) = active(state_dir)? else {
        return Ok(());
    };
    let journal = load(state_dir, &operation)?;
    if journal.phase != Phase::Swapped {
        return Ok(());
    }
    let ready = Readiness {
        operation_id: operation,
        nonce: journal.nonce.clone(),
        sha256: journal.ticket.sha256.clone(),
        device_id: device_id.into(),
        boot_id: boot_id.into(),
        run_id: run_id.into(),
    };
    ensure!(
        readiness_matches(&journal, &ready),
        "Update readiness does not match this boot, device, and new process run"
    );
    let release = historical(&journal.new_release_jws, &journal.trust)?;
    ensure!(
        release.state_schema_version == current_schema(state_dir)?,
        "Update changed the management database schema"
    );
    ensure!(
        std::env::current_exe()? == journal.executable,
        "Update readiness came from another executable"
    );
    verify_artifact_file(&journal.executable, artifact(&release)?)?;
    verify_running_artifact(artifact(&release)?)?;
    atomic(
        &operation_directory(state_dir, &ready.operation_id)?.join("agent-ready.json"),
        &serde_json::to_vec(&ready)?,
    )
}

/// Runs from the retained old executable in a separate transient user service.
pub async fn guard(state_dir: &Path, operation_id: &str) -> Result<()> {
    ensure!(
        cfg!(target_os = "linux"),
        "Update watchdog requires Linux systemd"
    );
    let mut journal = load(state_dir, operation_id)?;
    ensure!(
        journal.phase == Phase::Armed && unix_time()?.saturating_sub(journal.created_at) <= 600,
        "Update watchdog was not freshly armed"
    );
    ensure!(
        std::env::current_exe()? == journal.backup,
        "Update watchdog must run from the retained verified binary"
    );
    let old = historical(&journal.old_release_jws, &journal.trust)?;
    verify_artifact_file(&journal.backup, artifact(&old)?)?;
    verify_running_artifact(artifact(&old)?)?;
    vault::write_new_private(
        &operation_directory(state_dir, operation_id)?.join("watchdog-ready.json"),
        &serde_json::to_vec(&journal.nonce)?,
    )?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        journal = load(state_dir, operation_id)?;
        if matches!(
            journal.phase,
            Phase::Completed | Phase::RolledBack | Phase::Failed
        ) {
            return Ok(());
        }
        let ready_path = operation_directory(state_dir, operation_id)?.join("agent-ready.json");
        if journal.phase == Phase::Swapped && ready_path.try_exists()? {
            let ready = vault::read_private(&ready_path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<Readiness>(&bytes).ok());
            if !ready
                .as_ref()
                .is_some_and(|ready| readiness_matches(&journal, ready))
            {
                if tokio::time::Instant::now() >= deadline {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
            let release = historical(&journal.new_release_jws, &journal.trust)?;
            verify_artifact_file(&journal.executable, artifact(&release)?)?;
            atomic(
                &state_dir.join("active-release.jws"),
                journal.new_release_jws.as_bytes(),
            )?;
            journal.trust.minimum_sequence =
                journal.trust.minimum_sequence.max(journal.ticket.sequence);
            atomic(
                &state_dir.join("release-trust.json"),
                &serde_json::to_vec_pretty(&journal.trust)?,
            )?;
            journal.phase = Phase::Completed;
            save(state_dir, &journal)?;
            // The old image remains available until a later operator cleanup, including this running guard.
            return Ok(());
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    rollback(state_dir, &mut journal)?;
    restart().await
}

fn rollback(state_dir: &Path, journal: &mut Journal) -> Result<()> {
    let old = historical(&journal.old_release_jws, &journal.trust)?;
    verify_artifact_file(&journal.backup, artifact(&old)?)?;
    if journal.phase == Phase::Swapped {
        let present = file_identity(&journal.executable)?;
        ensure!(
            present.is_none()
                || present == journal.previous_file
                || present == journal.candidate_file,
            "Installed binary was replaced outside this update; rollback needs operator review"
        );
        let temporary = journal
            .executable
            .parent()
            .context("Executable has no parent")?
            .join(format!(
                ".flow-like-rollback-{}",
                journal.ticket.operation_id
            ));
        copy_executable(&journal.backup, &temporary, artifact(&old)?)?;
        std::fs::rename(temporary, &journal.executable)?;
        File::open(
            journal
                .executable
                .parent()
                .context("Executable has no parent")?,
        )?
        .sync_all()?;
    }
    journal.phase = Phase::RolledBack;
    save(state_dir, journal)
}

/// A transient watchdog does not survive an OS reboot. Recover its journal before opening the DB.
pub fn recover_after_boot(state_dir: &Path, boot_id: &str) -> Result<bool> {
    let Some(operation) = active(state_dir)? else {
        return Ok(false);
    };
    let mut journal = load(state_dir, &operation)?;
    if journal.boot_id == boot_id {
        return Ok(false);
    }
    if journal.phase == Phase::Staged {
        journal.phase = Phase::Failed;
        save(state_dir, &journal)?;
        return Ok(false);
    }
    if !matches!(journal.phase, Phase::Armed | Phase::Swapped) {
        return Ok(false);
    }
    rollback(state_dir, &mut journal)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{SigningKey, sign_standalone_release};
    use sha2::{Digest, Sha256};

    #[cfg(unix)]
    fn swapped_fixture() -> Result<(tempfile::TempDir, Journal)> {
        let directory = tempfile::tempdir()?;
        let operation_id = uuid::Uuid::new_v4().to_string();
        let updates = operation_directory(directory.path(), &operation_id)?;
        std::fs::create_dir_all(&updates)?;
        let executable = directory.path().join("agent");
        let candidate = directory
            .path()
            .join(format!(".flow-like-update-{operation_id}"));
        let backup = updates.join("previous-binary");
        std::fs::write(&executable, b"old-image")?;
        std::fs::write(&candidate, b"new-image")?;
        std::fs::write(&backup, b"old-image")?;
        let key = SigningKey::generate();
        let now = unix_time()?;
        let release = |sequence, bytes: &[u8]| StandaloneRelease {
            version: 1,
            state_schema_version: crate::state::SCHEMA_VERSION.try_into().unwrap(),
            sequence,
            release_version: format!("{sequence}.0.0"),
            issued_at: now,
            expires_at: now + 300,
            artifacts: vec![StandaloneArtifact {
                target: ReleaseTarget::current().unwrap(),
                url: "https://releases.example/agent".into(),
                size: bytes.len() as u64,
                sha256: format!("{:x}", Sha256::digest(bytes)),
            }],
            container: None,
        };
        let old = release(1, b"old-image");
        let new = release(2, b"new-image");
        let journal = Journal {
            version: 1,
            ticket: UpdateTicket {
                operation_id: operation_id.clone(),
                release_version: new.release_version.clone(),
                sequence: 2,
                sha256: artifact(&new)?.sha256.clone(),
            },
            phase: Phase::Swapped,
            created_at: now,
            previous_file: file_identity(&executable)?,
            candidate_file: file_identity(&candidate)?,
            executable: executable.clone(),
            candidate: candidate.clone(),
            backup,
            old_release_jws: sign_standalone_release(&old, &key)?,
            new_release_jws: sign_standalone_release(&new, &key)?,
            trust: ReleaseTrust {
                manifest_url: "https://releases.example/release.jws".into(),
                public_keys: vec![URL_SAFE_NO_PAD.encode(key.public_key().to_bytes()?)],
                minimum_sequence: 1,
            },
            device_id: "device".into(),
            boot_id: "old-boot".into(),
            previous_run_id: "old-run".into(),
            nonce: uuid::Uuid::new_v4().to_string(),
        };
        std::fs::rename(&candidate, &executable)?;
        save(directory.path(), &journal)?;
        atomic(
            &directory.path().join("updates/active.json"),
            &serde_json::to_vec(&operation_id)?,
        )?;
        Ok((directory, journal))
    }
    #[test]
    fn readiness_requires_exact_new_binary_operation_device_boot_and_run() {
        let journal = Journal {
            version: 1,
            ticket: UpdateTicket {
                operation_id: uuid::Uuid::new_v4().to_string(),
                release_version: "1.2.3".into(),
                sequence: 2,
                sha256: "a".repeat(64),
            },
            phase: Phase::Swapped,
            created_at: 0,
            executable: "/agent".into(),
            candidate: "/candidate".into(),
            backup: "/backup".into(),
            old_release_jws: String::new(),
            new_release_jws: String::new(),
            trust: ReleaseTrust {
                manifest_url: String::new(),
                public_keys: vec![],
                minimum_sequence: 0,
            },
            device_id: "device".into(),
            boot_id: "boot".into(),
            previous_run_id: "old-run".into(),
            nonce: "nonce".into(),
            previous_file: None,
            candidate_file: None,
        };
        let mut ready = Readiness {
            operation_id: journal.ticket.operation_id.clone(),
            nonce: "nonce".into(),
            sha256: "a".repeat(64),
            device_id: "device".into(),
            boot_id: "boot".into(),
            run_id: "new-run".into(),
        };
        assert!(readiness_matches(&journal, &ready));
        ready.run_id = "old-run".into();
        assert!(!readiness_matches(&journal, &ready));
        ready.run_id = "new-run".into();
        ready.sha256 = "b".repeat(64);
        assert!(!readiness_matches(&journal, &ready));
        ready.sha256 = "a".repeat(64);
        ready.nonce = "stale".into();
        assert!(!readiness_matches(&journal, &ready));
    }
    #[test]
    fn journal_paths_and_atomic_files_never_follow_links() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join("state");
        std::fs::write(&target, b"untouched")?;
        assert!(operation_directory(directory.path(), "../../agent").is_err());
        #[cfg(unix)]
        {
            let link = directory.path().join("link");
            std::os::unix::fs::symlink(&target, &link)?;
            assert!(atomic(&link, b"changed").is_err());
            assert_eq!(std::fs::read(&target)?, b"untouched");
        }
        let output = directory.path().join("journal");
        atomic(&output, b"one")?;
        atomic(&output, b"two")?;
        assert_eq!(&**vault::read_private(&output)?, b"two");
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn rollback_restores_verified_old_binary_and_refuses_an_unrelated_replacement() -> Result<()> {
        let (directory, mut journal) = swapped_fixture()?;
        rollback(directory.path(), &mut journal)?;
        assert_eq!(std::fs::read(&journal.executable)?, b"old-image");
        assert_eq!(std::fs::read(&journal.backup)?, b"old-image");
        assert!(load(directory.path(), &journal.ticket.operation_id)?.phase == Phase::RolledBack);
        let (directory, mut journal) = swapped_fixture()?;
        let replacement = directory.path().join("operator-binary");
        std::fs::write(&replacement, b"operator change")?;
        std::fs::rename(replacement, &journal.executable)?;
        assert!(rollback(directory.path(), &mut journal).is_err());
        assert_eq!(std::fs::read(&journal.executable)?, b"operator change");
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn reboot_recovery_restores_before_opening_or_migrating_the_database() -> Result<()> {
        let (directory, journal) = swapped_fixture()?;
        assert!(!recover_after_boot(directory.path(), "old-boot")?);
        assert!(recover_after_boot(directory.path(), "new-boot")?);
        assert_eq!(std::fs::read(&journal.executable)?, b"old-image");
        assert!(!directory.path().join("management.sqlite").exists());
        assert!(!recover_after_boot(directory.path(), "new-boot")?);
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn a_staged_update_cannot_activate_after_a_different_boot() -> Result<()> {
        let (directory, mut journal) = swapped_fixture()?;
        journal.phase = Phase::Staged;
        save(directory.path(), &journal)?;
        assert!(!recover_after_boot(directory.path(), "old-boot")?);
        assert_eq!(
            operation_outcome(directory.path(), &journal.ticket.operation_id)?.state,
            "staged"
        );
        assert!(!recover_after_boot(directory.path(), "new-boot")?);
        assert_eq!(
            operation_outcome(directory.path(), &journal.ticket.operation_id)?.state,
            "failed"
        );
        assert!(!directory.path().join("management.sqlite").exists());
        Ok(())
    }
}
