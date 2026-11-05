//! Which display a bot's computer uses and, on Linux, the private X server it runs on.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use flow_like_types::{Result, anyhow};
use tokio::process::{Child, Command};

pub const FIRST_DISPLAY: u32 = 90;
pub const SCREEN_SIZE: &str = "1280x800x24";
const SOCKET_WAIT: Duration = Duration::from_secs(10);
const WINDOW_MANAGERS: &[&str] = &["openbox", "fluxbox", "matchbox-window-manager", "xfwm4"];

static TAKEN: LazyLock<Mutex<HashSet<u32>>> = LazyLock::new(Mutex::default);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DisplayPolicy {
    Auto,
    Virtual,
    Session,
    Off,
}

impl DisplayPolicy {
    /// `FLOW_LIKE_PLAYGROUND=virtual|session|off|auto`.
    pub fn from_env() -> Self {
        Self::parse(std::env::var("FLOW_LIKE_PLAYGROUND").ok().as_deref())
    }

    pub fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("virtual") => Self::Virtual,
            Some("session") => Self::Session,
            Some("off" | "none") => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// What the machine offers the bots' computer tools.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum DisplayKind {
    Virtual,
    Session,
    None(String),
}

#[derive(Clone, Copy, Debug)]
pub struct Facts {
    pub linux: bool,
    pub xvfb: bool,
    pub x_display: bool,
}

impl Facts {
    pub fn detect() -> Self {
        Self {
            linux: cfg!(target_os = "linux"),
            xvfb: find_on_path("Xvfb").is_some(),
            x_display: std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty()),
        }
    }
}

const INSTALL_XVFB: &str =
    "computer use needs a display: install xvfb (for example `apt install xvfb openbox xauth`)";

pub fn resolve_display(policy: DisplayPolicy, facts: Facts) -> DisplayKind {
    let session = || {
        if !facts.linux || facts.x_display {
            DisplayKind::Session
        } else {
            DisplayKind::None(INSTALL_XVFB.into())
        }
    };
    let virtual_display = || match (facts.linux, facts.xvfb) {
        (true, true) => DisplayKind::Virtual,
        (true, false) => DisplayKind::None(INSTALL_XVFB.into()),
        (false, _) => DisplayKind::None(
            "computer use here runs on the logged-in screen; virtual displays need Linux with xvfb"
                .into(),
        ),
    };
    match policy {
        DisplayPolicy::Off => {
            DisplayKind::None("computer use is switched off (FLOW_LIKE_PLAYGROUND=off)".into())
        }
        DisplayPolicy::Virtual => virtual_display(),
        DisplayPolicy::Session => session(),
        DisplayPolicy::Auto if facts.linux && facts.xvfb => DisplayKind::Virtual,
        DisplayPolicy::Auto => session(),
    }
}

/// The first display number from [`FIRST_DISPLAY`] whose socket (`X{N}` in /tmp/.X11-unix) or
/// lock file (`.X{N}-lock` in /tmp) is not listed.
pub fn pick_display(sockets: &[String], tmp: &[String], reserved: &HashSet<u32>) -> u32 {
    (FIRST_DISPLAY..)
        .find(|n| {
            !reserved.contains(n)
                && !sockets.iter().any(|name| *name == format!("X{n}"))
                && !tmp.iter().any(|name| *name == format!(".X{n}-lock"))
        })
        .unwrap_or(FIRST_DISPLAY)
}

fn listing(dir: &str) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

fn claim_display() -> u32 {
    let mut reserved = TAKEN.lock().unwrap_or_else(|e| e.into_inner());
    let number = pick_display(&listing("/tmp/.X11-unix"), &listing("/tmp"), &reserved);
    reserved.insert(number);
    number
}

pub fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

pub fn first_available<'a>(names: &[&'a str], exists: impl Fn(&str) -> bool) -> Option<&'a str> {
    names.iter().copied().find(|name| exists(name))
}

