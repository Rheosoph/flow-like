//! WASM Node Registry
//!
//! Provides types and functionality for a node package registry system.
//! Supports local development registries and remote shared registries.

use crate::manifest::{PackageManifest, PackageNodeEntry};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Registry entry status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PackageStatus {
    /// Package is active and usable
    #[default]
    Active,
    /// Package is deprecated (still usable but shows warning)
    Deprecated,
    /// Package is disabled (not usable)
    Disabled,
    /// Package is pending review
    PendingReview,
}

/// Source type for a package
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackageSource {
    /// Local development package
    Local { path: PathBuf },
    /// Remote registry package
    Remote {
        registry_url: String,
        download_url: String,
    },
    /// Embedded package (built-in)
    Embedded { data: Vec<u8> },
}

/// Registry entry for a single package version
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageVersion {
    /// Version string (semver)
    pub version: String,
    /// WASM file hash (SHA-256)
    pub wasm_hash: String,
    /// WASM file size in bytes
    pub wasm_size: u64,
    /// Download URL (for remote packages)
    #[serde(default)]
    pub download_url: Option<String>,
    /// When this version was published
    pub published_at: chrono::DateTime<chrono::Utc>,
    /// Minimum Flow-Like version required
    #[serde(default)]
    pub min_flow_like_version: Option<String>,
    /// Release notes
    #[serde(default)]
    pub release_notes: Option<String>,
    /// Whether this version is yanked
    #[serde(default)]
    pub yanked: bool,
}

/// Full registry entry for a package
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Package ID
    pub id: String,
    /// Package manifest (latest version)
    pub manifest: PackageManifest,
    /// Nodes extracted from the WASM binary
    #[serde(default)]
    pub nodes: Vec<PackageNodeEntry>,
    /// All available versions
    pub versions: Vec<PackageVersion>,
    /// Package status
    #[serde(default)]
    pub status: PackageStatus,
    /// Total download count
    #[serde(default)]
    pub download_count: u64,
    /// When the package was first published
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the package was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Package source
    pub source: PackageSource,
    /// Verification status (for remote packages)
    #[serde(default)]
    pub verified: bool,
}

impl RegistryEntry {
    /// Get the latest non-yanked version
    pub fn latest_version(&self) -> Option<&PackageVersion> {
        self.versions.iter().find(|v| !v.yanked)
    }

    /// Get a specific version
    pub fn get_version(&self, version: &str) -> Option<&PackageVersion> {
        self.versions.iter().find(|v| v.version == version)
    }
}

/// Cached package data for offline use
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedPackage {
    /// Registry entry metadata
    pub entry: RegistryEntry,
    /// WASM binary data
    pub wasm_data: Vec<u8>,
    /// When this was cached
    pub cached_at: chrono::DateTime<chrono::Utc>,
    /// Cache expiry time
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Registry index - lightweight listing of available packages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    /// Registry name
    pub name: String,
    /// Registry URL
    pub url: String,
    /// When the index was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Package summaries
    pub packages: Vec<PackageSummary>,
}

/// Lightweight package summary for index
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct PackageSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub latest_version: String,
    pub download_count: u64,
    pub status: PackageStatus,
    pub keywords: Vec<String>,
    pub verified: bool,
    #[serde(default)]
    pub price: i64,
    #[serde(default)]
    pub visibility: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_category: Option<String>,
}

/// Registry configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryConfig {
    /// Default registry URL
    pub default_registry: String,
    /// Additional registry URLs
    #[serde(default)]
    pub additional_registries: Vec<String>,
    /// Local development paths to scan
    #[serde(default)]
    pub local_paths: Vec<PathBuf>,
    /// Cache directory
    pub cache_dir: PathBuf,
    /// Cache duration for remote packages (hours)
    #[serde(default = "default_cache_hours")]
    pub cache_duration_hours: u32,
    /// Auto-update index on startup
    #[serde(default = "default_true")]
    pub auto_update_index: bool,
    /// Allow unverified packages
    #[serde(default)]
    pub allow_unverified: bool,
    /// Bearer token for authenticated API requests
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
}

