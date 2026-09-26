use anyhow::{Context, Result, ensure};
use std::path::{Component, Path, PathBuf};

pub const SYSTEMD_UNIT_NAME: &str = "flow-like-standalone.service";
pub const LAUNCHD_LABEL: &str = "com.flow-like.standalone";

#[cfg(any(target_os = "macos", all(unix, test)))]
#[cfg_attr(all(test, not(target_os = "macos")), allow(dead_code))]
mod launchd;

#[derive(Debug, serde::Serialize)]
pub struct ServiceStatus {
    pub manager: &'static str,
    pub startup: &'static str,
    pub definition: PathBuf,
    pub installed: bool,
    pub manager_available: Option<bool>,
    pub loaded: bool,
    pub process_id: Option<u32>,
}

pub fn service_startup() -> &'static str {
    if cfg!(target_os = "macos") {
        "user_login"
    } else {
        "boot"
    }
}

/// Print the platform's service definition without creating state or keys.
pub fn user_service_definition(executable: &Path, state_dir: &Path) -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        launchd_user_plist(executable, state_dir)
    }
    #[cfg(not(target_os = "macos"))]
    {
        systemd_user_unit(executable, state_dir)
    }
}

/// A LaunchAgent starts after GUI login, including the first login after reboot.
pub fn launchd_user_plist(executable: &Path, state_dir: &Path) -> Result<String> {
    let executable = unit_path(executable, "Executable")?;
    let state_dir = unit_path(state_dir, "State directory")?;
    ensure!(!executable.ends_with('/'), "Executable must name a file");
    let escape = |value: &str| {
        value
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    };
    let executable = escape(executable);
    let state_dir = escape(state_dir);
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<!-- Operator isolation policy is read from agent.env in the state directory. -->
<key>Label</key><string>{LAUNCHD_LABEL}</string>
<key>ProgramArguments</key><array><string>{executable}</string><string>--state-dir</string><string>{state_dir}</string><string>run</string></array>
<key>WorkingDirectory</key><string>{state_dir}</string>
<key>LimitLoadToSessionType</key><string>Aqua</string>
<key>RunAtLoad</key><true/>
<key>KeepAlive</key><dict><key>SuccessfulExit</key><false/></dict>
<key>ThrottleInterval</key><integer>5</integer>
<key>ExitTimeOut</key><integer>25</integer>
<key>Umask</key><integer>63</integer>
</dict></plist>
"#
    ))
}

pub async fn user_service_status(executable: &Path, state_dir: &Path) -> Result<ServiceStatus> {
    #[cfg(target_os = "linux")]
    {
        linux::status(executable, state_dir).await
    }
    #[cfg(target_os = "macos")]
    {
        launchd::status(executable, state_dir).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (executable, state_dir);
        anyhow::bail!("Service inspection supports Linux and macOS only")
    }
}

/// Generate a unit without accessing the filesystem or changing the host.
pub fn systemd_user_unit(executable: &Path, state_dir: &Path) -> Result<String> {
    let executable = unit_path(executable, "Executable")?;
    let state_dir = unit_path(state_dir, "State directory")?;
    ensure!(
        !executable.contains(['\\', '\'', '"', '*', '?', '[', ']']),
        "systemd does not support quotes, backslashes, or glob characters in the executable path"
    );
    ensure!(!executable.ends_with('/'), "Executable must name a file");
    ensure!(
        !state_dir.ends_with('\\') && state_dir.trim_end() == state_dir,
        "State directory must not end with whitespace or a backslash"
    );

    // ':' disables environment expansion. Both fields still expand '%' specifiers.
    let command = exec_word(&format!(":{executable}"));
    let state_argument = exec_word(state_dir);
    // WorkingDirectory is a raw path, unlike the quoted words in ExecStart.
    let working_directory = state_dir.replace('%', "%%");
    Ok(format!(
        "# Managed by Flow-Like standalone service installation.\n\
         # Operator isolation policy is read from agent.env in the state directory.\n\
         [Unit]\n\
         Description=Flow-Like standalone device agent\n\
         StartLimitIntervalSec=0\n\
         \n\
         [Service]\n\
         Type=exec\n\
         ExecStart={command} --state-dir {state_argument} run\n\
         WorkingDirectory={working_directory}\n\
         Restart=on-failure\n\
         RestartSec=5s\n\
         KillMode=mixed\n\
         TimeoutStopSec=25s\n\
         UMask=0077\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n"
    ))
}

