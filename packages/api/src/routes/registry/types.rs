//! Registry data-transfer types owned by the API crate.
//!
//! These are the canonical wire types for the server-side registry API.
//! The wasm crate keeps its own (structurally identical) copies for the
//! `RegistryClient` used by the desktop app; JSON compatibility is all
//! that matters at the HTTP boundary.

use flow_like_storage::Path as FlowPath;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_wasm::manifest::{PackageManifest, PackageNodeEntry};
use serde::{Deserialize, Serialize};
use std::time::Duration;
use utoipa::ToSchema;

use crate::entity::meta;

/// Resolved metadata summary for a single language
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MetaSummary {
    pub lang: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
}

impl MetaSummary {
    pub fn from_model(m: &meta::Model) -> Self {
        Self {
            lang: m.lang.clone(),
            name: m.name.clone(),
            description: m.description.clone(),
            icon: m.icon.clone(),
            thumbnail: m.thumbnail.clone(),
        }
    }

    pub fn pick_best<'a>(metas: &'a [meta::Model], language: &str) -> Option<&'a meta::Model> {
        metas
            .iter()
            .find(|m| m.lang == language)
            .or_else(|| metas.iter().find(|m| m.lang == "en"))
            .or_else(|| metas.first())
    }

    pub async fn presign_media(&mut self, package_id: &str, store: &FlowLikeStore) {
        let prefix = FlowPath::from("media").child("packages").child(package_id);
        if let Some(icon) = &self.icon {
            if !icon.starts_with("http://") && !icon.starts_with("https://") {
                let path = prefix.child(format!("{icon}.webp"));
                if let Ok(url) = store.sign("GET", &path, Duration::from_secs(86400)).await {
                    self.icon = Some(url.to_string());
                }
            }
        }
        if let Some(thumb) = &self.thumbnail {
            if !thumb.starts_with("http://") && !thumb.starts_with("https://") {
                let path = prefix.child(format!("{thumb}.webp"));
                if let Ok(url) = store.sign("GET", &path, Duration::from_secs(86400)).await {
                    self.thumbnail = Some(url.to_string());
                }
            }
        }
    }
}

/// Registry entry status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum PackageStatus {
    #[default]
    Active,
    Deprecated,
    Disabled,
    PendingReview,
}

/// Source type for a package
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PackageSource {
    Local {
        path: std::path::PathBuf,
    },
    Remote {
        registry_url: String,
        download_url: String,
    },
    Embedded {
        data: Vec<u8>,
    },
}

/// Registry entry for a single package version
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageVersion {
    pub version: String,
    pub wasm_hash: String,
    pub wasm_size: u64,
    #[serde(default)]
    pub download_url: Option<String>,
    pub published_at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub min_flow_like_version: Option<String>,
    #[serde(default)]
    pub release_notes: Option<String>,
    #[serde(default)]
    pub yanked: bool,
}

/// Full registry entry for a package
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegistryEntry {
    pub id: String,
    pub manifest: PackageManifest,
    #[serde(default)]
    pub nodes: Vec<PackageNodeEntry>,
    pub versions: Vec<PackageVersion>,
    #[serde(default)]
    pub status: PackageStatus,
    #[serde(default)]
    pub download_count: u64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub source: PackageSource,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub price: i64,
    #[serde(default)]
    pub visibility: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_rating: Option<f64>,
    #[serde(default)]
    pub rating_count: i64,
    #[serde(default)]
    pub rating_sum: i64,
    /// The caller's permission bits on this package (None if unauthenticated)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_user_permission: Option<i32>,
}

impl RegistryEntry {
    pub fn latest_version(&self) -> Option<&PackageVersion> {
        self.versions.iter().find(|v| !v.yanked)
    }

    pub fn get_version(&self, version: &str) -> Option<&PackageVersion> {
        self.versions.iter().find(|v| v.version == version)
    }
}

/// Registry index — lightweight listing of available packages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    pub name: String,
    pub url: String,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub packages: Vec<PackageSummary>,
}

/// Lightweight package summary for index
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub primary_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secondary_category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_rating: Option<f64>,
    #[serde(default)]
    pub rating_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetaSummary>,
}

/// Search filters for registry queries
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SearchFilters {
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub verified_only: bool,
    #[serde(default)]
    pub include_deprecated: bool,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "default_limit")]
    pub limit: usize,
    #[serde(default)]
    pub sort_by: SortField,
    #[serde(default)]
    pub sort_desc: bool,
    #[serde(default)]
    pub language: Option<String>,
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
            language: None,
        }
    }
}

fn default_limit() -> usize {
    50
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, ToSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    pub packages: Vec<PackageSummary>,
    pub total_count: usize,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PublishResponse {
    pub success: bool,
    pub package_id: String,
    pub version: String,
    #[serde(default)]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DownloadRequest {
    pub package_id: String,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct DownloadResponse {
    pub package_id: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub wasm_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_url: Option<String>,
    pub manifest: PackageManifest,
    /// Resolved package metadata (icon, thumbnail, localized name/description)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<MetaSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegistryError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: Option<serde_json::Value>,
}
