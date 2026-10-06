// Inspired by the Tauri project implementation
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command as StdCommand, ExitStatus};

use flow_like_types::tokio::process::{self, Command};

pub fn executable_path() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?;
    let parent = path.parent()?;
    Some(parent.to_path_buf())
}

fn sidecar_file(dir: &Path, name: &Path) -> PathBuf {
    let path = dir.join(name);
    if cfg!(windows) {
        path.with_extension("exe")
    } else {
        path
    }
}

fn side_car_path(command: &Path) -> flow_like_types::Result<PathBuf> {
    let executable =
        executable_path().ok_or(flow_like_types::anyhow!("Could not get executable path"))?;
    Ok(sidecar_file(&executable, command))
}

/// The variable through which a sidecar finds the libraries it ships with.
#[cfg(target_os = "macos")]
const LIBRARY_PATH_VAR: Option<&str> = Some("DYLD_LIBRARY_PATH");
#[cfg(target_os = "linux")]
const LIBRARY_PATH_VAR: Option<&str> = Some("LD_LIBRARY_PATH");
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
const LIBRARY_PATH_VAR: Option<&str> = None;

fn set_library_path(cmd: &mut StdCommand, binary_path: &Path) {
    if let (Some(var), Some(dir)) = (LIBRARY_PATH_VAR, binary_path.parent()) {
        cmd.env(var, dir);
    }
}

fn set_library_path_async(cmd: &mut Command, binary_path: &Path) {
    if let (Some(var), Some(dir)) = (LIBRARY_PATH_VAR, binary_path.parent()) {
        cmd.env(var, dir);
    }
}

#[cfg(windows)]
fn hide_sidecar_window(cmd: &mut StdCommand) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_sidecar_window(_: &mut StdCommand) {}

/// Creates a sidecar command to run a script or executable.
/// If `with_bash` is true, it will run the command using `bash`. Important for some Systems and binaries
/// Otherwise, it will run the command directly.
/// Returns a `flow_like_types::Result<StdCommand>`
/// which can be used to execute the command asynchronously.
pub async fn sidecar(
    command: &Path,
    with_bash: Option<bool>,
) -> flow_like_types::Result<StdCommand> {
    let path = side_car_path(command)?;
    tracing::debug!(?path, "Sidecar path");

    if !path.exists() {
        return Err(flow_like_types::anyhow!(
            "Sidecar not found at path: {:?}",
            path
        ));
    }

    if !path.is_file() {
        return Err(flow_like_types::anyhow!(
            "Sidecar is not a file: {:?}",
            path
        ));
    }

    let with_bash = with_bash.unwrap_or(false);

    if with_bash {
        #[cfg(target_os = "linux")]
        {
            let mut sidecar = StdCommand::new("bash");
            sidecar.arg(&path);
            set_library_path(&mut sidecar, &path);
            return Ok(sidecar);
        }
    }

    let mut sidecar = StdCommand::new(&path);
    set_library_path(&mut sidecar, &path);
    hide_sidecar_window(&mut sidecar);
    Ok(sidecar)
}

//
pub async fn async_sidecar(command: &Path) -> flow_like_types::Result<Command> {
    let path = side_car_path(command)?;

    if !path.exists() {
        return Err(flow_like_types::anyhow!(
            "Sidecar not found at path: {:?}",
            path
        ));
    }

    if !path.is_file() {
        return Err(flow_like_types::anyhow!(
            "Sidecar is not a file: {:?}",
            path
        ));
    }

    #[cfg(not(target_os = "linux"))]
    {
        let mut sidecar = process::Command::new(&path);
        set_library_path_async(&mut sidecar, &path);
        Ok(sidecar)
    }

    #[cfg(target_os = "linux")]
    {
        let mut sidecar = process::Command::new("bash");
        sidecar.arg(&path);
        set_library_path_async(&mut sidecar, &path);
        Ok(sidecar)
    }
}

/// A llama-server install: the binaries the desktop bundles next to the app, or a runtime pack.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LlamaServerRuntime {
    pub entrypoint: PathBuf,
    /// On the library search path of every start.
    pub library_dir: PathBuf,
    /// Libraries a host may lack, such as libstdc++ and the Vulkan loader. A start adds them only
    /// after one without them failed to load, so GPU drivers keep the host's C++ runtime.
    pub fallback_dir: Option<PathBuf>,
}

