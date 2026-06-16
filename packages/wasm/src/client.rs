//! Registry client for fetching, caching, and publishing WASM packages

use crate::{
    manifest::PackageManifest,
    registry::{
        CachedPackage, DownloadRequest, DownloadResponse, InstalledPackage, InstalledVersion,
        LocalRegistryState, PackageSource, PackageVersion, PublishRequest, PublishResponse,
        RegistryConfig, RegistryEntry, SearchFilters, SearchResults,
    },
};
use anyhow::{Result, anyhow};
use chrono::Utc;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::sync::RwLock;

/// Registry client for managing WASM packages
#[derive(Clone)]
pub struct RegistryClient {
    config: RegistryConfig,
    state: Arc<RwLock<LocalRegistryState>>,
    http_client: reqwest::Client,
}

impl RegistryClient {
    fn target_platform_for_download() -> Option<String> {
        Some(crate::aot_cache::host_platform_key())
    }

    fn cwasm_sidecar_path(wasm_path: &Path) -> PathBuf {
        if cfg!(target_os = "ios") {
            wasm_path.with_extension(format!("{}.cwasm", crate::aot_cache::host_platform_key()))
        } else {
            wasm_path.with_extension("cwasm")
        }
    }

    fn compact_error_body(body: &str) -> Option<String> {
        let trimmed = body.trim();
        if trimmed.is_empty() {
            return None;
        }

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(trimmed) {
            for key in ["message", "error", "details"] {
                if let Some(value) = json.get(key) {
                    if let Some(text) = value.as_str() {
                        return Some(Self::truncate_error_detail(text));
                    }

                    return Some(Self::truncate_error_detail(&value.to_string()));
                }
            }

            return Some(Self::truncate_error_detail(&json.to_string()));
        }

        Some(Self::truncate_error_detail(trimmed))
    }

    fn truncate_error_detail(detail: &str) -> String {
        const MAX_ERROR_DETAIL_CHARS: usize = 2048;

        let compact = detail.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut chars = compact.chars();
        let truncated: String = chars.by_ref().take(MAX_ERROR_DETAIL_CHARS).collect();

        if chars.next().is_some() {
            format!("{}…", truncated)
        } else {
            truncated
        }
    }

    async fn http_response_error(action: &str, response: reqwest::Response) -> anyhow::Error {
        let status = response.status();
        let detail = match response.text().await {
            Ok(body) => Self::compact_error_body(&body),
            Err(error) => Some(format!("Failed to read error response body: {}", error)),
        };

        match detail {
            Some(detail) if !detail.is_empty() => anyhow!("{}: {}: {}", action, status, detail),
            _ => anyhow!("{}: {}", action, status),
        }
    }

