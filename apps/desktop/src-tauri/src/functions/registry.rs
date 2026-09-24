use crate::{
    functions::TauriFunctionError,
    profile::UserProfile,
    state::{TauriFlowLikeState, TauriRegistryState, TauriSettingsState, TauriWasmEngineState},
    widget_grants::{
        WIDGET_ENGINE_GATE, WIDGET_GRANTS, WidgetGrantMint, describe_unpacked_widget,
        mint_widget_grant,
    },
    widget_protocol::bundle_source,
};
use flow_like::a2ui::micro_widget::{PackageWidgetRef, PackageWidgetSource};
use flow_like::flow::node::NodeLogic;
use flow_like::hub::{Hub, HubWidgetStorage};
use flow_like_types::sync::Mutex;
use flow_like_wasm::widget_policy::{
    PlatformStorageScope, WIDGET_POLICY_SOURCE_LOCAL, WidgetPolicyDescriptor, WidgetPolicySubject,
    WidgetRuntimeContext, WidgetRuntimeSourceRequest, registry_policy_source, reserved_host,
};
use flow_like_wasm::{
    client::RegistryClient,
    registry::{
        CachedPackage, InstalledPackage, PackageAccessFilter, PackageSource, RegistryConfig,
        SearchFilters, SearchResults,
    },
};
use futures::future::join_all;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

/// How long a describe waits for an uncached hub before treating it as
/// announcing no widget storage.
const HUB_WIDGET_STORAGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Bridges the widget provider to whichever registry client is installed right
/// now. `registry_init` replaces the client, so registering a clone once at
/// startup would leave the provider resolving against a stale snapshot.
pub struct RegistryWidgetSource(pub Arc<Mutex<Option<RegistryClient>>>);

#[flow_like_types::async_trait]
impl PackageWidgetSource for RegistryWidgetSource {
    async fn list_widgets(
        &self,
        packages: &HashMap<String, String>,
    ) -> flow_like_types::Result<Vec<PackageWidgetRef>> {
        // Clone out of the guard: the lock must not be held across the lookup.
        let client = { self.0.lock().await.clone() };
        match client {
            Some(client) => client.list_widgets(packages).await,
            None => Ok(Vec::new()),
        }
    }
}

/// Cache dir backing `RegistryConfig` — also the root of the content-addressed
/// widget store (`widgets/{package_id}/{bundle_hash}`) served by the
/// `flow-widget://` protocol. Single derivation point: keep both consumers here.
pub(crate) fn wasm_registry_cache_dir(project_dir: &std::path::Path) -> std::path::PathBuf {
    project_dir
        .parent()
        .unwrap_or(project_dir)
        .join("wasm_registry_cache")
}

/// Get the registry client with the auth token refreshed on the stored instance.
/// This ensures every API-calling command uses a fresh token and the stored
/// client stays up-to-date for future calls (e.g. search uses stored token).
async fn get_client_with_token(
    app_handle: &AppHandle,
    token: Option<String>,
) -> Result<RegistryClient, TauriFunctionError> {
    use tauri::Manager;
    let state = app_handle
        .try_state::<TauriRegistryState>()
        .ok_or_else(|| TauriFunctionError::new("Registry state not found"))?;
    let mut guard = state.0.lock().await;
    let client = guard
        .as_mut()
        .ok_or_else(|| TauriFunctionError::new("Registry client not initialized"))?;
    if let Some(t) = token {
        client.set_auth_token(Some(t));
    }
    Ok(client.clone())
}

pub(crate) fn emit_package_status(app_handle: &AppHandle, package_id: &str, status: &str) {
    crate::utils::emit_to_ui(
        app_handle,
        "package-status",
        serde_json::json!({ "packageId": package_id, "status": status }),
    );
}

fn clear_package_status(app_handle: &AppHandle, package_id: &str) {
    emit_package_status(app_handle, package_id, "idle");
}

fn log_registry_package_error(command: &str, package_id: &str, error: &impl std::fmt::Display) {
    println!("{} failed for {}: {}", command, package_id, error);
    tracing::error!(command, package_id = %package_id, error = %error, "Registry package command failed");
}

