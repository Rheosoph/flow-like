use crate::{
    config::{PlacementConfig, ProjectSource},
    supervisor, vault,
};
use anyhow::{Context, Result, ensure};
use flow_like_device_protocol::{
    PROJECT_ARTIFACT_MAX_BYTES, PROJECT_ARTIFACT_MAX_FILE_BYTES, PROJECT_ARTIFACT_MAX_FILES,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tokio_util::sync::CancellationToken;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Binding {
    version: u32,
    placement_id: String,
    project_id: String,
    deployment_id: String,
    source: ProjectSource,
    initial_snapshot: PathBuf,
}

fn directory(path: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        match std::fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {
                if let Some(parent) = path.parent() {
                    File::open(parent)?.sync_all()?;
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        existing_directory(path)
    }
    #[cfg(not(unix))]
    anyhow::bail!("Placement data requires Unix file permissions");
}

fn existing_directory(path: &Path) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::symlink_metadata(path)?;
        ensure!(
            metadata.is_dir()
                && !metadata.file_type().is_symlink()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Placement data directory must be private and owned"
        );
        Ok(path.to_path_buf())
    }
    #[cfg(not(unix))]
    anyhow::bail!("Placement data requires Unix file permissions");
}

fn validate_binding(path: &Path, config: &PlacementConfig) -> Result<PathBuf> {
    existing_directory(path)?;
    let binding: Binding =
        serde_json::from_slice(&vault::read_private(&path.join("binding.json"))?)?;
    ensure!(
        binding.version == 1
            && binding.placement_id == config.id
            && binding.project_id == config.project_id
            && binding.deployment_id == config.deployment_id
            && binding.source == config.source,
        "Placement data identity or source type changed"
    );
    existing_directory(&path.join("store"))
}

fn record_initialized(parent: &Path, current: &Path) -> Result<()> {
    let binding = vault::read_private(&current.join("binding.json"))?;
    let receipt = parent.join("initialized.json");
    if receipt.try_exists()? {
        ensure!(
            *vault::read_private(&receipt)? == *binding,
            "Placement initialization receipt differs"
        );
    } else {
        let temporary = parent.join(format!(".initialized-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            vault::write_new_private(&temporary, &binding)?;
            std::fs::rename(&temporary, &receipt)?;
            File::open(parent)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temporary);
        }
        result?;
    }
    Ok(())
}

/// Validate a supervisor-selected data root without accepting a path from a deployment manifest.
pub(crate) fn validate_root(path: &Path, config: &PlacementConfig) -> Result<()> {
    ensure!(
        path.is_absolute() && path.file_name().is_some_and(|name| name == "store"),
        "Invalid placement data root"
    );
    let parent = path
        .parent()
        .context("Placement data root is missing its binding")?;
    ensure!(
        parent.file_name().is_some_and(|name| name == "current")
            && parent
                .parent()
                .and_then(Path::file_name)
                .is_some_and(|name| name == config.id.as_str()),
        "Placement data root identity differs"
    );
    ensure!(
        validate_binding(parent, config)? == path,
        "Placement data binding differs"
    );
    Ok(())
}

#[derive(Default)]
struct Bounds {
    files: usize,
    directories: usize,
    bytes: u64,
}

#[cfg(unix)]
fn stamp(metadata: &std::fs::Metadata) -> (u64, u64, u64, i64, i64, i64, i64) {
    use std::os::unix::fs::MetadataExt;
    (
        metadata.dev(),
        metadata.ino(),
        metadata.len(),
        metadata.mtime(),
        metadata.mtime_nsec(),
        metadata.ctime(),
        metadata.ctime_nsec(),
    )
}

