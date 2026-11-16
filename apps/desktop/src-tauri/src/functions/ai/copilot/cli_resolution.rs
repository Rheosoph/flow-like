//! Platform-aware CLI discovery and executable search paths.

use super::backend_types::FlowPilotAgentBackendKind;
pub(super) use flow_like::flow_like_model_provider::llm::claude_code::claude_ide_extension_binaries;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CliResolutionSource {
    EnvOverride,
    BundledResource,
    CodexStandalone,
    CodexNpmPackage,
    Path,
    IdeExtensionFallback,
}

#[derive(Debug, Clone)]
pub(super) struct CliResolution {
    pub(super) executable: PathBuf,
    pub(super) path_dirs: Vec<PathBuf>,
    pub(super) source: CliResolutionSource,
}

impl CliResolution {
    pub(super) fn new(executable: PathBuf, source: CliResolutionSource) -> Self {
        Self {
            executable,
            path_dirs: Vec::new(),
            source,
        }
    }

    pub(super) fn with_path_dirs(
        executable: PathBuf,
        source: CliResolutionSource,
        path_dirs: Vec<PathBuf>,
    ) -> Self {
        Self {
            executable,
            path_dirs,
            source,
        }
    }
}

pub(super) fn codex_binary_name() -> &'static str {
    if cfg!(windows) { "codex.exe" } else { "codex" }
}

pub(super) fn codex_target() -> Option<(&'static str, &'static str)> {
    let target = if cfg!(target_os = "linux") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-unknown-linux-musl"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-musl"
        } else {
            return None;
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-apple-darwin"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            return None;
        }
    } else if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-pc-windows-msvc"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-pc-windows-msvc"
        } else {
            return None;
        }
    } else {
        return None;
    };

    let package = match target {
        "x86_64-unknown-linux-musl" => "@openai/codex-linux-x64",
        "aarch64-unknown-linux-musl" => "@openai/codex-linux-arm64",
        "x86_64-apple-darwin" => "@openai/codex-darwin-x64",
        "aarch64-apple-darwin" => "@openai/codex-darwin-arm64",
        "x86_64-pc-windows-msvc" => "@openai/codex-win32-x64",
        "aarch64-pc-windows-msvc" => "@openai/codex-win32-arm64",
        _ => return None,
    };

    Some((target, package))
}

/// Collect extra bin directories that are typically absent from a bundled-app
/// PATH (Homebrew, nvm, volta, fnm, mise, pnpm, bun, npm-global, …).
pub(super) fn extra_bin_dirs() -> Vec<std::path::PathBuf> {
    use std::path::PathBuf;

    let Some(home) = dirs_next::home_dir() else {
        return vec![];
    };

    let mut dirs: Vec<PathBuf> = vec![
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".volta/bin"),
        home.join(".bun/bin"),
        home.join(".local/share/pnpm"),
        home.join(".local/bin"),
        home.join(".asdf/shims"),
    ];

    // Homebrew's documented Linux installation uses the linuxbrew home rather
    // than either macOS prefix above. A per-user Linuxbrew install is also common.
    #[cfg(target_os = "linux")]
    dirs.extend([
        PathBuf::from("/home/linuxbrew/.linuxbrew/bin"),
        home.join(".linuxbrew/bin"),
    ]);

    // GUI apps on Windows may not see newly added user PATH entries until the
    // next login. Probe the standard npm and WinGet links directly, plus the
    // GitHub Copilot CLI location documented by GitHub's SDK guide.
    #[cfg(windows)]
    {
        if let Some(data_dir) = dirs_next::data_dir() {
            dirs.push(data_dir.join("npm"));
        }
        if let Some(local_data_dir) = dirs_next::data_local_dir() {
            dirs.push(local_data_dir.join("Microsoft/WinGet/Links"));
            dirs.push(local_data_dir.join("pnpm"));
        }
        for variable in ["ProgramFiles", "ProgramW6432"] {
            if let Ok(program_files) = std::env::var(variable) {
                let trimmed = program_files.trim();
                if !trimmed.is_empty() {
                    dirs.push(PathBuf::from(trimmed).join("GitHub"));
                }
            }
        }
    }

    // nvm – scan all installed node versions
    let nvm_dir = std::env::var("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".nvm"));
    if let Ok(entries) = std::fs::read_dir(nvm_dir.join("versions/node")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("bin"));
        }
    }

    // fnm
    if let Ok(entries) = std::fs::read_dir(home.join(".local/share/fnm/node-versions")) {
        for entry in entries.flatten() {
            dirs.push(entry.path().join("installation/bin"));
        }
    }

    // mise / rtx node shims
    dirs.push(home.join(".local/share/mise/shims"));

    // npm global prefix variants
    dirs.push(home.join(".npm-global/bin"));
    dirs.push(home.join(".npm-packages/bin"));
    dirs.push(home.join(".npm/bin"));

    // Claude Code local install (e.g. after `claude migrate-installer`)
    let claude_home = std::env::var("CLAUDE_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".claude"));
    dirs.push(claude_home.join("local"));

    dirs.extend(codex_standalone_visible_dirs(&home));

    dirs.sort();
    dirs.dedup();
    dirs
}

