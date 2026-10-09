//! Registry client for fetching, caching, and publishing WASM packages

use crate::{
    manifest::{PackageManifest, PackageWidgetEntry},
    registry::{
        CachedPackage, DownloadRequest, DownloadResponse, InstalledPackage, InstalledVersion,
        LocalRegistryState, PackageSource, PackageVersion, PublishRequest, PublishResponse,
        RegistryConfig, RegistryEntry, SearchFilters, SearchResults, OFFICIAL_REGISTRY_URL,
    },
    widget_bundle::{sha256_hex, widget_store_dir, WidgetBundleReader},
    widget_frame::is_valid_package_id,
};
use anyhow::{anyhow, Result};
use chrono::Utc;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, PoisonError,
    },
};
use tokio::sync::RwLock;

/// The packages whose files an install is replacing right now, by the name of
/// their directory, each with its id and the number of installs writing there.
type DirectoryClaims = Arc<Mutex<HashMap<String, (String, usize)>>>;

/// Holds a package's directory for one install; see
/// [`RegistryClient::claim_directory`].
struct DirectoryClaim {
    claims: DirectoryClaims,
    name: String,
}

impl Drop for DirectoryClaim {
    fn drop(&mut self) {
        let mut claims = self.claims.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, installs)) = claims.get_mut(&self.name) {
            *installs -= 1;
            if *installs == 0 {
                claims.remove(&self.name);
            }
        }
    }
}

/// Registry client for managing WASM packages
#[derive(Clone)]
pub struct RegistryClient {
    config: RegistryConfig,
    state: Arc<RwLock<LocalRegistryState>>,
    writing: DirectoryClaims,
    /// Held while the widget store is pruned, and from the unpacking of a
    /// bundle to the record that names it: a prune removes what no record
    /// names. Taken before the state lock, never while holding it.
    widget_store: Arc<tokio::sync::Mutex<()>>,
    http_client: reqwest::Client,
}

impl RegistryClient {
    fn target_platform_for_download() -> Option<String> {
        Some(crate::aot_cache::host_platform_key())
    }

    /// A precompiled artifact is native code that is loaded without the
    /// sandbox's validation, so only the registry Flow-Like runs may deliver
    /// one. Every other registry delivers the portable `.wasm`, which this
    /// device compiles itself.
    fn trusts_precompiled(registry_url: &str) -> bool {
        registry_url == OFFICIAL_REGISTRY_URL
    }

    fn source_trusts_precompiled(source: &PackageSource) -> bool {
        matches!(
            source,
            PackageSource::Remote { registry_url, .. } if Self::trusts_precompiled(registry_url)
        )
    }

    /// Whether an installed copy came from the registry this client talks to.
    /// Another registry may publish other bytes under the same id and version.
    pub fn from_current_registry(&self, installed: &InstalledPackage) -> bool {
        match &installed.source {
            PackageSource::Remote { registry_url, .. } => {
                registry_url == &self.config.default_registry
            }
            _ => true,
        }
    }

    /// Whether an installed copy answers an install request without asking
    /// the registry: the requested version from this registry, complete on
    /// disk and with its nodes.
    fn satisfies_install(&self, installed: &InstalledPackage, version: Option<&str>) -> bool {
        version.is_none_or(|version| version == installed.version)
            && self.from_current_registry(installed)
            && !installed.manifest.nodes_withheld()
            && self.installed_package_ready(installed)
    }

