use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriRegistryState, TauriSettingsState, TauriWasmEngineState},
};
use dashmap::DashMap;
use flow_like::flow::node::{Node, NodeLogic, NodePermission, NodeWasm};
use flow_like_wasm::abi::{WasmExecutionInput, WasmExecutionResult, WasmNodeDefinition};
use flow_like_wasm::host_functions::ModelContext;
use flow_like_wasm::manifest::PackageManifest;
use flow_like_wasm::{WasmEngine, WasmNodeLogic, WasmSecurityConfig, build_node_from_definition};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::SystemTime;
use tauri::{AppHandle, Emitter};

static INSPECTION_CACHE: LazyLock<DashMap<String, (SystemTime, PackageInspection)>> =
    LazyLock::new(DashMap::new);

fn wasm_file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).ok()?.modified().ok()
}

pub fn clear_inspection_cache() {
    INSPECTION_CACHE.clear();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperProject {
    pub id: String,
    pub path: String,
    pub language: String,
    pub name: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperProjectStore {
    pub projects: Vec<DeveloperProject>,
    #[serde(default)]
    pub preferred_editor: String,
}

impl Default for DeveloperProjectStore {
    fn default() -> Self {
        Self {
            projects: Vec::new(),
            preferred_editor: String::from("vscode"),
        }
    }
}

fn store_path(user_dir: &Path) -> PathBuf {
    user_dir.join("developer-projects.json")
}

fn detect_project_language(project_path: &Path) -> Option<String> {
    let entries = std::fs::read_dir(project_path).ok()?;
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();

    let lower: std::collections::HashSet<String> = names.iter().map(|n| n.to_lowercase()).collect();

    let src_lower = collect_dir_names(&project_path.join("src"));

    if lower.iter().any(|f| f.ends_with(".gr")) || src_lower.iter().any(|f| f.ends_with(".gr")) {
        return Some("grain".into());
    }
    if lower.iter().any(|f| f.ends_with(".nimble"))
        || lower.contains("nim.cfg")
        || lower.contains("config.nims")
    {
        return Some("nim".into());
    }
    if lower
        .iter()
        .any(|f| f.ends_with(".lua") || f.ends_with(".rockspec"))
        || lower.contains(".luacheckrc")
        || src_lower.iter().any(|f| f.ends_with(".lua"))
    {
        return Some("lua".into());
    }
    if lower.contains("package.swift") {
        return Some("swift".into());
    }
    if lower.contains("build.zig") || lower.contains("build.zig.zon") {
        return Some("zig".into());
    }
    if lower.contains("go.mod") {
        return Some("go".into());
    }
    if lower.contains("build.gradle.kts") || lower.contains("settings.gradle.kts") {
        return Some("kotlin".into());
    }
    if lower.iter().any(|f| f.ends_with(".csproj")) {
        return Some("csharp".into());
    }
    if lower.contains("pom.xml") {
        return Some("java".into());
    }
    if lower.contains("cmakelists.txt")
        || lower
            .iter()
            .any(|f| f.ends_with(".cpp") || f.ends_with(".cc"))
    {
        return Some("cpp".into());
    }
    if lower.contains("pyproject.toml") || lower.contains("requirements.txt") {
        return Some("python".into());
    }
    if lower.contains("asconfig.json") {
        return Some("assemblyscript".into());
    }
    if lower.contains("package.json") {
        return Some("typescript".into());
    }
    if lower.contains("cargo.toml") {
        return Some("rust".into());
    }
    None
}

fn collect_dir_names(dir: &Path) -> std::collections::HashSet<String> {
    std::fs::read_dir(dir)
        .ok()
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .map(|n| n.to_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

fn load_store(user_dir: &Path) -> DeveloperProjectStore {
    let path = store_path(user_dir);
    if path.exists() {
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    } else {
        DeveloperProjectStore::default()
    }
}

fn save_store(user_dir: &Path, store: &DeveloperProjectStore) -> Result<(), TauriFunctionError> {
    let path = store_path(user_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    }
    let json =
        serde_json::to_string_pretty(store).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    std::fs::write(&path, json).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn developer_list_projects(
    app_handle: AppHandle,
) -> Result<Vec<DeveloperProject>, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let settings_guard = settings.lock().await;
    let mut store = load_store(&settings_guard.user_dir);

    let mut changed = false;
    for project in &mut store.projects {
        let path = Path::new(&project.path);
        if let Some(detected) = detect_project_language(path)
            && project.language != detected
        {
            project.language = detected;
            changed = true;
        }
    }
    if changed {
        let _ = save_store(&settings_guard.user_dir, &store);
    }

    Ok(store.projects)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AddProjectInput {
    pub path: String,
    pub language: String,
    pub name: String,
}

#[tauri::command]
pub async fn developer_add_project(
    app_handle: AppHandle,
    input: AddProjectInput,
) -> Result<DeveloperProject, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let settings_guard = settings.lock().await;
    let mut store = load_store(&settings_guard.user_dir);

    if store.projects.iter().any(|p| p.path == input.path) {
        return Err(TauriFunctionError::new("Project already registered"));
    }

    let project = DeveloperProject {
        id: format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
        ),
        path: input.path.clone(),
        language: detect_project_language(Path::new(&input.path)).unwrap_or(input.language),
        name: input.name,
        created_at: chrono::Utc::now().to_rfc3339(),
    };

    store.projects.push(project.clone());
    save_store(&settings_guard.user_dir, &store)?;
    Ok(project)
}

#[tauri::command]
pub async fn developer_remove_project(
    app_handle: AppHandle,
    project_id: String,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let settings_guard = settings.lock().await;
    let mut store = load_store(&settings_guard.user_dir);

    let removed_project = store.projects.iter().find(|p| p.id == project_id).cloned();
    store.projects.retain(|p| p.id != project_id);
    save_store(&settings_guard.user_dir, &store)?;
    drop(settings_guard);

    if let Some(project) = removed_project {
        let project_path = PathBuf::from(&project.path);
        if let Ok(wasm_path) = find_wasm_file(&project_path)
            && let Some(manifest) = load_manifest_for_registration(&project_path, &wasm_path)
            && let Ok(client) = TauriRegistryState::get_client(&app_handle).await
        {
            let _ = client.unregister_local_package(&manifest.id).await;
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn developer_list_local_files(
    _app_handle: AppHandle,
    path: String,
) -> Result<Vec<String>, TauriFunctionError> {
    let dir = PathBuf::from(&path);
    if !dir.is_dir() {
        return Err(TauriFunctionError::new("Path is not a directory"));
    }
    let entries = std::fs::read_dir(&dir).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    Ok(names)
}

#[tauri::command]
pub async fn developer_get_manifest(
    _app_handle: AppHandle,
    project_path: String,
) -> Result<serde_json::Value, TauriFunctionError> {
    let manifest_path = Path::new(&project_path).join("flow-like.toml");
    if !manifest_path.exists() {
        return Err(TauriFunctionError::new(
            "No flow-like.toml found in project",
        ));
    }
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let value: toml::Value =
        toml::from_str(&content).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let json = serde_json::to_value(value).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    Ok(json)
}

#[tauri::command]
pub async fn developer_save_manifest(
    _app_handle: AppHandle,
    project_path: String,
    manifest: serde_json::Value,
) -> Result<(), TauriFunctionError> {
    let manifest_path = Path::new(&project_path).join("flow-like.toml");
    let toml_value: toml::Value = serde_json::from_value(manifest)
        .map_err(|e| TauriFunctionError::new(&format!("Invalid manifest: {}", e)))?;
    let toml_str =
        toml::to_string_pretty(&toml_value).map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    std::fs::write(&manifest_path, toml_str)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    Ok(())
}

#[tauri::command]
pub async fn developer_open_in_editor(
    app_handle: AppHandle,
    project_path: String,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let settings_guard = settings.lock().await;
    let store = load_store(&settings_guard.user_dir);
    let editor = &store.preferred_editor;
    drop(settings_guard);

    let cmd = match editor.as_str() {
        "vscode" => "code",
        "cursor" => "cursor",
        "zed" => "zed",
        "idea" | "jetbrains" => "idea",
        "fleet" => "fleet",
        "sublime" => "subl",
        "vim" | "nvim" => "nvim",
        other => {
            return Err(TauriFunctionError::new(&format!(
                "Unknown editor '{}'. Please select a supported editor in settings.",
                other
            )));
        }
    };

    std::process::Command::new(cmd)
        .arg(&project_path)
        .spawn()
        .map_err(|e| {
            TauriFunctionError::new(&format!(
                "Failed to open editor '{}': {}. Make sure it is installed and available in PATH.",
                cmd, e
            ))
        })?;

    Ok(())
}

#[tauri::command]
pub async fn developer_get_settings(
    app_handle: AppHandle,
) -> Result<DeveloperSettings, TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let settings_guard = settings.lock().await;
    let store = load_store(&settings_guard.user_dir);
    Ok(DeveloperSettings {
        preferred_editor: store.preferred_editor,
        dev_mode: settings_guard.dev_mode,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeveloperSettings {
    pub preferred_editor: String,
    pub dev_mode: bool,
}

#[tauri::command]
pub async fn developer_save_settings(
    app_handle: AppHandle,
    dev_settings: DeveloperSettings,
) -> Result<(), TauriFunctionError> {
    let settings = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let mut settings_guard = settings.lock().await;
    settings_guard.dev_mode = dev_settings.dev_mode;

    let mut store = load_store(&settings_guard.user_dir);
    store.preferred_editor = dev_settings.preferred_editor;
    save_store(&settings_guard.user_dir, &store)?;

    crate::settings::Settings::serialize(&mut settings_guard);
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldInput {
    pub target_dir: String,
    pub language: String,
    pub project_name: String,
}

#[tauri::command]
pub async fn developer_scaffold_project(
    app_handle: AppHandle,
    input: ScaffoldInput,
) -> Result<DeveloperProject, TauriFunctionError> {
    let template_dir = match input.language.as_str() {
        "rust" => "wasm-node-rust",
        "python" => "wasm-node-python",
        "assemblyscript" | "as" => "wasm-node-assemblyscript",
        "go" => "wasm-node-go",
        "cpp" | "c" | "c++" => "wasm-node-cpp",
        "csharp" | "c#" => "wasm-node-csharp",
        "kotlin" | "kt" => "wasm-node-kotlin",
        "zig" => "wasm-node-zig",
        "nim" => "wasm-node-nim",
        "lua" => "wasm-node-lua",
        "swift" => "wasm-node-swift",
        "java" => "wasm-node-java",
        "grain" => "wasm-node-grain",
        "moonbit" => "wasm-node-moonbit",
        other => {
            return Err(TauriFunctionError::new(&format!(
                "Unsupported language: {}",
                other
            )));
        }
    };

    let api_url = format!(
        "https://api.github.com/repos/Rheosoph/flow-like/contents/templates/{}?ref=dev",
        template_dir
    );

    let target = PathBuf::from(&input.target_dir);
    if target.exists()
        && std::fs::read_dir(&target)
            .map(|mut d| d.next().is_some())
            .unwrap_or(false)
    {
        return Err(TauriFunctionError::new("Target directory is not empty"));
    }
    std::fs::create_dir_all(&target).map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    download_github_dir(&api_url, &target).await?;

    patch_manifest(&target, &input.project_name)?;

    let add_input = AddProjectInput {
        path: target.to_string_lossy().to_string(),
        language: input.language,
        name: input.project_name,
    };
    developer_add_project(app_handle, add_input).await
}

async fn download_github_dir(api_url: &str, target: &Path) -> Result<(), TauriFunctionError> {
    let client = flow_like_types::reqwest::Client::builder()
        .user_agent("flow-like-desktop")
        .build()
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    let resp = client
        .get(api_url)
        .header("Accept", "application/vnd.github.v3+json")
        .send()
        .await
        .map_err(|e| TauriFunctionError::new(&format!("GitHub API request failed: {}", e)))?;

    if !resp.status().is_success() {
        return Err(TauriFunctionError::new(&format!(
            "GitHub API returned {}",
            resp.status()
        )));
    }

    let entries: Vec<serde_json::Value> = resp
        .json()
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to parse GitHub response: {}", e)))?;

    for entry in &entries {
        let entry_type = entry["type"].as_str().unwrap_or("");
        let name = entry["name"].as_str().unwrap_or("");

        if name.starts_with('.') || name == "build" || name == "__pycache__" {
            continue;
        }

        let safe_name = match std::path::Path::new(name).file_name() {
            Some(n) => n.to_owned(),
            None => continue,
        };

        match entry_type {
            "file" => {
                let download_url = entry["download_url"]
                    .as_str()
                    .ok_or_else(|| TauriFunctionError::new("Missing download_url"))?;

                let content = client
                    .get(download_url)
                    .send()
                    .await
                    .map_err(|e| TauriFunctionError::new(&e.to_string()))?
                    .bytes()
                    .await
                    .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

                let file_path = target.join(&safe_name);
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
                }
                std::fs::write(&file_path, &content)
                    .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
            }
            "dir" => {
                let sub_url = entry["url"]
                    .as_str()
                    .ok_or_else(|| TauriFunctionError::new("Missing dir url"))?;
                let sub_dir = target.join(&safe_name);
                std::fs::create_dir_all(&sub_dir)
                    .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
                Box::pin(download_github_dir(sub_url, &sub_dir)).await?;
            }
            _ => {}
        }
    }

    Ok(())
}

fn patch_manifest(target: &Path, project_name: &str) -> Result<(), TauriFunctionError> {
    let manifest_path = target.join("flow-like.toml");
    if !manifest_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let mut doc = content
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    if let Some(pkg) = doc.get_mut("package").and_then(|v| v.as_table_mut()) {
        let slug = project_name
            .to_lowercase()
            .replace(' ', "-")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-')
            .collect::<String>();
        pkg["name"] = toml_edit::value(project_name);
        pkg["id"] = toml_edit::value(format!("com.custom.{}", slug));
    }

    std::fs::write(&manifest_path, doc.to_string())
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    Ok(())
}

#[tauri::command]
pub async fn developer_inspect_node(
    app_handle: AppHandle,
    wasm_path: String,
) -> Result<Vec<WasmNodeDefinition>, TauriFunctionError> {
    let engine = TauriWasmEngineState::construct(&app_handle)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    tokio::spawn(async move {
        let path = PathBuf::from(&wasm_path);
        if !path.exists() {
            return Err(TauriFunctionError::new("WASM file not found"));
        }

        let loaded = engine
            .load_auto_from_file(&path)
            .await
            .map_err(|e| TauriFunctionError::new(&format!("Failed to load WASM module: {}", e)))?;

        let security = WasmSecurityConfig::permissive();
        let mut instance = loaded.instantiate(&engine, security).await.map_err(|e| {
            TauriFunctionError::new(&format!("Failed to instantiate module: {}", e))
        })?;

        instance
            .call_get_nodes()
            .await
            .map_err(|e| TauriFunctionError::new(&format!("Failed to get node definitions: {}", e)))
    })
    .await
    .map_err(|e| TauriFunctionError::new(&format!("Task panicked: {}", e)))?
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageInspection {
    pub nodes: Vec<WasmNodeDefinition>,
    pub manifest: Option<PackageManifest>,
    pub is_package: bool,
    pub wasm_path: String,
}

#[tauri::command]
pub async fn developer_inspect_package(
    app_handle: AppHandle,
    project_path: String,
) -> Result<PackageInspection, TauriFunctionError> {
    let engine = TauriWasmEngineState::construct(&app_handle)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    tokio::spawn(async move {
        let project = PathBuf::from(&project_path);
        let wasm_path = find_wasm_file(&project)?;

        if let Some(mtime) = wasm_file_mtime(&wasm_path) {
            let cache_key = wasm_path.to_string_lossy().to_string();
            if let Some(entry) = INSPECTION_CACHE.get(&cache_key) {
                let (cached_mtime, cached_result) = entry.value();
                if *cached_mtime == mtime {
                    return Ok(cached_result.clone());
                }
            }
        }

        let manifest = load_manifest_typed(&project).ok();
        let loaded = engine
            .load_auto_from_file(&wasm_path)
            .await
            .map_err(|e| TauriFunctionError::new(&format!("Failed to load WASM module: {}", e)))?;

        let security = WasmSecurityConfig::permissive();
        let mut instance = loaded.instantiate(&engine, security).await.map_err(|e| {
            TauriFunctionError::new(&format!("Failed to instantiate module: {}", e))
        })?;

        let is_package = instance.is_package();
        let nodes = instance.call_get_nodes().await.map_err(|e| {
            TauriFunctionError::new(&format!("Failed to get node definitions: {}", e))
        })?;

        let result = PackageInspection {
            nodes,
            manifest,
            is_package,
            wasm_path: wasm_path.to_string_lossy().to_string(),
        };

        if let Some(mtime) = wasm_file_mtime(&wasm_path) {
            let cache_key = wasm_path.to_string_lossy().to_string();
            INSPECTION_CACHE.insert(cache_key, (mtime, result.clone()));
        }

        Ok(result)
    })
    .await
    .map_err(|e| TauriFunctionError::new(&format!("Task panicked: {}", e)))?
}

#[tauri::command]
pub async fn developer_find_publish_wasm(
    project_path: String,
) -> Result<String, TauriFunctionError> {
    let project = PathBuf::from(&project_path);
    let wasm_path = find_wasm_for_publish(&project)?;
    Ok(wasm_path.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn developer_read_manifest(
    project_path: String,
) -> Result<PackageManifest, TauriFunctionError> {
    let project = PathBuf::from(&project_path);
    load_manifest_typed(&project)
}

fn load_manifest_typed(project_path: &Path) -> Result<PackageManifest, TauriFunctionError> {
    let manifest_path = project_path.join("flow-like.toml");
    let content = std::fs::read_to_string(&manifest_path)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    PackageManifest::from_toml(&content)
        .map_err(|e| TauriFunctionError::new(&format!("Invalid manifest: {}", e)))
}

fn find_wasm_file(project_path: &Path) -> Result<PathBuf, TauriFunctionError> {
    find_wasm_with_mode(project_path, WasmLookupMode::Debug)
}

fn find_wasm_for_publish(project_path: &Path) -> Result<PathBuf, TauriFunctionError> {
    find_wasm_with_mode(project_path, WasmLookupMode::Release)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WasmLookupMode {
    Debug,
    Release,
}

fn find_wasm_with_mode(
    project_path: &Path,
    mode: WasmLookupMode,
) -> Result<PathBuf, TauriFunctionError> {
    // 1. Check manifest wasm_path first
    if let Ok(manifest) = load_manifest_typed(project_path)
        && let Some(wasm_path) = &manifest.wasm_path
    {
        let p = project_path.join(wasm_path);
        if p.exists() {
            return Ok(p);
        }
    }

    // 2. Check Rust wasm32-wasip2 target outputs (most common for this project)
    let rust_release = find_rust_wasm(project_path, "release");
    let rust_debug = find_rust_wasm(project_path, "debug");

    match mode {
        WasmLookupMode::Debug => {
            // Prefer the newest build (debug or release)
            match (&rust_debug, &rust_release) {
                (Some(dbg), Some(rel)) => {
                    let dbg_time = wasm_file_mtime(dbg);
                    let rel_time = wasm_file_mtime(rel);
                    if dbg_time >= rel_time {
                        return Ok(dbg.clone());
                    }
                    return Ok(rel.clone());
                }
                (Some(dbg), None) => return Ok(dbg.clone()),
                (None, Some(rel)) => return Ok(rel.clone()),
                (None, None) => {}
            }
        }
        WasmLookupMode::Release => {
            if let Some(rel) = &rust_release {
                // If debug is newer than release, warn the user
                if let Some(dbg) = &rust_debug {
                    let dbg_time = wasm_file_mtime(dbg);
                    let rel_time = wasm_file_mtime(rel);
                    if dbg_time > rel_time {
                        return Err(TauriFunctionError::new(
                            "Release build is older than debug build. \
                             Run `cargo build --release` to rebuild before publishing.",
                        ));
                    }
                }
                return Ok(rel.clone());
            }
            // No release build found — fail for publish
            if rust_debug.is_some() {
                return Err(TauriFunctionError::new(
                    "Only a debug build was found. \
                     Run `cargo build --release` to create a release build before publishing.",
                ));
            }
        }
    }

    // 3. Check well-known output paths (non-Rust templates)
    let candidates = [
        "build/node.wasm",
        "build/release.wasm",
        "build/debug.wasm",
        "node.wasm",
        ".build/release/Node.wasm",
        "target/wasm/classes.wasm",
    ];
    for candidate in &candidates {
        let p = project_path.join(candidate);
        if p.exists() {
            return Ok(p);
        }
    }

    // 4. Recursively search for .wasm files, skipping build tooling dirs
    let skip_dirs: &[&str] = &[
        "node_modules",
        ".venv",
        "__pycache__",
        ".git",
        "deps",
        "examples",
        ".zig-cache",
        "gradle",
        ".gradle",
        "obj",
        "wasm-sdk-rust",
        "wasm-sdk-go",
        "wasm-sdk-cpp",
        "wasm-sdk-kotlin",
        "wasm-sdk-zig",
        "wasm-sdk-assemblyscript",
        "wasm-sdk-typescript",
        "wasm-sdk-python",
        "wasm-sdk-csharp",
        "wasm-sdk-nim",
        "wasm-sdk-grain",
        "wasm-sdk-moonbit",
        "wasm-sdk-lua",
        "wasm-sdk-swift",
        "wasm-sdk-java",
    ];
    if let Some(wasm) = find_wasm_recursive(project_path, project_path, skip_dirs, 0, mode) {
        return Ok(wasm);
    }

    let rust_dir = project_path.join("target").join("wasm32-wasip2");
    let hint = if rust_dir.is_dir() {
        format!(
            "No .wasm file found under {}. The directory exists but contains no wasm binaries at the top level of debug/ or release/.",
            rust_dir.display()
        )
    } else {
        format!(
            "No built .wasm file found in '{}'. Build your project first.",
            project_path.display()
        )
    };
    Err(TauriFunctionError::new(&hint))
}

/// Find a Rust-built WASM binary under {target_dir}/wasm32-wasip2/{profile}/
/// Uses `cargo metadata` to resolve the correct target directory, which handles
/// workspaces, CARGO_TARGET_DIR, and .cargo/config.toml overrides.
fn find_rust_wasm(project_path: &Path, profile: &str) -> Option<PathBuf> {
    let target_dir = cargo_target_dir(project_path)?;
    let dir = target_dir.join("wasm32-wasip2").join(profile);
    scan_dir_for_wasm(&dir)
}

/// Ask Cargo for the target directory of a project.
fn cargo_target_dir(project_path: &Path) -> Option<PathBuf> {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(project_path)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    json.get("target_directory")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
}

/// Scan a single directory for the newest .wasm file (non-recursive).
fn scan_dir_for_wasm(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<PathBuf> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "wasm") {
            match &best {
                None => best = Some(path),
                Some(cur) => {
                    if wasm_file_mtime(&path) > wasm_file_mtime(cur) {
                        best = Some(path);
                    }
                }
            }
        }
    }
    best
}

fn find_wasm_recursive(
    dir: &Path,
    project_root: &Path,
    skip_dirs: &[&str],
    depth: u32,
    mode: WasmLookupMode,
) -> Option<PathBuf> {
    if depth > 8 {
        return None;
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<PathBuf> = None;
    let mut subdirs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|ext| ext == "wasm") {
            let path_str = path
                .strip_prefix(project_root)
                .unwrap_or(&path)
                .to_string_lossy();
            if path_str.contains("/deps/") {
                continue;
            }
            // .NET publish/ outputs are not self-contained (need external ICU data etc.)
            if path_str.contains("/publish/")
                && path.file_name().is_some_and(|n| n == "dotnet.wasm")
            {
                continue;
            }
            best = pick_better(best, path, project_root, mode);
        } else if path.is_dir() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !skip_dirs.contains(&name_str.as_ref()) && !name_str.starts_with('.') {
                subdirs.push(path);
            }
        }
    }

    // Check all subdirectories and pick the overall best match
    for sub in subdirs {
        if let Some(found) = find_wasm_recursive(&sub, project_root, skip_dirs, depth + 1, mode) {
            best = pick_better(best, found, project_root, mode);
        }
    }
    best
}

fn pick_better(
    current: Option<PathBuf>,
    candidate: PathBuf,
    project_root: &Path,
    mode: WasmLookupMode,
) -> Option<PathBuf> {
    let cand_str = candidate
        .strip_prefix(project_root)
        .unwrap_or(&candidate)
        .to_string_lossy();
    let Some(cur) = current else {
        return Some(candidate);
    };
    let cur_str = cur
        .strip_prefix(project_root)
        .unwrap_or(&cur)
        .to_string_lossy();

    // AppBundle (single-file bundle) is always preferred
    if cur_str.contains("AppBundle") && !cand_str.contains("AppBundle") {
        return Some(cur);
    }
    if cand_str.contains("AppBundle") {
        return Some(candidate);
    }

    match mode {
        WasmLookupMode::Debug => {
            // For debug, prefer the newest file
            let cur_time = wasm_file_mtime(&cur);
            let cand_time = wasm_file_mtime(&candidate);
            if cand_time > cur_time {
                return Some(candidate);
            }
            Some(cur)
        }
        WasmLookupMode::Release => {
            // For release, prefer release/production paths
            let cur_is_release = cur_str.contains("release") || cur_str.contains("production");
            let cand_is_release = cand_str.contains("release") || cand_str.contains("production");
            if cand_is_release && !cur_is_release {
                return Some(candidate);
            }
            if cur_is_release && !cand_is_release {
                return Some(cur);
            }
            // Both same type — prefer newer
            let cur_time = wasm_file_mtime(&cur);
            let cand_time = wasm_file_mtime(&candidate);
            if cand_time > cur_time {
                return Some(candidate);
            }
            Some(cur)
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunNodeInput {
    pub wasm_path: String,
    pub inputs: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub node_name: String,
}

fn resolve_debug_node_definition(
    definitions: Vec<WasmNodeDefinition>,
    node_name: &str,
) -> Result<WasmNodeDefinition, TauriFunctionError> {
    if let Some(definition) = definitions.iter().find(|def| def.name == node_name) {
        return Ok(definition.clone());
    }

    if definitions.len() == 1 {
        return definitions
            .into_iter()
            .next()
            .ok_or_else(|| TauriFunctionError::new("No node definitions found"));
    }

    if node_name.is_empty() {
        return Err(TauriFunctionError::new(
            "Node name is required when the WASM package exports multiple nodes",
        ));
    }

    Err(TauriFunctionError::new(&format!(
        "Node '{}' not found in the selected WASM module",
        node_name
    )))
}

async fn current_registry_auth_token(app_handle: &AppHandle) -> Option<String> {
    let state = TauriRegistryState::construct(app_handle).await.ok()?;
    let guard = state.lock().await;
    guard
        .as_ref()
        .and_then(|client| client.auth_token().cloned())
}

#[tauri::command]
pub async fn developer_run_node(
    app_handle: AppHandle,
    input: RunNodeInput,
) -> Result<WasmExecutionResult, TauriFunctionError> {
    let engine = TauriWasmEngineState::construct(&app_handle)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let flow_like_state = TauriFlowLikeState::construct(&app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let execution_settings = TauriSettingsState::current_profile(&app_handle)
        .await
        .ok()
        .map(|profile| profile.execution_settings);
    let registry_auth_token = current_registry_auth_token(&app_handle).await;

    tokio::spawn(async move {
        let path = PathBuf::from(&input.wasm_path);
        if !path.exists() {
            return Err(TauriFunctionError::new("WASM file not found"));
        }

        let loaded = engine
            .load_auto_from_file(&path)
            .await
            .map_err(|e| TauriFunctionError::new(&format!("Failed to load WASM module: {}", e)))?;

        let mut inspect_instance = loaded
            .instantiate(&engine, WasmSecurityConfig::permissive())
            .await
            .map_err(|e| {
                TauriFunctionError::new(&format!("Failed to instantiate module: {}", e))
            })?;

        let definition = resolve_debug_node_definition(
            inspect_instance.call_get_nodes().await.map_err(|e| {
                TauriFunctionError::new(&format!("Failed to get node definitions: {}", e))
            })?,
            &input.node_name,
        )?;

        let mut instance = loaded
            .instantiate(
                &engine,
                WasmSecurityConfig::from_node_permissions(&definition.permissions),
            )
            .await
            .map_err(|e| {
                TauriFunctionError::new(&format!("Failed to instantiate module: {}", e))
            })?;

        if definition
            .permissions
            .iter()
            .any(|permission| matches!(permission, NodePermission::Models))
        {
            if let Some(settings) = execution_settings.clone() {
                flow_like_state
                    .model_factory
                    .lock()
                    .await
                    .set_execution_settings(settings);
            }

            instance.host_state_mut().model_context = Some(ModelContext {
                app_state: flow_like_state.clone(),
                token: registry_auth_token.clone(),
            });
        }

        let exec_input = WasmExecutionInput {
            inputs: input.inputs,
            node_id: "debug".to_string(),
            run_id: "debug".to_string(),
            app_id: "debug".to_string(),
            board_id: "debug".to_string(),
            user_id: "debug".to_string(),
            stream_state: false,
            log_level: 0,
            node_name: definition.name,
        };

        instance
            .call_run(&exec_input)
            .await
            .map_err(|e| TauriFunctionError::new(&format!("Node execution failed: {}", e)))
    })
    .await
    .map_err(|e| TauriFunctionError::new(&format!("Task panicked: {}", e)))?
}

async fn load_wasm_nodes_from_path(
    wasm_path: &Path,
    engine: Arc<WasmEngine>,
    manifest_package_id: Option<&str>,
) -> Result<Vec<(Node, Arc<dyn NodeLogic>)>, TauriFunctionError> {
    let loaded = engine
        .load_auto_from_file(wasm_path)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to load WASM module: {}", e)))?;

    let security = WasmSecurityConfig::permissive();
    let mut instance = loaded
        .instantiate(&engine, security.clone())
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to instantiate module: {}", e)))?;

    let definitions = instance
        .call_get_nodes()
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to get node definitions: {}", e)))?;

    Ok(definitions
        .into_iter()
        .map(|def| {
            let package_id = match manifest_package_id {
                Some(id) => id.to_string(),
                None => format!("local::{}", def.name),
            };
            let mut node = build_node_from_definition(&def);
            let permissions = node
                .wasm
                .as_ref()
                .map(|w| w.permissions.clone())
                .unwrap_or_default();
            node.wasm = Some(NodeWasm {
                package_id: package_id.clone(),
                permissions,
            });
            {
                use std::hash::{Hash, Hasher};
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                node.hash.hash(&mut hasher);
                package_id.hash(&mut hasher);
                node.hash = Some(hasher.finish());
            }
            let logic = Arc::new(
                WasmNodeLogic::from_loaded_with_target(
                    loaded.clone(),
                    engine.clone(),
                    security.clone(),
                    def,
                )
                .with_package_id(package_id),
            ) as Arc<dyn NodeLogic>;
            (node, logic)
        })
        .collect())
}

fn load_manifest_for_registration(
    project_path: &Path,
    wasm_path: &Path,
) -> Option<PackageManifest> {
    if let Ok(manifest) = load_manifest_typed(project_path) {
        return Some(manifest);
    }
    let file_name = wasm_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("local_package");
    Some(PackageManifest::new(
        &format!("local.{}", file_name),
        file_name,
        "0.0.0",
        "Locally loaded package",
    ))
}

async fn register_developer_package(
    app_handle: &AppHandle,
    wasm_path: &Path,
    manifest: PackageManifest,
) {
    if let Ok(client) = TauriRegistryState::get_client(app_handle).await
        && let Err(e) = client.register_local_package(wasm_path, manifest).await
    {
        tracing::debug!("Failed to register developer package in registry: {}", e);
    }
}

/// Re-registers all developer projects in the RegistryClient.
/// Called from `registry_init` to handle the timing gap where
/// `load_all_developer_nodes` runs before the RegistryClient is available.
pub async fn register_all_developer_packages(app_handle: &AppHandle) {
    let user_dir = match TauriSettingsState::construct(app_handle).await {
        Ok(settings) => {
            let guard = settings.lock().await;
            guard.user_dir.clone()
        }
        Err(e) => {
            tracing::debug!(
                "Failed to get settings for developer package registration: {}",
                e
            );
            return;
        }
    };

    let store = load_store(&user_dir);
    if store.projects.is_empty() {
        return;
    }

    for project in &store.projects {
        let project_path = PathBuf::from(&project.path);
        if let Ok(wasm_path) = find_wasm_file(&project_path)
            && let Some(manifest) = load_manifest_for_registration(&project_path, &wasm_path)
        {
            register_developer_package(app_handle, &wasm_path, manifest).await;
        }
    }

    tracing::info!(
        "Registered {} developer project(s) in registry",
        store.projects.len()
    );
}

#[tauri::command]
pub async fn developer_load_into_catalog(
    app_handle: AppHandle,
    project_path: String,
) -> Result<usize, TauriFunctionError> {
    let _ = app_handle.emit(
        "package-status",
        serde_json::json!({ "packageId": format!("dev:{}", project_path), "status": "compiling" }),
    );

    let engine = TauriWasmEngineState::construct(&app_handle)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
    let project = PathBuf::from(&project_path);
    let wasm_path = find_wasm_file(&project).inspect_err(|_e| {
        let _ = app_handle.emit(
            "package-status",
            serde_json::json!({ "packageId": format!("dev:{}", project_path), "status": "error" }),
        );
    })?;

    let manifest = load_manifest_for_registration(&project, &wasm_path);
    let manifest_id = manifest.as_ref().map(|m| m.id.as_str());

    let node_pairs = match load_wasm_nodes_from_path(&wasm_path, engine, manifest_id).await {
        Ok(pairs) => pairs,
        Err(e) => {
            let _ = app_handle.emit(
                "package-status",
                serde_json::json!({ "packageId": format!("dev:{}", project_path), "status": "error" }),
            );
            return Err(e);
        }
    };
    let count = node_pairs.len();

    if count > 0 {
        let flow_state = TauriFlowLikeState::construct(&app_handle)
            .await
            .map_err(|e| TauriFunctionError::new(&e.to_string()))?;
        let registry_guard = flow_state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        let mut inner = flow_like::state::FlowNodeRegistryInner {
            registry: registry.node_registry.registry.clone(),
        };
        for (node, logic) in node_pairs {
            inner.insert(node, logic);
        }
        registry.node_registry = Arc::new(inner);
        drop(registry);
        let _ = app_handle.emit("catalog-updated", ());
    }

    if let Some(manifest) = manifest {
        register_developer_package(&app_handle, &wasm_path, manifest).await;
    }

    let _ = app_handle.emit(
        "package-status",
        serde_json::json!({ "packageId": format!("dev:{}", project_path), "status": "ready" }),
    );

    Ok(count)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StalePackageInfo {
    pub package_id: String,
    pub project_path: Option<String>,
}

#[tauri::command]
pub async fn developer_check_staleness(
    app_handle: AppHandle,
) -> Result<Vec<StalePackageInfo>, TauriFunctionError> {
    let client = TauriRegistryState::get_client(&app_handle).await?;
    let stale_entries = client.check_local_staleness().await;

    let user_dir = match TauriSettingsState::construct(&app_handle).await {
        Ok(settings) => {
            let guard = settings.lock().await;
            guard.user_dir.clone()
        }
        Err(_) => {
            return Ok(stale_entries
                .iter()
                .map(|(id, _, _)| StalePackageInfo {
                    package_id: id.clone(),
                    project_path: None,
                })
                .collect());
        }
    };

    let store = load_store(&user_dir);
    let result: Vec<StalePackageInfo> = stale_entries
        .iter()
        .map(|(id, _, _)| {
            let project_path = store.projects.iter().find_map(|p| {
                let project_path = PathBuf::from(&p.path);
                if let Ok(wasm_path) = find_wasm_file(&project_path) {
                    let manifest = load_manifest_for_registration(&project_path, &wasm_path);
                    if manifest.as_ref().map(|m| m.id.as_str()) == Some(id.as_str()) {
                        return Some(p.path.clone());
                    }
                }
                None
            });
            StalePackageInfo {
                package_id: id.clone(),
                project_path,
            }
        })
        .collect();

    if !stale_entries.is_empty() {
        for (id, _, _) in &stale_entries {
            let _ = app_handle.emit(
                "package-status",
                serde_json::json!({ "packageId": id, "status": "stale" }),
            );
        }
    }

    for info in &result {
        if let Some(ref path) = info.project_path {
            let _ = app_handle.emit(
                "package-status",
                serde_json::json!({ "packageId": format!("dev:{}", path), "status": "stale" }),
            );
        }
    }

    Ok(result)
}

pub async fn load_all_developer_nodes(app_handle: &AppHandle) {
    let engine = match TauriWasmEngineState::construct(app_handle) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!("Failed to get WasmEngine for developer node loading: {}", e);
            return;
        }
    };

    let user_dir = match TauriSettingsState::construct(app_handle).await {
        Ok(settings) => {
            let guard = settings.lock().await;
            guard.user_dir.clone()
        }
        Err(e) => {
            tracing::warn!("Failed to get settings for developer node loading: {}", e);
            return;
        }
    };

    let store = load_store(&user_dir);
    if store.projects.is_empty() {
        return;
    }

    let flow_state = match TauriFlowLikeState::construct(app_handle).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Failed to get flow state for developer node loading: {}", e);
            return;
        }
    };

    let mut all_node_pairs: Vec<(Node, Arc<dyn NodeLogic>)> = Vec::new();
    let mut manifests_to_register: Vec<(PathBuf, PackageManifest)> = Vec::new();

    for project in &store.projects {
        let project_path = PathBuf::from(&project.path);
        match find_wasm_file(&project_path) {
            Ok(wasm_path) => {
                let manifest = load_manifest_for_registration(&project_path, &wasm_path);
                let manifest_id = manifest.as_ref().map(|m| m.id.as_str());

                match load_wasm_nodes_from_path(&wasm_path, engine.clone(), manifest_id).await {
                    Ok(pairs) => {
                        tracing::info!(
                            "Loaded {} developer node(s) from '{}'",
                            pairs.len(),
                            project.name
                        );
                        all_node_pairs.extend(pairs);
                        if let Some(m) = manifest {
                            manifests_to_register.push((wasm_path, m));
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to load developer nodes from '{}': {:?}",
                            project.name,
                            e
                        );
                    }
                }
            }
            Err(e) => {
                tracing::debug!(
                    "No WASM file found for developer project '{}': {:?}",
                    project.name,
                    e
                );
            }
        }
    }

    if !all_node_pairs.is_empty() {
        let registry_guard = flow_state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        let mut inner = flow_like::state::FlowNodeRegistryInner {
            registry: registry.node_registry.registry.clone(),
        };
        for (node, logic) in all_node_pairs {
            inner.insert(node, logic);
        }
        registry.node_registry = Arc::new(inner);
        drop(registry);
        // Dispatch the emit on the main thread to avoid a startup deadlock:
        // calling `emit` here from a tokio worker grabs Tauri's internal
        // `webviews_lock` while iterating webviews and synchronously waits on
        // the main loop for `Webview::eval`. If the main thread is concurrently
        // servicing a URL-scheme task that also needs `webviews_lock` (very
        // likely during early startup) the two deadlock. Running the emit on
        // the main thread keeps it serialized with URL-scheme handlers.
        let emit_handle = app_handle.clone();
        if let Err(e) = app_handle.run_on_main_thread(move || {
            let _ = emit_handle.emit("catalog-updated", ());
        }) {
            tracing::warn!(
                "Failed to schedule catalog-updated emit on main thread: {:?}",
                e
            );
        }
        tracing::info!("Developer nodes loaded into catalog");
    }

    for (wasm_path, manifest) in manifests_to_register {
        register_developer_package(app_handle, &wasm_path, manifest).await;
    }
}
