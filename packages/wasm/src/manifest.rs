//! Package manifest for WASM node packages
//!
//! Each WASM package can contain multiple nodes and declares its
//! required permissions, resource limits, and OAuth scopes upfront.

use crate::limits::{WasmCapabilities, WasmLimits};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Current manifest version
pub const MANIFEST_VERSION: u32 = 1;

/// Memory tier presets for packages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    /// 16 MB - minimal processing
    Minimal,
    /// 32 MB - light processing
    Light,
    /// 64 MB - standard processing (default)
    #[default]
    Standard,
    /// 128 MB - heavy processing
    Heavy,
    /// 256 MB - intensive workloads
    Intensive,
    /// 512 MB - large data processing
    Large,
    /// 1 GB - very large workloads
    Huge,
    /// 2 GB - extreme workloads
    Extreme,
    /// 4 GB - maximum allowed
    Maximum,
}

impl MemoryTier {
    pub fn bytes(&self) -> usize {
        match self {
            Self::Minimal => 16 * 1024 * 1024,
            Self::Light => 32 * 1024 * 1024,
            Self::Standard => 64 * 1024 * 1024,
            Self::Heavy => 128 * 1024 * 1024,
            Self::Intensive => 256 * 1024 * 1024,
            Self::Large => 512 * 1024 * 1024,
            Self::Huge => 1024 * 1024 * 1024,
            Self::Extreme => 2 * 1024 * 1024 * 1024,
            Self::Maximum => 4 * 1024 * 1024 * 1024,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Minimal => "Minimal (16 MB)",
            Self::Light => "Light (32 MB)",
            Self::Standard => "Standard (64 MB)",
            Self::Heavy => "Heavy (128 MB)",
            Self::Intensive => "Intensive (256 MB)",
            Self::Large => "Large (512 MB)",
            Self::Huge => "Huge (1 GB)",
            Self::Extreme => "Extreme (2 GB)",
            Self::Maximum => "Maximum (4 GB)",
        }
    }
}

/// Timeout tier presets
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TimeoutTier {
    /// 5 seconds - quick operations
    Quick,
    /// 30 seconds - standard (default)
    #[default]
    Standard,
    /// 60 seconds - extended
    Extended,
    /// 300 seconds - long running
    LongRunning,
    /// 600 seconds - very long running
    VeryLong,
    /// 1800 seconds - maximum
    Maximum,
}

impl TimeoutTier {
    pub fn duration(&self) -> std::time::Duration {
        match self {
            Self::Quick => std::time::Duration::from_secs(5),
            Self::Standard => std::time::Duration::from_secs(30),
            Self::Extended => std::time::Duration::from_secs(60),
            Self::LongRunning => std::time::Duration::from_secs(300),
            Self::VeryLong => std::time::Duration::from_secs(600),
            Self::Maximum => std::time::Duration::from_secs(1800),
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Quick => "Quick (5s)",
            Self::Standard => "Standard (30s)",
            Self::Extended => "Extended (60s)",
            Self::LongRunning => "Long Running (5min)",
            Self::VeryLong => "Very Long (10min)",
            Self::Maximum => "Maximum (30min)",
        }
    }
}

/// OAuth scope requirement for a specific provider
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthScopeRequirement {
    /// OAuth provider ID (e.g., "google", "github", "microsoft")
    pub provider: String,
    /// Required scopes for this provider
    pub scopes: Vec<String>,
    /// Human-readable reason for needing these scopes
    pub reason: String,
    /// Whether this OAuth access is required or optional
    #[serde(default)]
    pub required: bool,
}

/// Network access requirements
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetworkPermissions {
    /// Allow outbound HTTP requests
    #[serde(default)]
    pub http_enabled: bool,
    /// Specific allowed hosts (empty = all hosts allowed if http_enabled)
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    /// Allow WebSocket connections
    #[serde(default)]
    pub websocket_enabled: bool,
    /// Allow TCP socket connections
    #[serde(default)]
    pub tcp_enabled: bool,
    /// Allow UDP socket connections
    #[serde(default)]
    pub udp_enabled: bool,
    /// Allow DNS lookups
    #[serde(default)]
    pub dns_enabled: bool,
}

/// File system access requirements
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileSystemPermissions {
    /// Access to node-scoped storage
    #[serde(default)]
    pub node_storage: bool,
    /// Access to user-scoped storage
    #[serde(default)]
    pub user_storage: bool,
    /// Access to upload directory
    #[serde(default)]
    pub upload_dir: bool,
    /// Access to cache directory
    #[serde(default)]
    pub cache_dir: bool,
}

/// Package permissions declaration
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackagePermissions {
    /// Memory tier required
    #[serde(default)]
    pub memory: MemoryTier,
    /// Timeout tier required
    #[serde(default)]
    pub timeout: TimeoutTier,
    /// Network access requirements
    #[serde(default)]
    pub network: NetworkPermissions,
    /// File system access requirements
    #[serde(default)]
    pub filesystem: FileSystemPermissions,
    /// OAuth scope requirements per provider
    #[serde(default)]
    pub oauth_scopes: Vec<OAuthScopeRequirement>,
    /// Access to execution variables
    #[serde(default)]
    pub variables: bool,
    /// Access to execution cache
    #[serde(default)]
    pub cache: bool,
    /// Streaming output capability
    #[serde(default)]
    pub streaming: bool,
    /// A2UI capability
    #[serde(default)]
    pub a2ui: bool,
    /// Access to LLM/model providers
    #[serde(default)]
    pub models: bool,
}