fn unit_path<'a>(path: &'a Path, name: &str) -> Result<&'a str> {
    ensure!(path.is_absolute(), "{name} must be an absolute path");
    ensure!(
        !path
            .components()
            .any(|part| matches!(part, Component::ParentDir)),
        "{name} must not contain parent-directory components"
    );
    let value = path
        .to_str()
        .with_context(|| format!("{name} must be UTF-8"))?;
    ensure!(
        !value.chars().any(char::is_control),
        "{name} must not contain control characters, newlines, or NUL"
    );
    ensure!(value.len() <= 4096, "{name} is too long");
    Ok(value)
}

fn exec_word(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('%', "%%")
    )
}

/// Install and start the agent. Call only for an explicit service-install command.
/// The supplied state directory must already exist; installation creates no device keys.
pub async fn install_user_service(executable: &Path, state_dir: &Path) -> Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        linux::install(executable, state_dir).await
    }
    #[cfg(target_os = "macos")]
    {
        launchd::install(executable, state_dir).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (executable, state_dir);
        anyhow::bail!(
            "Automatic service installation supports Linux with systemd and macOS after user login"
        )
    }
}

/// Stop and remove this exact managed unit, preserving device state and account lingering.
/// Returns false when this user's unit file does not exist.
pub async fn uninstall_user_service(executable: &Path, state_dir: &Path) -> Result<bool> {
    #[cfg(target_os = "linux")]
    {
        linux::uninstall(executable, state_dir).await
    }
    #[cfg(target_os = "macos")]
    {
        launchd::uninstall(executable, state_dir).await
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = (executable, state_dir);
        anyhow::bail!(
            "Automatic service removal supports Linux with systemd and macOS LaunchAgents"
        )
    }
}

/// Read-only check used before an authenticated update of an installed service.
pub async fn verify_user_service(executable: &Path, state_dir: &Path) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::verify(executable, state_dir).await
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (executable, state_dir);
        anyhow::bail!("Managed updates require Linux with systemd")
    }
}

#[cfg(unix)]
mod private_file {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        io::{ErrorKind, Read, Write},
        os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    };

    pub(super) fn matches(path: &Path, expected: &str) -> Result<bool> {
        let file = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => {
                return Err(error).context("Open existing service unit without following symlinks");
            }
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file(),
            "Existing service unit is not a regular file"
        );
        // SAFETY: geteuid has no arguments or preconditions.
        ensure!(
            metadata.uid() == unsafe { libc::geteuid() },
            "Existing service unit belongs to another user"
        );
        ensure!(
            metadata.mode() & 0o077 == 0,
            "Existing service unit must have owner-only permissions (0600)"
        );
        ensure!(
            metadata.nlink() == 1,
            "Existing service unit must not have hard links"
        );
        ensure!(
            metadata.len() == expected.len() as u64,
            "Existing service unit differs; refusing to replace or remove it"
        );
        let mut actual = String::new();
        file.take(expected.len() as u64 + 1)
            .read_to_string(&mut actual)?;
        ensure!(
            actual == expected,
            "Existing service unit differs; refusing to replace or remove it"
        );
        Ok(true)
    }

    pub(super) fn create_or_verify(path: &Path, expected: &str) -> Result<()> {
        if matches(path, expected)? {
            return Ok(());
        }
        let mut file = match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                ensure!(
                    matches(path, expected)?,
                    "Service unit disappeared during installation"
                );
                return Ok(());
            }
            Err(error) => return Err(error).context("Create private service unit"),
        };
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        file.write_all(expected.as_bytes())
            .context("Write service unit")?;
        file.sync_all().context("Persist service unit")?;
        if let Some(parent) = path.parent() {
            File::open(parent)?
                .sync_all()
                .context("Persist service unit directory")?;
        }
        Ok(())
    }
}