pub fn xvfb_args(display: u32, authority: Option<&Path>) -> Vec<String> {
    let mut args = vec![
        format!(":{display}"),
        "-screen".into(),
        "0".into(),
        SCREEN_SIZE.into(),
        "-nolisten".into(),
        "tcp".into(),
    ];
    if let Some(path) = authority {
        args.push("-auth".into());
        args.push(path.display().to_string());
    }
    args
}

pub fn xauth_args(authority: &Path, display: u32, key_hex: &str) -> Vec<String> {
    vec![
        "-f".into(),
        authority.display().to_string(),
        "add".into(),
        format!(":{display}"),
        ".".into(),
        key_hex.into(),
    ]
}

/// The environment a worker needs to use display `:N` and never the host's Wayland session.
pub fn worker_env(display: u32, authority: Option<&Path>) -> Vec<(String, String)> {
    let mut env = vec![
        ("DISPLAY".to_string(), format!(":{display}")),
        ("XDG_SESSION_TYPE".to_string(), "x11".to_string()),
    ];
    if let Some(path) = authority {
        env.push(("XAUTHORITY".to_string(), path.display().to_string()));
    }
    env
}

fn random_key() -> Result<String> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| anyhow!("Failed to read /dev/urandom for the X authority key: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map(|_| ())
}

#[cfg(not(unix))]
fn private_file(path: &Path) -> std::io::Result<()> {
    std::fs::File::create(path).map(|_| ())
}