fn default_cache_hours() -> u32 {
    24 * 7 // 1 week
}

fn default_true() -> bool {
    true
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            default_registry: "https://api.flow-like.com/api/v1/registry".to_string(),
            additional_registries: Vec::new(),
            local_paths: Vec::new(),
            cache_dir: dirs_next::cache_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("flow-like")
                .join("wasm-registry"),
            cache_duration_hours: default_cache_hours(),
            auto_update_index: true,
            allow_unverified: false,
            auth_token: None,
        }
    }
}

/// Search filters for registry queries
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SearchFilters {
    /// Search query (matches name, description, keywords)
    #[serde(default)]
    pub query: Option<String>,
    /// Filter by category
    #[serde(default)]
    pub category: Option<String>,
    /// Filter by keywords
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Filter by author
    #[serde(default)]
    pub author: Option<String>,
    /// Only show verified packages
    #[serde(default)]
    pub verified_only: bool,
    /// Include deprecated packages
    #[serde(default)]
    pub include_deprecated: bool,
    /// Pagination offset
    #[serde(default)]
    pub offset: usize,
    /// Pagination limit
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// Sort field
    #[serde(default)]
    pub sort_by: SortField,
    /// Sort direction
    #[serde(default)]
    pub sort_desc: bool,
}

impl Default for SearchFilters {
    fn default() -> Self {
        Self {
            query: None,
            category: None,
            keywords: Vec::new(),
            author: None,
            verified_only: false,
            include_deprecated: false,
            offset: 0,
            limit: default_limit(),
            sort_by: SortField::default(),
            sort_desc: false,
        }
    }
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SortField {
    #[default]
    Relevance,
    Name,
    Downloads,
    UpdatedAt,
    CreatedAt,
}

/// Search results
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub packages: Vec<PackageSummary>,
    pub total_count: usize,
    pub offset: usize,
    pub limit: usize,
}