impl LlamaServerRuntime {
    pub fn command(&self, with_fallback: bool) -> StdCommand {
        let mut command = StdCommand::new(&self.entrypoint);
        if let Some(var) = LIBRARY_PATH_VAR {
            command.env(var, self.library_path(with_fallback));
        }
        hide_sidecar_window(&mut command);
        command
    }

    fn library_path(&self, with_fallback: bool) -> OsString {
        let mut path = self.library_dir.clone().into_os_string();
        if let Some(fallback) = self.fallback_dir.as_ref().filter(|_| with_fallback) {
            path.push(":");
            path.push(fallback);
        }
        path
    }
}

/// The MLX helper: next to the app on the desktop, or in a runtime pack's `bin/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MlxServiceRuntime {
    pub entrypoint: PathBuf,
    /// Holds the helper's SwiftPM resource bundles with its Metal kernels. The helper finds them
    /// on its own, beside itself or in its app bundle, so a host only has to keep them readable.
    pub resources_dir: PathBuf,
}

impl MlxServiceRuntime {
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.entrypoint);
        set_library_path_async(&mut command, &self.entrypoint);
        command
    }
}

/// Where this host's local model runtimes are. Hosts that install runtime packs point it at them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeLocator {
    pub llama_server: Option<LlamaServerRuntime>,
    pub mlx_service: Option<MlxServiceRuntime>,
}

impl RuntimeLocator {
    /// The desktop layout: every runtime next to the current executable.
    pub fn beside_current_exe() -> Self {
        executable_path().map_or(
            Self {
                llama_server: None,
                mlx_service: None,
            },
            |dir| Self::bundled_in(&dir),
        )
    }

    pub fn bundled_in(dir: &Path) -> Self {
        Self {
            llama_server: Some(LlamaServerRuntime {
                entrypoint: sidecar_file(dir, Path::new("llama-server")),
                library_dir: dir.to_path_buf(),
                fallback_dir: None,
            }),
            mlx_service: Some(MlxServiceRuntime {
                entrypoint: sidecar_file(dir, Path::new("flow-like-mlx-service")),
                resources_dir: dir.to_path_buf(),
            }),
        }
    }

    pub fn installed_llama_server(&self) -> flow_like_types::Result<&LlamaServerRuntime> {
        const NAME: &str = "llama-server";
        let runtime = self
            .llama_server
            .as_ref()
            .ok_or_else(|| unavailable(NAME))?;
        require_executable(NAME, &runtime.entrypoint)?;
        Ok(runtime)
    }

    pub fn installed_mlx_service(&self) -> flow_like_types::Result<&MlxServiceRuntime> {
        const NAME: &str = "The MLX service";
        let runtime = self.mlx_service.as_ref().ok_or_else(|| unavailable(NAME))?;
        require_executable(NAME, &runtime.entrypoint)?;
        Ok(runtime)
    }
}

fn unavailable(name: &str) -> flow_like_types::Error {
    flow_like_types::anyhow!("{name} is not available on this host")
}

fn require_executable(name: &str, path: &Path) -> flow_like_types::Result<()> {
    if is_executable_file(path) {
        return Ok(());
    }
    Err(flow_like_types::anyhow!(
        "{name} is not installed: no executable at {}",
        path.display()
    ))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        metadata.is_file()
    }
}

