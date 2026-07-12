use crate::{
    functions::TauriFunctionError,
    settings::{LogRetentionSettings, Settings},
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::app::App;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageItem {
    pub id: String,
    pub name: String,
    pub detail: String,
    pub size_bytes: u64,
    pub file_count: u64,
    pub updated_at_ms: Option<u64>,
    pub deletable: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCategory {
    pub key: String,
    pub label: String,
    pub description: String,
    pub size_bytes: u64,
    pub item_count: usize,
    pub items: Vec<StorageItem>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageOverview {
    pub total_bytes: u64,
    pub generated_at_ms: u64,
    pub log_retention: LogRetentionSettings,
    pub categories: Vec<StorageCategory>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageDeleteResult {
    pub deleted_items: usize,
    pub freed_bytes: u64,
    pub skipped_items: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRetentionInput {
    pub enabled: bool,
    pub days: u32,
}

#[derive(Default, Debug, Clone, Copy)]
struct PathStats {
    size_bytes: u64,
    file_count: u64,
    updated_at: Option<SystemTime>,
}

impl PathStats {
    fn merge(&mut self, other: Self) {
        self.size_bytes = self.size_bytes.saturating_add(other.size_bytes);
        self.file_count = self.file_count.saturating_add(other.file_count);
        self.updated_at = match (self.updated_at, other.updated_at) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (None, value) | (value, None) => value,
        };
    }
}

fn path_stats(path: &Path) -> PathStats {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return PathStats::default();
    };
    let mut stats = PathStats {
        size_bytes: if metadata.is_file() {
            metadata.len()
        } else {
            0
        },
        file_count: u64::from(metadata.is_file()),
        updated_at: metadata.modified().ok(),
    };
    if !metadata.is_dir() {
        return stats;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return stats;
    };
    for entry in entries.flatten() {
        stats.merge(path_stats(&entry.path()));
    }
    stats
}

fn to_ms(time: Option<SystemTime>) -> Option<u64> {
    time.and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
}

fn direct_items(
    root: &Path,
    detail: &str,
    excluded_names: &HashSet<String>,
    deletable: bool,
) -> Vec<StorageItem> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut items = entries
        .flatten()
        .filter_map(|entry| {
            let id = entry.file_name().to_string_lossy().to_string();
            if id.is_empty() || excluded_names.contains(&id) {
                return None;
            }
            let stats = path_stats(&entry.path());
            Some(StorageItem {
                name: id.clone(),
                id,
                detail: detail.to_string(),
                size_bytes: stats.size_bytes,
                file_count: stats.file_count,
                updated_at_ms: to_ms(stats.updated_at),
                deletable,
            })
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    items
}

fn log_items(logs_dir: &Path, active_runs: &HashSet<String>) -> Vec<StorageItem> {
    let runs_root = logs_dir.join("runs");
    let Ok(apps) = fs::read_dir(&runs_root) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for app in apps
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    {
        let app_id = app.file_name().to_string_lossy().to_string();
        let Ok(boards) = fs::read_dir(app.path()) else {
            continue;
        };
        for board in boards
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        {
            let board_id = board.file_name().to_string_lossy().to_string();
            let Ok(runs) = fs::read_dir(board.path()) else {
                continue;
            };
            for run in runs.flatten() {
                let file_name = run.file_name().to_string_lossy().to_string();
                let run_id = file_name.strip_suffix(".lance").unwrap_or(&file_name);
                if run_id.is_empty() {
                    continue;
                }
                // `runs.lance` is the shared per-board metadata/index table, not
                // an individual run log. Skip it so it isn't shown as a
                // deletable run (deleting it would wipe the board's run index).
                if run_id == "runs" {
                    continue;
                }
                let stats = path_stats(&run.path());
                items.push(StorageItem {
                    id: format!("{app_id}/{board_id}/{file_name}"),
                    name: run_id.to_string(),
                    detail: format!("App {} · Board {}", short_id(&app_id), short_id(&board_id)),
                    size_bytes: stats.size_bytes,
                    file_count: stats.file_count,
                    updated_at_ms: to_ms(stats.updated_at),
                    deletable: !active_runs.contains(run_id),
                });
            }
        }
    }
    items.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
    items
}

fn offloaded_blob_items(blob_dir: &Path) -> Vec<StorageItem> {
    let ref_counts = fs::read(blob_dir.join("_refcounts.json"))
        .ok()
        .and_then(|data| {
            serde_json::from_slice::<std::collections::HashMap<String, u64>>(&data).ok()
        })
        .unwrap_or_default();
    let Ok(prefixes) = fs::read_dir(blob_dir) else {
        return Vec::new();
    };
    let mut items = Vec::new();
    for prefix in prefixes
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    {
        let Ok(blobs) = fs::read_dir(prefix.path()) else {
            continue;
        };
        for blob in blobs
            .flatten()
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        {
            let hash = blob.file_name().to_string_lossy().to_string();
            if !valid_component(&hash) {
                continue;
            }
            let stats = path_stats(&blob.path());
            let references = ref_counts.get(&hash).copied().unwrap_or_default();
            items.push(StorageItem {
                id: hash.clone(),
                name: format!("Browser payload {}", short_id(&hash)),
                detail: if references == 0 {
                    "Offloaded from browser storage · reference pending or untracked".to_string()
                } else {
                    format!(
                        "Offloaded from browser storage · {references} {}",
                        if references == 1 {
                            "reference"
                        } else {
                            "references"
                        }
                    )
                },
                size_bytes: stats.size_bytes,
                file_count: 1,
                updated_at_ms: to_ms(stats.updated_at),
                deletable: false,
            });
        }
    }
    items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
    items
}

fn short_id(value: &str) -> &str {
    value.get(..8).unwrap_or(value)
}

fn category(key: &str, label: &str, description: &str, items: Vec<StorageItem>) -> StorageCategory {
    StorageCategory {
        key: key.to_string(),
        label: label.to_string(),
        description: description.to_string(),
        size_bytes: items.iter().map(|item| item.size_bytes).sum(),
        item_count: items.len(),
        items,
    }
}

fn build_overview(settings: &Settings, active_runs: &HashSet<String>) -> StorageOverview {
    let app_root = settings.project_dir.join("apps");
    let mut apps = direct_items(
        &app_root,
        "Local app and project files",
        &HashSet::new(),
        true,
    );
    for app in &mut apps {
        app.detail = format!("Local app · {} files", app.file_count);
    }

    let mut bits = direct_items(
        &settings.bit_dir,
        "Downloaded model or runtime artifact",
        &HashSet::from(["deps-cache".to_string()]),
        true,
    );
    for bit in &mut bits {
        bit.name = format!("Artifact {}", short_id(&bit.id));
    }

    let logs = log_items(&settings.logs_dir, active_runs);
    let offloaded = offloaded_blob_items(&settings.user_dir.join("blob_store"));
    let temporary = direct_items(
        &settings.temporary_dir,
        "Temporary execution file",
        &HashSet::new(),
        true,
    );

    let mut cache_exclusions = HashSet::new();
    cache_exclusions.insert("blob_store".to_string());
    for known in [
        &settings.bit_dir,
        &settings.project_dir,
        &settings.logs_dir,
        &settings.temporary_dir,
    ] {
        if known.parent() == Some(settings.user_dir.as_path()) {
            if let Some(name) = known.file_name() {
                cache_exclusions.insert(name.to_string_lossy().to_string());
            }
        }
    }
    let cache = direct_items(
        &settings.user_dir,
        "Cache, local database, or supporting data",
        &cache_exclusions,
        false,
    );

    let categories = vec![
        category(
            "apps",
            "Apps & projects",
            "Boards, media, databases, and files owned by local apps.",
            apps,
        ),
        category(
            "bits",
            "Downloaded bits",
            "Local model weights and other reusable runtime artifacts.",
            bits,
        ),
        category(
            "logs",
            "Run logs",
            "Debug and execution history stored for individual local runs.",
            logs,
        ),
        category(
            "offloaded",
            "Offloaded browser files",
            "Large IndexedDB values moved onto disk by Studio to keep the WebView responsive.",
            offloaded,
        ),
        category(
            "cache",
            "Cache & support data",
            "Rebuildable caches plus local supporting databases.",
            cache,
        ),
        category(
            "temporary",
            "Temporary files",
            "Intermediate files created while workflows are running.",
            temporary,
        ),
    ];
    StorageOverview {
        total_bytes: categories.iter().map(|entry| entry.size_bytes).sum(),
        generated_at_ms: to_ms(Some(SystemTime::now())).unwrap_or_default(),
        log_retention: settings.log_retention.clone(),
        categories,
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

fn remove_path(path: &Path) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn item_path(settings: &Settings, category: &str, id: &str) -> Option<PathBuf> {
    match category {
        "apps" if valid_component(id) => Some(settings.project_dir.join("apps").join(id)),
        "bits" if valid_component(id) && id != "deps-cache" => Some(settings.bit_dir.join(id)),
        "cache" if valid_component(id) => Some(settings.user_dir.join(id)),
        "temporary" if valid_component(id) => Some(settings.temporary_dir.join(id)),
        "logs" => {
            let parts = id.split('/').collect::<Vec<_>>();
            if parts.len() == 3 && parts.iter().all(|part| valid_component(part)) {
                Some(
                    settings
                        .logs_dir
                        .join("runs")
                        .join(parts[0])
                        .join(parts[1])
                        .join(parts[2]),
                )
            } else {
                None
            }
        }
        _ => None,
    }
}

fn run_id_from_item_id(id: &str) -> Option<&str> {
    id.rsplit('/')
        .next()
        .map(|value| value.strip_suffix(".lance").unwrap_or(value))
}

fn active_run_ids(app_handle: &AppHandle) -> HashSet<String> {
    app_handle
        .try_state::<TauriFlowLikeState>()
        .map(|state| {
            state
                .0
                .board_run_registry
                .iter()
                .map(|entry| entry.key().clone())
                .collect()
        })
        .unwrap_or_default()
}

fn cleanup_logs(
    settings: &Settings,
    active_runs: &HashSet<String>,
    days: u32,
) -> StorageDeleteResult {
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(u64::from(days.max(1)) * 86_400))
        .unwrap_or(UNIX_EPOCH);
    let mut result = StorageDeleteResult {
        deleted_items: 0,
        freed_bytes: 0,
        skipped_items: Vec::new(),
    };
    for item in log_items(&settings.logs_dir, active_runs) {
        let is_expired = item
            .updated_at_ms
            .and_then(|millis| UNIX_EPOCH.checked_add(Duration::from_millis(millis)))
            .is_some_and(|modified| modified < cutoff);
        if !is_expired || !item.deletable {
            continue;
        }
        let Some(path) = item_path(settings, "logs", &item.id) else {
            continue;
        };
        match remove_path(&path) {
            Ok(()) => {
                result.deleted_items += 1;
                result.freed_bytes = result.freed_bytes.saturating_add(item.size_bytes);
            }
            Err(_) => result.skipped_items.push(item.id),
        }
    }
    result
}

pub async fn run_configured_log_cleanup(
    app_handle: &AppHandle,
) -> Result<StorageDeleteResult, TauriFunctionError> {
    let settings_state = TauriSettingsState::construct(app_handle)
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
    let mut settings = settings_state.lock().await;
    if !settings.log_retention.enabled {
        return Ok(StorageDeleteResult {
            deleted_items: 0,
            freed_bytes: 0,
            skipped_items: Vec::new(),
        });
    }
    let result = cleanup_logs(
        &settings,
        &active_run_ids(app_handle),
        settings.log_retention.days,
    );
    settings.log_retention.last_cleanup_ms = to_ms(Some(SystemTime::now()));
    Settings::serialize(&mut settings);
    Ok(result)
}

#[tauri::command(async)]
pub async fn get_local_storage_overview(
    app_handle: AppHandle,
) -> Result<StorageOverview, TauriFunctionError> {
    let settings_state = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
    let settings = settings_state.lock().await;
    let mut overview = build_overview(&settings, &active_run_ids(&app_handle));
    drop(settings);

    if let Ok(state) = TauriFlowLikeState::construct(&app_handle).await {
        if let Some(apps) = overview
            .categories
            .iter_mut()
            .find(|entry| entry.key == "apps")
        {
            for item in &mut apps.items {
                if let Ok(meta) = App::get_meta(item.id.clone(), state.clone(), None, None).await {
                    item.name = meta.name;
                    item.detail = format!("Local app · {} files", item.file_count);
                }
            }
        }
    }
    Ok(overview)
}

#[tauri::command(async)]
pub async fn set_log_retention_policy(
    app_handle: AppHandle,
    policy: LogRetentionInput,
) -> Result<StorageDeleteResult, TauriFunctionError> {
    if !(1..=3650).contains(&policy.days) {
        return Err(TauriFunctionError::new(
            "Log retention must be between 1 and 3650 days",
        ));
    }
    let settings_state = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
    {
        let mut settings = settings_state.lock().await;
        settings.log_retention.enabled = policy.enabled;
        settings.log_retention.days = policy.days;
        Settings::serialize(&mut settings);
    }
    run_configured_log_cleanup(&app_handle).await
}

#[tauri::command(async)]
pub async fn run_log_cleanup(
    app_handle: AppHandle,
) -> Result<StorageDeleteResult, TauriFunctionError> {
    run_configured_log_cleanup(&app_handle).await
}

#[tauri::command(async)]
pub async fn delete_local_storage_items(
    app_handle: AppHandle,
    category: String,
    ids: Vec<String>,
) -> Result<StorageDeleteResult, TauriFunctionError> {
    if ids.is_empty() || ids.len() > 500 {
        return Err(TauriFunctionError::new("Select between 1 and 500 items"));
    }
    let active_runs = active_run_ids(&app_handle);
    let settings_state = TauriSettingsState::construct(&app_handle)
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
    let mut settings = settings_state.lock().await;
    let overview = build_overview(&settings, &active_runs);
    let Some(inventory) = overview
        .categories
        .iter()
        .find(|entry| entry.key == category)
    else {
        return Err(TauriFunctionError::new("Unknown storage category"));
    };
    let known = inventory
        .items
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<std::collections::HashMap<_, _>>();
    let mut result = StorageDeleteResult {
        deleted_items: 0,
        freed_bytes: 0,
        skipped_items: Vec::new(),
    };

    for id in ids {
        let Some(item) = known.get(id.as_str()) else {
            result.skipped_items.push(id);
            continue;
        };
        if !item.deletable
            || (category == "logs"
                && run_id_from_item_id(&id).is_some_and(|run| active_runs.contains(run)))
        {
            result.skipped_items.push(id);
            continue;
        }
        let Some(path) = item_path(&settings, &category, &id) else {
            result.skipped_items.push(id);
            continue;
        };
        match remove_path(&path) {
            Ok(()) => {
                result.deleted_items += 1;
                result.freed_bytes = result.freed_bytes.saturating_add(item.size_bytes);
                if category == "apps" {
                    for profile in settings.profiles.values_mut() {
                        if let Some(apps) = &mut profile.hub_profile.apps {
                            apps.retain(|app| app.app_id != id);
                        }
                    }
                }
            }
            Err(_) => result.skipped_items.push(id),
        }
    }
    Settings::serialize(&mut settings);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_path_traversal_components() {
        assert!(valid_component("safe-id"));
        assert!(!valid_component("../unsafe"));
        assert!(!valid_component("nested/path"));
        assert!(!valid_component("nested\\path"));
    }

    #[test]
    fn recursive_stats_do_not_follow_symlinks() {
        let root = std::env::temp_dir().join(format!(
            "flow-like-storage-stats-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("nested/file.bin"), [1_u8, 2, 3, 4]).unwrap();
        let stats = path_stats(&root);
        assert_eq!(stats.size_bytes, 4);
        assert_eq!(stats.file_count, 1);
        fs::remove_dir_all(root).unwrap();
    }
}
