use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriRegistryState, TauriSettingsState, TauriWasmEngineState},
};
use flow_like::flow::node::NodeLogic;
use flow_like_types::intercom::InterComEvent;
use flow_like_wasm::{
    client::RegistryClient,
    registry::{CachedPackage, InstalledPackage, RegistryConfig, SearchFilters, SearchResults},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

const WASM_COMPILE_TOAST_ID: &str = "wasm-compile-progress";

fn emit_progress(app_handle: &AppHandle, message: &str, done: bool, success: bool) {
    let event = InterComEvent::with_type(
        "progress",
        serde_json::json!({
            "id": WASM_COMPILE_TOAST_ID,
            "message": message,
            "done": done,
            "success": success
        }),
    );
    let _ = app_handle.emit("progress", vec![event]);
}

fn emit_package_status(app_handle: &AppHandle, package_id: &str, status: &str) {
    let _ = app_handle.emit(
        "package-status",
        serde_json::json!({ "packageId": package_id, "status": status }),
    );
}

async fn reload_wasm_nodes(app_handle: &AppHandle) -> Result<(), TauriFunctionError> {
    let registry_client = TauriRegistryState::get_client(app_handle).await?;
    let flow_state = TauriFlowLikeState::construct(app_handle).await?;

    let installed = registry_client.list_installed().await.unwrap_or_default();
    let total = installed.len();

    if total == 0 {
        let _ = app_handle.emit("catalog-updated", ());
        return Ok(());
    }

    emit_progress(
        app_handle,
        &format!("Compiling {} package(s)...", total),
        false,
        true,
    );

    for pkg in &installed {
        emit_package_status(app_handle, &pkg.id, "compiling");
    }

    let engine = TauriWasmEngineState::construct(app_handle)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    let mut wasm_nodes: Vec<Arc<dyn NodeLogic>> = Vec::new();
    let mut compiled = 0usize;

    for pkg in &installed {
        match registry_client.load_nodes(&pkg.id, engine.clone()).await {
            Ok(nodes) => {
                compiled += nodes.len();
                for node in nodes {
                    wasm_nodes.push(Arc::new(node));
                }
                emit_package_status(app_handle, &pkg.id, "ready");
            }
            Err(e) => {
                tracing::warn!("Failed to load package '{}': {}", pkg.id, e);
                emit_package_status(app_handle, &pkg.id, "error");
            }
        }
    }

    if !wasm_nodes.is_empty() {
        let registry_guard = flow_state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        registry.push_nodes(wasm_nodes);
    }

    emit_progress(
        app_handle,
        &format!("Compiled {} WASM nodes", compiled),
        true,
        true,
    );
    let _ = app_handle.emit("catalog-updated", ());

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFiltersInput {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub keywords: Option<Vec<String>>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub verified_only: Option<bool>,
    #[serde(default)]
    pub include_deprecated: Option<bool>,
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_desc: Option<bool>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

impl From<SearchFiltersInput> for SearchFilters {
    fn from(input: SearchFiltersInput) -> Self {
        use flow_like_wasm::registry::SortField;

        let sort_by = input.sort_by.and_then(|s| match s.as_str() {
            "relevance" => Some(SortField::Relevance),
            "name" => Some(SortField::Name),
            "downloads" => Some(SortField::Downloads),
            "updated_at" => Some(SortField::UpdatedAt),
            "created_at" => Some(SortField::CreatedAt),
            _ => None,
        });

        SearchFilters {
            query: input.query,
            category: input.category,
            keywords: input.keywords.unwrap_or_default(),
            author: input.author,
            verified_only: input.verified_only.unwrap_or(false),
            include_deprecated: input.include_deprecated.unwrap_or(false),
            sort_by: sort_by.unwrap_or_default(),
            sort_desc: input.sort_desc.unwrap_or(true),
            offset: input.offset.unwrap_or(0),
            limit: input.limit.unwrap_or(20),
        }
    }
}

#[tauri::command]
pub async fn registry_search_packages(
    app_handle: AppHandle,
    filters: SearchFiltersInput,
) -> Result<SearchResults, TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let search_filters: SearchFilters = filters.into();
    let results = registry_client.search(&search_filters).await?;
    Ok(results)
}

#[tauri::command]
pub async fn registry_get_package(
    app_handle: AppHandle,
    package_id: String,
) -> Result<Option<InstalledPackage>, TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let installed = registry_client.get_installed(&package_id).await;
    Ok(installed)
}

#[tauri::command]
pub async fn registry_install_package(
    app_handle: AppHandle,
    package_id: String,
    version: Option<String>,
    token: Option<String>,
) -> Result<CachedPackage, TauriFunctionError> {
    emit_package_status(&app_handle, &package_id, "downloading");
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let installed = registry_client
        .install(&package_id, version.as_deref(), token.as_deref())
        .await
        .inspect_err(|_e| {
            emit_package_status(&app_handle, &package_id, "error");
        })?;

    if let Err(e) = reload_wasm_nodes(&app_handle).await {
        tracing::warn!("Failed to reload WASM nodes after install: {:?}", e);
    }

    Ok(installed)
}

#[tauri::command]
pub async fn registry_uninstall_package(
    app_handle: AppHandle,
    package_id: String,
) -> Result<(), TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    registry_client.uninstall(&package_id).await?;

    if let Err(e) = reload_wasm_nodes(&app_handle).await {
        tracing::warn!("Failed to reload WASM nodes after uninstall: {:?}", e);
    }

    Ok(())
}

