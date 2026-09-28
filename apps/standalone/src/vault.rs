#[cfg(not(unix))]
use anyhow::bail;
use anyhow::{Context, Result, ensure};
use rand_core::{OsRng, RngCore};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
    time::Duration,
};
use zeroize::Zeroizing;

const MAX_SECRET_FILE: u64 = 1024 * 1024;
/// Staging files are locked while written; an unlocked one this old was left
/// by a crash. The age also covers the instant between creating and locking.
const ABANDONED_STAGING_AGE: Duration = Duration::from_secs(60);

pub use flow_like_device_crypto::vault::{controller_context, invitation_context, open, seal};

/// Create once, never replace data or follow a symlink. The file appears at its
/// final path only after its contents are durable, so a crash never leaves a torn
/// file there. An empty file left by an interrupted older write holds no data and
/// is replaced. Caller owns the private parent directory.
///
/// A crash after publishing can leave a full private copy in a
/// `.{name}.{uuid}.tmp` staging file. The next write of the same name removes
/// it; directories whose files are deleted or never rewritten call
/// [`remove_abandoned_private_staging`] when they clean up.
pub fn write_new_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(not(unix))]
    bail!("Private key storage currently requires Unix file permissions");
    #[cfg(unix)]
    {
        let name = path
            .file_name()
            .with_context(|| format!("Private file path {} has no file name", path.display()))?
            .to_string_lossy();
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        remove_abandoned_staging(parent, |candidate| {
            staging_target(candidate) == Some(name.as_ref())
        });
        let temporary = parent.join(format!(".{name}.{}.tmp", uuid::Uuid::new_v4()));
        let result = publish_private(&temporary, path, parent, bytes);
        if let Err(error) = std::fs::remove_file(&temporary) {
            if error.kind() != std::io::ErrorKind::NotFound && result.is_ok() {
                tracing::warn!(
                    "Private file {} was published, but its staging file remains: {error}",
                    path.display()
                );
            }
        }
        result
    }
}

#[cfg(unix)]
fn publish_private(temporary: &Path, path: &Path, parent: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(temporary)
        .with_context(|| format!("Create staging file for {}", path.display()))?;
    file.lock()
        .and_then(|()| file.write_all(bytes))
        .and_then(|()| file.sync_all())
        .with_context(|| format!("Write private file {}", path.display()))?;
    // A hard link publishes atomically and, unlike rename, never replaces a name.
    match std::fs::hard_link(temporary, path) {
        Ok(()) => {}
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                && !bytes.is_empty()
                && replace_abandoned(temporary, path)? => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(error).with_context(|| {
                format!(
                    "Create private file {} without replacing existing data",
                    path.display()
                )
            });
        }
        // Filesystems without hard links still publish whole files; only a
        // concurrent first creation can race there.
        Err(error)
            if matches!(error.raw_os_error(), Some(libc::EPERM | libc::EOPNOTSUPP))
                || error.kind() == std::io::ErrorKind::Unsupported =>
        {
            match std::fs::symlink_metadata(path) {
                Err(missing) if missing.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::rename(temporary, path)
                        .with_context(|| format!("Publish private file {}", path.display()))?;
                }
                _ => {
                    return Err(error).with_context(|| {
                        format!(
                            "Create private file {} without replacing existing data",
                            path.display()
                        )
                    });
                }
            }
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Publish private file {}", path.display()));
        }
    }
    File::open(parent)?.sync_all()?;
    Ok(())
}

#[cfg(unix)]
fn open_private_nofollow(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
}

#[cfg(unix)]
fn same_file(held: &std::fs::Metadata, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path)
        .is_ok_and(|current| (current.dev(), current.ino()) == (held.dev(), held.ino()))
}

/// Replace an empty agent-owned file left by an interrupted older write. The
/// lock on that file makes concurrent recoveries publish exactly one
/// replacement; a later one finds the name already holding data.
#[cfg(unix)]
fn replace_abandoned(temporary: &Path, path: &Path) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;
    let Ok(existing) = open_private_nofollow(path) else {
        return Ok(false);
    };
    existing
        .lock()
        .with_context(|| format!("Lock interrupted private file {}", path.display()))?;
    let held = existing.metadata()?;
    if !(held.is_file()
        && held.len() == 0
        && held.uid() == unsafe { libc::geteuid() }
        && same_file(&held, path))
    {
        return Ok(false);
    }
    std::fs::rename(temporary, path)
        .with_context(|| format!("Replace interrupted empty private file {}", path.display()))?;
    Ok(true)
}