fn background(program: &Path, args: &[String], envs: &[(String, String)]) -> Result<Child> {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(envs.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    command
        .spawn()
        .map_err(|e| anyhow!("Failed to start {}: {e}", program.display()))
}

/// A private X server (and window manager) for one bot; both end when this drops.
pub struct VirtualDisplay {
    pub number: u32,
    pub authority: Option<PathBuf>,
    _xvfb: Child,
    _window_manager: Option<Child>,
}

impl VirtualDisplay {
    pub fn env(&self) -> Vec<(String, String)> {
        worker_env(self.number, self.authority.as_deref())
    }

    pub async fn start(dir: &Path) -> Result<Self> {
        let xvfb = find_on_path("Xvfb").ok_or_else(|| anyhow!(INSTALL_XVFB))?;
        std::fs::create_dir_all(dir).map_err(|e| {
            anyhow!(
                "Failed to create {} for the virtual display: {e}",
                dir.display()
            )
        })?;
        let number = claim_display();
        match Self::start_numbered(&xvfb, dir, number).await {
            Ok(display) => Ok(display),
            Err(error) => {
                TAKEN
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&number);
                Err(error)
            }
        }
    }

    async fn start_numbered(xvfb: &Path, dir: &Path, number: u32) -> Result<Self> {
        let authority = Self::authority(dir, number).await;
        let mut server = background(xvfb, &xvfb_args(number, authority.as_deref()), &[])?;
        Self::wait_for_socket(&mut server, number).await?;
        let envs = worker_env(number, authority.as_deref());
        let window_manager = first_available(WINDOW_MANAGERS, |name| find_on_path(name).is_some())
            .and_then(find_on_path)
            .and_then(|path| background(&path, &[], &envs).ok());
        Ok(Self {
            number,
            authority,
            _xvfb: server,
            _window_manager: window_manager,
        })
    }

    async fn authority(dir: &Path, number: u32) -> Option<PathBuf> {
        let xauth = find_on_path("xauth")?;
        let path = dir.join(format!("Xauthority-{number}"));
        private_file(&path).ok()?;
        let key = random_key().ok()?;
        let status = Command::new(xauth)
            .args(xauth_args(&path, number, &key))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .ok()?;
        status.success().then_some(path)
    }

    async fn wait_for_socket(server: &mut Child, number: u32) -> Result<()> {
        let socket = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
        let deadline = tokio::time::Instant::now() + SOCKET_WAIT;
        while tokio::time::Instant::now() < deadline {
            if socket.exists() {
                return Ok(());
            }
            if let Ok(Some(status)) = server.try_wait() {
                return Err(anyhow!("Xvfb :{number} exited at startup ({status})"));
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Err(anyhow!(
            "Xvfb :{number} did not open {} within {} s",
            socket.display(),
            SOCKET_WAIT.as_secs()
        ))
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        TAKEN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.number);
        if let Some(path) = &self.authority {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn displays_start_at_ninety_and_skip_sockets_locks_and_reservations() {
        let none = HashSet::new();
        assert_eq!(pick_display(&[], &[], &none), 90);
        assert_eq!(
            pick_display(&names(&["X90", "X92"]), &names(&[".X91-lock"]), &none),
            93
        );
        assert_eq!(
            pick_display(&names(&["X0", "X1"]), &names(&["other"]), &none),
            90
        );
        assert_eq!(pick_display(&[], &[], &HashSet::from([90, 91])), 92);
    }

    #[test]
    fn the_xvfb_command_is_private_and_fixed_size() {
        let expected = [":91", "-screen", "0", "1280x800x24", "-nolisten", "tcp"];
        assert_eq!(xvfb_args(91, None), expected);
        let with_auth = xvfb_args(91, Some(Path::new("/run/x/auth")));
        assert_eq!(&with_auth[with_auth.len() - 2..], ["-auth", "/run/x/auth"]);
        assert_eq!(
            xauth_args(Path::new("/run/x/auth"), 91, "00ff"),
            ["-f", "/run/x/auth", "add", ":91", ".", "00ff"]
        );
    }

    #[test]
    fn the_worker_env_targets_the_display_and_never_wayland() {
        let env = worker_env(91, Some(Path::new("/run/x/auth")));
        assert!(env.contains(&("DISPLAY".into(), ":91".into())));
        assert!(env.contains(&("XDG_SESSION_TYPE".into(), "x11".into())));
        assert!(env.contains(&("XAUTHORITY".into(), "/run/x/auth".into())));
        assert!(!env.iter().any(|(k, _)| k == "WAYLAND_DISPLAY"));
    }

    #[test]
    fn the_first_installed_window_manager_wins() {
        let installed = |name: &str| name == "fluxbox" || name == "xfwm4";
        assert_eq!(first_available(WINDOW_MANAGERS, installed), Some("fluxbox"));
        assert_eq!(first_available(WINDOW_MANAGERS, |_| false), None);
    }

    fn facts(linux: bool, xvfb: bool, x_display: bool) -> Facts {
        Facts {
            linux,
            xvfb,
            x_display,
        }
    }

    #[test]
    fn auto_prefers_a_virtual_display_on_linux_and_the_session_elsewhere() {
        use DisplayPolicy::Auto;
        assert_eq!(
            resolve_display(Auto, facts(true, true, true)),
            DisplayKind::Virtual
        );
        assert_eq!(
            resolve_display(Auto, facts(true, false, true)),
            DisplayKind::Session
        );
        assert_eq!(
            resolve_display(Auto, facts(false, false, false)),
            DisplayKind::Session
        );
        let DisplayKind::None(message) = resolve_display(Auto, facts(true, false, false)) else {
            panic!("a headless Linux box without xvfb has no computer");
        };
        assert!(message.contains("install xvfb"), "{message}");
    }

    #[test]
    fn overrides_win_and_say_why_when_they_cannot_be_met() {
        assert_eq!(
            resolve_display(DisplayPolicy::Session, facts(true, true, true)),
            DisplayKind::Session
        );
        assert!(matches!(
            resolve_display(DisplayPolicy::Off, facts(true, true, true)),
            DisplayKind::None(_)
        ));
        assert!(matches!(
            resolve_display(DisplayPolicy::Virtual, facts(false, false, false)),
            DisplayKind::None(_)
        ));
        assert!(
            matches!(resolve_display(DisplayPolicy::Virtual, facts(true, false, true)), DisplayKind::None(m) if m.contains("xvfb"))
        );
        assert_eq!(
            DisplayPolicy::parse(Some(" Virtual ")),
            DisplayPolicy::Virtual
        );
        assert_eq!(DisplayPolicy::parse(Some("nonsense")), DisplayPolicy::Auto);
        assert_eq!(DisplayPolicy::parse(None), DisplayPolicy::Auto);
    }
}
