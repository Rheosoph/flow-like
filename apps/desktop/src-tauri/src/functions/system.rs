use serde::{Deserialize, Serialize};
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::collections::HashSet;
#[cfg(any(target_os = "windows", target_os = "linux"))]
use std::path::Path;
use std::process::Command;

use flow_like::utils::device::{get_cores, get_ram};

use crate::functions::TauriFunctionError;

#[derive(Serialize, Deserialize, Debug, Clone, Hash, PartialEq, Eq)]
pub struct SystemInfo {
    ram: u64,
    cores: u64,
}

#[tauri::command(async)]
pub fn get_system_info() -> SystemInfo {
    let ram = get_ram().unwrap_or(0);
    let cores = get_cores().unwrap_or(0);

    SystemInfo { ram, cores }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppEntry {
    pub name: String,
    pub path: String,
    pub is_default: bool,
}

#[cfg(any(target_os = "windows", target_os = "linux"))]
fn run_command(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).to_string())
}

fn run_status(program: &str, args: &[&str]) -> bool {
    Command::new(program)
        .args(args)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn parse_tsv_apps(stdout: &str) -> Vec<AppEntry> {
    stdout
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() != 3 {
                return None;
            }
            let name = parts[0].trim();
            let path = parts[1].trim();
            let is_default = parts[2].trim() == "true";
            if name.is_empty() || path.is_empty() {
                return None;
            }
            Some(AppEntry {
                name: name.to_string(),
                path: path.to_string(),
                is_default,
            })
        })
        .collect()
}

#[cfg(target_os = "macos")]
fn get_apps_for_path(file_path: &str) -> Vec<AppEntry> {
    let script = r#"
ObjC.import('AppKit');
ObjC.import('Foundation');
const env = $.NSProcessInfo.processInfo.environment;
const filePath = ObjC.unwrap(env.objectForKey('FILE_PATH'));
const ws = $.NSWorkspace.sharedWorkspace;
const url = $.NSURL.fileURLWithPath(filePath);
const apps = ws.URLsForApplicationsToOpenURL(url);
const defaultApp = ws.URLForApplicationToOpenURL(url);
for (let i = 0; i < apps.count; i++) {
  const app = apps.objectAtIndex(i);
  const appPath = ObjC.unwrap(app.path);
  const appName = ObjC.unwrap(app.lastPathComponent.stringByDeletingPathExtension);
  const isDefault = defaultApp && ObjC.unwrap(defaultApp.path) === appPath;
  console.log(`${appName}\t${appPath}\t${isDefault ? 'true' : 'false'}`);
}
"#;

    let stdout = match Command::new("osascript")
        .args(["-l", "JavaScript", "-e", script])
        .env("FILE_PATH", file_path)
        .output()
    {
        Ok(output) if output.status.success() => {
            String::from_utf8_lossy(&output.stdout).to_string()
        }
        _ => return Vec::new(),
    };

    parse_tsv_apps(&stdout)
}

#[cfg(target_os = "windows")]
fn get_apps_for_path(file_path: &str) -> Vec<AppEntry> {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!(".{ext}"));

    let Some(extension) = extension else {
        return Vec::new();
    };

    let assoc_line = run_command("cmd", &["/C", "assoc", &extension]).and_then(|s| {
        s.lines()
            .find(|line| line.contains('='))
            .map(|line| line.to_string())
    });

    let prog_id = assoc_line
        .and_then(|line| {
            line.split_once('=')
                .map(|(_, prog)| prog.trim().to_string())
        })
        .unwrap_or_default();

    let default_command = if prog_id.is_empty() {
        None
    } else {
        run_command("cmd", &["/C", "ftype", &prog_id])
    };

    let mut apps = Vec::new();
    let mut seen = HashSet::new();

    if let Some(command) = default_command {
        let app_path = command
            .split('=')
            .nth(1)
            .unwrap_or_default()
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        if !app_path.is_empty() {
            let name = Path::new(&app_path)
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("Default")
                .to_string();
            seen.insert(app_path.clone());
            apps.push(AppEntry {
                name,
                path: app_path,
                is_default: true,
            });
        }
    }

    let open_with_key = format!(
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\{}\OpenWithList",
        extension
    );
    if let Some(output) = run_command("reg", &["query", &open_with_key]) {
        for line in output.lines() {
            if !line.contains("REG_SZ") {
                continue;
            }
            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(exe_name) = parts.last() {
                let exe_name = exe_name.trim();
                if exe_name.is_empty() || !seen.insert(exe_name.to_string()) {
                    continue;
                }
                apps.push(AppEntry {
                    name: exe_name.trim_end_matches(".exe").to_string(),
                    path: exe_name.to_string(),
                    is_default: false,
                });
            }
        }
    }

    apps
}