/// API request/response types for registry HTTP API

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PublishRequest {
    /// Package manifest
    pub manifest: PackageManifest,
    /// Base64-encoded WASM file
    pub wasm_base64: String,
    /// Optional API key for authentication
    #[serde(default)]
    pub api_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PublishResponse {
    pub success: bool,
    pub package_id: String,
    pub version: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DownloadRequest {
    pub package_id: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// Resolved metadata summary for a single language (icon, thumbnail, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MetaSummary {
    pub lang: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DownloadResponse {
    pub package_id: String,
    pub version: String,
    /// Base64-encoded WASM file (optional, for backward compat)
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wasm_base64: String,
    /// Direct download URL (CDN or signed URL)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    pub manifest: PackageManifest,
    /// Resolved package metadata (icon, thumbnail, localized name/description)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetaSummary>,
}

/// Registry API error
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RegistryError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}

/// Local registry state (persisted to disk)
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalRegistryState {
    /// Installed packages (id -> installed version info)
    pub installed: HashMap<String, InstalledPackage>,
    /// Package cache metadata
    pub cache_metadata: HashMap<String, CacheMetadata>,
    /// Last index refresh times per registry
    pub index_refresh: HashMap<String, chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledVersion {
    pub version: String,
    pub wasm_path: PathBuf,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    pub manifest: PackageManifest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetaSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledPackage {
    pub id: String,
    pub version: String,
    pub source: PackageSource,
    pub installed_at: chrono::DateTime<chrono::Utc>,
    pub wasm_path: PathBuf,
    pub manifest: PackageManifest,
    #[serde(default)]
    pub versions: HashMap<String, InstalledVersion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetaSummary>,
}

impl InstalledPackage {
    pub fn active_version(&self) -> &str {
        &self.version
    }

    pub fn get_version(&self, version: &str) -> Option<&InstalledVersion> {
        self.versions.get(version)
    }

    pub fn has_version(&self, version: &str) -> bool {
        self.versions.contains_key(version)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    pub package_id: String,
    pub version: String,
    pub hash: String,
    pub size: u64,
    pub cached_at: chrono::DateTime<chrono::Utc>,
    pub last_accessed: chrono::DateTime<chrono::Utc>,
    pub access_count: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_package_status_default() {
        let status = PackageStatus::default();
        assert_eq!(status, PackageStatus::Active);
    }

    #[test]
    fn test_package_status_serialization() {
        assert_eq!(
            serde_json::to_string(&PackageStatus::Active).unwrap(),
            "\"active\""
        );
        assert_eq!(
            serde_json::to_string(&PackageStatus::PendingReview).unwrap(),
            "\"pending_review\""
        );
        assert_eq!(
            serde_json::to_string(&PackageStatus::Deprecated).unwrap(),
            "\"deprecated\""
        );
        assert_eq!(
            serde_json::to_string(&PackageStatus::Disabled).unwrap(),
            "\"disabled\""
        );
    }

    #[test]
    fn test_package_status_deserialization() {
        assert_eq!(
            serde_json::from_str::<PackageStatus>("\"active\"").unwrap(),
            PackageStatus::Active
        );
        assert_eq!(
            serde_json::from_str::<PackageStatus>("\"pending_review\"").unwrap(),
            PackageStatus::PendingReview
        );
    }

    #[test]
    fn test_registry_entry_latest_version() {
        let manifest = crate::manifest::PackageManifest::new(
            "test.package",
            "Test Package",
            "1.0.0",
            "A test package",
        );

        let entry = RegistryEntry {
            id: "test.package".to_string(),
            manifest,
            nodes: vec![],
            versions: vec![
                PackageVersion {
                    version: "0.9.0".to_string(),
                    wasm_hash: "hash1".to_string(),
                    wasm_size: 1000,
                    download_url: None,
                    published_at: chrono::Utc::now() - chrono::Duration::days(10),
                    min_flow_like_version: None,
                    release_notes: None,
                    yanked: true,
                },
                PackageVersion {
                    version: "1.0.0".to_string(),
                    wasm_hash: "hash2".to_string(),
                    wasm_size: 1200,
                    download_url: None,
                    published_at: chrono::Utc::now(),
                    min_flow_like_version: None,
                    release_notes: None,
                    yanked: false,
                },
            ],
            status: PackageStatus::Active,
            download_count: 100,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source: PackageSource::Remote {
                registry_url: "https://registry.example.com".to_string(),
                download_url: "https://cdn.example.com/test.wasm".to_string(),
            },
            verified: true,
        };

        let latest = entry.latest_version().unwrap();
        assert_eq!(latest.version, "1.0.0");
        assert!(!latest.yanked);
    }

    #[test]
    fn test_registry_entry_get_version() {
        let manifest = crate::manifest::PackageManifest::new(
            "test.package",
            "Test Package",
            "1.0.0",
            "A test package",
        );

        let entry = RegistryEntry {
            id: "test.package".to_string(),
            manifest,
            nodes: vec![],
            versions: vec![
                PackageVersion {
                    version: "0.9.0".to_string(),
                    wasm_hash: "hash1".to_string(),
                    wasm_size: 1000,
                    download_url: None,
                    published_at: chrono::Utc::now(),
                    min_flow_like_version: None,
                    release_notes: None,
                    yanked: false,
                },
                PackageVersion {
                    version: "1.0.0".to_string(),
                    wasm_hash: "hash2".to_string(),
                    wasm_size: 1200,
                    download_url: None,
                    published_at: chrono::Utc::now(),
                    min_flow_like_version: None,
                    release_notes: None,
                    yanked: false,
                },
            ],
            status: PackageStatus::Active,
            download_count: 100,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            source: PackageSource::Local {
                path: PathBuf::from("/test/path"),
            },
            verified: false,
        };

        assert!(entry.get_version("0.9.0").is_some());
        assert!(entry.get_version("1.0.0").is_some());
        assert!(entry.get_version("2.0.0").is_none());
    }

    #[test]
    fn test_search_filters_default() {
        let filters = SearchFilters::default();
        assert!(filters.query.is_none());
        assert!(filters.category.is_none());
        assert!(filters.keywords.is_empty());
        assert!(!filters.verified_only);
        assert!(!filters.include_deprecated);
        assert_eq!(filters.offset, 0);
        assert_eq!(filters.limit, 50);
    }

    #[test]
    fn test_sort_field_default() {
        let sort = SortField::default();
        assert!(matches!(sort, SortField::Relevance));
    }

    #[test]
    fn test_sort_field_serialization() {
        assert_eq!(
            serde_json::to_string(&SortField::Downloads).unwrap(),
            "\"downloads\""
        );
        assert_eq!(serde_json::to_string(&SortField::Name).unwrap(), "\"name\"");
        assert_eq!(
            serde_json::to_string(&SortField::UpdatedAt).unwrap(),
            "\"updated_at\""
        );
    }

    #[test]
    fn test_registry_config_default() {
        let config = RegistryConfig::default();
        assert_eq!(config.default_registry, "https://api.flow-like.com/api/v1/registry");
        assert!(config.additional_registries.is_empty());
        assert!(config.local_paths.is_empty());
        assert_eq!(config.cache_duration_hours, 24 * 7);
        assert!(config.auto_update_index);
        assert!(!config.allow_unverified);
    }

    #[test]
    fn test_package_source_serialization() {
        let local = PackageSource::Local {
            path: PathBuf::from("/test/path"),
        };
        let json = serde_json::to_string(&local).unwrap();
        assert!(json.contains("\"type\":\"local\""));

        let remote = PackageSource::Remote {
            registry_url: "https://registry.example.com".to_string(),
            download_url: "https://cdn.example.com/pkg.wasm".to_string(),
        };
        let json = serde_json::to_string(&remote).unwrap();
        assert!(json.contains("\"type\":\"remote\""));
    }

    #[test]
    fn test_publish_request_serialization() {
        let manifest =
            crate::manifest::PackageManifest::new("test.package", "Test", "1.0.0", "Description");
        let request = PublishRequest {
            manifest,
            wasm_base64: "AGFzbQE=".to_string(),
            api_key: Some("test-key".to_string()),
        };

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"wasm_base64\""));
        assert!(json.contains("\"api_key\""));
    }

    #[test]
    fn test_publish_response_serialization() {
        let response = PublishResponse {
            success: true,
            package_id: "test.package".to_string(),
            version: "1.0.0".to_string(),
            message: Some("Published successfully".to_string()),
        };

        let json = serde_json::to_string(&response).unwrap();
        let parsed: PublishResponse = serde_json::from_str(&json).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.package_id, "test.package");
    }

    #[test]
    fn test_local_registry_state_default() {
        let state = LocalRegistryState::default();
        assert!(state.installed.is_empty());
        assert!(state.cache_metadata.is_empty());
        assert!(state.index_refresh.is_empty());
    }

    #[test]
    fn test_search_results_serialization() {
        let results = SearchResults {
            packages: vec![PackageSummary {
                id: "test.package".to_string(),
                name: "Test Package".to_string(),
                description: "A test".to_string(),
                latest_version: "1.0.0".to_string(),
                download_count: 100,
                status: PackageStatus::Active,
                keywords: vec!["test".to_string()],
                verified: true,
                price: 0,
                visibility: "public".to_string(),
                primary_category: None,
                secondary_category: None,
            }],
            total_count: 1,
            offset: 0,
            limit: 50,
        };

        let json = serde_json::to_string(&results).unwrap();
        let parsed: SearchResults = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.packages.len(), 1);
        assert_eq!(parsed.total_count, 1);
    }
}