/// Re-run derived node schemas for boards that were opened before the package
/// registry became ready. Keep the cached Board instances themselves: evicting
/// them here could discard in-memory edits that have not reached storage yet.
async fn refresh_open_board_definitions(
    app_handle: &AppHandle,
) -> Result<usize, TauriFunctionError> {
    let flow_state = TauriFlowLikeState::construct(app_handle).await?;
    let boards = flow_state
        .board_registry()
        .iter()
        .map(|entry| entry.value().clone())
        .collect::<Vec<_>>();

    for board in &boards {
        board
            .lock()
            .await
            .refresh_node_definitions(flow_state.clone())
            .await;
    }
    // Definitions changed without moving `updated_at`/`hash`; cached sync snapshots would
    // otherwise keep serving the old node metadata.
    crate::state::TauriBoardSyncState::invalidate_all(app_handle);

    Ok(boards.len())
}

/// Rebuild the global node registry from scratch: builtin catalog nodes +
/// every currently-installed WASM package + developer (local) project nodes.
///
/// The registry is append-only (`push_nodes`/`insert` never remove entries),
/// so after an update or uninstall the previous version's nodes would linger
/// until an app restart rebuilt the registry from nothing — which is why nodes
/// could go stale or fail to reconcile without a restart. Rebuilding from the
/// authoritative installed set makes update/uninstall/downgrade deterministic.
async fn rebuild_node_registry(
    app_handle: &AppHandle,
    emit_catalog_updated: bool,
) -> Result<(), TauriFunctionError> {
    let registry_client = TauriRegistryState::get_client(app_handle).await?;
    let flow_state = TauriFlowLikeState::construct(app_handle).await?;
    let engine = TauriWasmEngineState::construct(app_handle)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    // 1. Developer (local project) nodes — collected first because they are
    // also registered in the installed list and would otherwise be loaded a
    // second time below. Inserted last so they win name collisions.
    let dev_pairs = super::developer::collect_developer_node_pairs(app_handle).await;
    let dev_package_ids: std::collections::HashSet<&str> = dev_pairs
        .iter()
        .filter_map(|(node, _)| node.wasm.as_ref().map(|w| w.package_id.as_str()))
        .collect();

    // 2. Builtin catalog nodes (cheap Arc clones of the cached catalog).
    let mut logic_nodes: Vec<Arc<dyn NodeLogic>> = flow_like_catalog::get_catalog();

    // 3. Installed WASM package nodes (active version of each), skipping
    // developer projects already loaded above.
    let installed = registry_client.list_installed().await.unwrap_or_default();
    for pkg in &installed {
        if dev_package_ids.contains(pkg.id.as_str()) {
            continue;
        }
        match registry_client.load_nodes(&pkg.id, engine.clone()).await {
            Ok(nodes) => {
                logic_nodes.extend(nodes.into_iter().map(|n| Arc::new(n) as Arc<dyn NodeLogic>));
            }
            Err(e) => {
                tracing::warn!("Failed to load package '{}': {}", pkg.id, e);
            }
        }
    }

    let mut inner =
        flow_like::state::FlowNodeRegistryInner::new(logic_nodes.len() + dev_pairs.len());
    for logic in logic_nodes {
        let node = logic.get_node();
        inner.insert(node, logic);
    }
    for (node, logic) in dev_pairs {
        inner.insert(node, logic);
    }

    {
        let registry_guard = flow_state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        registry.node_registry = Arc::new(inner);
    }

    // Catalog replacements can alter both static schemas and dynamic widget
    // contracts. Refresh the native Board cache before telling React Query to
    // fetch those boards again.
    if emit_catalog_updated {
        if let Err(e) = refresh_open_board_definitions(app_handle).await {
            tracing::warn!(
                "Failed to refresh open boards after registry rebuild: {:?}",
                e
            );
        }
        super::developer::emit_catalog_updated(app_handle);
    }

    Ok(())
}