fn codex_standalone_visible_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(install_dir) = std::env::var("CODEX_INSTALL_DIR") {
        let trimmed = install_dir.trim();
        if !trimmed.is_empty() {
            dirs.push(PathBuf::from(trimmed));
        }
    }

    let codex_home = std::env::var("CODEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join(".codex"));
    dirs.push(codex_home.join("packages/standalone/current/bin"));
    dirs.push(codex_home.join("packages/standalone/current"));

    #[cfg(not(windows))]
    dirs.push(home.join(".local/bin"));

    #[cfg(windows)]
    if let Some(local_app_data) = dirs_next::data_local_dir() {
        dirs.push(local_app_data.join("Programs/OpenAI/Codex/bin"));
    }

    dirs
}

pub(super) fn codex_ide_extension_candidate_dirs(home: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for root in [
        home.join(".vscode/extensions"),
        home.join(".vscode-insiders/extensions"),
        home.join(".vscode-oss/extensions"),
        home.join(".cursor/extensions"),
        home.join(".windsurf/extensions"),
    ] {
        collect_codex_cli_dirs(&root, 5, &mut dirs);
    }

    dirs.sort_by(|a, b| b.cmp(a));
    dirs.dedup();
    dirs
}

fn collect_codex_cli_dirs(root: &std::path::Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    if depth == 0 || !root.is_dir() {
        return;
    }

    let codex_executable = root.join(codex_binary_name());
    if is_executable_file(&codex_executable) {
        out.push(root.to_path_buf());
        let helper_path = root.join("codex-path");
        if helper_path.is_dir() {
            out.push(helper_path);
        }
        return;
    }

    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_codex_cli_dirs(&path, depth - 1, out);
        }
    }
}

/// Resolve the Copilot CLI path, searching beyond the (possibly limited) bundled-app PATH.
///
/// On macOS/Linux, apps launched from Finder/Dock inherit a minimal PATH that
/// excludes npm-global, nvm, volta, mise, and Homebrew directories. This
/// function probes those common locations so that prod builds can find the CLI.
fn find_cli_path(kind: FlowPilotAgentBackendKind) -> Option<std::path::PathBuf> {
    find_cli_resolution(kind, None).map(|resolution| resolution.executable)
}

pub(super) fn find_cli_resolution(
    kind: FlowPilotAgentBackendKind,
    app_handle: Option<&AppHandle>,
) -> Option<CliResolution> {
    if let Ok(p) = std::env::var(kind.env_path_var()) {
        let trimmed = p.trim();
        if trimmed.is_empty() {
            return None;
        }

        let path = PathBuf::from(trimmed);
        if is_executable_file(&path) {
            return Some(CliResolution::new(path, CliResolutionSource::EnvOverride));
        }

        if path.is_dir() {
            let candidate = path.join(kind.cli_name());
            if is_executable_file(&candidate) {
                return Some(CliResolution::new(
                    candidate,
                    CliResolutionSource::EnvOverride,
                ));
            }
        }

        if path.components().count() == 1
            && let Some(found) = find_executable_in_path(trimmed, &augmented_path())
        {
            return Some(CliResolution::new(found, CliResolutionSource::EnvOverride));
        }

        // An explicit override is authoritative. Falling through to another
        // installation would hide the invalid configured path and make the
        // remediation point at a different executable than the one in use.
        return None;
    }

    if kind == FlowPilotAgentBackendKind::Codex {
        if let Some(resolution) = find_bundled_codex_cli(app_handle) {
            return Some(resolution);
        }
        if let Some(resolution) = find_codex_standalone_cli() {
            return Some(resolution);
        }
        if let Some(resolution) = find_codex_npm_package_cli(app_handle) {
            return Some(resolution);
        }
    }

    if let Some(found) = find_executable_in_path(kind.cli_name(), &augmented_path()) {
        return Some(CliResolution::new(found, CliResolutionSource::Path));
    }

    if kind == FlowPilotAgentBackendKind::Codex
        && let Some(home) = dirs_next::home_dir()
    {
        for dir in codex_ide_extension_candidate_dirs(&home) {
            if let Some(candidate) = find_codex_executable_in_dir(&dir) {
                let mut path_dirs = Vec::new();
                let helper_path = dir.join("codex-path");
                if helper_path.is_dir() {
                    path_dirs.push(helper_path);
                }
                return Some(CliResolution::with_path_dirs(
                    candidate,
                    CliResolutionSource::IdeExtensionFallback,
                    path_dirs,
                ));
            }
        }
    }

    if kind == FlowPilotAgentBackendKind::ClaudeCode
        && let Some(home) = dirs_next::home_dir()
        && let Some(candidate) = claude_ide_extension_binaries(&home).into_iter().next()
    {
        return Some(CliResolution::new(
            candidate,
            CliResolutionSource::IdeExtensionFallback,
        ));
    }

    None
}