/// glibc's loader exits with 127 when a binary cannot load, as with a missing library or a
/// libstdc++ older than the one it was built against.
pub fn failed_to_load(status: &ExitStatus) -> bool {
    cfg!(target_os = "linux") && status.code() == Some(127)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library_path_of(command: &StdCommand) -> Option<OsString> {
        let var = LIBRARY_PATH_VAR?;
        command
            .get_envs()
            .find(|(key, _)| *key == var)
            .and_then(|(_, value)| value.map(ToOwned::to_owned))
    }

    #[test]
    fn bundled_runtimes_sit_in_the_directory() {
        let dir = Path::new("/opt/flow-like");
        let locator = RuntimeLocator::bundled_in(dir);
        let llama_server = locator.llama_server.unwrap();
        let mlx_service = locator.mlx_service.unwrap();

        assert_eq!(
            llama_server.entrypoint,
            sidecar_file(dir, Path::new("llama-server"))
        );
        assert_eq!(llama_server.library_dir, dir);
        assert_eq!(llama_server.fallback_dir, None);
        assert_eq!(
            mlx_service.entrypoint,
            sidecar_file(dir, Path::new("flow-like-mlx-service"))
        );
        assert_eq!(mlx_service.resources_dir, dir);
    }

    #[test]
    fn the_default_locator_looks_next_to_the_current_executable() {
        let locator = RuntimeLocator::beside_current_exe();
        let dir = executable_path().unwrap();
        assert_eq!(locator, RuntimeLocator::bundled_in(&dir));
        assert_eq!(
            locator.llama_server.unwrap().entrypoint,
            side_car_path(Path::new("llama-server")).unwrap()
        );
    }

    #[test]
    fn a_missing_runtime_names_what_is_missing() {
        let locator = RuntimeLocator {
            llama_server: None,
            mlx_service: None,
        };
        let error = locator.installed_llama_server().unwrap_err().to_string();
        assert_eq!(error, "llama-server is not available on this host");

        let dir = tempfile::tempdir().unwrap();
        let locator = RuntimeLocator::bundled_in(dir.path());
        let error = locator.installed_mlx_service().unwrap_err().to_string();
        assert!(error.contains(&dir.path().display().to_string()), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn only_existing_executable_files_count_as_installed() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let locator = RuntimeLocator::bundled_in(dir.path());
        let entrypoint = locator.llama_server.clone().unwrap().entrypoint;
        assert!(locator.installed_llama_server().is_err());

        std::fs::create_dir(&entrypoint).unwrap();
        assert!(locator.installed_llama_server().is_err());
        std::fs::remove_dir(&entrypoint).unwrap();

        std::fs::write(&entrypoint, "#!/bin/sh\n").unwrap();
        assert!(locator.installed_llama_server().is_err());

        std::fs::set_permissions(&entrypoint, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(
            locator.installed_llama_server().unwrap().entrypoint,
            entrypoint
        );
        assert!(locator.installed_mlx_service().is_err());
    }

    #[test]
    fn the_fallback_directory_joins_the_library_path_only_on_request() {
        let runtime = LlamaServerRuntime {
            entrypoint: PathBuf::from("/packs/llamacpp/llama-server"),
            library_dir: PathBuf::from("/packs/llamacpp"),
            fallback_dir: Some(PathBuf::from("/packs/llamacpp/fallback")),
        };

        let first = runtime.command(false);
        let retry = runtime.command(true);

        assert_eq!(first.get_program(), runtime.entrypoint.as_os_str());
        if LIBRARY_PATH_VAR.is_some() {
            assert_eq!(
                library_path_of(&first),
                Some(OsString::from("/packs/llamacpp"))
            );
            assert_eq!(
                library_path_of(&retry),
                Some(OsString::from("/packs/llamacpp:/packs/llamacpp/fallback"))
            );
        } else {
            assert_eq!(library_path_of(&first), None);
        }
    }

    #[test]
    fn the_bundled_library_path_is_the_executable_directory_as_before() {
        let dir = Path::new("/Applications/Flow-Like.app/Contents/MacOS");
        let runtime = RuntimeLocator::bundled_in(dir).llama_server.unwrap();
        let mut legacy = StdCommand::new(&runtime.entrypoint);
        set_library_path(&mut legacy, &runtime.entrypoint);

        assert_eq!(
            library_path_of(&runtime.command(true)),
            library_path_of(&legacy)
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_loader_exits_on_linux_count_as_load_failures() {
        use std::os::unix::process::ExitStatusExt;

        let loader_exit = ExitStatus::from_raw(127 << 8);
        assert_eq!(failed_to_load(&loader_exit), cfg!(target_os = "linux"));
        assert!(!failed_to_load(&ExitStatus::from_raw(1 << 8)));
        assert!(!failed_to_load(&ExitStatus::from_raw(6)));
    }
}
