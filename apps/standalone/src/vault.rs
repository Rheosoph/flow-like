#[cfg(not(unix))]
use anyhow::bail;
use anyhow::{Context, Result, ensure};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use zeroize::Zeroizing;

const MAX_SECRET_FILE: u64 = 1024 * 1024;

pub use flow_like_device_crypto::vault::{controller_context, invitation_context, open, seal};

/// Create once, never replace a key or follow a symlink. Caller owns the private parent directory.
pub fn write_new_private(path: &Path, bytes: &[u8]) -> Result<()> {
    #[cfg(not(unix))]
    bail!("Private key storage currently requires Unix file permissions");
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)
            .context("Create private file without replacing existing data")?;
        file.write_all(bytes)?;
        file.sync_all()?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    }
}

pub fn read_private(path: &Path) -> Result<Zeroizing<Vec<u8>>> {
    #[cfg(not(unix))]
    bail!("Private key storage currently requires Unix file permissions");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
            .context("Open private file")?;
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
}