async fn load_installed_package_nodes(
    app_handle: &AppHandle,
    registry_client: &RegistryClient,
    package_id: &str,
    emit_catalog_updated: bool,
) -> Result<(), TauriFunctionError> {
    let flow_state = TauriFlowLikeState::construct(app_handle).await?;
    let engine = TauriWasmEngineState::construct(app_handle)
        .map_err(|e| TauriFunctionError::new(&e.to_string()))?;

    let nodes = registry_client
        .load_nodes(package_id, engine)
        .await
        .map_err(|e| {
            TauriFunctionError::new(&format!("Failed to load package '{}': {}", package_id, e))
        })?;

    if !nodes.is_empty() {
        let registry_guard = flow_state.node_registry.clone();
        let mut registry = registry_guard.write().await;
        registry.push_nodes(
            nodes
                .into_iter()
                .map(|node| Arc::new(node) as Arc<dyn NodeLogic>)
                .collect(),
        );
    }

    if let Err(e) = refresh_open_board_definitions(app_handle).await {
        tracing::warn!(
            "Failed to refresh open boards after package install: {:?}",
            e
        );
    }

    if emit_catalog_updated {
        super::developer::emit_catalog_updated(app_handle);
    }

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
    pub include_disabled: Option<bool>,
    #[serde(default)]
    pub sort_by: Option<String>,
    #[serde(default)]
    pub sort_desc: Option<bool>,
    #[serde(default)]
    pub offset: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub access: Option<PackageAccessFilter>,
    #[serde(default)]
    pub ids: Option<Vec<String>>,
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
            include_disabled: input.include_disabled.unwrap_or(false),
            sort_by: sort_by.unwrap_or_default(),
            sort_desc: input.sort_desc.unwrap_or(true),
            offset: input.offset.unwrap_or(0),
            limit: input.limit.unwrap_or(20),
            access: input.access,
            ids: input.ids,
        }
    }
}

#[tauri::command]
pub async fn registry_search_packages(
    app_handle: AppHandle,
    filters: SearchFiltersInput,
    token: Option<String>,
) -> Result<SearchResults, TauriFunctionError> {
    let registry_client = get_client_with_token(&app_handle, token).await?;
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
    app_id: Option<String>,
) -> Result<CachedPackage, TauriFunctionError> {
    emit_package_status(&app_handle, &package_id, "downloading");
    let registry_client = get_client_with_token(&app_handle, token.clone()).await?;
    let installed = match app_id.as_deref() {
        Some(app_id) => {
            registry_client
                .install_for_app(&package_id, version.as_deref(), token.as_deref(), app_id)
                .await
        }
        None => {
            registry_client
                .install(&package_id, version.as_deref(), token.as_deref())
                .await
        }
    };
    let installed = installed.inspect_err(|error| {
        log_registry_package_error("registry_install_package", &package_id, error);
        emit_package_status(&app_handle, &package_id, "error");
    })?;

    load_installed_package_nodes(&app_handle, &registry_client, &package_id, true)
        .await
        .inspect_err(|error| {
            log_registry_package_error("registry_install_package:load_nodes", &package_id, error);
            emit_package_status(&app_handle, &package_id, "error");
        })?;

    clear_package_status(&app_handle, &package_id);

    Ok(installed)
}

