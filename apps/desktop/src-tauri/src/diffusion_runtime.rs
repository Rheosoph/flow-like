//! Register the private diffusion runtime installed by the desktop build.

#[cfg(not(target_os = "macos"))]
use tauri::Manager;

pub fn configure(_app: &tauri::App) -> flow_like_types::Result<()> {
    // Tauri signs the static macOS server as a sidecar and installs it beside
    // the application executable. Windows and Linux keep private libraries
    // together with their server in the resource directory.
    #[cfg(target_os = "macos")]
    let bundled = std::env::current_exe()?
        .parent()
        .ok_or_else(|| flow_like_types::anyhow!("Application executable has no parent directory"))?
        .join("sd-server");
    #[cfg(not(target_os = "macos"))]
    let bundled = _app
        .path()
        .resource_dir()?
        .join("runtimes/stablediffusion")
        .join(if cfg!(windows) {
            "sd-server.exe"
        } else {
            "sd-server"
        });

    #[cfg(debug_assertions)]
    let bundled = {
        let (platform, executable) = match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => ("mac-arm", "sd-server-aarch64-apple-darwin"),
            ("macos", "x86_64") => ("mac-intel", "sd-server-x86_64-apple-darwin"),
            ("linux", "x86_64") => ("linux-x64", "sd-server"),
            ("windows", "x86_64") => ("win-x64", "sd-server.exe"),
            _ => ("unsupported", "sd-server"),
        };
        let development = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("runtimes/stablediffusion")
            .join(platform)
            .join(executable);
        if development.is_file() {
            development
        } else {
            bundled
        }
    };

    // An endpoint or an administrator's FLOW_LIKE_SD_SERVER override also works
    // on desktop targets for which no runtime is bundled.
    if bundled.is_file() {
        flow_like::flow_like_model_provider::stablediffusion::set_runtime_path(bundled)?;
    }
    Ok(())
}