/// The final file name of a [`write_new_private`] staging file name.
#[cfg(unix)]
fn staging_target(candidate: &str) -> Option<&str> {
    let (name, id) = candidate
        .strip_prefix('.')?
        .strip_suffix(".tmp")?
        .rsplit_once('.')?;
    (!name.is_empty() && uuid::Uuid::parse_str(id).is_ok()).then_some(name)
}

/// Remove every abandoned [`write_new_private`] staging file in `directory`,
/// whatever final name it was written for.
#[cfg(unix)]
pub fn remove_abandoned_private_staging(directory: &Path) {
    remove_abandoned_staging(directory, |candidate| staging_target(candidate).is_some());
}

/// Remove staging files an interrupted write left in `directory`: agent-owned,
/// unlocked and older than [`ABANDONED_STAGING_AGE`]. Best effort; a staging
/// file still being written is never touched.
#[cfg(unix)]
pub fn remove_abandoned_staging(directory: &Path, is_staging: impl Fn(&str) -> bool) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if !name.to_str().is_some_and(&is_staging) {
            continue;
        }
        let path = entry.path();
        if let Err(error) = remove_if_abandoned(&path) {
            tracing::debug!(
                "Unable to remove abandoned staging file {}: {error:#}",
                path.display()
            );
        }
    }
}

#[cfg(unix)]
fn remove_if_abandoned(path: &Path) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    let file = open_private_nofollow(path)?;
    let held = file.metadata()?;
    let stale = held.is_file()
        && held.uid() == unsafe { libc::geteuid() }
        && held
            .modified()?
            .elapsed()
            .is_ok_and(|age| age >= ABANDONED_STAGING_AGE);
    if stale && file.try_lock().is_ok() && same_file(&held, path) {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

/// Load a fixed-size agent key, creating it on first use. Concurrent first users
/// agree on one key, and an empty file from an interrupted creation is replaced.
pub fn load_or_create_key<const N: usize>(path: &Path, label: &str) -> Result<Zeroizing<[u8; N]>> {
    let missing = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata.is_file() && metadata.len() == 0,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            return Err(error).with_context(|| format!("Inspect {label} {}", path.display()));
        }
    };
    if missing {
        let mut key = Zeroizing::new([0; N]);
        OsRng.fill_bytes(key.as_mut());
        if let Err(error) = write_new_private(path, key.as_ref()) {
            if std::fs::symlink_metadata(path).map_or(true, |metadata| metadata.len() == 0) {
                return Err(error);
            }
        }
    }
    let bytes = read_private(path)?;
    ensure!(
        bytes.len() == N,
        "Invalid {label} {}: expected {N} bytes, found {}",
        path.display(),
        bytes.len()
    );
    let mut key = Zeroizing::new([0; N]);
    key.copy_from_slice(&bytes);
    Ok(key)
}

pub fn read_private(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    #[cfg(not(unix))]
    bail!("Private key storage currently requires Unix file permissions");
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let file = open_private_nofollow(path).context("Open private file")?;
        let metadata = file.metadata()?;
        ensure!(metadata.is_file(), "Private data must be a regular file");
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o077 == 0,
            "Private files must belong to the current user with mode 0600 or 0400"
        );
        ensure!(
            metadata.len() <= MAX_SECRET_FILE,
            "Private file exceeds the size limit"
        );
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_SECRET_FILE + 1).read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_SECRET_FILE,
            "Private file exceeds the size limit"
        );
        Ok(bytes)
    }
}