#[cfg(unix)]
fn copy_tree(
    source: &Path,
    destination: &Path,
    relative: &str,
    project: &str,
    bounds: &mut Bounds,
    cancel: &CancellationToken,
) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    ensure!(
        !cancel.is_cancelled(),
        "Placement data initialization cancelled"
    );
    let before = std::fs::symlink_metadata(source)?;
    ensure!(
        before.is_dir() && !before.file_type().is_symlink(),
        "Initial data must not contain symlinks"
    );
    bounds.directories += 1;
    ensure!(
        bounds.directories <= PROJECT_ARTIFACT_MAX_FILES,
        "Initial data has too many directories"
    );
    directory(destination)?;
    let mut names = std::collections::HashSet::new();
    for entry in std::fs::read_dir(source)? {
        ensure!(
            !cancel.is_cancelled(),
            "Placement data initialization cancelled"
        );
        let entry = entry?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .context("Initial data file name must be UTF-8")?;
        let relative = flow_like_device_protocol::normalize_artifact_path(
            project,
            &format!("{relative}/{name}"),
        )?;
        let normalized_name = relative
            .rsplit('/')
            .next()
            .context("Initial data file name missing")?;
        ensure!(
            names.insert(normalized_name.to_lowercase()),
            "Initial data has colliding file names"
        );
        let source = entry.path();
        let destination = destination.join(normalized_name);
        let metadata = std::fs::symlink_metadata(&source)?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "Initial data must not contain symlinks"
        );
        if metadata.is_dir() {
            copy_tree(&source, &destination, &relative, project, bounds, cancel)?;
            continue;
        }
        ensure!(
            metadata.is_file()
                && metadata.nlink() == 1
                && metadata.len() <= PROJECT_ARTIFACT_MAX_FILE_BYTES,
            "Initial data must contain bounded regular files without hardlinks"
        );
        bounds.files += 1;
        bounds.bytes = bounds
            .bytes
            .checked_add(metadata.len())
            .context("Initial data size overflow")?;
        ensure!(
            bounds.files <= PROJECT_ARTIFACT_MAX_FILES
                && bounds.bytes <= PROJECT_ARTIFACT_MAX_BYTES,
            "Initial placement data exceeds its file or size limit"
        );
        let mut input = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(&source)?;
        ensure!(
            stamp(&input.metadata()?) == stamp(&metadata),
            "Initial data changed during initialization"
        );
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(destination)?;
        let mut bytes = [0u8; 128 * 1024];
        let mut copied = 0u64;
        loop {
            ensure!(
                !cancel.is_cancelled(),
                "Placement data initialization cancelled"
            );
            let count = input.read(&mut bytes)?;
            if count == 0 {
                break;
            }
            copied += count as u64;
            ensure!(
                copied <= metadata.len(),
                "Initial data changed during initialization"
            );
            output.write_all(&bytes[..count])?;
        }
        ensure!(
            copied == metadata.len()
                && stamp(&input.metadata()?) == stamp(&metadata)
                && stamp(&std::fs::symlink_metadata(&source)?) == stamp(&metadata),
            "Initial data changed during initialization"
        );
        output.sync_all()?;
    }
    ensure!(
        stamp(&std::fs::symlink_metadata(source)?) == stamp(&before),
        "Initial data directory changed during initialization"
    );
    File::open(destination)?.sync_all()?;
    Ok(())
}

/// The source must be a stopped project or a consistent database snapshot. Copying
/// files cannot manufacture a transactionally consistent snapshot of a live database.
pub(crate) fn prepare_for_launch(
    root: &Path,
    config: &PlacementConfig,
    revision: u64,
    intent: u64,
    cancel: &CancellationToken,
) -> Result<PathBuf> {
    prepare(root, config, cancel, || {
        let store = crate::state::StateStore::open(&root.join("management.sqlite"))?;
        store.connection.execute_batch("BEGIN IMMEDIATE")?;
        let record = store
            .get_placement(&config.id)?
            .context("Placement was removed while initializing data")?;
        ensure!(
            record.config_revision == revision
                && record.intent_revision == intent
                && record.desired_state == crate::state::DesiredState::Running,
            "Placement changed while initializing data"
        );
        Ok(store)
    })
}