#[tauri::command]
pub async fn registry_get_installed_packages(
    app_handle: AppHandle,
) -> Result<Vec<InstalledPackage>, TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let packages = registry_client.list_installed().await?;
    Ok(packages)
}

#[tauri::command]
pub async fn registry_is_package_installed(
    app_handle: AppHandle,
    package_id: String,
) -> Result<bool, TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let installed = registry_client.get_installed(&package_id).await;
    Ok(installed.is_some())
}

#[tauri::command]
pub async fn registry_get_installed_version(
    app_handle: AppHandle,
    package_id: String,
) -> Result<Option<String>, TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let installed = registry_client.get_installed(&package_id).await;
    Ok(installed.map(|i| i.version))
}

#[tauri::command]
pub async fn registry_update_package(
    app_handle: AppHandle,
    package_id: String,
    version: Option<String>,
    token: Option<String>,
) -> Result<CachedPackage, TauriFunctionError> {
    emit_package_status(&app_handle, &package_id, "downloading");
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let installed = registry_client
        .install(&package_id, version.as_deref(), token.as_deref())
        .await
        .inspect_err(|_e| {
            emit_package_status(&app_handle, &package_id, "error");
        })?;

    if let Err(e) = reload_wasm_nodes(&app_handle).await {
        tracing::warn!("Failed to reload WASM nodes after update: {:?}", e);
    }

    Ok(installed)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageUpdate {
    pub package_id: String,
    pub current_version: String,
    pub latest_version: String,
}

#[tauri::command]
pub async fn registry_check_for_updates(
    app_handle: AppHandle,
    token: Option<String>,
) -> Result<Vec<PackageUpdate>, TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    let update_tuples = registry_client.check_updates(token.as_deref()).await?;

    let updates: Vec<PackageUpdate> = update_tuples
        .into_iter()
        .map(|(id, current, latest)| PackageUpdate {
            package_id: id,
            current_version: current,
            latest_version: latest,
        })
        .collect();

    Ok(updates)
}

#[tauri::command]
pub async fn registry_set_auth_token(
    app_handle: AppHandle,
    token: Option<String>,
) -> Result<(), TauriFunctionError> {
    use tauri::Manager;
    let state = app_handle
        .try_state::<TauriRegistryState>()
        .ok_or_else(|| TauriFunctionError::new("Registry state not found"))?;

    let mut guard = state.0.lock().await;
    if let Some(client) = guard.as_mut() {
        client.set_auth_token(token);
    }

    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryInitConfig {
    #[serde(default)]
    pub registry_url: Option<String>,
}

#[tauri::command]
pub async fn registry_load_local(
    app_handle: AppHandle,
    path: String,
) -> Result<CachedPackage, TauriFunctionError> {
    let registry_client = TauriRegistryState::get_client(&app_handle).await?;
    let local_path = std::path::Path::new(&path);
    let cached = registry_client.load_local(local_path).await?;

    // Register in the installed list so reload_wasm_nodes can find it
    let _ = registry_client
        .register_local_package(local_path, cached.entry.manifest.clone())
        .await;

    emit_package_status(&app_handle, &cached.entry.id, "compiling");

    if let Err(e) = reload_wasm_nodes(&app_handle).await {
        tracing::warn!("Failed to reload WASM nodes after local load: {:?}", e);
    }

    Ok(cached)
}

#[tauri::command]
pub async fn registry_init(
    app_handle: AppHandle,
    config: Option<RegistryInitConfig>,
) -> Result<(), TauriFunctionError> {
    use tauri::Manager;

    let settings = TauriSettingsState::construct(&app_handle).await?;
    let settings_guard = settings.lock().await;

    let cache_dir = settings_guard
        .project_dir
        .parent()
        .unwrap_or(&settings_guard.project_dir)
        .join("wasm_registry_cache");

    let default_registry = config
        .and_then(|c| c.registry_url)
        .unwrap_or_else(|| "https://api.flow-like.com/api/v1/registry".to_string());

    drop(settings_guard);

    // Preserve auth token from existing client (if any) so re-init doesn't
    // lose the token that was set via pushAuthContext / setAuthToken.
    let state = app_handle
        .try_state::<TauriRegistryState>()
        .ok_or_else(|| anyhow::anyhow!("Registry state not found"))?;

    let existing_token = {
        let guard = state.0.lock().await;
        guard.as_ref().and_then(|c| c.auth_token().cloned())
    };

    let registry_config = RegistryConfig {
        default_registry,
        additional_registries: vec![],
        local_paths: vec![],
        cache_dir,
        cache_duration_hours: 24 * 7,
        auto_update_index: true,
        allow_unverified: false,
        auth_token: existing_token,
    };

    let client = RegistryClient::new(registry_config)?;
    client.init().await?;

    let mut guard = state.0.lock().await;
    *guard = Some(client);
    drop(guard);

    if let Err(e) = reload_wasm_nodes(&app_handle).await {
        tracing::warn!("Failed to load WASM nodes during registry init: {:?}", e);
    }

    super::developer::register_all_developer_packages(&app_handle).await;

    Ok(())
}
