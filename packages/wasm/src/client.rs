//! Registry client for fetching, caching, and publishing WASM packages

use crate::{
    manifest::PackageManifest,
    registry::{
        CachedPackage, DownloadRequest, DownloadResponse, InstalledPackage, InstalledVersion,
        LocalRegistryState, PackageSource, PackageSummary, PackageVersion, PublishRequest,
        PublishResponse, RegistryConfig, RegistryEntry, RegistryIndex, SearchFilters,
        SearchResults,
    },
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::{collections::HashMap, path::{Path, PathBuf}, sync::Arc};
use tokio::sync::RwLock;

/// Registry client for managing WASM packages
#[derive(Clone)]
pub struct RegistryClient {
    config: RegistryConfig,
    state: Arc<RwLock<LocalRegistryState>>,
    http_client: reqwest::Client,
}

impl RegistryClient {
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

    /// Fetch package index from remote registry
    pub async fn fetch_index(&self, registry_url: &str) -> Result<RegistryIndex> {
        let url = format!("{}/index.json", registry_url);
        let response = self.http_client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "Failed to fetch registry index: {}",
                response.status()
            ));
        }

        let index: RegistryIndex = response.json().await?;

        let index_file = sanitize_filename(registry_url);
        let index_path = self
            .config
            .cache_dir
            .join(format!("{}.index.json", index_file));
        let data = serde_json::to_string_pretty(&index)?;
        tokio::fs::write(&index_path, data).await?;

        let mut state = self.state.write().await;
        state
            .index_refresh
            .insert(registry_url.to_string(), Utc::now());
        drop(state);
        self.save_state().await?;

        Ok(index)
    }

    /// Get cached index (offline-first)
    pub async fn get_index(&self) -> Result<RegistryIndex> {
        let registry_url = &self.config.default_registry;
        let index_file = sanitize_filename(registry_url);
        let index_path = self
            .config
            .cache_dir
            .join(format!("{}.index.json", index_file));

        let state = self.state.read().await;
        let last_refresh = state.index_refresh.get(registry_url).cloned();
        drop(state);

        let should_refresh = match last_refresh {
            Some(time) => {
                let age_hours = Utc::now().signed_duration_since(time).num_hours();
                age_hours >= self.config.cache_duration_hours as i64
            }
            None => true,
        };

        if !should_refresh && index_path.exists() {
            let data = tokio::fs::read_to_string(&index_path).await?;
            if let Ok(index) = serde_json::from_str::<RegistryIndex>(&data) {
                return Ok(index);
            }
        }

        match self.fetch_index(registry_url).await {
            Ok(index) => Ok(index),
            Err(e) => {
                if index_path.exists() {
                    tracing::warn!("Using stale index due to network error: {}", e);
                    let data = tokio::fs::read_to_string(&index_path).await?;
                    Ok(serde_json::from_str(&data)?)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Search packages with filters
    pub async fn search(&self, filters: &SearchFilters) -> Result<SearchResults> {
        let index = self.get_index().await?;

        let mut results: Vec<PackageSummary> = index
            .packages
            .into_iter()
            .filter(|pkg| {
                if let Some(query) = &filters.query {
                    let q = query.to_lowercase();
                    let name_match = pkg.name.to_lowercase().contains(&q);
                    let desc_match = pkg.description.to_lowercase().contains(&q);
                    let keyword_match = pkg.keywords.iter().any(|k| k.to_lowercase().contains(&q));
                    if !name_match && !desc_match && !keyword_match {
                        return false;
                    }
                }

                if filters.verified_only && !pkg.verified {
                    return false;
                }

                true
            })
            .collect();

        let total_count = results.len();

        match filters.sort_by {
            crate::registry::SortField::Downloads => {
                results.sort_by(|a, b| {
                    if filters.sort_desc {
                        b.download_count.cmp(&a.download_count)
                    } else {
                        a.download_count.cmp(&b.download_count)
                    }
                });
            }
            crate::registry::SortField::Name => {
                results.sort_by(|a, b| {
                    if filters.sort_desc {
                        b.name.cmp(&a.name)
                    } else {
                        a.name.cmp(&b.name)
                    }
                });
            }
            _ => {}
        }

        let results: Vec<PackageSummary> = results
            .into_iter()
            .skip(filters.offset)
            .take(filters.limit)
            .collect();

        Ok(SearchResults {
            packages: results,
            total_count,
            offset: filters.offset,
            limit: filters.limit,
        })
    }

    /// Download and cache a package
    pub async fn download_package(
        &self,
        package_id: &str,
        version: Option<&str>,
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
        };

        let url = format!("{}/download", self.config.default_registry);
        let response = self.http_client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            return Err(anyhow!("Failed to download package: {}", response.status()));
        }

        let download: DownloadResponse = response.json().await?;

        // Fetch WASM data - either from download_url or decode from base64
        let wasm_data = if let Some(download_url) = &download.download_url {
            // Download from CDN/signed URL
            let wasm_response = self.http_client.get(download_url).send().await?;
            if !wasm_response.status().is_success() {
                return Err(anyhow!(
                    "Failed to download WASM from CDN: {}",
                    wasm_response.status()
                ));
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

        let now = Utc::now();
        let installed_version = InstalledVersion {
            version: download.version.clone(),
            wasm_path: wasm_path.clone(),
            installed_at: now,
            manifest: download.manifest.clone(),
        };

        let mut state = self.state.write().await;
        if let Some(existing) = state.installed.get_mut(&download.package_id) {
            existing.version = download.version.clone();
            existing.installed_at = now;
            existing.wasm_path = wasm_path.clone();
            existing.manifest = download.manifest.clone();
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
    pub async fn install(&self, package_id: &str, version: Option<&str>) -> Result<CachedPackage> {
        self.download_package(package_id, version).await
    }

    pub async fn install_version(
        &self,
        package_id: &str,
        version: &str,
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

        self.download_package(package_id, Some(version)).await
    }

    pub async fn batch_install(
        &self,
        packages: &[(String, Option<String>)],
    ) -> Result<Vec<(String, Result<CachedPackage>)>> {
        let mut results = Vec::with_capacity(packages.len());
        for (package_id, version) in packages {
            let result = self.install(package_id, version.as_deref()).await;
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

    /// Check for updates to installed packages
    pub async fn check_updates(&self) -> Result<Vec<(String, String, String)>> {
        let state = self.state.read().await;
        let installed: Vec<_> = state
            .installed
            .iter()
            .map(|(k, v)| (k.clone(), v.version.clone()))
            .collect();
        drop(state);

        let index = self.get_index().await?;
        let mut updates = Vec::new();

        for (id, current_version) in installed {
            if let Some(pkg) = index.packages.iter().find(|p| p.id == id) {
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
            let error: serde_json::Value = response.json().await?;
            return Err(anyhow!(
                "Failed to publish: {}",
                error
                    .get("message")
                    .map(|e| e.to_string())
                    .unwrap_or_default()
            ));
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

        let security = installed.manifest.permissions.to_security_config();
        let loaded = engine.load_auto(&wasm_bytes).await?;

        let mut instance = loaded.instantiate(&engine, security.clone()).await?;
        let definitions = instance.call_get_nodes().await?;

        let nodes: Vec<crate::WasmNodeLogic> = definitions
            .into_iter()
            .map(|def| {
                crate::WasmNodeLogic::from_loaded_with_target(
                    loaded.clone(),
                    engine.clone(),
                    security.clone(),
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
        let security = iv.manifest.permissions.to_security_config();
        let loaded = engine.load_auto(&wasm_bytes).await?;
        let mut instance = loaded.instantiate(&engine, security.clone()).await?;
        let definitions = instance.call_get_nodes().await?;

        Ok(definitions
            .into_iter()
            .map(|def| {
                crate::WasmNodeLogic::from_loaded_with_target(
                    loaded.clone(),
                    engine.clone(),
                    security.clone(),
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