impl PackagePermissions {
    /// Convert to WasmCapabilities bitflags
    pub fn to_capabilities(&self) -> WasmCapabilities {
        let mut caps = WasmCapabilities::NONE;

        // Network
        if self.network.http_enabled {
            caps |= WasmCapabilities::HTTP_ALL;
        }
        if self.network.websocket_enabled {
            caps |= WasmCapabilities::WEBSOCKET;
        }
        if self.network.tcp_enabled {
            caps |= WasmCapabilities::TCP;
        }
        if self.network.udp_enabled {
            caps |= WasmCapabilities::UDP;
        }
        if self.network.dns_enabled {
            caps |= WasmCapabilities::DNS;
        }

        // Filesystem
        if self.filesystem.node_storage || self.filesystem.user_storage {
            caps |= WasmCapabilities::STORAGE_ALL;
        }

        // Variables
        if self.variables {
            caps |= WasmCapabilities::VARIABLES_ALL;
        }

        // Cache
        if self.cache {
            caps |= WasmCapabilities::CACHE_ALL;
        }

        // OAuth
        if !self.oauth_scopes.is_empty() {
            caps |= WasmCapabilities::OAUTH;
        }

        // Streaming
        if self.streaming {
            caps |= WasmCapabilities::STREAMING;
        }

        // A2UI
        if self.a2ui {
            caps |= WasmCapabilities::A2UI;
        }

        // Models
        if self.models {
            caps |= WasmCapabilities::MODELS;
        }

        caps
    }

    /// Convert to WasmLimits
    pub fn to_limits(&self) -> WasmLimits {
        WasmLimits {
            memory_limit: self.memory.bytes(),
            timeout: self.timeout.duration(),
            ..Default::default()
        }
    }

    /// Convert to WasmSecurityConfig
    pub fn to_security_config(&self) -> crate::limits::WasmSecurityConfig {
        let capabilities = self.to_capabilities();
        let has_network = capabilities.intersects(WasmCapabilities::NETWORK_ALL);
        crate::limits::WasmSecurityConfig {
            limits: self.to_limits(),
            capabilities,
            allow_wasi: false,
            allow_wasi_network: has_network,
            allowed_hosts: if self.network.allowed_hosts.is_empty() {
                None
            } else {
                Some(self.network.allowed_hosts.clone())
            },
        }
    }

    /// Get human-readable summary of permissions
    pub fn summary(&self) -> Vec<String> {
        let mut perms = Vec::new();

        perms.push(format!("Memory: {}", self.memory.display_name()));
        perms.push(format!("Timeout: {}", self.timeout.display_name()));

        if self.network.http_enabled {
            if self.network.allowed_hosts.is_empty() {
                perms.push("Network: All hosts".to_string());
            } else {
                perms.push(format!(
                    "Network: {}",
                    self.network.allowed_hosts.join(", ")
                ));
            }
        }

        if self.filesystem.node_storage {
            perms.push("Storage: Node-scoped".to_string());
        }
        if self.filesystem.user_storage {
            perms.push("Storage: User-scoped".to_string());
        }

        for oauth in &self.oauth_scopes {
            perms.push(format!(
                "OAuth {}: {} ({})",
                oauth.provider,
                oauth.scopes.join(", "),
                oauth.reason
            ));
        }

        if self.streaming {
            perms.push("Streaming: Enabled".to_string());
        }
        if self.a2ui {
            perms.push("A2UI: Enabled".to_string());
        }
        if self.models {
            perms.push("Models/LLM: Enabled".to_string());
        }

        perms
    }
}