fn prepare<G>(
    root: &Path,
    config: &PlacementConfig,
    cancel: &CancellationToken,
    before_publish: impl FnOnce() -> Result<G>,
) -> Result<PathBuf> {
    config.validate()?;
    let base = directory(&directory(root)?.join("placement-data"))?;
    crate::config::ensure_unaliased_child(&base, &config.id)?;
    let base = directory(&base.join(&config.id))?;
    // Admission precedes copying a bundle so an unprovisioned sandbox cannot
    // fill the agent filesystem before its worker is rejected.
    crate::isolation::preflight(config, root, &base)?;
    let target = base.join("current");
    let _lock = supervisor::lock_file(&base.join("initialize.lock"))?;
    if target.try_exists()? {
        let _publication_guard = before_publish()?;
        ensure!(
            !cancel.is_cancelled(),
            "Placement data initialization cancelled"
        );
        let data = validate_binding(&target, config)?;
        record_initialized(&base, &target)?;
        return Ok(data);
    }
    ensure!(
        !base.join("initialized.json").try_exists()?,
        "Previously initialized placement data is missing; restore it before starting"
    );
    // A surviving child from an older agent can still own the source database.
    let mut source_locks = Vec::new();
    for slot in 0..32 {
        let path = root.join(format!("placement-{}-slot-{slot}.lock", config.id));
        if path.try_exists()? {
            source_locks.push(supervisor::lock_file(&path)?);
        }
    }
    let staging = base.join("staging");
    if staging.try_exists()? {
        directory(&staging)?;
        std::fs::remove_dir_all(&staging)?;
    }
    directory(&staging)?;
    let result = (|| -> Result<PathBuf> {
        let store = directory(&staging.join("store"))?;
        let apps = directory(&store.join("apps"))?;
        let project = directory(&apps.join(&config.project_id))?;
        if config.source == ProjectSource::Offline {
            let mut source = config.project_path.canonicalize()?;
            let mut available = true;
            for component in ["apps", &config.project_id] {
                crate::config::ensure_unaliased_child(&source, component)?;
                source.push(component);
                match std::fs::symlink_metadata(&source) {
                    Ok(metadata) => ensure!(
                        metadata.is_dir() && !metadata.file_type().is_symlink(),
                        "Initial project data path must not contain symlinks"
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        available = false;
                        break;
                    }
                    Err(error) => return Err(error.into()),
                }
            }
            if available {
                let mut bounds = Bounds::default();
                for namespace in ["storage", "files", "upload", "metadata"] {
                    crate::config::ensure_unaliased_child(&source, namespace)?;
                    let input = source.join(namespace);
                    match std::fs::symlink_metadata(&input) {
                        Ok(_) => {
                            #[cfg(unix)]
                            copy_tree(
                                &input,
                                &project.join(namespace),
                                &format!("apps/{}/{namespace}", config.project_id),
                                &config.project_id,
                                &mut bounds,
                                cancel,
                            )?;
                            #[cfg(not(unix))]
                            anyhow::bail!("Placement data initialization requires Unix");
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                // The exporter includes one selected user's project data. Offline
                // workers use `local`, so do not preserve the source account path.
                let namespace = "deployment-user-data";
                crate::config::ensure_unaliased_child(&source, namespace)?;
                let input = source.join(namespace);
                match std::fs::symlink_metadata(&input) {
                    Ok(_) => {
                        let user = directory(&store.join("user"))?;
                        let users = directory(&user.join("users"))?;
                        let local = directory(&users.join("local"))?;
                        let apps = directory(&local.join("apps"))?;
                        #[cfg(unix)]
                        copy_tree(
                            &input,
                            &apps.join(&config.project_id),
                            &format!("apps/{}/{namespace}", config.project_id),
                            &config.project_id,
                            &mut bounds,
                            cancel,
                        )?;
                        #[cfg(not(unix))]
                        anyhow::bail!("Placement data initialization requires Unix");
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error.into()),
                }
            }
        }
        for namespace in ["user", "logs", "tmp"] {
            directory(&store.join(namespace))?;
        }
        let binding = Binding {
            version: 1,
            placement_id: config.id.clone(),
            project_id: config.project_id.clone(),
            deployment_id: config.deployment_id.clone(),
            source: config.source,
            initial_snapshot: config.project_path.clone(),
        };
        vault::write_new_private(
            &staging.join("binding.json"),
            &serde_json::to_vec(&binding)?,
        )?;
        File::open(&store)?.sync_all()?;
        File::open(&staging)?.sync_all()?;
        ensure!(
            !cancel.is_cancelled(),
            "Placement data initialization cancelled"
        );
        let _publication_guard = before_publish()?;
        ensure!(
            !cancel.is_cancelled(),
            "Placement data initialization cancelled"
        );
        ensure!(!target.try_exists()?, "Placement data already exists");
        std::fs::rename(&staging, &target)?;
        File::open(&base)?.sync_all()?;
        let data = validate_binding(&target, config)?;
        record_initialized(&base, &target)?;
        Ok(data)
    })();
    if result.is_err() && staging.exists() {
        let _ = std::fs::remove_dir_all(&staging);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state_root() -> Result<tempfile::TempDir> {
        let directory = tempfile::tempdir()?;
        supervisor::prepare_state_dir(directory.path())?;
        Ok(directory)
    }
    fn config(source: &Path) -> PlacementConfig {
        serde_json::from_value(serde_json::json!({"id":"placement","project_id":"project","deployment_id":"deployment","revision":"one","source":"offline","project_path":source,"events":[{"event_id":"event","event_version":[1,0,0],"board_version":[1,0,0]}]})).unwrap()
    }
    fn seed(root: &Path, value: &[u8]) {
        let storage = root.join("apps/project/storage/db");
        std::fs::create_dir_all(&storage).unwrap();
        std::fs::write(storage.join("table"), value).unwrap();
        std::fs::write(root.join("apps/project/manifest.app"), b"pinned metadata").unwrap();
        std::fs::create_dir_all(root.join("user")).unwrap();
        std::fs::write(root.join("user/private"), b"never imported").unwrap();
    }
    #[test]
    fn revisions_restarts_and_replicas_preserve_live_data_and_other_placements_are_isolated()
    -> Result<()> {
        let root = state_root()?;
        let first = tempfile::tempdir()?;
        let second = tempfile::tempdir()?;
        seed(first.path(), b"initial");
        seed(second.path(), b"new snapshot");
        let mut config = config(first.path());
        let cancel = CancellationToken::new();
        let data = prepare(root.path(), &config, &cancel, || Ok(()))?;
        let file = data.join("apps/project/storage/db/table");
        assert_eq!(std::fs::read(&file)?, b"initial");
        assert!(!data.join("apps/project/manifest.app").exists());
        assert!(!data.join("user/private").exists());
        std::fs::write(&file, b"live write")?;
        let wrapper = data.parent().unwrap().parent().unwrap();
        std::fs::remove_file(wrapper.join("initialized.json"))?;
        vault::write_new_private(
            &wrapper.join(format!(".initialized-{}.tmp", uuid::Uuid::new_v4())),
            b"partial receipt",
        )?;
        assert_eq!(prepare(root.path(), &config, &cancel, || Ok(()))?, data);
        assert!(wrapper.join("initialized.json").is_file());
        assert_eq!(std::fs::read(&file)?, b"live write");
        config.project_path = second.path().into();
        config.revision = "two".into();
        assert_eq!(prepare(root.path(), &config, &cancel, || Ok(()))?, data);
        assert_eq!(std::fs::read(&file)?, b"live write");
        config.id = "second".into();
        let other = prepare(root.path(), &config, &cancel, || Ok(()))?;
        assert_ne!(other, data);
        assert_eq!(
            std::fs::read(other.join("apps/project/storage/db/table"))?,
            b"new snapshot"
        );
        config.id = "placement".into();
        config.source = ProjectSource::Online;
        config.resource_grant = Some(crate::config::ResourceGrantRef {
            grant_id: "grant".into(),
            authz_version: 1,
            billing_grant_id: None,
            billing_authz_version: None,
        });
        assert!(prepare(root.path(), &config, &cancel, || Ok(())).is_err());
        assert_eq!(std::fs::read(&file)?, b"live write");
        Ok(())
    }
    #[test]
    fn selected_user_snapshot_is_seeded_only_into_the_offline_execution_identity() -> Result<()> {
        let root = state_root()?;
        let source = tempfile::tempdir()?;
        seed(source.path(), b"project");
        let user = source.path().join("apps/project/deployment-user-data");
        std::fs::create_dir_all(user.join("db/table.lance"))?;
        std::fs::write(user.join("file.txt"), b"selected user")?;
        std::fs::write(user.join("db/table.lance/data"), b"selected rows")?;
        let data = prepare(
            root.path(),
            &config(source.path()),
            &CancellationToken::new(),
            || Ok(()),
        )?;
        assert_eq!(
            std::fs::read(data.join("user/users/local/apps/project/file.txt"))?,
            b"selected user"
        );
        assert_eq!(
            std::fs::read(data.join("user/users/local/apps/project/db/table.lance/data"))?,
            b"selected rows"
        );
        assert!(!data.join("user/private").exists());
        assert!(!data.join("apps/project/deployment-user-data").exists());
        Ok(())
    }

    #[test]
    #[cfg(unix)]
    fn failed_or_interrupted_initialization_never_publishes_partial_data() -> Result<()> {
        let root = state_root()?;
        let source = tempfile::tempdir()?;
        seed(source.path(), b"initial");
        let config = config(source.path());
        let cancel = CancellationToken::new();
        let link = source.path().join("apps/project/storage/link");
        std::os::unix::fs::symlink(source.path().join("user"), &link)?;
        assert!(prepare(root.path(), &config, &cancel, || Ok(())).is_err());
        assert!(
            !root
                .path()
                .join("placement-data/placement/current")
                .exists()
        );
        std::fs::remove_file(link)?;
        assert!(
            prepare(root.path(), &config, &cancel, || -> Result<()> {
                anyhow::bail!("stale launch")
            })
            .is_err()
        );
        assert!(
            !root
                .path()
                .join("placement-data/placement/current")
                .exists()
        );
        let staging = root.path().join("placement-data/placement/staging");
        directory(&staging)?;
        vault::write_new_private(&staging.join("partial"), b"interrupted")?;
        let data = prepare(root.path(), &config, &cancel, || Ok(()))?;
        assert_eq!(
            std::fs::read(data.join("apps/project/storage/db/table"))?,
            b"initial"
        );
        assert!(!staging.exists());
        Ok(())
    }

    #[test]
    fn damaged_published_data_never_reseeds_or_creates_an_empty_store() -> Result<()> {
        let root = state_root()?;
        let source = tempfile::tempdir()?;
        seed(source.path(), b"initial");
        let config = config(source.path());
        let cancel = CancellationToken::new();
        let data = prepare(root.path(), &config, &cancel, || Ok(()))?;
        std::fs::remove_dir_all(&data)?;
        assert!(validate_root(&data, &config).is_err());
        assert!(prepare(root.path(), &config, &cancel, || Ok(())).is_err());
        assert!(!data.exists());
        std::fs::remove_dir_all(data.parent().unwrap())?;
        assert!(prepare(root.path(), &config, &cancel, || Ok(())).is_err());
        assert!(!data.exists());
        Ok(())
    }

    #[test]
    fn publication_guard_survives_rename_and_cancellation_after_acquiring_it_prevents_publish()
    -> Result<()> {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        struct Guard {
            current: PathBuf,
            observed: Arc<AtomicBool>,
        }
        impl Drop for Guard {
            fn drop(&mut self) {
                self.observed.store(
                    self.current.join("binding.json").is_file(),
                    Ordering::SeqCst,
                );
            }
        }
        let root = state_root()?;
        let source = tempfile::tempdir()?;
        seed(source.path(), b"initial");
        let config = config(source.path());
        let cancel = CancellationToken::new();
        let observed = Arc::new(AtomicBool::new(false));
        let current = root.path().join("placement-data/placement/current");
        prepare(root.path(), &config, &cancel, || {
            Ok(Guard {
                current,
                observed: observed.clone(),
            })
        })?;
        assert!(observed.load(Ordering::SeqCst));
        let other = state_root()?;
        assert!(
            prepare(other.path(), &config, &cancel, || {
                cancel.cancel();
                Ok(())
            })
            .is_err()
        );
        assert!(
            !other
                .path()
                .join("placement-data/placement/current")
                .exists()
        );
        Ok(())
    }
}