fn find_bundled_codex_cli(app_handle: Option<&AppHandle>) -> Option<CliResolution> {
    let mut roots = Vec::new();
    if let Some(app_handle) = app_handle
        && let Ok(resource_dir) = app_handle.path().resource_dir()
    {
        roots.extend([
            resource_dir.clone(),
            resource_dir.join("codex"),
            resource_dir.join("binaries"),
            resource_dir.join("bin"),
            resource_dir.join("node_modules"),
        ]);
    }

    for root in roots {
        if let Some(resolution) =
            find_codex_packaged_cli_under_root(&root, CliResolutionSource::BundledResource)
        {
            return Some(resolution);
        }
        if let Some(candidate) = find_codex_executable_in_dir(&root) {
            return Some(CliResolution::new(
                candidate,
                CliResolutionSource::BundledResource,
            ));
        }
    }

    None
}

fn find_codex_standalone_cli() -> Option<CliResolution> {
    let home = dirs_next::home_dir()?;
    for dir in codex_standalone_visible_dirs(&home) {
        if let Some(candidate) = find_codex_executable_in_dir(&dir) {
            return Some(CliResolution::new(
                candidate,
                CliResolutionSource::CodexStandalone,
            ));
        }
    }
    None
}

fn find_codex_npm_package_cli(app_handle: Option<&AppHandle>) -> Option<CliResolution> {
    for root in codex_npm_search_roots(app_handle) {
        if let Some(resolution) =
            find_codex_packaged_cli_under_root(&root, CliResolutionSource::CodexNpmPackage)
        {
            return Some(resolution);
        }
    }
    None
}

fn codex_npm_search_roots(app_handle: Option<&AppHandle>) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(app_handle) = app_handle
        && let Ok(resource_dir) = app_handle.path().resource_dir()
    {
        roots.extend([
            resource_dir.join("node_modules"),
            resource_dir.join("codex/node_modules"),
        ]);
    }

    if let Some(home) = dirs_next::home_dir() {
        roots.extend([
            home.join(".npm-global/lib/node_modules"),
            home.join(".npm-packages/lib/node_modules"),
            home.join(".bun/install/global/node_modules"),
        ]);

        let nvm_dir = std::env::var("NVM_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home.join(".nvm"));
        if let Ok(entries) = std::fs::read_dir(nvm_dir.join("versions/node")) {
            for entry in entries.flatten() {
                roots.push(entry.path().join("lib/node_modules"));
            }
        }

        if let Ok(entries) = std::fs::read_dir(home.join(".local/share/fnm/node-versions")) {
            for entry in entries.flatten() {
                roots.push(entry.path().join("installation/lib/node_modules"));
            }
        }
    }

    #[cfg(windows)]
    if let Some(data_dir) = dirs_next::data_dir() {
        roots.push(data_dir.join("npm/node_modules"));
    }

    roots.sort();
    roots.dedup();
    roots
}

pub(super) fn find_codex_packaged_cli_under_root(
    root: &Path,
    source: CliResolutionSource,
) -> Option<CliResolution> {
    let (target, platform_package) = codex_target()?;
    let package_leaf = platform_package
        .rsplit('/')
        .next()
        .unwrap_or(platform_package);
    let package_roots = [
        root.join(platform_package),
        root.join("@openai").join(package_leaf),
        root.join("@openai/codex/node_modules")
            .join(platform_package),
        root.join("@openai/codex/node_modules/@openai")
            .join(package_leaf),
        root.join("@openai/codex"),
        root.to_path_buf(),
    ];

    for package_root in package_roots {
        if let Some(resolution) = resolve_codex_native_package(&package_root, target, source) {
            return Some(resolution);
        }
    }

    None
}

