// Inspired by the Tauri project implementation
use std::path::PathBuf;
use std::process::Command as StdCommand;

use flow_like_types::tokio::process::{self, Command};

pub fn executable_path() -> Option<PathBuf> {
    let path = std::env::current_exe().ok()?;
    let parent = path.parent()?;
    Some(parent.to_path_buf())
}

fn side_car_path(command: &PathBuf) -> flow_like_types::Result<PathBuf> {
    let executable =
        executable_path().ok_or(flow_like_types::anyhow!("Could not get executable path"))?;
    #[cfg(windows)]
    return Ok(executable.join(&command).with_extension("exe"));
    #[cfg(not(windows))]
    return Ok(executable.join(command));
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn set_library_path(cmd: &mut StdCommand, binary_path: &std::path::Path) {
    if let Some(dir) = binary_path.parent() {
        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", dir);
        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", dir);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn set_library_path(_: &mut StdCommand, _: &std::path::Path) {}

#[cfg(windows)]
fn hide_sidecar_window(cmd: &mut StdCommand) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x08000000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_sidecar_window(_: &mut StdCommand) {}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn set_library_path_async(cmd: &mut Command, binary_path: &std::path::Path) {
    if let Some(dir) = binary_path.parent() {
        #[cfg(target_os = "macos")]
        cmd.env("DYLD_LIBRARY_PATH", dir);
        #[cfg(target_os = "linux")]
        cmd.env("LD_LIBRARY_PATH", dir);
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn set_library_path_async(_: &mut Command, _: &std::path::Path) {}

/// Creates a sidecar command to run a script or executable.
/// If `with_bash` is true, it will run the command using `bash`. Important for some Systems and binaries
/// Otherwise, it will run the command directly.
/// Returns a `flow_like_types::Result<StdCommand>`
/// which can be used to execute the command asynchronously.
pub async fn sidecar(
    command: &PathBuf,
    with_bash: Option<bool>,
) -> flow_like_types::Result<StdCommand> {
    let path = side_car_path(command)?;
    println!("Sidecar path: {:?}", path);

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
pub async fn async_sidecar(command: &PathBuf) -> flow_like_types::Result<Command> {
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

// ==================== IDEAS ====================
// - Sidecar BIT function