#[cfg(target_os = "linux")]
fn resolve_desktop_exec(desktop_id: &str) -> String {
    let search_paths = [
        format!("/usr/share/applications/{desktop_id}"),
        format!("/usr/local/share/applications/{desktop_id}"),
        format!(
            "{}/.local/share/applications/{desktop_id}",
            std::env::var("HOME").unwrap_or_default()
        ),
    ];

    for path in search_paths {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        for line in content.lines() {
            if let Some(exec) = line.strip_prefix("Exec=") {
                let command = exec
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                if !command.is_empty() {
                    return command;
                }
            }
        }
    }

    desktop_id.to_string()
}

#[cfg(target_os = "linux")]
fn get_apps_for_path(file_path: &str) -> Vec<AppEntry> {
    let mime_type = match run_command("xdg-mime", &["query", "filetype", file_path]) {
        Some(output) => output.trim().to_string(),
        None => return Vec::new(),
    };

    if mime_type.is_empty() {
        return Vec::new();
    }

    let output = match run_command("gio", &["mime", &mime_type]) {
        Some(output) => output,
        None => return Vec::new(),
    };

    let mut default_app = String::new();
    let mut apps = Vec::new();
    let mut seen = HashSet::new();
    let mut in_registered = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Default application for") {
            if let Some((_, app)) = rest.split_once(':') {
                default_app = app.trim().trim_end_matches('.').to_string();
            }
            continue;
        }

        if trimmed.starts_with("Registered applications:") {
            in_registered = true;
            continue;
        }

        if in_registered {
            if trimmed.is_empty() || trimmed.ends_with(':') {
                in_registered = false;
                continue;
            }

            let desktop_id = trimmed.trim_end_matches(';').to_string();
            if desktop_id.is_empty() || !seen.insert(desktop_id.clone()) {
                continue;
            }

            let command = resolve_desktop_exec(&desktop_id);
            let name = desktop_id.trim_end_matches(".desktop").to_string();
            let is_default = !default_app.is_empty() && desktop_id == default_app;
            apps.push(AppEntry {
                name,
                path: command,
                is_default,
            });
        }
    }

    if apps.is_empty() && !default_app.is_empty() {
        let command = resolve_desktop_exec(&default_app);
        apps.push(AppEntry {
            name: default_app.trim_end_matches(".desktop").to_string(),
            path: command,
            is_default: true,
        });
    }

    apps
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn get_apps_for_path(_file_path: &str) -> Vec<AppEntry> {
    Vec::new()
}

#[tauri::command(async)]
pub fn list_apps_for_file(file_path: String) -> Vec<AppEntry> {
    get_apps_for_path(&file_path)
}

#[tauri::command(async)]
pub fn open_file_with_app(file_path: String, app_path: String) -> Result<(), TauriFunctionError> {
    #[cfg(target_os = "macos")]
    {
        if run_status("open", &["-a", &app_path, &file_path]) {
            return Ok(());
        }
        return Err(TauriFunctionError::new(
            "Failed to open file with selected app",
        ));
    }

    #[cfg(target_os = "windows")]
    {
        if run_status("cmd", &["/C", "start", "", &app_path, &file_path]) {
            return Ok(());
        }
        return Err(TauriFunctionError::new(
            "Failed to open file with selected app",
        ));
    }

    #[cfg(target_os = "linux")]
    {
        if run_status(&app_path, &[&file_path]) {
            return Ok(());
        }
        return Err(TauriFunctionError::new(
            "Failed to open file with selected app",
        ));
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = (file_path, app_path);
        Err(TauriFunctionError::new(
            "Open with selected app is not supported on this platform",
        ))
    }
}