/// Read from a terminal with echo disabled. Passwords never enter argv or the environment.
pub fn read_password(confirm: bool) -> Result<Zeroizing<Vec<u8>>> {
    #[cfg(not(unix))]
    bail!("Interactive password entry currently requires Unix");
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let mut terminal = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/tty")
            .context("Open an interactive terminal for password entry")?;
        let fd = terminal.as_raw_fd();
        let mut original = unsafe { std::mem::zeroed::<libc::termios>() };
        ensure!(
            unsafe { libc::tcgetattr(fd, &mut original) } == 0,
            "Read terminal settings"
        );
        struct Restore {
            fd: i32,
            original: libc::termios,
        }
        impl Drop for Restore {
            fn drop(&mut self) {
                unsafe {
                    libc::tcsetattr(self.fd, libc::TCSAFLUSH, &self.original);
                }
            }
        }
        let _restore = Restore { fd, original };
        let mut hidden = original;
        // Read Ctrl-C as input so the guard restores terminal echo before returning.
        hidden.c_lflag &= !(libc::ECHO | libc::ISIG);
        ensure!(
            unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &hidden) } == 0,
            "Disable password echo"
        );
        fn read_line(terminal: &mut File, prompt: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
            terminal.write_all(prompt)?;
            terminal.flush()?;
            let mut value = Zeroizing::new(Vec::new());
            loop {
                let mut byte = [0];
                ensure!(terminal.read(&mut byte)? == 1, "Password input ended");
                ensure!(!matches!(byte[0], 3 | 26), "Password input cancelled");
                if byte[0] == b'\n' {
                    break;
                }
                ensure!(value.len() < 4096, "Password exceeds the size limit");
                value.push(byte[0]);
            }
            terminal.write_all(b"\n")?;
            Ok(value)
        }
        let password = read_line(&mut terminal, b"Device management password: ")?;
        if confirm {
            ensure!(password.len() >= 12, "Use a password of at least 12 bytes");
            let confirmation = read_line(&mut terminal, b"Repeat password: ")?;
            ensure!(*password == *confirmation, "Passwords do not match");
        }
        Ok(password)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vault_rejects_wrong_password_context_and_tampering() {
        let password = b"a memorable password for testing";
        let mut sealed = seal(password, b"device-a", b"controller key").unwrap();
        assert_eq!(
            &**open(password, b"device-a", &sealed).unwrap(),
            b"controller key"
        );
        assert!(open(b"wrong password", b"device-a", &sealed).is_err());
        assert!(open(password, b"device-b", &sealed).is_err());
        *sealed.last_mut().unwrap() ^= 1;
        assert!(open(password, b"device-a", &sealed).is_err());
        assert!(open(password, b"device-a", b"truncated").is_err());
    }

    #[test]
    #[cfg(unix)]
    fn private_files_reject_replacement_symlinks_and_world_readability() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("key");
        write_new_private(&path, b"secret").unwrap();
        assert!(write_new_private(&path, b"replacement").is_err());
        assert_eq!(&**read_private(&path).unwrap(), b"secret");
        let link = dir.path().join("link");
        symlink(&path, &link).unwrap();
        assert!(read_private(&link).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(read_private(&path).is_err());
    }

    #[cfg(unix)]
    fn entries(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    #[cfg(unix)]
    fn private_files_are_published_whole_and_leave_no_staging_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("receipt.json");
        write_new_private(&path, b"complete receipt").unwrap();
        assert!(write_new_private(&path, b"other").is_err());
        assert_eq!(&**read_private(&path).unwrap(), b"complete receipt");
        assert_eq!(entries(dir.path()), ["receipt.json"]);
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(dir.path().join("missing"), &link).unwrap();
        assert!(write_new_private(&link, b"through symlink").is_err());
        assert!(!dir.path().join("missing").exists());
        assert_eq!(entries(dir.path()), ["link", "receipt.json"]);
    }

    #[test]
    #[cfg(unix)]
    fn interrupted_empty_files_are_replaced_but_never_real_data() {
        let dir = tempfile::tempdir().unwrap();
        let torn = dir.path().join("secrets.key");
        std::fs::File::create(&torn).unwrap();
        write_new_private(&torn, b"recovered").unwrap();
        assert_eq!(&**read_private(&torn).unwrap(), b"recovered");
        assert!(write_new_private(&torn, b"replacement").is_err());
        let empty = dir.path().join("empty");
        write_new_private(&empty, b"").unwrap();
        assert!(write_new_private(&empty, b"").is_err());
        assert_eq!(entries(dir.path()), ["empty", "secrets.key"]);
    }

    #[test]
    #[cfg(unix)]
    fn agent_keys_recover_from_interrupted_creation_and_reject_damage() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("telemetry.key");
        let first = load_or_create_key::<32>(&path, "telemetry key").unwrap();
        assert_eq!(
            *load_or_create_key::<32>(&path, "telemetry key").unwrap(),
            *first
        );
        std::fs::remove_file(&path).unwrap();
        std::fs::File::create(&path).unwrap();
        let recovered = load_or_create_key::<32>(&path, "telemetry key").unwrap();
        assert_eq!(&**read_private(&path).unwrap(), recovered.as_slice());
        let damaged = dir.path().join("mls.key");
        write_new_private(&damaged, &[7; 16]).unwrap();
        let error = load_or_create_key::<32>(&damaged, "MLS key").unwrap_err();
        assert!(error.to_string().contains("expected 32 bytes, found 16"));
    }

    #[test]
    #[cfg(unix)]
    fn concurrent_recoveries_of_an_interrupted_key_agree_on_one_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.key");
        for _ in 0..16 {
            std::fs::File::create(&path).unwrap();
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
            let keys: Vec<_> = (0..8)
                .map(|_| {
                    let barrier = barrier.clone();
                    let path = path.clone();
                    std::thread::spawn(move || {
                        barrier.wait();
                        *load_or_create_key::<32>(&path, "secret storage key").unwrap()
                    })
                })
                .collect::<Vec<_>>()
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .collect();
            let stored = read_private(&path).unwrap();
            assert!(keys.iter().all(|key| key.as_slice() == stored.as_slice()));
            std::fs::remove_file(&path).unwrap();
        }
        assert!(entries(dir.path()).is_empty());
    }

    #[test]
    #[cfg(unix)]
    fn abandoned_staging_files_are_removed_but_active_ones_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let stage = |name: &str, age: Duration| {
            let path = dir.path().join(name);
            let file = File::create(&path).unwrap();
            file.set_modified(std::time::SystemTime::now() - age)
                .unwrap();
            file
        };
        let old = ABANDONED_STAGING_AGE * 2;
        let abandoned = format!(".secrets.key.{}.tmp", uuid::Uuid::new_v4());
        let fresh = format!(".secrets.key.{}.tmp", uuid::Uuid::new_v4());
        let writing = format!(".secrets.key.{}.tmp", uuid::Uuid::new_v4());
        let unrelated = ".secrets.key.backup.tmp";
        drop(stage(&abandoned, old));
        drop(stage(&fresh, Duration::ZERO));
        drop(stage(unrelated, old));
        let active = stage(&writing, old);
        active.lock().unwrap();
        write_new_private(&dir.path().join("secrets.key"), b"key").unwrap();
        let mut expected = vec![fresh, writing, unrelated.to_owned(), "secrets.key".into()];
        expected.sort();
        assert_eq!(entries(dir.path()), expected);
    }

    #[test]
    #[cfg(unix)]
    fn directory_sweeps_remove_abandoned_copies_of_deleted_private_files() {
        let dir = tempfile::tempdir().unwrap();
        let stage = |name: &str, age: Duration| {
            let path = dir.path().join(name);
            let file = File::create(&path).unwrap();
            file.set_modified(std::time::SystemTime::now() - age)
                .unwrap();
            file
        };
        let old = ABANDONED_STAGING_AGE * 2;
        for name in ["certificate.key", "receipt.json", "7.partial"] {
            drop(stage(&format!(".{name}.{}.tmp", uuid::Uuid::new_v4()), old));
        }
        let fresh = format!(".certificate.key.{}.tmp", uuid::Uuid::new_v4());
        let writing = format!(".receipt.json.{}.tmp", uuid::Uuid::new_v4());
        drop(stage(&fresh, Duration::ZERO));
        let active = stage(&writing, old);
        active.lock().unwrap();
        for unrelated in ["..tmp", ".backup.tmp", "certificate.json"] {
            drop(stage(unrelated, old));
        }
        remove_abandoned_private_staging(dir.path());
        let mut expected = vec![
            fresh,
            writing,
            "..tmp".into(),
            ".backup.tmp".into(),
            "certificate.json".into(),
        ];
        expected.sort();
        assert_eq!(entries(dir.path()), expected);
    }
}