// Compile the Unix-only implementation in macOS tests without running service commands.
#[cfg(any(target_os = "linux", all(unix, test)))]
#[cfg_attr(all(test, not(target_os = "linux")), allow(dead_code))]
mod linux {
    use super::*;
    use std::{
        fs::{DirBuilder, File},
        io::ErrorKind,
        os::unix::fs::{DirBuilderExt, MetadataExt},
        process::{Output, Stdio},
        time::Duration,
    };
    use tokio::process::Command;

    fn user_id() -> u32 {
        // SAFETY: geteuid has no arguments or preconditions.
        unsafe { libc::geteuid() }
    }

    fn config_directory() -> Result<PathBuf> {
        let base = match std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => PathBuf::from(
                std::env::var_os("HOME").context("HOME or XDG_CONFIG_HOME must be set")?,
            )
            .join(".config"),
        };
        unit_path(&base, "User configuration directory")?;
        Ok(base)
    }

    fn check_directory(path: &Path) -> Result<()> {
        let metadata = std::fs::symlink_metadata(path)
            .with_context(|| format!("Inspect {}", path.display()))?;
        ensure!(
            metadata.is_dir(),
            "{} must be a directory, not a symlink",
            path.display()
        );
        ensure!(
            metadata.uid() == user_id(),
            "{} must belong to the current user",
            path.display()
        );
        ensure!(
            metadata.mode() & 0o022 == 0,
            "{} must not be writable by other users",
            path.display()
        );
        Ok(())
    }

    fn trusted_executable_owner(owner: u32, current_user: u32) -> bool {
        owner == 0 || owner == current_user
    }

    fn check_executable(path: &Path) -> Result<()> {
        let metadata = std::fs::symlink_metadata(path).context("Inspect service executable")?;
        ensure!(
            metadata.is_file() && !metadata.file_type().is_symlink(),
            "Service executable must be a regular file, not a symlink"
        );
        ensure!(
            metadata.mode() & 0o111 != 0,
            "Service executable must have execute permission"
        );
        ensure!(
            trusted_executable_owner(metadata.uid(), user_id()),
            "Service executable must belong to the current user or root"
        );
        ensure!(
            metadata.mode() & 0o022 == 0,
            "Service executable must not be writable by other users"
        );
        Ok(())
    }

    fn unit_path_for_user(create: bool) -> Result<PathBuf> {
        let base = config_directory()?;
        let systemd = base.join("systemd");
        let user = systemd.join("user");
        for directory in [&base, &systemd, &user] {
            if create {
                DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(directory)
                    .with_context(|| format!("Create {}", directory.display()))?;
            } else if let Err(error) = std::fs::symlink_metadata(directory) {
                if error.kind() == ErrorKind::NotFound {
                    return Ok(user.join(SYSTEMD_UNIT_NAME));
                }
                return Err(error).context("Inspect user service directory");
            }
            check_directory(directory)?;
        }
        Ok(user.join(SYSTEMD_UNIT_NAME))
    }

    async fn command(program: &str, args: &[&str]) -> Result<Output> {
        tokio::time::timeout(
            Duration::from_secs(45),
            Command::new(program)
                .args(args)
                .env("LC_ALL", "C")
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .with_context(|| {
            format!("{program} timed out; inspect the user service state before retrying")
        })?
        .with_context(|| {
            format!("Run {program}; Linux with systemd and a running user manager is required")
        })
    }

    fn successful(output: Output, action: &str) -> Result<String> {
        ensure!(
            output.status.success(),
            "{action} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(2048)
                .collect::<String>()
                .trim()
        );
        String::from_utf8(output.stdout).context("systemd returned invalid UTF-8")
    }

    async fn systemctl(args: &[&str]) -> Result<String> {
        let mut selected = vec!["--user", "--no-pager", "--no-ask-password"];
        selected.extend_from_slice(args);
        successful(
            command("systemctl", &selected).await?,
            "User service operation",
        )
    }

    async fn verify_manager_unit(path: &Path, must_exist: bool) -> Result<()> {
        let output = systemctl(&[
            "show",
            "--all",
            "--property=FragmentPath",
            "--property=DropInPaths",
            SYSTEMD_UNIT_NAME,
        ])
        .await?;
        let mut fragment = None;
        let mut drop_ins = None;
        for line in output.lines() {
            if let Some(value) = line.strip_prefix("FragmentPath=") {
                fragment = Some(value);
            } else if let Some(value) = line.strip_prefix("DropInPaths=") {
                drop_ins = Some(value);
            }
        }
        let fragment = fragment.context("systemd did not report a service fragment path")?;
        ensure!(
            drop_ins == Some(""),
            "Service has drop-in configuration; refusing to change an overridden unit"
        );
        ensure!(
            (!must_exist && fragment.is_empty()) || Path::new(fragment) == path,
            "systemd resolves the service to another unit; refusing to replace or manage it"
        );
        Ok(())
    }

    async fn ensure_lingering() -> Result<()> {
        let uid = user_id().to_string();
        let show_args = [
            "--no-pager",
            "--no-ask-password",
            "show-user",
            uid.as_str(),
            "--property=Linger",
            "--value",
        ];
        if command("loginctl", &show_args).await.is_ok_and(|output| {
            output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "yes"
        }) {
            return Ok(());
        }
        let instructions = format!(
            "Boot persistence is not configured. Have an administrator run `loginctl enable-linger {uid}`, then retry installation"
        );
        successful(
            command("loginctl", &["--no-ask-password", "enable-linger", &uid])
                .await
                .context(instructions.clone())?,
            "Enable user manager at boot",
        )
        .context(instructions.clone())?;
        let linger = successful(
            command("loginctl", &show_args)
                .await
                .context(instructions.clone())?,
            "Verify boot persistence",
        )
        .context(instructions.clone())?;
        ensure!(linger.trim() == "yes", "{instructions}");
        Ok(())
    }

    pub(super) async fn install(executable: &Path, state_dir: &Path) -> Result<PathBuf> {
        ensure!(
            cfg!(feature = "runtime"),
            "Service installation requires a standalone binary built with the runtime feature"
        );
        let expected = systemd_user_unit(executable, state_dir)?;
        check_executable(executable)?;
        check_directory(state_dir)?;
        let path = unit_path_for_user(true)?;
        private_file::matches(&path, &expected)?;
        systemctl(&["daemon-reload"]).await?;
        verify_manager_unit(&path, false).await?;
        ensure_lingering().await?;
        private_file::create_or_verify(&path, &expected)?;
        systemctl(&["daemon-reload"]).await.context(
            "Unit was written; reload failed. Retry installation after restoring the user manager",
        )?;
        verify_manager_unit(&path, true).await?;
        systemctl(&["enable", "--now", SYSTEMD_UNIT_NAME]).await
            .context("Unit was written; starting or enabling failed. Inspect its status and retry installation")?;
        ensure!(
            systemctl(&["is-enabled", SYSTEMD_UNIT_NAME]).await?.trim() == "enabled",
            "Service is not persistently enabled"
        );
        ensure!(
            systemctl(&["is-active", SYSTEMD_UNIT_NAME]).await?.trim() == "active",
            "Service did not remain active after startup"
        );
        Ok(path)
    }

    pub(super) async fn uninstall(executable: &Path, state_dir: &Path) -> Result<bool> {
        let expected = systemd_user_unit(executable, state_dir)?;
        let path = unit_path_for_user(false)?;
        if !private_file::matches(&path, &expected)? {
            return Ok(false);
        }
        systemctl(&["daemon-reload"]).await?;
        verify_manager_unit(&path, true).await?;
        systemctl(&["disable", "--now", SYSTEMD_UNIT_NAME]).await?;
        ensure!(
            private_file::matches(&path, &expected)?,
            "Service unit changed during removal"
        );
        std::fs::remove_file(&path).context("Remove managed service unit")?;
        File::open(
            path.parent()
                .context("Service unit has no parent directory")?,
        )?
        .sync_all()?;
        systemctl(&["daemon-reload"])
            .await
            .context("Unit was removed; the user manager still needs daemon-reload")?;
        // Lingering can serve other units owned by this user, so removal leaves it enabled.
        Ok(true)
    }

    pub(super) async fn verify(executable: &Path, state_dir: &Path) -> Result<()> {
        let expected = systemd_user_unit(executable, state_dir)?;
        check_executable(executable)?;
        check_directory(state_dir)?;
        let path = unit_path_for_user(false)?;
        ensure!(
            private_file::matches(&path, &expected)?,
            "Install this exact managed user service before requesting automatic updates"
        );
        verify_manager_unit(&path, true).await
    }

    pub(super) async fn status(executable: &Path, state_dir: &Path) -> Result<ServiceStatus> {
        let expected = systemd_user_unit(executable, state_dir)?;
        let path = unit_path_for_user(false)?;
        let installed = private_file::matches(&path, &expected)?;
        let mut result = ServiceStatus {
            manager: "systemd",
            startup: "boot",
            definition: path.clone(),
            installed,
            manager_available: None,
            loaded: false,
            process_id: None,
        };
        if !installed {
            return Ok(result);
        }
        verify_manager_unit(&path, true).await?;
        let output = systemctl(&[
            "show",
            "--property=LoadState",
            "--property=MainPID",
            SYSTEMD_UNIT_NAME,
        ])
        .await?;
        result.manager_available = Some(true);
        for line in output.lines() {
            if let Some(value) = line.strip_prefix("LoadState=") {
                result.loaded = value == "loaded";
            }
            if let Some(value) = line.strip_prefix("MainPID=") {
                result.process_id = value.parse::<u32>().ok().filter(|pid| *pid > 0);
            }
        }
        Ok(result)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::os::unix::fs::{PermissionsExt, symlink};

        #[test]
        fn executable_rejects_symlinks_shared_writes_and_non_executable_files() -> Result<()> {
            let directory = tempfile::tempdir()?;
            let executable = directory.path().join("agent");
            std::fs::write(&executable, b"fixture")?;
            for mode in [0o700, 0o755, 0o555] {
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(mode))?;
                check_executable(&executable)?;
            }
            for mode in [0o644, 0o770, 0o757, 0o777] {
                std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(mode))?;
                assert!(
                    check_executable(&executable).is_err(),
                    "accepted mode {mode:o}"
                );
            }
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))?;
            let linked = directory.path().join("linked-agent");
            symlink(&executable, &linked)?;
            assert!(check_executable(&linked).is_err());
            assert!(check_executable(directory.path()).is_err());
            Ok(())
        }

        #[test]
        fn executable_ownership_allows_root_installations_but_not_other_users() {
            assert!(trusted_executable_owner(1000, 1000));
            assert!(trusted_executable_owner(0, 1000));
            assert!(trusted_executable_owner(0, 0));
            assert!(!trusted_executable_owner(1001, 1000));
            assert!(!trusted_executable_owner(1000, 0));
        }

        #[test]
        fn service_directories_reject_shared_writes_and_symlinks() -> Result<()> {
            let directory = tempfile::tempdir()?;
            let state = directory.path().join("state");
            DirBuilder::new().mode(0o700).create(&state)?;
            check_directory(&state)?;
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o770))?;
            assert!(check_directory(&state).is_err());
            std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700))?;
            let linked = directory.path().join("linked");
            symlink(&state, &linked)?;
            assert!(check_directory(&linked).is_err());
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchagent_uses_literal_arguments_and_login_lifecycle() -> Result<()> {
        let plist = launchd_user_plist(
            Path::new("/Applications/Flow & Like/agent'$%<x>\""),
            Path::new("/Users/owner/device & data"),
        )?;
        assert!(plist.contains(
            "<string>/Applications/Flow &amp; Like/agent&apos;$%&lt;x&gt;&quot;</string>"
        ));
        assert!(plist.contains("<string>--state-dir</string><string>/Users/owner/device &amp; data</string><string>run</string>"));
        assert!(plist.contains("<key>LimitLoadToSessionType</key><string>Aqua</string>"));
        assert!(plist.contains("<key>SuccessfulExit</key><false/>"));
        assert!(plist.contains("<key>ExitTimeOut</key><integer>25</integer>"));
        assert!(plist.contains("<key>Umask</key><integer>63</integer>"));
        assert!(!plist.contains("/bin/sh"));
        for path in ["relative", "/Users/../state", "/state\n", "/state\0"] {
            assert!(launchd_user_plist(Path::new("/agent"), Path::new(path)).is_err());
            assert!(launchd_user_plist(Path::new(path), Path::new("/state")).is_err());
        }
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unit_preserves_paths_and_disables_expansion() -> Result<()> {
        let unit = systemd_user_unit(
            Path::new("/opt/Flow Like/$agent%one"),
            Path::new("/home/user/state $literal%name/quoted\"dir\\part"),
        )?;
        assert!(unit.contains("ExecStart=\":/opt/Flow Like/$agent%%one\" --state-dir \"/home/user/state $literal%%name/quoted\\\"dir\\\\part\" run\n"));
        assert!(
            unit.contains("WorkingDirectory=/home/user/state $literal%%name/quoted\"dir\\part\n")
        );
        assert!(unit.contains("Restart=on-failure\n"));
        assert!(unit.contains("KillMode=mixed\nTimeoutStopSec=25s\nUMask=0077\n"));
        assert!(unit.ends_with("WantedBy=default.target\n"));
        Ok(())
    }

    #[test]
    fn unit_rejects_ambiguous_or_injected_paths() {
        let executable = Path::new("/opt/flow-like-standalone");
        for state in [
            "relative",
            "/home/../state",
            "/state\nExecStart=/bad",
            "/state\0",
            "/state\r",
            "/state\t",
            "/state ",
            "/state\\",
        ] {
            assert!(
                systemd_user_unit(executable, Path::new(state)).is_err(),
                "accepted {state:?}"
            );
        }
        for executable in [
            "relative",
            "/opt/../binary",
            "/opt/bin\n",
            "/opt/bin\0",
            "/opt/a\"b",
            "/opt/a'b",
            "/opt/a\\b",
            "/opt/b*n",
            "/opt/bin/",
        ] {
            assert!(
                systemd_user_unit(Path::new(executable), Path::new("/state")).is_err(),
                "accepted {executable:?}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn private_unit_is_idempotent_and_does_not_replace_other_files() -> Result<()> {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let directory = tempfile::tempdir()?;
        let path = directory.path().join(SYSTEMD_UNIT_NAME);
        let expected = systemd_user_unit(Path::new("/opt/agent"), Path::new("/state"))?;
        private_file::create_or_verify(&path, &expected)?;
        assert_eq!(
            std::fs::metadata(&path)?.permissions().mode() & 0o777,
            0o600
        );
        private_file::create_or_verify(&path, &expected)?;
        let other_state = systemd_user_unit(Path::new("/opt/agent"), Path::new("/other-state"))?;
        assert!(private_file::create_or_verify(&path, &other_state).is_err());
        assert_eq!(std::fs::read_to_string(&path)?, expected);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))?;
        assert!(private_file::matches(&path, &expected).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        let linked = directory.path().join("symlink.service");
        symlink(&path, &linked)?;
        assert!(private_file::create_or_verify(&linked, &expected).is_err());
        let hard_link = directory.path().join("hard-link.service");
        std::fs::hard_link(&path, &hard_link)?;
        assert!(private_file::create_or_verify(&hard_link, &expected).is_err());
        Ok(())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    #[tokio::test]
    async fn unsupported_platform_does_not_install() {
        assert!(
            install_user_service(Path::new("/agent"), Path::new("/state"))
                .await
                .is_err()
        );
        assert!(
            uninstall_user_service(Path::new("/agent"), Path::new("/state"))
                .await
                .is_err()
        );
    }
}