#[tauri::command]
pub async fn registry_uninstall_package(
    app_handle: AppHandle,
    package_id: String,
) -> Result<(), TauriFunctionError> {
    let registry_client: RegistryClient = TauriRegistryState::get_client(&app_handle).await?;
    WIDGET_GRANTS.revoke(&package_id, None);
    registry_client.uninstall(&package_id).await?;

    if let Err(e) = rebuild_node_registry(&app_handle, true).await {
        tracing::warn!("Failed to rebuild node registry after uninstall: {:?}", e);
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
    let registry_client = get_client_with_token(&app_handle, token.clone()).await?;
    let target_version = match version {
        Some(version) => Some(version),
        None => match registry_client.check_updates(token.as_deref()).await {
            Ok(updates) => updates,
            Err(error) => {
                log_registry_package_error(
                    "registry_update_package:check_updates",
                    &package_id,
                    &error,
                );
                emit_package_status(&app_handle, &package_id, "error");
                return Err(error.into());
            }
        }
        .into_iter()
        .find(|(id, _, _)| id == &package_id)
        .map(|(_, _, latest_version)| latest_version),
    };

    if target_version.is_none() {
        println!(
            "registry_update_package failed for {}: No update available",
            package_id
        );
        clear_package_status(&app_handle, &package_id);
        return Err(TauriFunctionError::new("No update available"));
    }

    let installed = registry_client
        .install(&package_id, target_version.as_deref(), token.as_deref())
        .await
        .inspect_err(|error| {
            log_registry_package_error("registry_update_package:install", &package_id, error);
            emit_package_status(&app_handle, &package_id, "error");
        })?;

    if let Err(e) = rebuild_node_registry(&app_handle, true).await {
        tracing::warn!("Failed to rebuild node registry after update: {:?}", e);
    } else {
        clear_package_status(&app_handle, &package_id);
    }

    Ok(installed)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    let registry_client = get_client_with_token(&app_handle, token.clone()).await?;
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

/// Every distinct hub of the default hub and of every profile.
fn profile_hub_urls<'a>(
    default_hub: &'a str,
    profiles: impl IntoIterator<Item = &'a UserProfile>,
) -> Vec<String> {
    let hubs = std::iter::once(default_hub).chain(profiles.into_iter().flat_map(|profile| {
        std::iter::once(profile.hub_profile.hub.as_str())
            .chain(profile.hub_profile.hubs.iter().map(String::as_str))
    }));
    let mut urls: Vec<String> = Vec::new();
    for hub in hubs.map(str::trim) {
        if !hub.is_empty() && !urls.iter().any(|url| url == hub) {
            urls.push(hub.to_string());
        }
    }
    urls
}

/// Hub hosts a widget may never name as a CSP source.
fn hub_hosts(hub_urls: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for host in hub_urls.iter().filter_map(|url| reserved_host(url)) {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

/// Hosts a hub serves besides its API URL (API domain, app, web, CDN).
fn hub_service_hosts(
    domain: &str,
    app: Option<&str>,
    web: Option<&str>,
    cdn: Option<&str>,
) -> Vec<String> {
    [Some(domain), app, web, cdn]
        .into_iter()
        .flatten()
        .filter_map(reserved_host)
        .collect()
}

struct HubWidgetFacts {
    platform_storage: Vec<PlatformStorageScope>,
    reserved_hosts: Vec<String>,
}

/// Every reachable hub reserves its own service hosts, but only `storage_hub`
/// (the current profile's hub, which serves the app) may announce
/// platform-storage scopes. A hub that cannot be reached in time contributes
/// nothing, so its storage hosts classify like any host.
async fn hub_widget_facts(
    app_handle: &AppHandle,
    hub_urls: &[String],
    storage_hub: Option<&str>,
) -> HubWidgetFacts {
    let http_client = match TauriFlowLikeState::http_client(app_handle).await {
        Ok(http_client) => http_client,
        Err(error) => {
            tracing::warn!(%error, "No HTTP client to read hub widget facts");
            return HubWidgetFacts {
                platform_storage: Vec::new(),
                reserved_hosts: Vec::new(),
            };
        }
    };
    let hubs = join_all(hub_urls.iter().map(|hub_url| {
        let http_client = http_client.clone();
        async move {
            let hub = match tokio::time::timeout(
                HUB_WIDGET_STORAGE_TIMEOUT,
                Hub::new(hub_url, http_client),
            )
            .await
            {
                Ok(Ok(hub)) => Some(hub),
                Ok(Err(error)) => {
                    tracing::debug!(hub = %hub_url, %error, "Hub info unavailable for widget policy");
                    None
                }
                Err(_) => {
                    tracing::debug!(hub = %hub_url, "Hub info timed out for widget policy");
                    None
                }
            };
            (hub_url, hub)
        }
    }))
    .await;
    let mut reserved_hosts = Vec::new();
    let mut storage = Vec::new();
    for (hub_url, hub) in hubs {
        let Some(hub) = hub else { continue };
        reserved_hosts.extend(hub_service_hosts(
            &hub.domain,
            hub.app.as_deref(),
            hub.web.as_deref(),
            hub.cdn.as_deref(),
        ));
        if storage_hub == Some(hub_url.as_str()) {
            storage.extend(hub.widget_storage);
        }
    }
    HubWidgetFacts {
        platform_storage: platform_storage_scopes(storage),
        reserved_hosts,
    }
}

/// Well-formed, distinct scopes in a stable order. A scope must yield an
/// `https` app path source for a valid app id.
fn platform_storage_scopes(
    storage: impl IntoIterator<Item = HubWidgetStorage>,
) -> Vec<PlatformStorageScope> {
    let mut scopes: Vec<PlatformStorageScope> = storage
        .into_iter()
        .map(|storage| PlatformStorageScope {
            origin: storage.origin,
            path_prefix: storage.path_prefix,
        })
        .filter(|scope| {
            let valid = scope.origin.starts_with("https://") && scope.app_source("app").is_some();
            if !valid {
                tracing::warn!(
                    origin = %scope.origin,
                    path_prefix = %scope.path_prefix,
                    "Ignoring a malformed hub widget storage scope"
                );
            }
            valid
        })
        .collect();
    scopes.sort_by(|a, b| (&a.origin, &a.path_prefix).cmp(&(&b.origin, &b.path_prefix)));
    scopes.dedup();
    scopes
}

/// Host of the registry that installed `bundle_hash` as a widget bundle of
/// this package, or `None` for local, developer and unknown bundles.
fn registry_bundle_host(installed: Option<&InstalledPackage>, bundle_hash: &str) -> Option<String> {
    let installed = installed?;
    let PackageSource::Remote { registry_url, .. } = &installed.source else {
        return None;
    };
    let is_installed_bundle = installed.manifest.widget_bundle_hash.as_deref() == Some(bundle_hash)
        || installed
            .versions
            .values()
            .any(|version| version.widget_bundle_hash.as_deref() == Some(bundle_hash));
    is_installed_bundle
        .then(|| reserved_host(registry_url))
        .flatten()
}

/// One describe or mint request of the host.
struct WidgetPolicyRequest {
    package_id: String,
    bundle_hash: String,
    widget_id: String,
    preview: bool,
    app_id: Option<String>,
    runtime_sources: Vec<WidgetRuntimeSourceRequest>,
}

/// Derives the descriptor against this machine's profile hubs, their widget
/// storage scopes, the registry that installed the bundle and the webview's
/// engine gate. `network_at` (unix seconds) adds the display-only `network`.
async fn describe_widget_policy(
    app_handle: &AppHandle,
    request: WidgetPolicyRequest,
    network_at: Option<i64>,
) -> Result<WidgetPolicyDescriptor, TauriFunctionError> {
    let settings = TauriSettingsState::construct(app_handle).await?;
    let (cache_dir, hub_urls, storage_hub) = {
        let guard = settings.lock().await;
        (
            wasm_registry_cache_dir(&guard.project_dir),
            profile_hub_urls(&guard.default_hub, guard.profiles.values()),
            guard
                .get_current_profile()
                .ok()
                .map(|profile| profile.hub_profile.hub.trim().to_string())
                .filter(|hub| !hub.is_empty()),
        )
    };
    let mut reserved_hosts = hub_hosts(&hub_urls);
    let facts = hub_widget_facts(app_handle, &hub_urls, storage_hub.as_deref()).await;
    for host in facts.reserved_hosts {
        if !reserved_hosts.contains(&host) {
            reserved_hosts.push(host);
        }
    }
    let platform_storage = facts.platform_storage;
    let registry_client = TauriRegistryState::get_client(app_handle).await?;
    let installed = registry_client.get_installed(&request.package_id).await;
    let registry_host = registry_bundle_host(installed.as_ref(), &request.bundle_hash);
    let source = registry_host.as_deref().map_or_else(
        || WIDGET_POLICY_SOURCE_LOCAL.to_string(),
        registry_policy_source,
    );
    reserved_hosts.extend(registry_host);
    let WidgetPolicyRequest {
        package_id,
        bundle_hash,
        widget_id,
        preview,
        app_id,
        runtime_sources,
    } = request;
    let bundle_sources = [bundle_source(&package_id, &bundle_hash)];
    let subject = WidgetPolicySubject {
        source,
        package_id,
        package_version: None,
        bundle_hash,
        widget_id,
        preview,
    };

    tokio::task::spawn_blocking(move || {
        let context = WidgetRuntimeContext {
            reserved_hosts: &reserved_hosts,
            platform_storage: &platform_storage,
            app_id: app_id.as_deref(),
            bundle_sources: &bundle_sources,
            engine: *WIDGET_ENGINE_GATE,
        };
        describe_unpacked_widget(&cache_dir, subject, &runtime_sources, &context, network_at)
    })
    .await
    .map_err(|error| TauriFunctionError::new(&format!("Widget policy task failed: {error}")))?
    .map_err(|error| TauriFunctionError::new(&error))
}

/// Authoritative policy of one widget of an unpacked bundle, derived from its
/// declared `contract.json` plus the runtime sources the host extracted for
/// its declared network inputs. The host renders consent from this
/// descriptor. Malformed runtime sources fail with `invalid_runtime_sources: …`,
/// runtime sources on a preview with `runtime_sources_in_preview: …`.
#[tauri::command]
pub async fn registry_describe_widget_policy(
    app_handle: AppHandle,
    package_id: String,
    bundle_hash: String,
    widget_id: String,
    preview: bool,
    app_id: Option<String>,
    runtime_sources: Option<Vec<WidgetRuntimeSourceRequest>>,
) -> Result<WidgetPolicyDescriptor, TauriFunctionError> {
    let request = WidgetPolicyRequest {
        package_id,
        bundle_hash,
        widget_id,
        preview,
        app_id,
        runtime_sources: runtime_sources.unwrap_or_default(),
    };
    describe_widget_policy(&app_handle, request, Some(chrono::Utc::now().timestamp())).await
}

/// Re-derives the widget policy with the same runtime sources and mints a
/// grant for it. Fails with `policy_changed: …` when `policy_digest` no
/// longer matches, and with the describe errors for malformed requests.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn registry_mint_widget_grant(
    app_handle: AppHandle,
    package_id: String,
    bundle_hash: String,
    widget_id: String,
    preview: bool,
    policy_digest: String,
    app_id: Option<String>,
    runtime_sources: Option<Vec<WidgetRuntimeSourceRequest>>,
) -> Result<WidgetGrantMint, TauriFunctionError> {
    let request = WidgetPolicyRequest {
        package_id,
        bundle_hash,
        widget_id,
        preview,
        app_id,
        runtime_sources: runtime_sources.unwrap_or_default(),
    };
    let descriptor = describe_widget_policy(&app_handle, request, None).await?;
    mint_widget_grant(&WIDGET_GRANTS, &descriptor, &policy_digest)
        .map_err(|error| TauriFunctionError::new(&error))
}

#[tauri::command]
pub async fn registry_revoke_widget_grants(
    package_id: String,
    widget_id: Option<String>,
) -> Result<(), TauriFunctionError> {
    WIDGET_GRANTS.revoke(&package_id, widget_id.as_deref());
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

    // Register in the installed list so rebuild_node_registry can find it
    let _ = registry_client
        .register_local_package(local_path, cached.entry.manifest.clone())
        .await;

    if let Err(e) = rebuild_node_registry(&app_handle, true).await {
        tracing::warn!("Failed to rebuild node registry after local load: {:?}", e);
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

    let cache_dir = wasm_registry_cache_dir(&settings_guard.project_dir);

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

    // Ensure this source is installed synchronously with registry readiness.
    // The early startup registration in `run` is intentionally best-effort and
    // may still be queued when a board opens.
    let flow_state = TauriFlowLikeState::construct(&app_handle).await?;
    flow_state
        .register_package_widget_source(Arc::new(RegistryWidgetSource(state.0.clone())))
        .await;

    if let Err(e) = rebuild_node_registry(&app_handle, false).await {
        tracing::warn!("Failed to load WASM nodes during registry init: {:?}", e);
    }

    super::developer::register_all_developer_packages(&app_handle).await;

    match refresh_open_board_definitions(&app_handle).await {
        Ok(count) if count > 0 => {
            tracing::debug!(count, "Refreshed open boards after registry initialization");
        }
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("Failed to refresh open boards after registry init: {:?}", e);
        }
    }

    // Emit only after open boards have recomputed their dynamic contracts, so
    // the frontend cannot refetch the stale pre-registry snapshot.
    super::developer::emit_catalog_updated(&app_handle);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_wasm::manifest::PackageManifest;
    use flow_like_wasm::registry::InstalledVersion;
    use std::path::PathBuf;

    fn installed(source: PackageSource, version_hashes: &[&str]) -> InstalledPackage {
        let manifest = PackageManifest::new("com.example.maps", "Maps", "1.0.0", "maps");
        let versions = version_hashes
            .iter()
            .enumerate()
            .map(|(index, hash)| {
                let version = format!("1.0.{index}");
                (
                    version.clone(),
                    InstalledVersion {
                        version,
                        wasm_path: PathBuf::from("/tmp/maps.wasm"),
                        installed_at: chrono::Utc::now(),
                        manifest: manifest.clone(),
                        metadata: None,
                        wasm_hash: None,
                        widget_bundle_path: None,
                        widget_bundle_hash: Some(hash.to_string()),
                    },
                )
            })
            .collect();
        InstalledPackage {
            id: manifest.id.clone(),
            version: "1.0.0".into(),
            source,
            installed_at: chrono::Utc::now(),
            wasm_path: PathBuf::from("/tmp/maps.wasm"),
            manifest,
            versions,
            metadata: None,
            wasm_hash: None,
        }
    }

    fn remote() -> PackageSource {
        PackageSource::Remote {
            registry_url: "https://API.flow-like.com:443/api/v1/registry".into(),
            download_url: String::new(),
        }
    }

    #[test]
    fn widget_policy_source_is_registry_only_for_installed_registry_bundles() {
        let old = "a".repeat(64);
        let current = "b".repeat(64);
        let package = installed(remote(), &[old.as_str(), current.as_str()]);

        assert_eq!(
            registry_bundle_host(Some(&package), &old).as_deref(),
            Some("api.flow-like.com")
        );
        assert_eq!(
            registry_bundle_host(Some(&package), &current).as_deref(),
            Some("api.flow-like.com")
        );
        assert_eq!(registry_bundle_host(Some(&package), &"c".repeat(64)), None);
        assert_eq!(registry_bundle_host(None, &current), None);

        let local = installed(
            PackageSource::Local {
                path: PathBuf::from("/dev/maps.wasm"),
            },
            &[current.as_str()],
        );
        assert_eq!(registry_bundle_host(Some(&local), &current), None);
    }

    #[test]
    fn widget_reserved_hosts_cover_every_profile_hub() {
        let mut first = UserProfile::new(flow_like::profile::Profile::default());
        first.hub_profile.hub = "hub.acme.eu:8443".into();
        first.hub_profile.hubs = vec![" https://api.flow-like.com ".into()];
        let mut second = UserProfile::new(flow_like::profile::Profile::default());
        second.hub_profile.hubs = vec!["https://Other.Example.org/api".into(), String::new()];

        let urls = profile_hub_urls("https://api.flow-like.com", [&first, &second]);
        assert_eq!(
            urls,
            [
                "https://api.flow-like.com",
                "hub.acme.eu:8443",
                "https://Other.Example.org/api"
            ]
        );
        let hosts = hub_hosts(&urls);
        assert_eq!(hosts.len(), 3);
        for host in ["api.flow-like.com", "hub.acme.eu", "other.example.org"] {
            assert!(
                hosts.iter().any(|entry| entry == host),
                "{host} in {hosts:?}"
            );
        }
        assert!(hosts.iter().all(|entry| !entry.is_empty()));
    }

    #[test]
    fn widget_reserved_hosts_include_each_hubs_service_hosts() {
        let hosts = hub_service_hosts(
            "api.flow-like.com",
            Some("https://app.flow-like.com"),
            Some("https://flow-like.com"),
            Some("https://cdn.flow-like.com/assets"),
        );
        assert_eq!(
            hosts,
            [
                "api.flow-like.com",
                "app.flow-like.com",
                "flow-like.com",
                "cdn.flow-like.com"
            ]
        );
        assert_eq!(
            hub_service_hosts("self-hosted.example.org", None, None, None),
            ["self-hosted.example.org"]
        );
    }

    #[test]
    fn widget_platform_storage_scopes_come_from_well_formed_hub_entries() {
        let storage = |origin: &str, path_prefix: &str| HubWidgetStorage {
            origin: origin.into(),
            path_prefix: path_prefix.into(),
        };
        let scopes = platform_storage_scopes([
            storage("https://storage.googleapis.com", "/content/apps/"),
            storage(
                "https://flow-like-content.s3.eu-central-1.amazonaws.com",
                "/apps/",
            ),
            storage("https://storage.googleapis.com", "/content/apps/"),
            storage("http://insecure.example.org", "/apps/"),
            storage("wss://socket.example.org", "/apps/"),
            storage("https://*.blob.core.windows.net", "/apps/"),
            storage("https://bucket.s3.amazonaws.com/", "/apps/"),
            storage("https://bucket.s3.amazonaws.com", "apps/"),
            storage("https://bucket.s3.amazonaws.com", "/apps"),
            storage("https://bucket.s3.amazonaws.com", "/../apps/"),
        ]);
        let scope = |origin: &str, path_prefix: &str| PlatformStorageScope {
            origin: origin.into(),
            path_prefix: path_prefix.into(),
        };
        assert_eq!(
            scopes,
            vec![
                scope(
                    "https://flow-like-content.s3.eu-central-1.amazonaws.com",
                    "/apps/"
                ),
                scope("https://storage.googleapis.com", "/content/apps/"),
            ]
        );
    }
}