/// Node entry in the package manifest
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageNodeEntry {
    /// Node identifier (used in code)
    pub id: String,
    /// Display name
    pub name: String,
    /// Optional human-friendly name
    #[serde(default)]
    pub friendly_name: Option<String>,
    /// Description
    pub description: String,
    /// Category path
    pub category: String,
    /// Icon (optional, base64 or URL)
    #[serde(default)]
    pub icon: Option<String>,
    /// Node quality / impact scores
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub scores: Option<flow_like::flow::node::NodeScores>,
    /// Pin definitions keyed by pin id
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = HashMap<String, Object>))]
    pub pins: HashMap<String, flow_like::flow::pin::Pin>,
    /// Whether this node is a start node
    #[serde(default)]
    pub start: Option<bool>,
    /// Whether the node is long-running
    #[serde(default)]
    pub long_running: Option<bool>,
    /// Documentation / help text
    #[serde(default)]
    pub docs: Option<String>,
    /// Whether the node supports event callbacks
    #[serde(default)]
    pub event_callback: Option<bool>,
    /// Function references
    #[serde(default)]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub fn_refs: Option<flow_like::flow::node::FnRefs>,
    /// Which OAuth providers this specific node needs
    #[serde(default)]
    pub oauth_providers: Vec<String>,
    /// Required OAuth scopes per provider
    #[serde(default)]
    pub required_oauth_scopes: Option<HashMap<String, Vec<String>>>,
    /// Whether this node can only run offline
    #[serde(default)]
    pub only_offline: bool,
    /// Schema version of this node entry
    #[serde(default)]
    pub version: Option<u32>,
    /// Node-level permission labels
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Additional node-specific metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// Domain category for WASM packages
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WasmPackageCategory {
    DocumentProcessing,
    DataTransformation,
    WorkflowAutomation,
    Communication,
    AnalyticsReporting,
    FinanceBilling,
    ComplianceRegulatory,
    HrPeople,
    AiMl,
    IntegrationConnectors,
    SecurityIdentity,
    Devops,
    IotIndustrial,
    RoboticsPhysicalAi,
    GamingSimulation,
    Healthcare,
    Veterinary,
    Legal,
    Manufacturing,
    Agriculture,
    RealEstate,
    Logistics,
    Energy,
    ConstructionTrades,
    Education,
    GovernmentDefense,
    Ecommerce,
    Insurance,
    Telecom,
    ScientificEngineering,
    Geospatial,
    MediaContent,
    Other,
}

impl std::fmt::Display for WasmPackageCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_value(self)
                .unwrap_or_default()
                .as_str()
                .unwrap_or("OTHER")
        )
    }
}

impl WasmPackageCategory {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
    }
}

/// Package author information
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageAuthor {
    pub name: String,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
}

/// Package manifest - declares everything about a WASM node package
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackageManifest {
    /// Manifest schema version
    pub manifest_version: u32,

    /// Package identifier (reverse domain style, e.g., "com.example.mypackage")
    pub id: String,
    /// Package display name
    pub name: String,
    /// Package version (semver)
    pub version: String,
    /// Package description
    pub description: String,

    /// Package authors
    #[serde(default)]
    pub authors: Vec<PackageAuthor>,
    /// License (SPDX identifier)
    #[serde(default)]
    pub license: Option<String>,
    /// Repository URL
    #[serde(default)]
    pub repository: Option<String>,
    /// Homepage URL
    #[serde(default)]
    pub homepage: Option<String>,

    /// Required permissions for this package
    #[serde(default)]
    pub permissions: PackagePermissions,

    /// Keywords for discovery
    #[serde(default)]
    pub keywords: Vec<String>,

    /// Primary domain category
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_category: Option<WasmPackageCategory>,
    /// Secondary domain category
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary_category: Option<WasmPackageCategory>,

    /// Minimum Flow-Like version required
    #[serde(default)]
    pub min_flow_like_version: Option<String>,

    /// WASM file path relative to manifest (for local development)
    #[serde(default)]
    pub wasm_path: Option<String>,

    /// SHA-256 hash of the WASM file (for integrity verification)
    #[serde(default)]
    pub wasm_hash: Option<String>,

    /// Additional package metadata
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

impl PackageManifest {
    /// Create a new package manifest
    pub fn new(id: &str, name: &str, version: &str, description: &str) -> Self {
        Self {
            manifest_version: MANIFEST_VERSION,
            id: id.to_string(),
            name: name.to_string(),
            version: version.to_string(),
            description: description.to_string(),
            authors: Vec::new(),
            license: None,
            repository: None,
            homepage: None,
            permissions: PackagePermissions::default(),
            keywords: Vec::new(),
            primary_category: None,
            secondary_category: None,
            min_flow_like_version: None,
            wasm_path: None,
            wasm_hash: None,
            metadata: HashMap::new(),
        }
    }

    /// Validate the manifest
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.id.is_empty() {
            errors.push("Package ID is required".to_string());
        }
        if self.name.is_empty() {
            errors.push("Package name is required".to_string());
        }
        if self.version.is_empty() {
            errors.push("Package version is required".to_string());
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Load from TOML string
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    /// Load from JSON string
    pub fn from_json(content: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(content)
    }

    /// Serialize to TOML
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }

    /// Serialize to JSON
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_tier() {
        assert_eq!(MemoryTier::Standard.bytes(), 64 * 1024 * 1024);
        assert_eq!(MemoryTier::Intensive.bytes(), 256 * 1024 * 1024);
        assert_eq!(MemoryTier::Large.bytes(), 512 * 1024 * 1024);
        assert_eq!(MemoryTier::Huge.bytes(), 1024 * 1024 * 1024);
        assert_eq!(MemoryTier::Extreme.bytes(), 2 * 1024 * 1024 * 1024);
        assert_eq!(MemoryTier::Maximum.bytes(), 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_manifest_validation() {
        let manifest = PackageManifest::new(
            "com.example.test",
            "Test Package",
            "1.0.0",
            "A test package",
        );

        assert!(manifest.validate().is_ok());

        let empty = PackageManifest::new("", "Test", "1.0.0", "desc");
        assert!(empty.validate().is_err());
    }
}
