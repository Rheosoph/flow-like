use anyhow::{Result, ensure};
use std::{
    collections::HashSet,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn private_directory(path: &Path) -> Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    match builder.create(path) {
        Ok(()) =>
        {
            #[cfg(unix)]
            if let Some(parent) = path.parent() {
                std::fs::File::open(parent)?.sync_all()?;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(e) => return Err(e.into()),
    }
    let meta = std::fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir() && !meta.file_type().is_symlink(),
        "Outbox directory must not be a symlink"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            meta.mode() & 0o077 == 0 && meta.uid() == unsafe { libc::geteuid() },
            "Outbox directory must be private and owned by this user"
        );
    }
    Ok(())
}

pub fn relative_path(value: &str, empty: bool) -> bool {
    (empty && value.is_empty())
        || (!value.is_empty()
            && value.len() <= 1024
            && !value.contains(['\\', '%', '\0'])
            && value
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."))
}

/// Same rules as the standalone placement identifier: at most 128 characters of
/// `[A-Za-z0-9._-]`, never `.` or `..`.
pub fn validate_namespace(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.'),
        "Invalid placement identifier"
    );
    Ok(())
}

pub fn unix_time() -> Result<i64> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .try_into()?)
}

/// Removes `<name>.lance` directories under `root` whose name is not retained.
pub fn clean_orphaned_tables(root: &Path, retained: &HashSet<String>) -> Result<()> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = name.strip_suffix(".lance") else {
            continue;
        };
        if retained.contains(name) {
            continue;
        }
        let metadata = std::fs::symlink_metadata(entry.path())?;
        ensure!(
            metadata.is_dir() && !metadata.file_type().is_symlink(),
            "Offline tables must be private directories"
        );
        let mut stack = vec![std::fs::read_dir(entry.path())?];
        while let Some(directory) = stack.last_mut() {
            let Some(child) = directory.next() else {
                stack.pop();
                continue;
            };
            let child = child?;
            let metadata = std::fs::symlink_metadata(child.path())?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "Orphaned offline tables must not contain symlinks"
            );
            if metadata.is_dir() {
                ensure!(
                    stack.len() < 64,
                    "Offline table directory nesting exceeds its limit"
                );
                stack.push(std::fs::read_dir(child.path())?);
            } else {
                ensure!(metadata.is_file(), "Unexpected file type in offline table");
            }
        }
        std::fs::remove_dir_all(entry.path())?;
    }
    Ok(())
}

/// Offline-class object-store errors: connect, DNS and timeout failures, lease refresh
/// failures (`AuthorizationError::Unavailable`/`Expired`) and final 5xx answers. `extra`
/// recognizes host-specific error types in the source chain. A 404, failed condition,
/// invalid credential or access denial is never offline-class.
#[cfg(feature = "runtime")]
pub fn is_offline_error(
    error: &flow_like_storage::object_store::Error,
    extra: &dyn Fn(&(dyn std::error::Error + 'static)) -> bool,
) -> bool {
    use flow_like_storage::object_store;
    use flow_like_types::authorization::AuthorizationError;
    if !matches!(error, object_store::Error::Generic { .. }) {
        return false;
    }
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(error) = source {
        if extra(error) {
            return true;
        }
        if let Some(error) = error.downcast_ref::<AuthorizationError>() {
            return matches!(
                error,
                AuthorizationError::Unavailable | AuthorizationError::Expired
            );
        }
        if let Some(error) = error.downcast_ref::<object_store::client::HttpError>() {
            use object_store::client::HttpErrorKind;
            if matches!(
                error.kind(),
                HttpErrorKind::Connect | HttpErrorKind::Timeout | HttpErrorKind::Interrupted
            ) {
                return true;
            }
        }
        if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            return (error.status().is_none()
                && (error.is_connect()
                    || error.is_timeout()
                    || error.is_body()
                    || error.is_request()))
                || error
                    .status()
                    .is_some_and(|status| status.is_server_error());
        }
        if let Some(error) = error.downcast_ref::<std::io::Error>() {
            return matches!(
                error.kind(),
                std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::NetworkUnreachable
                    | std::io::ErrorKind::HostUnreachable
            );
        }
        source = error.source();
    }
    false
}

/// Bytes of the regular files below `root`, without following symlinks. 0 when absent.
pub fn directory_bytes(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(directory) = stack.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        for entry in entries {
            let entry = entry?;
            let metadata = std::fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                stack.push(entry.path());
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}