    /// A version string that is one plain path segment.
    fn is_safe_version(version: &str) -> bool {
        !version.is_empty()
            && version != "."
            && version != ".."
            && version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'+'))
    }

    /// How a package id or version names its directory where the file system
    /// ignores letter case (macOS, Windows) or trailing dots (Windows).
    fn directory_name(name: &str) -> String {
        name.trim_end_matches('.').to_ascii_lowercase()
    }

    /// Whether two different names lead to the same directory.
    fn shares_directory(name: &str, other: &str) -> bool {
        name != other && Self::directory_name(name) == Self::directory_name(other)
    }

    /// The installed package whose files `package_id` would overwrite.
    async fn directory_rival(&self, package_id: &str) -> Option<String> {
        let state = self.state.read().await;
        state
            .installed
            .keys()
            .find(|installed| Self::shares_directory(installed, package_id))
            .cloned()
    }

    fn shared_directory_error(package_id: &str, rival: &str) -> anyhow::Error {
        anyhow!(
            "Package '{}' cannot be installed next to '{}': both names lead to the same directory on this device",
            package_id,
            rival
        )
    }

    /// Holds the directory of `package_id` until the claim is dropped. Of two
    /// installs whose names share it, the one that claims second is refused,
    /// whether the first is still writing or has left its record.
    async fn claim_directory(&self, package_id: &str) -> Result<DirectoryClaim> {
        let name = Self::directory_name(package_id);
        let writing_rival = {
            let mut claims = self.writing.lock().unwrap_or_else(PoisonError::into_inner);
            match claims.get_mut(&name) {
                Some((holder, _)) if holder != package_id => Some(holder.clone()),
                Some((_, installs)) => {
                    *installs += 1;
                    None
                }
                None => {
                    claims.insert(name.clone(), (package_id.to_string(), 1));
                    None
                }
            }
        };
        if let Some(rival) = writing_rival {
            return Err(Self::shared_directory_error(package_id, &rival));
        }
        let claim = DirectoryClaim {
            claims: self.writing.clone(),
            name,
        };
        match self.directory_rival(package_id).await {
            Some(rival) => Err(Self::shared_directory_error(package_id, &rival)),
            None => Ok(claim),
        }
    }

    /// Clears the way for the files of `package_id`@`version`. When the node
    /// binary of a copy another registry installed is about to be replaced,
    /// that copy leaves the record first: should the install fail after the
    /// write, nothing still names those bytes as that registry's. A copy whose
    /// files stay as they are stays on record until the install succeeded. A
    /// held version whose name leads to the same directory is refused.
    async fn release_version_directory(
        &self,
        package_id: &str,
        version: &str,
        replaces_nodes: bool,
    ) -> Result<()> {
        let mut state = self.state.write().await;
        let Some(installed) = state.installed.get(package_id) else {
            return Ok(());
        };
        let same_directory = |held: &str| held == version || Self::shares_directory(held, version);
        if !self.from_current_registry(installed) {
            let holds_directory = same_directory(&installed.version)
                || installed.versions.keys().any(|held| same_directory(held));
            if !(replaces_nodes && holds_directory) {
                return Ok(());
            }
            state.installed.remove(package_id);
            drop(state);
            return self.save_state().await;
        }
        if let Some(held) = installed
            .versions
            .keys()
            .find(|held| Self::shares_directory(held, version))
        {
            anyhow::bail!(
                "Version {} of package '{}' cannot be installed next to {}: both names lead to the same directory on this device",
                version,
                package_id,
                held
            );
        }
        Ok(())
    }

    /// The directory this client keeps its packages in.
    pub fn cache_dir(&self) -> &Path {
        &self.config.cache_dir
    }

    /// Replace a file in one step, so a write that fails halfway leaves the
    /// earlier file under the name an installed entry points at.
    async fn write_replacing(path: &Path, bytes: &[u8]) -> Result<()> {
        static STAGED: AtomicU64 = AtomicU64::new(0);
        let staged = path.with_extension(format!(
            "{}-{}.part",
            std::process::id(),
            STAGED.fetch_add(1, Ordering::Relaxed)
        ));
        let written = async {
            tokio::fs::write(&staged, bytes).await?;
            tokio::fs::rename(&staged, path).await
        }
        .await;
        if written.is_err() {
            let _ = tokio::fs::remove_file(&staged).await;
        }
        Ok(written?)
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
            Err(error) => Some(format!(
                "Failed to read error response body: {}",
                error.without_url()
            )),
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
            writing: DirectoryClaims::default(),
            widget_store: Arc::default(),
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

    fn versioned_widget_bundle_path(&self, package_id: &str, version: &str) -> PathBuf {
        self.config
            .cache_dir
            .join("wasm")
            .join("nodes")
            .join(package_id)
            .join(version)
            .join("widgets.flwb")
    }

    /// Whether the manifest ships a WASM node artifact. Widgets-only packages
    /// declare widgets but carry no (or an empty) wasm path/hash.
    fn manifest_has_wasm(manifest: &PackageManifest) -> bool {
        manifest.widgets.is_empty()
            || manifest.wasm_hash.as_deref().is_some_and(|h| !h.is_empty())
            || manifest.wasm_path.as_deref().is_some_and(|p| !p.is_empty())
    }

    /// Verify a widget bundle against `expected_hash` (when given) and unpack
    /// it into the content-addressed widget store. Returns the bundle hash and
    /// the unpacked directory. Skips unpacking when the store dir already
    /// exists for that hash.
    fn verify_and_unpack_widget_bundle(
        cache_dir: &Path,
        package_id: &str,
        bundle_bytes: Vec<u8>,
        expected_hash: Option<&str>,
    ) -> Result<(String, PathBuf)> {
        let actual_hash = sha256_hex(&bundle_bytes);
        if let Some(expected) = expected_hash.filter(|h| !h.is_empty()) {
            if expected != actual_hash {
                return Err(anyhow!(
                    "Widget bundle hash mismatch for '{}': manifest declares {}, downloaded bundle is {}",
                    package_id,
                    expected,
                    actual_hash
                ));
            }
        }

        let dest = widget_store_dir(cache_dir, package_id, &actual_hash);
        if dest.is_dir() {
            return Ok((actual_hash, dest));
        }

        let mut reader = WidgetBundleReader::from_bytes(bundle_bytes)?;
        reader.unpack(&dest)?;
        Ok((actual_hash, dest))
    }

    async fn install_widget_bundle(
        &self,
        package_id: &str,
        bundle_bytes: Vec<u8>,
        expected_hash: Option<String>,
    ) -> Result<(String, PathBuf)> {
        let cache_dir = self.config.cache_dir.clone();
        let package_id = package_id.to_string();
        tokio::task::spawn_blocking(move || {
            Self::verify_and_unpack_widget_bundle(
                &cache_dir,
                &package_id,
                bundle_bytes,
                expected_hash.as_deref(),
            )
        })
        .await?
    }

    fn installed_version_ready(&self, package_id: &str, iv: &InstalledVersion) -> bool {
        let wasm_ready = !Self::manifest_has_wasm(&iv.manifest) || iv.wasm_path.exists();
        let widgets_ready = iv.manifest.widgets.is_empty()
            || iv
                .widget_bundle_hash
                .as_ref()
                .map(|h| widget_store_dir(&self.config.cache_dir, package_id, h).is_dir())
                .unwrap_or(false);
        wasm_ready && widgets_ready
    }

    fn installed_package_ready(&self, installed: &InstalledPackage) -> bool {
        let wasm_ready =
            !Self::manifest_has_wasm(&installed.manifest) || installed.wasm_path.exists();
        let widgets_ready = installed.manifest.widgets.is_empty() || {
            let hash = installed
                .versions
                .get(&installed.version)
                .and_then(|iv| iv.widget_bundle_hash.clone())
                .or_else(|| installed.manifest.widget_bundle_hash.clone());
            hash.map(|h| widget_store_dir(&self.config.cache_dir, &installed.id, &h).is_dir())
                .unwrap_or(false)
        };
        wasm_ready && widgets_ready
    }

    /// Remove unpacked widget-store directories for a package that are no
    /// longer referenced by any installed version. Best-effort.
    async fn prune_widget_store(&self, package_id: &str) {
        let _store = self.widget_store.lock().await;
        let referenced: std::collections::HashSet<String> = {
            let state = self.state.read().await;
            state
                .installed
                .get(package_id)
                .map(|pkg| {
                    let mut set: std::collections::HashSet<String> = pkg
                        .versions
                        .values()
                        .filter_map(|iv| iv.widget_bundle_hash.clone())
                        .collect();
                    if let Some(hash) = pkg
                        .manifest
                        .widget_bundle_hash
                        .clone()
                        .filter(|h| !h.is_empty())
                    {
                        set.insert(hash);
                    }
                    set
                })
                .unwrap_or_default()
        };

        let package_dir = self.config.cache_dir.join("widgets").join(package_id);
        let mut entries = match tokio::fs::read_dir(&package_dir).await {
            Ok(entries) => entries,
            Err(_) => return,
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name().to_string_lossy().to_string();
            if referenced.contains(&name) {
                continue;
            }
            if let Err(e) = tokio::fs::remove_dir_all(entry.path()).await {
                tracing::warn!(
                    "Failed to prune widget store dir {:?} for '{}': {}",
                    entry.path(),
                    package_id,
                    e
                );
            }
        }
        if referenced.is_empty() {
            let _ = tokio::fs::remove_dir(&package_dir).await;
        }
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

    /// Point searches, downloads and publishes at another registry, such as
    /// the hub the signed-in profile uses.
    pub fn set_default_registry(&mut self, registry_url: String) {
        self.config.default_registry = registry_url;
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
            if let Some(access) = filters.access {
                params.append_pair("owned_only", "true");
                params.append_pair("access", access.as_str());
            } else if include_own {
                params.append_pair("include_own", "true");
            }
            if let Some(ids) = &filters.ids {
                params.append_pair("ids", &ids.join(","));
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

    /// Versions accumulate only onto an entry from the same registry. Any other
    /// entry is replaced wholesale so a bundle never inherits another source.
    /// A widgets-only copy never displaces a complete one: it joins the
    /// versions, and the nodes this device holds stay the active copy.
    fn merge_registry_install(
        existing: Option<InstalledPackage>,
        fresh: InstalledPackage,
    ) -> InstalledPackage {
        let same_registry = |existing: &InstalledPackage| match (&existing.source, &fresh.source) {
            (
                PackageSource::Remote {
                    registry_url: existing_url,
                    ..
                },
                PackageSource::Remote {
                    registry_url: fresh_url,
                    ..
                },
            ) => existing_url == fresh_url,
            _ => false,
        };
        let Some(mut existing) = existing.filter(same_registry) else {
            return fresh;
        };
        let widgets_only = fresh.manifest.nodes_withheld();
        if !widgets_only || existing.manifest.nodes_withheld() {
            existing.version = fresh.version;
            existing.installed_at = fresh.installed_at;
            existing.wasm_path = fresh.wasm_path;
            existing.wasm_hash = fresh.wasm_hash;
            existing.manifest = fresh.manifest;
            existing.metadata = fresh.metadata;
        }
        for (version, installed) in fresh.versions {
            let complete = existing
                .versions
                .get(&version)
                .is_some_and(|held| !held.manifest.nodes_withheld());
            if !(widgets_only && complete) {
                existing.versions.insert(version, installed);
            }
        }
        existing
    }

    /// Fetch the exact portable node package for a deployment without changing
    /// the desktop's installed version or downloading host-specific code.
    /// With `app_id` the registry authorizes it through that project's licence.
    pub async fn export_package_version(
        &self,
        package_id: &str,
        version: &str,
        app_id: Option<&str>,
    ) -> Result<(PackageManifest, Vec<u8>)> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(120))
            .build()?;
        let request = DownloadRequest {
            package_id: package_id.to_owned(),
            version: Some(version.to_owned()),
            target_platform: None,
            app_id: app_id.map(String::from),
        };
        let mut request = client
            .post(format!("{}/download", self.config.default_registry))
            .json(&request);
        if let Some(token) = &self.config.auth_token {
            request = request.bearer_auth(token);
        }
        let response = request.send().await.map_err(reqwest::Error::without_url)?;
        // Registries may return the portable binary inline, so this response is a payload.
        let bytes = Self::export_response(response).await?;
        let download: DownloadResponse = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("Registry returned invalid deployment package metadata"))?;
        anyhow::ensure!(
            download.package_id == package_id
                && download.version == version
                && download.manifest.id == package_id
                && download.manifest.version == version,
            "Registry returned a different package version than the project pin"
        );
        anyhow::ensure!(
            !download.manifest.nodes_withheld(),
            "The registry kept the nodes of {}@{} back: deploying them needs access to the project's flows or to the package",
            package_id,
            version
        );
        download
            .manifest
            .validate()
            .map_err(|_| anyhow!("Registry returned an invalid package manifest"))?;
        let wasm = if let Some(url) = &download.download_url {
            let url =
                reqwest::Url::parse(url).map_err(|_| anyhow!("Invalid package download URL"))?;
            anyhow::ensure!(
                url.scheme() == "https"
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.fragment().is_none(),
                "Deployment package downloads require HTTPS without embedded credentials"
            );
            let response = client
                .get(url)
                .send()
                .await
                .map_err(reqwest::Error::without_url)?;
            Self::export_response(response).await?
        } else {
            base64_decode(&download.wasm_base64)
                .map_err(|_| anyhow!("Registry returned invalid portable WASM bytes"))?
        };
        anyhow::ensure!(
            wasm.starts_with(b"\0asm"),
            "Package must contain portable WASM bytes"
        );
        Ok((download.manifest, wasm))
    }

    async fn export_response(mut response: reqwest::Response) -> Result<Vec<u8>> {
        anyhow::ensure!(
            response.status().is_success(),
            "Deployment dependency request failed ({})",
            response.status()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(reqwest::Error::without_url)?
        {
            bytes.try_reserve(chunk.len())?;
            bytes.extend_from_slice(&chunk);
        }
        Ok(bytes)
    }

    /// Download and cache a package
    pub async fn download_package(
        &self,
        package_id: &str,
        version: Option<&str>,
        auth_token: Option<&str>,
    ) -> Result<CachedPackage> {
        self.download_package_for(package_id, version, auth_token, None)
            .await
    }

    /// Download and cache a package; with `app_id` the registry authorizes the
    /// download through that project's license instead of the caller's own access.
    async fn download_package_for(
        &self,
        package_id: &str,
        version: Option<&str>,
        auth_token: Option<&str>,
        app_id: Option<&str>,
    ) -> Result<CachedPackage> {
        let state = self.state.read().await;
        if let Some(installed) = state.installed.get(package_id) {
            if self.satisfies_install(installed, version) {
                let wasm_data = if Self::manifest_has_wasm(&installed.manifest) {
                    tokio::fs::read(&installed.wasm_path).await?
                } else {
                    Vec::new()
                };
                let installed_version = installed.versions.get(&installed.version);
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
                            widget_bundle_hash: installed_version
                                .and_then(|iv| iv.widget_bundle_hash.clone()),
                            widget_bundle_size: None,
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

        if let Some(rival) = self.directory_rival(package_id).await {
            return Err(Self::shared_directory_error(package_id, &rival));
        }

        let precompiled = Self::trusts_precompiled(&self.config.default_registry);
        let request = DownloadRequest {
            package_id: package_id.to_string(),
            version: version.map(String::from),
            target_platform: precompiled
                .then(Self::target_platform_for_download)
                .flatten(),
            app_id: app_id.map(String::from),
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
        // The answer names files on this device and the entry it replaces, so
        // it must be the package that was asked for.
        anyhow::ensure!(
            is_valid_package_id(package_id)
                && download.package_id == package_id
                && download.manifest.id == package_id
                && Self::is_safe_version(&download.version)
                && version.is_none_or(|requested| requested == download.version),
            "Registry returned a different or malformed package than {}@{}",
            package_id,
            version.unwrap_or("latest")
        );
        let has_wasm = Self::manifest_has_wasm(&download.manifest);

        // Fetch WASM data - either from download_url or decode from base64
        let wasm_data = if !has_wasm {
            Vec::new()
        } else if let Some(download_url) = &download.download_url {
            // Download from CDN/signed URL
            let wasm_response = self
                .http_client
                .get(download_url)
                .send()
                .await
                .map_err(reqwest::Error::without_url)?;
            if !wasm_response.status().is_success() {
                return Err(anyhow!(
                    "Failed to download WASM from CDN: {}",
                    wasm_response.status()
                ));
            }
            wasm_response
                .bytes()
                .await
                .map_err(reqwest::Error::without_url)?
                .to_vec()
        } else if !download.wasm_base64.is_empty() {
            // Fallback to base64 decoding
            base64_decode(&download.wasm_base64)?
        } else {
            return Err(anyhow!("No download URL or WASM data in response"));
        };

        // Everything the registry can still fail happens before a file of this
        // version is replaced. The bundle is verified and unpacked first: its
        // store directory is named by its hash, so it stands in for nothing.
        let mut widget_bundle: Option<(Vec<u8>, String)> = None;
        let mut widget_bundle_size: Option<u64> = None;
        if !download.manifest.widgets.is_empty() {
            let expected_hash = download
                .manifest
                .widget_bundle_hash
                .clone()
                .filter(|h| !h.is_empty())
                .ok_or_else(|| {
                    anyhow!(
                        "Package '{}' declares widgets but its manifest carries no widget_bundle_hash",
                        download.package_id
                    )
                })?;
            let bundle_url = download.widget_bundle_download_url.as_ref().ok_or_else(|| {
                anyhow!(
                    "Package '{}' declares widgets but the registry returned no widget bundle download URL",
                    download.package_id
                )
            })?;

            let bundle_response = self
                .http_client
                .get(bundle_url)
                .send()
                .await
                .map_err(reqwest::Error::without_url)?;
            if !bundle_response.status().is_success() {
                return Err(anyhow!(
                    "Failed to download widget bundle: {}",
                    bundle_response.status()
                ));
            }
            let bundle_bytes = bundle_response
                .bytes()
                .await
                .map_err(reqwest::Error::without_url)?
                .to_vec();
            widget_bundle_size = Some(bundle_bytes.len() as u64);

            let _store = self.widget_store.lock().await;
            let (hash, _store_dir) = self
                .install_widget_bundle(
                    &download.package_id,
                    bundle_bytes.clone(),
                    Some(expected_hash),
                )
                .await?;
            widget_bundle = Some((bundle_bytes, hash));
        }

        // Held until the record is written: the check before the request saw
        // only installs that had finished by then.
        let _writing = self.claim_directory(package_id).await?;
        self.release_version_directory(package_id, &download.version, has_wasm)
            .await?;

        let wasm_path = self.versioned_wasm_path(&download.package_id, &download.version);
        if let Some(parent) = wasm_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        if has_wasm {
            // A sidecar left by an earlier install belongs to other bytes, so
            // it goes before they are replaced.
            let _ = tokio::fs::remove_file(Self::cwasm_sidecar_path(&wasm_path)).await;
            Self::write_replacing(&wasm_path, &wasm_data).await?;

            // If the server provided a precompiled .cwasm, download and store it
            // alongside the raw .wasm so `load_nodes` can inject it into the AOT cache.
            if let Some(cwasm_url) = download.cwasm_download_url.as_ref().filter(|_| precompiled) {
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
        }

        let mut widget_bundle_path: Option<PathBuf> = None;
        let mut widget_bundle_hash: Option<String> = None;
        let mut store = None;
        if let Some((bundle_bytes, hash)) = widget_bundle {
            let bundle_path =
                self.versioned_widget_bundle_path(&download.package_id, &download.version);
            Self::write_replacing(&bundle_path, &bundle_bytes).await?;
            // No record names the unpacked bundle yet, so an install or
            // uninstall of this package that pruned the store since removed
            // it. It is put back, and no prune runs until the record is written.
            store = Some(self.widget_store.lock().await);
            self.install_widget_bundle(&download.package_id, bundle_bytes, Some(hash.clone()))
                .await?;
            widget_bundle_path = Some(bundle_path);
            widget_bundle_hash = Some(hash);
        }

        let now = Utc::now();
        let wasm_hash = has_wasm.then(|| calculate_hash(&wasm_data));
        let installed_version = InstalledVersion {
            version: download.version.clone(),
            wasm_path: wasm_path.clone(),
            installed_at: now,
            manifest: download.manifest.clone(),
            metadata: download.metadata.clone(),
            wasm_hash: wasm_hash.clone(),
            widget_bundle_path,
            widget_bundle_hash: widget_bundle_hash.clone(),
        };

        let fresh = InstalledPackage {
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
        let mut state = self.state.write().await;
        let existing = state.installed.remove(&download.package_id);
        state.installed.insert(
            download.package_id.clone(),
            Self::merge_registry_install(existing, fresh),
        );
        drop(state);
        drop(store);
        self.save_state().await?;
        self.prune_widget_store(&download.package_id).await;

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
                    widget_bundle_hash,
                    widget_bundle_size,
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

    /// Install the version a project pins, downloading through the project's
    /// licence so members need not hold the package themselves.
    pub async fn install_for_app(
        &self,
        package_id: &str,
        version: Option<&str>,
        auth_token: Option<&str>,
        app_id: &str,
    ) -> Result<CachedPackage> {
        self.download_package_for(package_id, version, auth_token, Some(app_id))
            .await
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
                if self.installed_version_ready(package_id, iv) {
                    let wasm_data = if Self::manifest_has_wasm(&iv.manifest) {
                        tokio::fs::read(&iv.wasm_path).await?
                    } else {
                        Vec::new()
                    };
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
            for iv in installed.versions.values() {
                if let Some(bundle_path) = &iv.widget_bundle_path {
                    // Only delete bundles inside our cache; local dev bundles
                    // live in the developer's project and must be preserved.
                    if bundle_path.starts_with(&self.config.cache_dir) && bundle_path.exists() {
                        let _ = tokio::fs::remove_file(bundle_path).await;
                    }
                }
            }
        }

        state.cache_metadata.remove(package_id);
        drop(state);
        self.save_state().await?;
        self.prune_widget_store(package_id).await;
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
                widget_bundle_hash: manifest.widget_bundle_hash.clone(),
                widget_bundle_size: None,
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
    ///
    /// When the manifest points at a local widget bundle
    /// (`manifest.widget_bundle_path`, resolved relative to the wasm file's
    /// directory) it is read and unpacked into the content-addressed widget
    /// store. On re-registration after a rebuild the bundle hash changes and
    /// the bundle is re-unpacked under the new hash; stale hashes are pruned.
    /// The bundle's widget entries and actual hash are written back into the
    /// stored manifest, which is where widget consumers read contracts from.
    pub async fn register_local_package(
        &self,
        wasm_path: &Path,
        mut manifest: PackageManifest,
    ) -> Result<InstalledPackage> {
        let _writing = self.claim_directory(&manifest.id).await?;
        let now = Utc::now();
        let wasm_hash = match tokio::fs::read(wasm_path).await {
            Ok(bytes) => Some(blake3::hash(&bytes).to_hex().to_string()),
            Err(_) => None,
        };

        let store = self.widget_store.lock().await;
        let (widget_bundle_path, widget_bundle_hash, bundle_widgets) = self
            .prepare_local_widget_bundle(wasm_path, &manifest)
            .await?;
        if !bundle_widgets.is_empty() {
            manifest.widgets = bundle_widgets;
        }
        if let Some(hash) = &widget_bundle_hash {
            manifest.widget_bundle_hash = Some(hash.clone());
        }

        let version_entry = InstalledVersion {
            version: manifest.version.clone(),
            wasm_path: wasm_path.to_path_buf(),
            installed_at: now,
            manifest: manifest.clone(),
            metadata: None,
            wasm_hash: wasm_hash.clone(),
            widget_bundle_path,
            widget_bundle_hash,
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
        drop(store);
        self.save_state().await?;
        self.prune_widget_store(&installed.id).await;

        Ok(installed)
    }

    /// Resolve, verify, and unpack the widget bundle of a local developer
    /// package. Returns `(bundle_path, bundle_hash, widget_entries)` when the
    /// manifest points at a bundle, `(None, None, [])` otherwise. The entries
    /// are derived from the bundle itself so manifests that only declare
    /// `widget_bundle_path` (the scaffolded `flow-like.toml` shape) still end
    /// up with typed widget contracts.
    async fn prepare_local_widget_bundle(
        &self,
        wasm_path: &Path,
        manifest: &PackageManifest,
    ) -> Result<(Option<PathBuf>, Option<String>, Vec<PackageWidgetEntry>)> {
        let declared_path = manifest
            .widget_bundle_path
            .as_deref()
            .filter(|p| !p.is_empty());

        let Some(rel_path) = declared_path else {
            if manifest.widgets.is_empty() {
                return Ok((None, None, Vec::new()));
            }
            return Err(anyhow!(
                "Package '{}' declares widgets but no widget_bundle_path in its manifest",
                manifest.id
            ));
        };

        let candidate = Path::new(rel_path);
        let bundle_path = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            wasm_path
                .parent()
                .unwrap_or_else(|| Path::new("."))
                .join(candidate)
        };

        let bundle_bytes = tokio::fs::read(&bundle_path).await.map_err(|e| {
            anyhow!(
                "Failed to read widget bundle for '{}' at {:?}: {}",
                manifest.id,
                bundle_path,
                e
            )
        })?;

        let widgets = {
            let bytes = bundle_bytes.clone();
            let declared = manifest.widgets.clone();
            let package_id = manifest.id.clone();
            tokio::task::spawn_blocking(move || -> Result<Vec<PackageWidgetEntry>> {
                let mut reader = WidgetBundleReader::from_bytes(bytes)?;
                reader.manifest_widgets(&declared)
            })
            .await?
            .map_err(|e| {
                anyhow!(
                    "Failed to read widget contracts from the bundle of '{}' at {:?}: {}",
                    package_id,
                    bundle_path,
                    e
                )
            })?
        };

        // Local dev: the manifest hash may be stale after a rebuild, so the
        // bundle is keyed by its actual content hash instead of being rejected.
        let (hash, _store_dir) = self
            .install_widget_bundle(&manifest.id, bundle_bytes, None)
            .await?;

        Ok((Some(bundle_path), Some(hash), widgets))
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
        self.prune_widget_store(package_id).await;
        Ok(true)
    }

    /// Clear all cached packages
    pub async fn clear_cache(&self) -> Result<()> {
        let packages_dir = self.config.cache_dir.join("packages");
        if packages_dir.exists() {
            tokio::fs::remove_dir_all(&packages_dir).await?;
            tokio::fs::create_dir_all(&packages_dir).await?;
        }

        let widgets_dir = self.config.cache_dir.join("widgets");
        if widgets_dir.exists() {
            tokio::fs::remove_dir_all(&widgets_dir).await?;
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
                    widget_bundle_hash: iv.widget_bundle_hash.clone(),
                    widget_bundle_size: None,
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

        if !Self::manifest_has_wasm(&installed.manifest) {
            return Ok(Vec::new());
        }

        let wasm_bytes = tokio::fs::read(&installed.wasm_path).await.map_err(|e| {
            anyhow!(
                "Failed to read WASM file at {:?}: {}",
                installed.wasm_path,
                e
            )
        })?;

        if Self::source_trusts_precompiled(&installed.source) {
            Self::inject_precompiled_if_available(&wasm_bytes, &installed.wasm_path, &engine);
        }

        let manifest_security: crate::WasmSecurityConfig =
            installed.manifest.permissions.to_security_config();
        let loaded = engine.load_auto(&wasm_bytes).await?;
        let wasm_hash = loaded.hash().to_string();

        let definitions = if let Some(cached) = engine.get_cached_definitions(&wasm_hash) {
            cached
        } else {
            let mut instance = loaded
                .instantiate(&engine, manifest_security.for_metadata())
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
                        .with_package_settings(&manifest_security);
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

        if !Self::manifest_has_wasm(&iv.manifest) {
            return Ok(Vec::new());
        }

        let wasm_bytes = tokio::fs::read(&iv.wasm_path).await?;
        let manifest_security: crate::WasmSecurityConfig =
            iv.manifest.permissions.to_security_config();

        if Self::source_trusts_precompiled(&installed.source) {
            Self::inject_precompiled_if_available(&wasm_bytes, &iv.wasm_path, &engine);
        }

        let loaded = engine.load_auto(&wasm_bytes).await?;
        let wasm_hash = loaded.hash().to_string();

        let definitions = if let Some(cached) = engine.get_cached_definitions(&wasm_hash) {
            cached
        } else {
            let mut instance = loaded
                .instantiate(&engine, manifest_security.for_metadata())
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
                        .with_package_settings(&manifest_security);
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
        let cwasm_response = self
            .http_client
            .get(cwasm_url)
            .send()
            .await
            .map_err(reqwest::Error::without_url)?;
        if !cwasm_response.status().is_success() {
            return Err(anyhow!(
                "Failed to download cwasm: {}",
                cwasm_response.status()
            ));
        }
        let cwasm_bytes = cwasm_response
            .bytes()
            .await
            .map_err(reqwest::Error::without_url)?
            .to_vec();

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

#[cfg_attr(not(test), allow(dead_code))]
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
    use crate::widget::WidgetContract;
    use crate::widget_bundle::{BuilderWidget, WidgetBundleBuilder};

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

    fn build_test_bundle(package_id: &str, widget_id: &str, body: &str) -> (Vec<u8>, String) {
        WidgetBundleBuilder::new(package_id, "1.0.0")
            .created_at("2026-07-31T00:00:00Z")
            .add_widget(BuilderWidget {
                id: widget_id.to_string(),
                name: widget_id.to_string(),
                description: "test widget".into(),
                framework: Some("vanilla".into()),
                entry_html: format!("<html><body>{}</body></html>", body).into_bytes(),
                contract: WidgetContract::new(widget_id),
                assets: vec![],
                thumbnail: None,
            })
            .build()
            .unwrap()
    }

    fn widgets_only_manifest(package_id: &str, widget_id: &str) -> PackageManifest {
        let mut manifest = PackageManifest::new(package_id, "Test Widgets", "1.0.0", "widgets");
        manifest.widgets.push(crate::manifest::PackageWidgetEntry {
            id: widget_id.to_string(),
            name: widget_id.to_string(),
            description: "test widget".into(),
            icon: None,
            thumbnail: None,
            contract: WidgetContract::new(widget_id),
            keywords: vec![],
            network: None,
        });
        manifest.widget_bundle_path = Some("widgets.flwb".into());
        manifest
    }

    fn test_client(cache_dir: &Path) -> RegistryClient {
        let config = RegistryConfig {
            cache_dir: cache_dir.to_path_buf(),
            ..Default::default()
        };
        RegistryClient::new(config).unwrap()
    }

    fn search_pairs(
        filters: &SearchFilters,
        include_own: bool,
    ) -> std::collections::HashMap<String, String> {
        let temp = tempfile::tempdir().unwrap();
        let url = test_client(temp.path()).build_search_url(filters, include_own);
        reqwest::Url::parse(&url)
            .unwrap()
            .query_pairs()
            .into_owned()
            .collect()
    }

    #[test]
    fn test_search_url_forwards_access_as_owned_only() {
        let pairs = search_pairs(
            &SearchFilters {
                access: Some(crate::registry::PackageAccessFilter::Maintainer),
                ids: Some(vec!["a".into(), "b".into()]),
                ..Default::default()
            },
            true,
        );
        assert_eq!(pairs.get("access").map(String::as_str), Some("maintainer"));
        assert_eq!(pairs.get("owned_only").map(String::as_str), Some("true"));
        assert_eq!(pairs.get("ids").map(String::as_str), Some("a,b"));
        assert!(!pairs.contains_key("include_own"));
    }

    #[test]
    fn test_search_url_without_access_keeps_include_own() {
        let pairs = search_pairs(&SearchFilters::default(), true);
        assert_eq!(pairs.get("include_own").map(String::as_str), Some("true"));
        assert!(!pairs.contains_key("access"));
        assert!(!pairs.contains_key("owned_only"));
        assert!(!pairs.contains_key("ids"));
    }

    #[test]
    fn test_verify_and_unpack_rejects_hash_mismatch() {
        let temp = tempfile::tempdir().unwrap();
        let (bytes, _hash) = build_test_bundle("com.example.tamper", "kpi-card", "v1");

        let err = RegistryClient::verify_and_unpack_widget_bundle(
            temp.path(),
            "com.example.tamper",
            bytes,
            Some("deadbeef"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("hash mismatch"));
        assert!(!temp.path().join("widgets").exists());
    }

    #[test]
    fn test_verify_and_unpack_rejects_tampered_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let (mut bytes, _hash) = build_test_bundle("com.example.tamper", "kpi-card", "v1");

        // Flip a byte in the middle of the archive; pass the recomputed
        // whole-file hash so the per-entry verification has to catch it.
        let mid = bytes.len() / 2;
        bytes[mid] ^= 0xff;
        let tampered_hash = sha256_hex(&bytes);

        let result = RegistryClient::verify_and_unpack_widget_bundle(
            temp.path(),
            "com.example.tamper",
            bytes,
            Some(&tampered_hash),
        );
        assert!(result.is_err());
        assert!(
            !widget_store_dir(temp.path(), "com.example.tamper", &tampered_hash).exists(),
            "tampered bundle must not land in the widget store"
        );
    }

    #[test]
    fn test_verify_and_unpack_success() {
        let temp = tempfile::tempdir().unwrap();
        let (bytes, hash) = build_test_bundle("com.example.ok", "kpi-card", "v1");

        let (actual, dest) = RegistryClient::verify_and_unpack_widget_bundle(
            temp.path(),
            "com.example.ok",
            bytes,
            Some(&hash),
        )
        .unwrap();
        assert_eq!(actual, hash);
        assert_eq!(dest, widget_store_dir(temp.path(), "com.example.ok", &hash));
        assert!(dest.join("bundle.json").exists());
        assert!(dest.join("widgets/kpi-card/index.html").exists());
        assert!(dest.join("widgets/kpi-card/contract.json").exists());
    }

    #[tokio::test]
    async fn test_widgets_only_local_package_install_reload_and_gc() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let project_dir = temp.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let client = test_client(&cache_dir);
        client.init().await.unwrap();

        let package_id = "com.example.widgetsonly";
        let (bundle_v1, hash_v1) = build_test_bundle(package_id, "kpi-card", "v1");
        std::fs::write(project_dir.join("widgets.flwb"), &bundle_v1).unwrap();

        let manifest = widgets_only_manifest(package_id, "kpi-card");
        // Widgets-only: node.wasm does not exist in the project.
        let wasm_path = project_dir.join("node.wasm");
        let installed = client
            .register_local_package(&wasm_path, manifest.clone())
            .await
            .unwrap();

        let iv = installed.versions.get("1.0.0").unwrap();
        assert_eq!(iv.widget_bundle_hash.as_deref(), Some(hash_v1.as_str()));
        assert_eq!(
            iv.widget_bundle_path.as_deref(),
            Some(project_dir.join("widgets.flwb").as_path())
        );
        let store_v1 = widget_store_dir(&cache_dir, package_id, &hash_v1);
        assert!(store_v1.join("widgets/kpi-card/index.html").exists());

        // Reload after a rebuild: changed bundle re-unpacks under the new
        // hash and the stale hash dir is pruned.
        let (bundle_v2, hash_v2) = build_test_bundle(package_id, "kpi-card", "v2");
        assert_ne!(hash_v1, hash_v2);
        std::fs::write(project_dir.join("widgets.flwb"), &bundle_v2).unwrap();

        let installed = client
            .register_local_package(&wasm_path, manifest)
            .await
            .unwrap();
        let iv = installed.versions.get("1.0.0").unwrap();
        assert_eq!(iv.widget_bundle_hash.as_deref(), Some(hash_v2.as_str()));
        let store_v2 = widget_store_dir(&cache_dir, package_id, &hash_v2);
        assert!(store_v2.join("widgets/kpi-card/index.html").exists());
        assert!(
            !store_v1.exists(),
            "stale widget store dir must be pruned on reload"
        );

        // Uninstall removes the package's widget store entirely.
        client.uninstall(package_id).await.unwrap();
        assert!(!cache_dir.join("widgets").join(package_id).exists());
        // The developer's bundle file is preserved.
        assert!(project_dir.join("widgets.flwb").exists());
    }

    #[tokio::test]
    async fn test_local_package_backfills_widgets_from_bundle() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let project_dir = temp.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let client = test_client(&cache_dir);
        client.init().await.unwrap();

        let package_id = "com.example.backfill";
        let (bundle, hash) = build_test_bundle(package_id, "kpi-card", "v1");
        std::fs::write(project_dir.join("widgets.flwb"), &bundle).unwrap();

        // Scaffolded `flow-like.toml` shape: a bundle path, no [[widgets]].
        let mut manifest = PackageManifest::new(package_id, "Backfill", "1.0.0", "widgets");
        manifest.widget_bundle_path = Some("widgets.flwb".into());

        let installed = client
            .register_local_package(&project_dir.join("node.wasm"), manifest)
            .await
            .unwrap();

        assert_eq!(installed.manifest.widgets.len(), 1);
        let widget = &installed.manifest.widgets[0];
        assert_eq!(widget.id, "kpi-card");
        assert_eq!(widget.contract.id, "kpi-card");
        assert_eq!(
            installed.manifest.widget_bundle_hash.as_deref(),
            Some(hash.as_str())
        );
        assert_eq!(
            installed
                .versions
                .get("1.0.0")
                .unwrap()
                .manifest
                .widgets
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn test_local_package_missing_declared_bundle_fails() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let project_dir = temp.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let client = test_client(&cache_dir);
        client.init().await.unwrap();

        let manifest = widgets_only_manifest("com.example.missing", "kpi-card");
        let err = client
            .register_local_package(&project_dir.join("node.wasm"), manifest)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to read widget bundle"));
    }

    #[tokio::test]
    async fn test_widgets_only_state_roundtrip_and_ready_checks() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let project_dir = temp.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let package_id = "com.example.ready";
        let (bundle, hash) = build_test_bundle(package_id, "kpi-card", "v1");
        std::fs::write(project_dir.join("widgets.flwb"), &bundle).unwrap();

        {
            let client = test_client(&cache_dir);
            client.init().await.unwrap();
            client
                .register_local_package(
                    &project_dir.join("node.wasm"),
                    widgets_only_manifest(package_id, "kpi-card"),
                )
                .await
                .unwrap();
        }

        // Fresh client loads the persisted state and considers the package ready.
        let client = test_client(&cache_dir);
        client.init().await.unwrap();
        let installed = client.get_installed(package_id).await.unwrap();
        assert!(!RegistryClient::manifest_has_wasm(&installed.manifest));
        assert!(client.installed_package_ready(&installed));
        let iv = installed.versions.get("1.0.0").unwrap();
        assert!(client.installed_version_ready(package_id, iv));

        // Wiping the unpacked store makes it not-ready (forces re-unpack path).
        std::fs::remove_dir_all(widget_store_dir(&cache_dir, package_id, &hash)).unwrap();
        assert!(!client.installed_package_ready(&installed));
    }

    struct MockRelease {
        manifest: PackageManifest,
        bundle: Vec<u8>,
    }

    fn registry_release(package_id: &str, version: &str, body: &str) -> (MockRelease, String) {
        let (bundle, hash) = build_test_bundle(package_id, "live-map", body);
        let mut manifest = widgets_only_manifest(package_id, "live-map");
        manifest.version = version.to_string();
        manifest.widget_bundle_path = None;
        manifest.widget_bundle_hash = Some(hash.clone());
        (MockRelease { manifest, bundle }, hash)
    }

    async fn spawn_mock_registry(releases: Vec<MockRelease>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let mut routes: HashMap<String, Vec<u8>> = HashMap::new();
        for release in releases {
            let version = release.manifest.version.clone();
            let bundle_path = format!("/bundles/{}.flwb", version);
            let download = DownloadResponse {
                package_id: release.manifest.id.clone(),
                version: version.clone(),
                wasm_base64: String::new(),
                download_url: None,
                manifest: release.manifest,
                metadata: None,
                cwasm_download_url: None,
                cwasm_checksum: None,
                widget_bundle_download_url: Some(format!("{}{}", base, bundle_path)),
            };
            routes.insert(
                format!("/download@{}", version),
                serde_json::to_vec(&download).unwrap(),
            );
            routes.insert(bundle_path, release.bundle);
        }
        let routes = Arc::new(routes);
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve_mock_request(stream, routes.clone()));
            }
        });
        base
    }

    async fn serve_mock_request(
        mut stream: tokio::net::TcpStream,
        routes: Arc<HashMap<String, Vec<u8>>>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        let header_end = loop {
            let read = stream.read(&mut chunk).await.unwrap_or(0);
            if read == 0 {
                return;
            }
            buf.extend_from_slice(&chunk[..read]);
            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                break pos + 4;
            }
        };
        let head = String::from_utf8_lossy(&buf[..header_end]).to_string();
        let content_length = head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.trim().parse::<usize>().ok())
            .unwrap_or(0);
        while buf.len() < header_end + content_length {
            let read = stream.read(&mut chunk).await.unwrap_or(0);
            if read == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..read]);
        }

        let path = head.split_whitespace().nth(1).unwrap_or_default();
        let key = if path == "/download" {
            serde_json::from_slice::<DownloadRequest>(&buf[header_end..])
                .ok()
                .and_then(|request| request.version)
                .map(|version| format!("/download@{}", version))
                .unwrap_or_default()
        } else {
            path.to_string()
        };
        let (status, body) = match routes.get(&key) {
            Some(body) => ("200 OK", body.clone()),
            None => ("404 Not Found", Vec::new()),
        };
        let header = format!(
            "HTTP/1.1 {}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
            status,
            body.len()
        );
        let _ = stream.write_all(header.as_bytes()).await;
        let _ = stream.write_all(&body).await;
        let _ = stream.shutdown().await;
    }

    fn registry_client(cache_dir: &Path, registry: &str) -> RegistryClient {
        RegistryClient::new(RegistryConfig {
            default_registry: registry.to_string(),
            cache_dir: cache_dir.to_path_buf(),
            ..Default::default()
        })
        .unwrap()
    }

    #[tokio::test]
    async fn deployment_export_reads_payloads_above_the_former_file_limit() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let size = 64 * 1024 * 1024 + 1;
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let mut chunk = [0; 4096];
                let length = stream.read(&mut chunk).await.unwrap();
                assert!(length > 0);
                request.extend_from_slice(&chunk[..length]);
            }
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {size}\r\nConnection: close\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            let chunk = vec![7; 64 * 1024];
            let mut remaining = size;
            while remaining > 0 {
                let length = remaining.min(chunk.len());
                stream.write_all(&chunk[..length]).await.unwrap();
                remaining -= length;
            }
        });
        let response = reqwest::get(address).await.unwrap();
        let bytes = RegistryClient::export_response(response).await.unwrap();
        assert_eq!(bytes.len(), size);
        assert!(bytes.iter().all(|byte| *byte == 7));
        server.await.unwrap();
    }

    #[tokio::test]
    async fn deployment_export_authenticates_exact_pins_and_checks_portable_bytes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (returned_version, wasm, accepted) in [
            ("1.0.0", vec![0, b'a', b's', b'm', 1, 0, 0, 0], true),
            ("2.0.0", vec![0, b'a', b's', b'm', 1, 0, 0, 0], false),
            ("1.0.0", vec![0; 32], false),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let package_id = "com.example.deployment";
            let response = serde_json::to_vec(&DownloadResponse {
                package_id: package_id.into(),
                version: returned_version.into(),
                manifest: PackageManifest::new(
                    package_id,
                    "Deployment",
                    returned_version,
                    "Deployment test",
                ),
                wasm_base64: base64_encode(&wasm),
                download_url: None,
                metadata: None,
                cwasm_download_url: None,
                cwasm_checksum: None,
                widget_bundle_download_url: None,
            })
            .unwrap();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0; 4096];
                    let length = stream.read(&mut chunk).await.unwrap();
                    assert!(length > 0);
                    request.extend_from_slice(&chunk[..length]);
                    assert!(request.len() <= 8192);
                    if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]);
                        let length: usize = head
                            .lines()
                            .find_map(|line| {
                                line.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|size| size.trim().parse().unwrap())
                            })
                            .unwrap();
                        if request.len() < end + 4 + length {
                            continue;
                        }
                        assert!(head
                            .to_ascii_lowercase()
                            .contains("authorization: bearer selected-account"));
                        let body: DownloadRequest =
                            serde_json::from_slice(&request[end + 4..]).unwrap();
                        assert_eq!(body.package_id, package_id);
                        assert_eq!(body.version.as_deref(), Some("1.0.0"));
                        assert_eq!(body.target_platform, None);
                        break;
                    }
                }
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            response.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                stream.write_all(&response).await.unwrap();
            });
            let temporary = tempfile::tempdir().unwrap();
            let mut client = registry_client(temporary.path(), &base);
            client.set_auth_token(Some("selected-account".into()));
            let result = client
                .export_package_version(package_id, "1.0.0", None)
                .await;
            assert_eq!(result.is_ok(), accepted, "{result:?}");
            server.await.unwrap();
            assert!(client.list_installed().await.unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn test_registry_install_replaces_local_entry() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let project_dir = temp.path().join("project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let package_id = "com.acme.maps";
        let (local_bundle, local_hash) = build_test_bundle(package_id, "live-map", "local");
        std::fs::write(project_dir.join("widgets.flwb"), &local_bundle).unwrap();

        let (release, remote_hash) = registry_release(package_id, "2.0.0", "registry");
        assert_ne!(local_hash, remote_hash);
        let registry = spawn_mock_registry(vec![release]).await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        client
            .register_local_package(
                &project_dir.join("node.wasm"),
                widgets_only_manifest(package_id, "live-map"),
            )
            .await
            .unwrap();

        client
            .install(package_id, Some("2.0.0"), None)
            .await
            .unwrap();

        let assert_registry_entry = |installed: &InstalledPackage| {
            match &installed.source {
                PackageSource::Remote { registry_url, .. } => assert_eq!(registry_url, &registry),
                other => panic!("registry install kept a non-registry source: {:?}", other),
            }
            assert_eq!(installed.version, "2.0.0");
            assert_eq!(
                installed.versions.keys().collect::<Vec<_>>(),
                vec!["2.0.0"],
                "registry install must not inherit local versions"
            );
            assert!(installed
                .versions
                .values()
                .all(|iv| iv.widget_bundle_hash.as_deref() != Some(local_hash.as_str())));
            assert_eq!(
                installed.manifest.widget_bundle_hash.as_deref(),
                Some(remote_hash.as_str())
            );
        };

        assert_registry_entry(&client.get_installed(package_id).await.unwrap());
        assert!(widget_store_dir(&cache_dir, package_id, &remote_hash).is_dir());
        assert!(!widget_store_dir(&cache_dir, package_id, &local_hash).exists());
        assert!(project_dir.join("widgets.flwb").exists());

        let reloaded = registry_client(&cache_dir, &registry);
        reloaded.init().await.unwrap();
        assert_registry_entry(&reloaded.get_installed(package_id).await.unwrap());
    }

    #[tokio::test]
    async fn test_registry_install_merges_onto_registry_entry() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");

        let package_id = "com.acme.maps";
        let (release_v1, hash_v1) = registry_release(package_id, "1.0.0", "v1");
        let (release_v2, hash_v2) = registry_release(package_id, "2.0.0", "v2");
        let registry = spawn_mock_registry(vec![release_v1, release_v2]).await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        client
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();
        client
            .install(package_id, Some("2.0.0"), None)
            .await
            .unwrap();

        let installed = client.get_installed(package_id).await.unwrap();
        assert!(matches!(
            &installed.source,
            PackageSource::Remote { registry_url, .. } if registry_url == &registry
        ));
        assert_eq!(installed.version, "2.0.0");
        assert_eq!(
            installed
                .versions
                .get("1.0.0")
                .and_then(|iv| iv.widget_bundle_hash.as_deref()),
            Some(hash_v1.as_str())
        );
        assert_eq!(
            installed
                .versions
                .get("2.0.0")
                .and_then(|iv| iv.widget_bundle_hash.as_deref()),
            Some(hash_v2.as_str())
        );
        assert!(widget_store_dir(&cache_dir, package_id, &hash_v1).is_dir());
        assert!(widget_store_dir(&cache_dir, package_id, &hash_v2).is_dir());
    }

    async fn spawn_mock_routes(build: impl FnOnce(&str) -> HashMap<String, Vec<u8>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(build(&base));
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::spawn(serve_mock_request(stream, routes.clone()));
            }
        });
        base
    }

    /// A registry answer for the manifest's version with the node binary inline.
    fn inline_answer(manifest: PackageManifest, wasm: &[u8]) -> DownloadResponse {
        DownloadResponse {
            package_id: manifest.id.clone(),
            version: manifest.version.clone(),
            wasm_base64: base64_encode(wasm),
            download_url: None,
            manifest,
            metadata: None,
            cwasm_download_url: None,
            cwasm_checksum: None,
            widget_bundle_download_url: None,
        }
    }

    #[tokio::test]
    async fn test_install_rejects_another_package_than_requested() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let (release, _) = registry_release("com.acme.other", "1.0.0", "other");
        let registry = spawn_mock_registry(vec![release]).await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        let error = client
            .install("com.acme.maps", Some("1.0.0"), None)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("different or malformed package"),
            "{error}"
        );
        assert!(client.list_installed().await.unwrap().is_empty());
        let nodes = cache_dir.join("wasm").join("nodes");
        assert!(!nodes.join("com.acme.other").exists());
        assert!(!nodes.join("com.acme.maps").exists());
        assert!(!cache_dir.join("widgets").exists());
    }

    #[tokio::test]
    async fn test_install_rejects_a_version_that_is_not_a_plain_segment() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.maps";
        let (mut release, _) = registry_release(package_id, "1.0.0", "v1");
        release.manifest.version = "../../escape".to_string();
        let registry = spawn_mock_routes(|_| {
            HashMap::from([(
                String::new(),
                serde_json::to_vec(&inline_answer(release.manifest, &[])).unwrap(),
            )])
        })
        .await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        let error = client.install(package_id, None, None).await.unwrap_err();
        assert!(
            error.to_string().contains("different or malformed package"),
            "{error}"
        );
        assert!(client.list_installed().await.unwrap().is_empty());
        assert!(!cache_dir.join("wasm").join("escape").exists());
        for (version, safe) in [
            ("1.2.0", true),
            ("1.2.0-rc.1+build.5", true),
            ("", false),
            ("..", false),
            ("1.0/../x", false),
            ("1.0\\x", false),
        ] {
            assert_eq!(
                RegistryClient::is_safe_version(version),
                safe,
                "{version:?}"
            );
        }
    }

    #[tokio::test]
    async fn test_install_does_not_reuse_a_copy_from_another_registry() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.maps";
        let (release_a, hash_a) = registry_release(package_id, "1.0.0", "registry a");
        let (release_b, hash_b) = registry_release(package_id, "1.0.0", "registry b");
        assert_ne!(hash_a, hash_b);
        let registry_a = spawn_mock_registry(vec![release_a]).await;
        let registry_b = spawn_mock_registry(vec![release_b]).await;

        let client_a = registry_client(&cache_dir, &registry_a);
        client_a.init().await.unwrap();
        client_a
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();
        let from_a = client_a.get_installed(package_id).await.unwrap();
        assert!(client_a.satisfies_install(&from_a, Some("1.0.0")));
        assert!(client_a.satisfies_install(&from_a, None));
        assert!(!client_a.satisfies_install(&from_a, Some("2.0.0")));

        let mut client_b = registry_client(&cache_dir, &registry_a);
        client_b.init().await.unwrap();
        client_b.set_default_registry(registry_b.clone());
        assert!(!client_b.satisfies_install(&from_a, Some("1.0.0")));
        client_b
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();
        let from_b = client_b.get_installed(package_id).await.unwrap();
        assert!(matches!(
            &from_b.source,
            PackageSource::Remote { registry_url, .. } if registry_url == &registry_b
        ));
        assert_eq!(
            from_b.manifest.widget_bundle_hash.as_deref(),
            Some(hash_b.as_str())
        );
        assert!(widget_store_dir(&cache_dir, package_id, &hash_b).is_dir());
    }

    #[tokio::test]
    async fn test_a_copy_without_its_nodes_asks_the_registry_again() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.maps";
        let (mut release, hash) = registry_release(package_id, "1.0.0", "widgets");
        release.manifest.wasm_path = Some("node.wasm".into());
        release.manifest.wasm_hash = Some("abc".into());
        release.manifest.withhold_nodes();
        assert!(release.manifest.nodes_withheld());
        assert!(!RegistryClient::manifest_has_wasm(&release.manifest));
        let registry = spawn_mock_registry(vec![release]).await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        client
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();
        let installed = client.get_installed(package_id).await.unwrap();
        assert!(widget_store_dir(&cache_dir, package_id, &hash).is_dir());
        assert!(client.installed_package_ready(&installed));
        assert!(
            !client.satisfies_install(&installed, Some("1.0.0")),
            "a widgets-only view must not stand in for the full package"
        );
    }

    #[test]
    fn test_precompiled_code_is_trusted_from_the_official_registry_only() {
        assert!(RegistryClient::trusts_precompiled(OFFICIAL_REGISTRY_URL));
        assert!(!RegistryClient::trusts_precompiled(
            "https://hub.example.org/api/v1/registry"
        ));
        assert!(RegistryClient::source_trusts_precompiled(&remote_source(
            OFFICIAL_REGISTRY_URL
        )));
        assert!(!RegistryClient::source_trusts_precompiled(&remote_source(
            "https://hub.example.org/api/v1/registry"
        )));
        assert!(!RegistryClient::source_trusts_precompiled(
            &PackageSource::Local {
                path: PathBuf::from("/dev/node.wasm")
            }
        ));
    }

    #[tokio::test]
    async fn test_precompiled_code_of_another_registry_is_neither_stored_nor_kept() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.nodes";
        let wasm = vec![0, b'a', b's', b'm', 1, 0, 0, 0];
        let precompiled = b"native code".to_vec();
        let checksum = blake3::hash(&precompiled).to_hex().to_string();
        let registry = spawn_mock_routes(|base| {
            HashMap::from([
                (
                    "/download@1.0.0".to_string(),
                    serde_json::to_vec(&DownloadResponse {
                        cwasm_download_url: Some(format!("{base}/precompiled.cwasm")),
                        cwasm_checksum: Some(checksum),
                        ..inline_answer(
                            PackageManifest::new(package_id, "Nodes", "1.0.0", "nodes"),
                            &wasm,
                        )
                    })
                    .unwrap(),
                ),
                ("/precompiled.cwasm".to_string(), precompiled),
            ])
        })
        .await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        let wasm_path = client.versioned_wasm_path(package_id, "1.0.0");
        let sidecar = RegistryClient::cwasm_sidecar_path(&wasm_path);
        std::fs::create_dir_all(wasm_path.parent().unwrap()).unwrap();
        std::fs::write(&sidecar, b"left by an earlier install").unwrap();

        client
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();
        assert!(wasm_path.exists());
        assert!(
            !sidecar.exists(),
            "another registry's precompiled code must not be stored, and a stale sidecar must go"
        );
    }

    fn installed_entry(
        version: &str,
        bundle_hash: &str,
        source: PackageSource,
    ) -> InstalledPackage {
        let mut manifest = widgets_only_manifest("com.acme.maps", "live-map");
        manifest.version = version.to_string();
        manifest.widget_bundle_hash = Some(bundle_hash.to_string());
        let installed_version = InstalledVersion {
            version: version.to_string(),
            wasm_path: PathBuf::new(),
            installed_at: Utc::now(),
            manifest: manifest.clone(),
            metadata: None,
            wasm_hash: None,
            widget_bundle_path: None,
            widget_bundle_hash: Some(bundle_hash.to_string()),
        };
        InstalledPackage {
            id: manifest.id.clone(),
            version: version.to_string(),
            source,
            installed_at: Utc::now(),
            wasm_path: PathBuf::new(),
            manifest,
            versions: HashMap::from([(version.to_string(), installed_version)]),
            metadata: None,
            wasm_hash: None,
        }
    }

    fn remote_source(registry_url: &str) -> PackageSource {
        PackageSource::Remote {
            registry_url: registry_url.to_string(),
            download_url: String::new(),
        }
    }

    #[test]
    fn test_merge_registry_install_replaces_foreign_entries() {
        let foreign_sources = [
            PackageSource::Local {
                path: PathBuf::from("/project/node.wasm"),
            },
            PackageSource::Embedded {
                data: vec![1, 2, 3],
            },
            remote_source("https://other-registry.example"),
        ];
        for source in foreign_sources {
            let merged = RegistryClient::merge_registry_install(
                Some(installed_entry("1.0.0", "foreign-hash", source.clone())),
                installed_entry(
                    "2.0.0",
                    "fresh-hash",
                    remote_source("https://registry.example"),
                ),
            );
            assert!(
                matches!(
                    &merged.source,
                    PackageSource::Remote { registry_url, .. } if registry_url == "https://registry.example"
                ),
                "entry from {:?} must be replaced, got {:?}",
                source,
                merged.source
            );
            assert_eq!(merged.versions.keys().collect::<Vec<_>>(), vec!["2.0.0"]);
            assert!(merged
                .versions
                .values()
                .all(|iv| iv.widget_bundle_hash.as_deref() == Some("fresh-hash")));
        }
    }

    #[test]
    fn test_merge_registry_install_merges_same_registry_entry() {
        let merged = RegistryClient::merge_registry_install(
            Some(installed_entry(
                "1.0.0",
                "hash-v1",
                remote_source("https://registry.example"),
            )),
            installed_entry(
                "2.0.0",
                "hash-v2",
                remote_source("https://registry.example"),
            ),
        );
        assert_eq!(merged.version, "2.0.0");
        assert_eq!(merged.manifest.version, "2.0.0");
        let mut versions = merged.versions.keys().cloned().collect::<Vec<_>>();
        versions.sort();
        assert_eq!(versions, vec!["1.0.0", "2.0.0"]);
    }

    fn widgets_only_copy(mut entry: InstalledPackage) -> InstalledPackage {
        entry.manifest.withhold_nodes();
        for installed in entry.versions.values_mut() {
            installed.manifest.withhold_nodes();
        }
        entry
    }

    #[test]
    fn test_merge_registry_install_keeps_complete_copies_over_widgets_only_ones() {
        let complete = |version: &str| {
            let mut entry = installed_entry(
                version,
                &format!("hash-{version}"),
                remote_source("https://registry.example"),
            );
            entry.wasm_hash = Some(format!("wasm-{version}"));
            entry
        };
        let merge = |existing: InstalledPackage, fresh: InstalledPackage| {
            RegistryClient::merge_registry_install(Some(existing), fresh)
        };

        let merged = merge(complete("1.0.0"), widgets_only_copy(complete("2.0.0")));
        assert_eq!(merged.version, "1.0.0");
        assert_eq!(merged.wasm_hash.as_deref(), Some("wasm-1.0.0"));
        assert!(!merged.manifest.nodes_withheld());
        assert!(!merged.versions["1.0.0"].manifest.nodes_withheld());
        assert!(merged.versions["2.0.0"].manifest.nodes_withheld());

        let merged = merge(complete("1.0.0"), widgets_only_copy(complete("1.0.0")));
        assert!(!merged.manifest.nodes_withheld());
        assert!(!merged.versions["1.0.0"].manifest.nodes_withheld());

        let both = merge(complete("1.0.0"), complete("2.0.0"));
        assert_eq!(both.version, "2.0.0");
        assert_eq!(both.wasm_hash.as_deref(), Some("wasm-2.0.0"));
        let merged = merge(both, widgets_only_copy(complete("1.0.0")));
        assert_eq!(merged.version, "2.0.0");
        assert!(!merged.versions["1.0.0"].manifest.nodes_withheld());

        let merged = merge(widgets_only_copy(complete("1.0.0")), complete("1.0.0"));
        assert!(!merged.manifest.nodes_withheld());
        assert!(!merged.versions["1.0.0"].manifest.nodes_withheld());

        let merged = merge(
            widgets_only_copy(complete("1.0.0")),
            widgets_only_copy(complete("2.0.0")),
        );
        assert_eq!(merged.version, "2.0.0");
        assert!(merged.manifest.nodes_withheld());
    }

    /// A registry whose `com.acme.nodes@1.0.0` carries nodes and declares a
    /// widget bundle that cannot be installed, in the way `failure` names.
    async fn spawn_registry_with_a_broken_bundle(failure: &'static str) -> String {
        let mut manifest = widgets_only_manifest("com.acme.nodes", "live-map");
        manifest.wasm_hash = Some("declared".into());
        manifest.widget_bundle_path = None;
        manifest.widget_bundle_hash = Some("ab".repeat(32));
        spawn_mock_routes(|base| {
            let link = (failure != "no link").then(|| format!("{base}/bundle.flwb"));
            let mut routes = HashMap::from([(
                "/download@1.0.0".to_string(),
                serde_json::to_vec(&DownloadResponse {
                    widget_bundle_download_url: link,
                    ..inline_answer(manifest, b"registry b")
                })
                .unwrap(),
            )]);
            if failure == "other bytes" {
                routes.insert(
                    "/bundle.flwb".to_string(),
                    b"not the declared bundle".to_vec(),
                );
            }
            routes
        })
        .await
    }

    #[tokio::test]
    async fn test_a_failed_install_from_another_registry_leaves_the_installed_copy_alone() {
        let package_id = "com.acme.nodes";
        for (failure, message) in [
            ("no link", "no widget bundle download URL"),
            ("missing", "Failed to download widget bundle"),
            ("other bytes", "hash mismatch"),
        ] {
            let temp = tempfile::tempdir().unwrap();
            let cache_dir = temp.path().join("cache");
            let complete = PackageManifest::new(package_id, "Nodes", "1.0.0", "nodes");
            let registry_a = spawn_mock_routes(|_| {
                HashMap::from([(
                    "/download@1.0.0".to_string(),
                    serde_json::to_vec(&inline_answer(complete, b"registry a")).unwrap(),
                )])
            })
            .await;
            let client_a = registry_client(&cache_dir, &registry_a);
            client_a.init().await.unwrap();
            client_a
                .install(package_id, Some("1.0.0"), None)
                .await
                .unwrap();
            let wasm_path = client_a.versioned_wasm_path(package_id, "1.0.0");

            let registry_b = spawn_registry_with_a_broken_bundle(failure).await;
            let client_b = registry_client(&cache_dir, &registry_b);
            client_b.init().await.unwrap();
            let error = client_b
                .install(package_id, Some("1.0.0"), None)
                .await
                .unwrap_err();
            assert!(error.to_string().contains(message), "{failure}: {error}");

            assert_eq!(
                std::fs::read(&wasm_path).unwrap(),
                b"registry a",
                "{failure}: a failed install must not put its bytes under the earlier registry's record"
            );
            let kept = client_b.get_installed(package_id).await.unwrap();
            assert!(matches!(
                &kept.source,
                PackageSource::Remote { registry_url, .. } if registry_url == &registry_a
            ));
            assert_eq!(file_names(wasm_path.parent().unwrap()), vec!["node.wasm"]);
        }
    }

    fn file_names(directory: &Path) -> Vec<String> {
        let mut names = std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
            .collect::<Vec<_>>();
        names.sort();
        names
    }

    #[tokio::test]
    async fn test_another_registrys_copy_leaves_the_record_before_its_files_are_replaced() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.maps";
        let registry_a = "https://a.example/registry";

        let client_a = registry_client(&cache_dir, registry_a);
        client_a.init().await.unwrap();
        client_a.state.write().await.installed.insert(
            package_id.to_string(),
            installed_entry("1.0.0-rc1", "hash", remote_source(registry_a)),
        );
        client_a.save_state().await.unwrap();

        for version in ["1.0.0-rc1", "2.0.0"] {
            client_a
                .release_version_directory(package_id, version, true)
                .await
                .unwrap();
        }
        assert!(client_a.get_installed(package_id).await.is_some());
        for rival in ["1.0.0-RC1", "1.0.0-rc1."] {
            let error = client_a
                .release_version_directory(package_id, rival, true)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("same directory"), "{error}");
        }

        let mut client_b = registry_client(&cache_dir, registry_a);
        client_b.init().await.unwrap();
        client_b.set_default_registry("https://b.example/registry".to_string());
        for (version, replaces_nodes) in [("1.0.0-rc1", false), ("2.0.0", true)] {
            client_b
                .release_version_directory(package_id, version, replaces_nodes)
                .await
                .unwrap();
            assert!(
                client_b.get_installed(package_id).await.is_some(),
                "a copy whose node binary stays as it is stays on record"
            );
        }
        client_b
            .release_version_directory(package_id, "1.0.0-RC1", true)
            .await
            .unwrap();
        assert!(client_b.get_installed(package_id).await.is_none());

        let reloaded = registry_client(&cache_dir, registry_a);
        reloaded.init().await.unwrap();
        assert!(
            reloaded.get_installed(package_id).await.is_none(),
            "the saved state must not name the replaced files as the earlier registry's either"
        );
    }

    #[tokio::test]
    async fn test_a_record_holds_a_version_as_its_active_copy_or_among_its_versions() {
        let temp = tempfile::tempdir().unwrap();
        let registry_a = "https://a.example/registry";
        let mut client = registry_client(&temp.path().join("cache"), registry_a);
        client.init().await.unwrap();
        // A state file from before versions were recorded names only the active copy.
        let mut active_only = installed_entry("1.0.0", "hash", remote_source(registry_a));
        active_only.versions.clear();
        // The held version is not the active one.
        let mut among_versions = installed_entry("1.0.0", "hash", remote_source(registry_a));
        among_versions.version = "2.0.0".to_string();
        {
            let mut state = client.state.write().await;
            state.installed.insert("active.only".into(), active_only);
            state
                .installed
                .insert("among.versions".into(), among_versions);
        }

        client.set_default_registry("https://b.example/registry".to_string());
        for package_id in ["active.only", "among.versions"] {
            client
                .release_version_directory(package_id, "1.0.0", true)
                .await
                .unwrap();
            assert!(
                client.get_installed(package_id).await.is_none(),
                "{package_id}"
            );
        }
    }

    #[tokio::test]
    async fn test_a_failed_install_keeps_the_other_registrys_record_when_its_nodes_stay() {
        let package_id = "com.acme.nodes";
        let registry_a = "https://a.example/registry";
        for answers_nodes in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let cache_dir = temp.path().join("cache");
            let seeded = registry_client(&cache_dir, registry_a);
            seeded.init().await.unwrap();
            let mut held = installed_entry("1.0.0", "hash", remote_source(registry_a));
            held.id = package_id.to_string();
            seeded
                .state
                .write()
                .await
                .installed
                .insert(package_id.to_string(), held);
            seeded.save_state().await.unwrap();

            // Another version with nodes, or the held version without them: a
            // directory in the way makes the write after the release fail.
            let (version, blocked, registry_b) = if answers_nodes {
                let manifest = PackageManifest::new(package_id, "Nodes", "2.0.0", "nodes");
                let registry = spawn_mock_routes(|_| {
                    HashMap::from([(
                        "/download@2.0.0".to_string(),
                        serde_json::to_vec(&inline_answer(manifest, b"registry b")).unwrap(),
                    )])
                })
                .await;
                let blocked = seeded.versioned_wasm_path(package_id, "2.0.0");
                ("2.0.0", blocked, registry)
            } else {
                let (release, _) = registry_release(package_id, "1.0.0", "widgets");
                let blocked = seeded.versioned_widget_bundle_path(package_id, "1.0.0");
                ("1.0.0", blocked, spawn_mock_registry(vec![release]).await)
            };
            std::fs::create_dir_all(&blocked).unwrap();

            let client_b = registry_client(&cache_dir, &registry_b);
            client_b.init().await.unwrap();
            assert!(client_b
                .install(package_id, Some(version), None)
                .await
                .is_err());
            let reloaded = registry_client(&cache_dir, registry_a);
            reloaded.init().await.unwrap();
            for client in [&client_b, &reloaded] {
                let kept = client.get_installed(package_id).await.unwrap();
                assert!(
                    matches!(&kept.source, PackageSource::Remote { registry_url, .. } if registry_url == registry_a),
                    "answers_nodes={answers_nodes}: {:?}",
                    kept.source
                );
            }
        }
    }

    #[tokio::test]
    async fn test_install_is_refused_while_another_name_writes_the_same_directory() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.maps";
        let (release, _) = registry_release(package_id, "1.0.0", "v1");
        let registry = spawn_mock_registry(vec![release]).await;
        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();

        let rival = client.claim_directory("Com.Acme.Maps").await.unwrap();
        let error = client
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("same directory"), "{error}");
        assert!(!cache_dir.join("wasm/nodes").join(package_id).exists());
        let mut local = widgets_only_manifest(package_id, "live-map");
        local.widget_bundle_path = None;
        local.widgets.clear();
        let error = client
            .register_local_package(&temp.path().join("node.wasm"), local)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("same directory"), "{error}");
        assert!(client.list_installed().await.unwrap().is_empty());

        drop(rival);
        client
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn test_the_widget_store_is_not_pruned_while_an_install_holds_it() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let client = test_client(&cache_dir);
        client.init().await.unwrap();
        let unpacked = widget_store_dir(&cache_dir, "com.acme.maps", "not-on-record-yet");
        std::fs::create_dir_all(&unpacked).unwrap();

        let held = client.widget_store.lock().await;
        let pruned = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            client.prune_widget_store("com.acme.maps"),
        )
        .await;
        assert!(pruned.is_err(), "the prune waits for the install");
        assert!(unpacked.is_dir());

        drop(held);
        client.prune_widget_store("com.acme.maps").await;
        assert!(!unpacked.exists());
    }

    #[tokio::test]
    async fn test_a_write_that_fails_leaves_no_record_of_the_other_registrys_copy() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.nodes";
        let registry_a = "https://a.example/registry";
        let seeded = registry_client(&cache_dir, registry_a);
        seeded.init().await.unwrap();
        let mut held = installed_entry("1.0.0", "hash", remote_source(registry_a));
        held.id = package_id.to_string();
        seeded
            .state
            .write()
            .await
            .installed
            .insert(package_id.to_string(), held);
        seeded.save_state().await.unwrap();
        // A directory in the node binary's place makes the write fail after the record left.
        let wasm_path = seeded.versioned_wasm_path(package_id, "1.0.0");
        std::fs::create_dir_all(&wasm_path).unwrap();
        let sidecar = RegistryClient::cwasm_sidecar_path(&wasm_path);
        std::fs::write(&sidecar, b"compiled from the earlier bytes").unwrap();

        let manifest = PackageManifest::new(package_id, "Nodes", "1.0.0", "nodes");
        let registry_b = spawn_mock_routes(|_| {
            HashMap::from([(
                "/download@1.0.0".to_string(),
                serde_json::to_vec(&inline_answer(manifest, b"registry b")).unwrap(),
            )])
        })
        .await;
        let client_b = registry_client(&cache_dir, &registry_b);
        client_b.init().await.unwrap();
        assert!(client_b
            .install(package_id, Some("1.0.0"), None)
            .await
            .is_err());

        assert!(client_b.get_installed(package_id).await.is_none());
        let reloaded = registry_client(&cache_dir, registry_a);
        reloaded.init().await.unwrap();
        assert!(reloaded.get_installed(package_id).await.is_none());
        assert!(
            !sidecar.exists(),
            "the sidecar goes before the bytes it was compiled from"
        );
        assert_eq!(file_names(wasm_path.parent().unwrap()), vec!["node.wasm"]);
    }

    #[tokio::test]
    async fn test_install_refuses_a_version_that_shares_a_directory_with_a_held_one() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.maps";
        let (held, _) = registry_release(package_id, "1.0.0-rc1", "held");
        let (rival, _) = registry_release(package_id, "1.0.0-RC1", "rival");
        let registry = spawn_mock_registry(vec![held, rival]).await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        client
            .install(package_id, Some("1.0.0-rc1"), None)
            .await
            .unwrap();
        let error = client
            .install(package_id, Some("1.0.0-RC1"), None)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("same directory"), "{error}");
        let installed = client.get_installed(package_id).await.unwrap();
        assert_eq!(installed.versions.keys().collect::<Vec<_>>(), ["1.0.0-rc1"]);
    }

    #[tokio::test]
    async fn test_two_names_for_one_directory_are_not_written_at_once() {
        let temp = tempfile::tempdir().unwrap();
        let client = registry_client(&temp.path().join("cache"), "https://registry.example");
        client.init().await.unwrap();

        let first = client.claim_directory("com.acme.maps").await.unwrap();
        let other_window = client.clone();
        let again = other_window.claim_directory("com.acme.maps").await.unwrap();
        let error = client.claim_directory("Com.Acme.Maps").await.err().unwrap();
        assert!(error.to_string().contains("same directory"), "{error}");

        drop(first);
        assert!(
            client.claim_directory("Com.Acme.Maps").await.is_err(),
            "the other install of the same package still writes there"
        );
        drop(again);
        assert!(client.claim_directory("Com.Acme.Maps").await.is_ok());
        assert!(client.claim_directory("com.acme.maps").await.is_ok());
    }

    /// A registry that answers `com.acme.nodes@1.0.0` with nodes and widgets.
    async fn spawn_nodes_and_widgets_registry(body: &'static str) -> (String, Vec<u8>) {
        let (mut release, _) = registry_release("com.acme.nodes", "1.0.0", body);
        release.manifest.wasm_hash = Some("declared".into());
        let bundle = release.bundle.clone();
        let registry = spawn_mock_routes(|base| {
            HashMap::from([
                (
                    "/download@1.0.0".to_string(),
                    serde_json::to_vec(&DownloadResponse {
                        widget_bundle_download_url: Some(format!("{base}/bundle.flwb")),
                        ..inline_answer(release.manifest, body.as_bytes())
                    })
                    .unwrap(),
                ),
                ("/bundle.flwb".to_string(), release.bundle),
            ])
        })
        .await;
        (registry, bundle)
    }

    #[tokio::test]
    async fn test_an_install_replaces_files_instead_of_writing_into_them() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.nodes";
        let (registry_a, bundle_a) = spawn_nodes_and_widgets_registry("registry a").await;
        let (registry_b, bundle_b) = spawn_nodes_and_widgets_registry("registry b").await;

        let client_a = registry_client(&cache_dir, &registry_a);
        client_a.init().await.unwrap();
        client_a
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();
        // A second name for each file: a write into the file would show through it.
        let wasm_path = client_a.versioned_wasm_path(package_id, "1.0.0");
        let bundle_path = client_a.versioned_widget_bundle_path(package_id, "1.0.0");
        let (kept_wasm, kept_bundle) = (temp.path().join("wasm"), temp.path().join("bundle"));
        std::fs::hard_link(&wasm_path, &kept_wasm).unwrap();
        std::fs::hard_link(&bundle_path, &kept_bundle).unwrap();

        let client_b = registry_client(&cache_dir, &registry_b);
        client_b.init().await.unwrap();
        client_b
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();

        assert_eq!(std::fs::read(&wasm_path).unwrap(), b"registry b");
        assert_eq!(std::fs::read(&kept_wasm).unwrap(), b"registry a");
        assert_eq!(std::fs::read(&bundle_path).unwrap(), bundle_b);
        assert_eq!(std::fs::read(&kept_bundle).unwrap(), bundle_a);
    }

    #[tokio::test]
    async fn test_install_refuses_a_name_that_shares_a_directory_with_an_installed_package() {
        let temp = tempfile::tempdir().unwrap();
        let cache_dir = temp.path().join("cache");
        let package_id = "com.acme.maps";
        let (release, hash) = registry_release(package_id, "1.0.0", "v1");
        let registry = spawn_mock_registry(vec![release]).await;

        let client = registry_client(&cache_dir, &registry);
        client.init().await.unwrap();
        client
            .install(package_id, Some("1.0.0"), None)
            .await
            .unwrap();

        for rival in ["Com.Acme.Maps", "com.acme.maps."] {
            let error = client
                .install(rival, Some("1.0.0"), None)
                .await
                .unwrap_err();
            assert!(error.to_string().contains("same directory"), "{error}");
        }
        let mut local = widgets_only_manifest("COM.ACME.MAPS", "live-map");
        local.widget_bundle_path = None;
        local.widgets.clear();
        let error = client
            .register_local_package(&temp.path().join("node.wasm"), local)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("same directory"), "{error}");

        let installed = client.list_installed().await.unwrap();
        assert_eq!(installed.len(), 1);
        assert_eq!(installed[0].id, package_id);
        assert!(widget_store_dir(&cache_dir, package_id, &hash).is_dir());

        assert!(!RegistryClient::shares_directory(package_id, package_id));
        assert!(!RegistryClient::shares_directory(
            package_id,
            "com.acme.map"
        ));
        assert!(RegistryClient::shares_directory(
            package_id,
            "COM.ACME.MAPS"
        ));
        assert!(RegistryClient::shares_directory(
            package_id,
            "com.acme.maps.."
        ));
    }

    #[tokio::test]
    async fn test_export_names_a_widgets_only_answer_instead_of_calling_it_broken() {
        let temp = tempfile::tempdir().unwrap();
        let package_id = "com.acme.maps";
        let (mut release, _) = registry_release(package_id, "1.0.0", "widgets");
        release.manifest.withhold_nodes();
        let registry = spawn_mock_registry(vec![release]).await;

        let client = registry_client(&temp.path().join("cache"), &registry);
        let error = client
            .export_package_version(package_id, "1.0.0", Some("app-1"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("kept the nodes"), "{error}");
    }

    #[tokio::test]
    async fn test_write_replacing_swaps_the_file_and_leaves_nothing_staged() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("node.wasm");
        std::fs::write(&path, b"earlier").unwrap();
        RegistryClient::write_replacing(&path, b"fresh")
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"fresh");

        let blocked = temp.path().join("blocked.wasm");
        std::fs::create_dir(&blocked).unwrap();
        assert!(RegistryClient::write_replacing(&blocked, b"fresh")
            .await
            .is_err());
        assert!(blocked.is_dir());
        assert_eq!(file_names(temp.path()), vec!["blocked.wasm", "node.wasm"]);
    }
}