    pub fn new(config: RegistryConfig) -> Result<Self> {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            config,
            state: Arc::new(RwLock::new(LocalRegistryState::default())),
            http_client,
        })
    }

    pub async fn init(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.config.cache_dir).await?;
        tokio::fs::create_dir_all(self.config.cache_dir.join("packages")).await?;
        tokio::fs::create_dir_all(self.config.cache_dir.join("manifests")).await?;
        tokio::fs::create_dir_all(self.config.cache_dir.join("wasm").join("nodes")).await?;

        self.load_state().await?;
        Ok(())
    }

    fn versioned_wasm_path(&self, package_id: &str, version: &str) -> PathBuf {
        self.config
            .cache_dir
            .join("wasm")
            .join("nodes")
            .join(package_id)
            .join(version)
            .join("node.wasm")
    }

    async fn load_state(&self) -> Result<()> {
        let state_path = self.config.cache_dir.join("state.json");
        if state_path.exists() {
            let data = tokio::fs::read_to_string(&state_path).await?;
            if let Ok(loaded) = serde_json::from_str::<LocalRegistryState>(&data) {
                *self.state.write().await = loaded;
            }
        }
        Ok(())
    }

    async fn save_state(&self) -> Result<()> {
        let state_path = self.config.cache_dir.join("state.json");
        let state = self.state.read().await;
        let data = serde_json::to_string_pretty(&*state)?;
        tokio::fs::write(&state_path, data).await?;
        Ok(())
    }

    /// Set the auth token for authenticated API requests
    pub fn set_auth_token(&mut self, token: Option<String>) {
        self.config.auth_token = token;
    }

    /// Get the current auth token (if any)
    pub fn auth_token(&self) -> Option<&String> {
        self.config.auth_token.as_ref()
    }

    fn build_search_url(&self, filters: &SearchFilters, include_own: bool) -> String {
        let base = format!("{}/search", self.config.default_registry);
        let mut url = reqwest::Url::parse(&base).expect("invalid registry URL");

        {
            let mut params = url.query_pairs_mut();
            if let Some(q) = &filters.query {
                params.append_pair("query", q);
            }
            if let Some(cat) = &filters.category {
                params.append_pair("category", cat);
            }
            for kw in &filters.keywords {
                params.append_pair("keywords", kw);
            }
            if let Some(author) = &filters.author {
                params.append_pair("author", author);
            }
            if filters.verified_only {
                params.append_pair("verified_only", "true");
            }
            if filters.include_deprecated {
                params.append_pair("include_deprecated", "true");
            }
            if filters.include_disabled {
                params.append_pair("include_disabled", "true");
            }
            params.append_pair("offset", &filters.offset.to_string());
            params.append_pair("limit", &filters.limit.to_string());
            let sort_str = match filters.sort_by {
                crate::registry::SortField::Relevance => "relevance",
                crate::registry::SortField::Name => "name",
                crate::registry::SortField::Downloads => "downloads",
                crate::registry::SortField::UpdatedAt => "updated_at",
                crate::registry::SortField::CreatedAt => "created_at",
            };
            params.append_pair("sort_by", sort_str);
            params.append_pair("sort_desc", &filters.sort_desc.to_string());
            if include_own {
                params.append_pair("include_own", "true");
            }
        }

        url.to_string()
    }

    /// Search packages via the remote registry API
    pub async fn search_with_token(
        &self,
        filters: &SearchFilters,
        auth_token: Option<&str>,
    ) -> Result<SearchResults> {
        let effective_token = auth_token
            .map(String::from)
            .or_else(|| self.config.auth_token.clone());
        let url = self.build_search_url(filters, effective_token.is_some());

        let mut request = self.http_client.get(&url);
        if let Some(token) = &effective_token {
            request = request.header("Authorization", format!("Bearer {}", token));
        }

        let response = request.send().await?;

        if !response.status().is_success() {
            return Err(Self::http_response_error("Failed to search registry", response).await);
        }

        let results: SearchResults = response.json().await?;
        Ok(results)
    }

    /// Search packages via the remote registry API (uses stored token)
    pub async fn search(&self, filters: &SearchFilters) -> Result<SearchResults> {
        self.search_with_token(filters, None).await
    }

    /// Download and cache a package
    pub async fn download_package(
        &self,
        package_id: &str,
        version: Option<&str>,
        auth_token: Option<&str>,
    ) -> Result<CachedPackage> {
        let state = self.state.read().await;
        if let Some(installed) = state.installed.get(package_id) {
            if (version.is_none() || version == Some(&installed.version))
                && installed.wasm_path.exists()
            {
                let wasm_data = tokio::fs::read(&installed.wasm_path).await?;
                return Ok(CachedPackage {
                    entry: RegistryEntry {
                        id: installed.id.clone(),
                        manifest: installed.manifest.clone(),
                        nodes: vec![],
                        versions: vec![PackageVersion {
                            version: installed.version.clone(),
                            wasm_hash: String::new(),
                            wasm_size: wasm_data.len() as u64,
                            download_url: None,
                            published_at: installed.installed_at,
                            min_flow_like_version: None,
                            release_notes: None,
                            yanked: false,
                        }],
                        status: crate::registry::PackageStatus::Active,
                        download_count: 0,
                        created_at: installed.installed_at,
                        updated_at: installed.installed_at,
                        source: installed.source.clone(),
                        verified: false,
                    },
                    wasm_data,
                    cached_at: installed.installed_at,
                    expires_at: None,
                });
            }
        }
        drop(state);

        let request = DownloadRequest {
            package_id: package_id.to_string(),
            version: version.map(String::from),
            target_platform: Self::target_platform_for_download(),
        };

        let url = format!("{}/download", self.config.default_registry);
        let mut req = self.http_client.post(&url).json(&request);
        let effective_token = auth_token
            .map(String::from)
            .or_else(|| self.config.auth_token.clone());
        if let Some(token) = &effective_token {
            req = req.header("Authorization", format!("Bearer {}", token));
        }
        let response = req.send().await?;

        if !response.status().is_success() {
            return Err(Self::http_response_error("Failed to download package", response).await);
        }

        let download: DownloadResponse = response.json().await?;

        // Fetch WASM data - either from download_url or decode from base64
        let wasm_data = if let Some(download_url) = &download.download_url {
            // Download from CDN/signed URL
            let wasm_response = self.http_client.get(download_url).send().await?;
            if !wasm_response.status().is_success() {
                return Err(Self::http_response_error(
                    "Failed to download WASM from CDN",
                    wasm_response,
                )
                .await);
            }
            wasm_response.bytes().await?.to_vec()
        } else if !download.wasm_base64.is_empty() {
            // Fallback to base64 decoding
            base64_decode(&download.wasm_base64)?
        } else {
            return Err(anyhow!("No download URL or WASM data in response"));
        };

        let wasm_path = self.versioned_wasm_path(&download.package_id, &download.version);
        if let Some(parent) = wasm_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&wasm_path, &wasm_data).await?;

        // If the server provided a precompiled .cwasm, download and store it
        // alongside the raw .wasm so `load_nodes` can inject it into the AOT cache.
        if let Some(cwasm_url) = &download.cwasm_download_url {
            match self
                .download_cwasm(cwasm_url, download.cwasm_checksum.as_deref(), &wasm_path)
                .await
            {
                Ok(()) => {
                    tracing::info!("Downloaded precompiled cwasm for {}", download.package_id)
                }
                Err(e) => tracing::error!(
                    "Failed to download cwasm for {}: {}",
                    download.package_id,
                    e
                ),
            }
        }

        let now = Utc::now();
        let wasm_hash = Some(calculate_hash(&wasm_data));
        let installed_version = InstalledVersion {
            version: download.version.clone(),
            wasm_path: wasm_path.clone(),
            installed_at: now,
            manifest: download.manifest.clone(),
            metadata: download.metadata.clone(),
            wasm_hash: wasm_hash.clone(),
        };

        let mut state = self.state.write().await;
        if let Some(existing) = state.installed.get_mut(&download.package_id) {
            existing.version = download.version.clone();
            existing.installed_at = now;
            existing.wasm_path = wasm_path.clone();
            existing.manifest = download.manifest.clone();
            existing.metadata = download.metadata.clone();
            existing
                .versions
                .insert(download.version.clone(), installed_version);
        } else {
            let installed = InstalledPackage {
                id: download.package_id.clone(),
                version: download.version.clone(),
                source: PackageSource::Remote {
                    registry_url: self.config.default_registry.clone(),
                    download_url: download.download_url.clone().unwrap_or(url.clone()),
                },
                installed_at: now,
                wasm_path: wasm_path.clone(),
                manifest: download.manifest.clone(),
                versions: HashMap::from([(download.version.clone(), installed_version)]),
                metadata: download.metadata.clone(),
                wasm_hash,
            };
            state
                .installed
                .insert(download.package_id.clone(), installed);
        }
        drop(state);
        self.save_state().await?;

        Ok(CachedPackage {
            entry: RegistryEntry {
                id: download.package_id.clone(),
                manifest: download.manifest,
                nodes: vec![],
                versions: vec![PackageVersion {
                    version: download.version.clone(),
                    wasm_hash: calculate_hash(&wasm_data),
                    wasm_size: wasm_data.len() as u64,
                    download_url: download.download_url.or(Some(url)),
                    published_at: Utc::now(),
                    min_flow_like_version: None,
                    release_notes: None,
                    yanked: false,
                }],
                status: crate::registry::PackageStatus::Active,
                download_count: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                source: PackageSource::Remote {
                    registry_url: self.config.default_registry.clone(),
                    download_url: String::new(),
                },
                verified: false,
            },
            wasm_data,
            cached_at: Utc::now(),
            expires_at: None,
        })
    }

    /// Install a package (download + register)
    pub async fn install(
        &self,
        package_id: &str,
        version: Option<&str>,
        auth_token: Option<&str>,
    ) -> Result<CachedPackage> {
        self.download_package(package_id, version, auth_token).await
    }

    pub async fn install_version(
        &self,
        package_id: &str,
        version: &str,
        auth_token: Option<&str>,
    ) -> Result<CachedPackage> {
        let state = self.state.read().await;
        if let Some(installed) = state.installed.get(package_id) {
            if let Some(iv) = installed.get_version(version) {
                if iv.wasm_path.exists() {
                    let wasm_data = tokio::fs::read(&iv.wasm_path).await?;
                    return Ok(self.cached_package_from_version(installed, iv, wasm_data));
                }
            }
        }
        drop(state);

        self.download_package(package_id, Some(version), auth_token)
            .await
    }

    pub async fn batch_install(
        &self,
        packages: &[(String, Option<String>)],
        auth_token: Option<&str>,
    ) -> Result<Vec<(String, Result<CachedPackage>)>> {
        let mut results = Vec::with_capacity(packages.len());
        for (package_id, version) in packages {
            let result = self
                .install(package_id, version.as_deref(), auth_token)
                .await;
            results.push((package_id.clone(), result));
        }
        Ok(results)
    }

    /// Uninstall a package
    pub async fn uninstall(&self, package_id: &str) -> Result<()> {
        let mut state = self.state.write().await;

        if let Some(installed) = state.installed.remove(package_id) {
            if installed.wasm_path.exists() {
                tokio::fs::remove_file(&installed.wasm_path).await?;
            }
        }

        state.cache_metadata.remove(package_id);
        drop(state);
        self.save_state().await?;
        Ok(())
    }

    /// List installed packages
    pub async fn list_installed(&self) -> Result<Vec<InstalledPackage>> {
        let state = self.state.read().await;
        Ok(state.installed.values().cloned().collect())
    }

    /// Check which local packages have a changed WASM file on disk.
    /// Returns a list of (package_id, stored_hash, current_hash) for stale packages.
    pub async fn check_local_staleness(&self) -> Vec<(String, String, String)> {
        let state = self.state.read().await;
        let locals: Vec<_> = state
            .installed
            .values()
            .filter(|p| matches!(p.source, PackageSource::Local { .. }))
            .cloned()
            .collect();
        drop(state);

        let mut stale = Vec::new();
        for pkg in locals {
            let stored_hash = match &pkg.wasm_hash {
                Some(h) => h.clone(),
                None => continue,
            };
            let current_hash = match tokio::fs::read(&pkg.wasm_path).await {
                Ok(bytes) => blake3::hash(&bytes).to_hex().to_string(),
                Err(_) => continue,
            };
            if stored_hash != current_hash {
                stale.push((pkg.id.clone(), stored_hash, current_hash));
            }
        }
        stale
    }

    /// Update the stored wasm_hash for a local package after reloading.
    pub async fn update_local_hash(&self, package_id: &str, new_hash: String) -> Result<()> {
        let mut state = self.state.write().await;
        if let Some(pkg) = state.installed.get_mut(package_id) {
            if matches!(pkg.source, PackageSource::Local { .. }) {
                pkg.wasm_hash = Some(new_hash.clone());
                for iv in pkg.versions.values_mut() {
                    iv.wasm_hash = Some(new_hash.clone());
                }
            }
        }
        drop(state);
        self.save_state().await?;
        Ok(())
    }

    /// Check for updates to installed packages
    pub async fn check_updates(
        &self,
        auth_token: Option<&str>,
    ) -> Result<Vec<(String, String, String)>> {
        let state = self.state.read().await;
        let installed: Vec<_> = state
            .installed
            .iter()
            .filter(|(_, v)| !matches!(v.source, PackageSource::Local { .. }))
            .map(|(k, v)| (k.clone(), v.version.clone()))
            .collect();
        drop(state);

        if installed.is_empty() {
            return Ok(Vec::new());
        }

        let filters = SearchFilters {
            limit: 200,
            ..Default::default()
        };
        let results = self.search_with_token(&filters, auth_token).await?;

        let mut updates = Vec::new();
        for (id, current_version) in installed {
            if let Some(pkg) = results.packages.iter().find(|p| p.id == id) {
                if pkg.latest_version != current_version {
                    updates.push((id, current_version, pkg.latest_version.clone()));
                }
            }
        }

        Ok(updates)
    }

    /// Publish a package to the registry
    pub async fn publish(
        &self,
        manifest: PackageManifest,
        wasm_data: Vec<u8>,
        api_key: Option<String>,
    ) -> Result<PublishResponse> {
        if let Err(errors) = manifest.validate() {
            return Err(anyhow!("Manifest validation failed: {}", errors.join(", ")));
        }

        let request = PublishRequest {
            manifest,
            wasm_base64: base64_encode(&wasm_data),
            api_key,
        };

        let url = format!("{}/publish", self.config.default_registry);
        let response = self.http_client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            return Err(Self::http_response_error("Failed to publish", response).await);
        }

        let result: PublishResponse = response.json().await?;
        Ok(result)
    }

    /// Load package from local file
    pub async fn load_local(&self, path: &Path) -> Result<CachedPackage> {
        let wasm_data = tokio::fs::read(path).await?;

        let manifest_path = path.with_extension("toml");
        let manifest: PackageManifest = if manifest_path.exists() {
            let manifest_data = tokio::fs::read_to_string(&manifest_path).await?;
            toml::from_str(&manifest_data)?
        } else {
            let file_name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("local_package");

            PackageManifest::new(
                &format!("local.{}", file_name),
                file_name,
                "0.0.0",
                "Locally loaded package",
            )
        };

        let entry = RegistryEntry {
            id: manifest.id.clone(),
            manifest: manifest.clone(),
            nodes: vec![],
            versions: vec![PackageVersion {
                version: manifest.version.clone(),
                wasm_hash: calculate_hash(&wasm_data),
                wasm_size: wasm_data.len() as u64,
                download_url: None,
                published_at: Utc::now(),
                min_flow_like_version: None,
                release_notes: None,
                yanked: false,
            }],
            status: crate::registry::PackageStatus::Active,
            download_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            source: PackageSource::Local {
                path: path.to_path_buf(),
            },
            verified: false,
        };

        Ok(CachedPackage {
            entry,
            wasm_data,
            cached_at: Utc::now(),
            expires_at: None,
        })
    }

    /// Register a local package in the installed list without downloading.
    /// Used for developer projects so they appear in `list_installed()`.
    pub async fn register_local_package(
        &self,
        wasm_path: &Path,
        manifest: PackageManifest,
    ) -> Result<InstalledPackage> {
        let now = Utc::now();
        let wasm_hash = match tokio::fs::read(wasm_path).await {
            Ok(bytes) => Some(blake3::hash(&bytes).to_hex().to_string()),
            Err(_) => None,
        };
        let version_entry = InstalledVersion {
            version: manifest.version.clone(),
            wasm_path: wasm_path.to_path_buf(),
            installed_at: now,
            manifest: manifest.clone(),
            metadata: None,
            wasm_hash: wasm_hash.clone(),
        };
        let installed = InstalledPackage {
            id: manifest.id.clone(),
            version: manifest.version.clone(),
            source: PackageSource::Local {
                path: wasm_path.to_path_buf(),
            },
            installed_at: now,
            wasm_path: wasm_path.to_path_buf(),
            manifest,
            versions: HashMap::from([(version_entry.version.clone(), version_entry)]),
            metadata: None,
            wasm_hash,
        };

        let mut state = self.state.write().await;
        state
            .installed
            .insert(installed.id.clone(), installed.clone());
        drop(state);
        self.save_state().await?;

        Ok(installed)
    }

    /// Unregister a local package without deleting its WASM file.
    pub async fn unregister_local_package(&self, package_id: &str) -> Result<bool> {
        let mut state = self.state.write().await;
        let was_local = state
            .installed
            .get(package_id)
            .map(|p| matches!(p.source, PackageSource::Local { .. }))
            .unwrap_or(false);
        if !was_local {
            return Ok(false);
        }
        state.installed.remove(package_id);
        state.cache_metadata.remove(package_id);
        drop(state);
        self.save_state().await?;
        Ok(true)
    }

    /// Clear all cached packages
    pub async fn clear_cache(&self) -> Result<()> {
        let packages_dir = self.config.cache_dir.join("packages");
        if packages_dir.exists() {
            tokio::fs::remove_dir_all(&packages_dir).await?;
            tokio::fs::create_dir_all(&packages_dir).await?;
        }

        let mut state = self.state.write().await;
        state.installed.clear();
        state.cache_metadata.clear();
        drop(state);
        self.save_state().await?;

        Ok(())
    }

    /// Get cache size in bytes
    pub async fn cache_size(&self) -> Result<u64> {
        let packages_dir = self.config.cache_dir.join("packages");
        calculate_dir_size(&packages_dir).await
    }

    /// Get an installed package by ID
    pub async fn get_installed(&self, package_id: &str) -> Option<InstalledPackage> {
        let state = self.state.read().await;
        state.installed.get(package_id).cloned()
    }

    fn cached_package_from_version(
        &self,
        installed: &InstalledPackage,
        iv: &InstalledVersion,
        wasm_data: Vec<u8>,
    ) -> CachedPackage {
        CachedPackage {
            entry: RegistryEntry {
                id: installed.id.clone(),
                manifest: iv.manifest.clone(),
                nodes: vec![],
                versions: vec![PackageVersion {
                    version: iv.version.clone(),
                    wasm_hash: String::new(),
                    wasm_size: wasm_data.len() as u64,
                    download_url: None,
                    published_at: iv.installed_at,
                    min_flow_like_version: None,
                    release_notes: None,
                    yanked: false,
                }],
                status: crate::registry::PackageStatus::Active,
                download_count: 0,
                created_at: iv.installed_at,
                updated_at: iv.installed_at,
                source: installed.source.clone(),
                verified: false,
            },
            wasm_data,
            cached_at: iv.installed_at,
            expires_at: None,
        }
    }

    /// Load WASM nodes from an installed package
    /// Returns one WasmNodeLogic per node definition (supports multi-node packages)
    pub async fn load_nodes(
        &self,
        package_id: &str,
        engine: Arc<crate::WasmEngine>,
    ) -> Result<Vec<crate::WasmNodeLogic>> {
        let installed = self
            .get_installed(package_id)
            .await
            .ok_or_else(|| anyhow!("Package '{}' is not installed", package_id))?;

        let wasm_bytes = tokio::fs::read(&installed.wasm_path).await.map_err(|e| {
            anyhow!(
                "Failed to read WASM file at {:?}: {}",
                installed.wasm_path,
                e
            )
        })?;

        // Inject precompiled .cwasm into AOT cache if available
        Self::inject_precompiled_if_available(&wasm_bytes, &installed.wasm_path, &engine);

        let manifest_security = installed.manifest.permissions.to_security_config();
        let loaded = engine.load_auto(&wasm_bytes).await?;
        let wasm_hash = loaded.hash().to_string();

        let definitions = if let Some(cached) = engine.get_cached_definitions(&wasm_hash) {
            cached
        } else {
            let mut instance = loaded
                .instantiate(&engine, manifest_security.clone())
                .await?;
            let defs = instance.call_get_nodes().await?;
            engine.cache_definitions(wasm_hash, defs.clone());
            defs
        };

        let nodes: Vec<crate::WasmNodeLogic> = definitions
            .into_iter()
            .map(|def| {
                let node_security =
                    crate::WasmSecurityConfig::from_node_permissions(&def.permissions)
                        .with_limits(manifest_security.limits.clone());
                crate::WasmNodeLogic::from_loaded_with_target(
                    loaded.clone(),
                    engine.clone(),
                    node_security,
                    def,
                )
                .with_package_id(package_id.to_string())
            })
            .collect();

        Ok(nodes)
    }

    pub async fn load_nodes_version(
        &self,
        package_id: &str,
        version: &str,
        engine: Arc<crate::WasmEngine>,
    ) -> Result<Vec<crate::WasmNodeLogic>> {
        let installed = self
            .get_installed(package_id)
            .await
            .ok_or_else(|| anyhow!("Package '{}' is not installed", package_id))?;

        let iv = installed
            .get_version(version)
            .ok_or_else(|| anyhow!("Version '{}' not installed for '{}'", version, package_id))?;

        let wasm_bytes = tokio::fs::read(&iv.wasm_path).await?;
        let manifest_security = iv.manifest.permissions.to_security_config();

        // Inject precompiled .cwasm into AOT cache if available
        Self::inject_precompiled_if_available(&wasm_bytes, &iv.wasm_path, &engine);

        let loaded = engine.load_auto(&wasm_bytes).await?;
        let wasm_hash = loaded.hash().to_string();

        let definitions = if let Some(cached) = engine.get_cached_definitions(&wasm_hash) {
            cached
        } else {
            let mut instance = loaded
                .instantiate(&engine, manifest_security.clone())
                .await?;
            let defs = instance.call_get_nodes().await?;
            engine.cache_definitions(wasm_hash, defs.clone());
            defs
        };

        Ok(definitions
            .into_iter()
            .map(|def| {
                let node_security =
                    crate::WasmSecurityConfig::from_node_permissions(&def.permissions)
                        .with_limits(manifest_security.limits.clone());
                crate::WasmNodeLogic::from_loaded_with_target(
                    loaded.clone(),
                    engine.clone(),
                    node_security,
                    def,
                )
                .with_package_id(package_id.to_string())
            })
            .collect())
    }

    /// Load all nodes from all installed packages
    pub async fn load_all_nodes(
        &self,
        engine: Arc<crate::WasmEngine>,
    ) -> Result<Vec<(String, Vec<crate::WasmNodeLogic>)>> {
        let state = self.state.read().await;
        let package_ids: Vec<String> = state.installed.keys().cloned().collect();
        drop(state);

        let mut packages = Vec::new();
        for package_id in package_ids {
            match self.load_nodes(&package_id, engine.clone()).await {
                Ok(nodes) => packages.push((package_id, nodes)),
                Err(e) => {
                    tracing::warn!("Failed to load package '{}': {}", package_id, e);
                }
            }
        }
        Ok(packages)
    }

    /// Download a precompiled `.cwasm` artifact and store it next to the `.wasm` file.
    async fn download_cwasm(
        &self,
        cwasm_url: &str,
        expected_checksum: Option<&str>,
        wasm_path: &Path,
    ) -> Result<()> {
        let cwasm_response = self.http_client.get(cwasm_url).send().await?;
        if !cwasm_response.status().is_success() {
            return Err(
                Self::http_response_error("Failed to download cwasm", cwasm_response).await,
            );
        }
        let cwasm_bytes = cwasm_response.bytes().await?.to_vec();

        if let Some(expected) = expected_checksum {
            let actual = blake3::hash(&cwasm_bytes).to_hex().to_string();
            if actual != expected {
                return Err(anyhow!(
                    "cwasm checksum mismatch: expected {}, got {}",
                    expected,
                    actual
                ));
            }
        }

        let cwasm_path = Self::cwasm_sidecar_path(wasm_path);
        tokio::fs::write(&cwasm_path, &cwasm_bytes).await?;

        Ok(())
    }

    /// If a `.cwasm` file exists next to the `.wasm`, inject it into the AOT
    /// cache so `load_module` / `load_component` can find it without compiling.
    fn inject_precompiled_if_available(
        wasm_bytes: &[u8],
        wasm_path: &Path,
        engine: &crate::WasmEngine,
    ) {
        let cwasm_path = Self::cwasm_sidecar_path(wasm_path);
        let cwasm_bytes = match std::fs::read(&cwasm_path) {
            Ok(b) => b,
            Err(_) => return,
        };

        let wasm_hash = calculate_hash(wasm_bytes);
        if let Some(aot) = engine.aot_cache() {
            #[cfg(feature = "component-model")]
            if crate::component::is_component_model(wasm_bytes) {
                if let Err(e) = aot.inject_component(&wasm_hash, &cwasm_bytes) {
                    tracing::warn!("Failed to inject component cwasm into AOT cache: {}", e);
                } else {
                    tracing::info!(
                        "Injected precompiled component cwasm into AOT cache for {}",
                        wasm_hash
                    );
                }
                return;
            }

            if let Err(e) = aot.inject_module(&wasm_hash, &cwasm_bytes) {
                tracing::warn!("Failed to inject cwasm into AOT cache: {}", e);
            } else {
                tracing::info!(
                    "Injected precompiled cwasm into AOT cache for {}",
                    wasm_hash
                );
            }
        }
    }
}

fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn calculate_hash(data: &[u8]) -> String {
    let hash = blake3::hash(data);
    hash.to_hex().to_string()
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn base64_decode(s: &str) -> Result<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|e| anyhow!("Base64 decode error: {}", e))
}

async fn calculate_dir_size(path: &Path) -> Result<u64> {
    let mut total = 0u64;

    if !path.exists() {
        return Ok(0);
    }

    let mut entries = tokio::fs::read_dir(path).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata = entry.metadata().await?;
        if metadata.is_file() {
            total += metadata.len();
        } else if metadata.is_dir() {
            total += Box::pin(calculate_dir_size(&entry.path())).await?;
        }
    }

    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash() {
        let data = b"test data";
        let hash = calculate_hash(data);
        assert!(!hash.is_empty());
    }

    #[test]
    fn test_sanitize() {
        assert_eq!(
            sanitize_filename("https://example.com"),
            "https___example_com"
        );
    }
}