fn resolve_codex_native_package(
    package_root: &Path,
    target: &str,
    source: CliResolutionSource,
) -> Option<CliResolution> {
    let target_root = package_root.join("vendor").join(target);
    let package_binary = target_root.join("bin").join(codex_binary_name());
    if is_executable_file(&package_binary) && target_root.join("codex-package.json").is_file() {
        let path_dirs = [target_root.join("codex-path")]
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect();
        return Some(CliResolution::with_path_dirs(
            package_binary,
            source,
            path_dirs,
        ));
    }

    let legacy_binary = target_root.join("codex").join(codex_binary_name());
    if is_executable_file(&legacy_binary) {
        let path_dirs = [target_root.join("path")]
            .into_iter()
            .filter(|dir| dir.is_dir())
            .collect();
        return Some(CliResolution::with_path_dirs(
            legacy_binary,
            source,
            path_dirs,
        ));
    }

    None
}

fn find_codex_executable_in_dir(dir: &Path) -> Option<PathBuf> {
    for file_name in codex_executable_file_names() {
        let candidate = dir.join(file_name);
        if is_executable_file(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn codex_executable_file_names() -> Vec<String> {
    let mut names = vec![codex_binary_name().to_string()];
    if let Some((target, _)) = codex_target() {
        names.push(if cfg!(windows) {
            format!("codex-{target}.exe")
        } else {
            format!("codex-{target}")
        });
    }
    names
}

pub(super) fn find_copilot_cli_path() -> Option<std::path::PathBuf> {
    find_cli_path(FlowPilotAgentBackendKind::GithubCopilot)
}

/// Build an augmented PATH that prepends the extra bin directories to the
/// current PATH so that the spawned copilot CLI process (a Node.js script)
/// can locate `node` and other tools even in production builds.
pub(super) fn augmented_path() -> String {
    augmented_path_with_dirs(&[])
}

pub(super) fn augmented_path_with_dirs(prefix_dirs: &[PathBuf]) -> String {
    let mut entries: Vec<PathBuf> = prefix_dirs
        .iter()
        .cloned()
        .chain(extra_bin_dirs())
        .filter(|d| d.exists())
        .collect();

    let current = std::env::var("PATH").unwrap_or_default();
    entries.extend(std::env::split_paths(&current));

    std::env::join_paths(entries)
        .unwrap_or_else(|_| current.into())
        .to_string_lossy()
        .into_owned()
}

pub(super) fn find_executable_in_path(name: &str, path_value: &str) -> Option<std::path::PathBuf> {
    for dir in std::env::split_paths(path_value) {
        for file_name in executable_file_names(name) {
            let candidate = dir.join(file_name);
            if is_executable_file(&candidate) {
                return Some(candidate);
            }
        }
    }

    None
}

fn executable_file_names(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        let path = std::path::Path::new(name);
        if path.extension().is_some() {
            return vec![name.to_string()];
        }

        let pathext =
            std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
        let mut names = vec![name.to_string()];
        for ext in pathext.split(';').filter(|ext| !ext.trim().is_empty()) {
            names.push(format!("{name}{}", ext.to_ascii_lowercase()));
            names.push(format!("{name}{}", ext.to_ascii_uppercase()));
        }
        names.sort();
        names.dedup();
        names
    }

    #[cfg(not(windows))]
    {
        vec![name.to_string()]
    }
}

fn is_executable_file(path: &std::path::Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }

    #[cfg(not(unix))]
    {
        true
    }
}

pub(super) fn external_agent_cli_resolution_failure(kind: FlowPilotAgentBackendKind) -> String {
    if let Ok(override_value) = std::env::var(kind.env_path_var()) {
        let trimmed = override_value.trim();
        if trimmed.is_empty() {
            return format!(
                "{} is set but empty, so the {} CLI cannot be resolved.",
                kind.env_path_var(),
                kind.label()
            );
        }

        let path = PathBuf::from(trimmed);
        if path.is_file() {
            return format!(
                "{} points to {}, but that file is not executable.",
                kind.env_path_var(),
                path.display()
            );
        }
        if path.is_dir() {
            return format!(
                "{} points to {}, but that directory does not contain an executable named {}.",
                kind.env_path_var(),
                path.display(),
                kind.cli_name()
            );
        }
        if path.components().count() > 1 {
            return format!(
                "{} points to {}, but that executable does not exist.",
                kind.env_path_var(),
                path.display()
            );
        }
        return format!(
            "{} names `{trimmed}`, but that executable was not found on Flow-Like's PATH.",
            kind.env_path_var()
        );
    }

    format!("{} CLI was not found.", kind.label())
}
