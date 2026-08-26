use crate::limits::{WasmCapabilities, WasmLimits};
use serde::{Deserialize, Serialize};

/// Memory tier presets for packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MemoryTier {
    Minimal,
    Light,
    #[default]
    Standard,
    Heavy,
    Intensive,
    Large,
    Huge,
    Extreme,
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

/// Timeout tier presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TimeoutTier {
    Quick,
    #[default]
    Standard,
    Extended,
    LongRunning,
    VeryLong,
    Maximum,
}

impl TimeoutTier {
    pub fn duration(&self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Self::Quick => 5,
            Self::Standard => 30,
            Self::Extended => 60,
            Self::LongRunning => 300,
            Self::VeryLong => 600,
            Self::Maximum => 1800,
        })
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

/// OAuth scope requirement for a specific provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuthScopeRequirement {
    pub provider: String,
    pub scopes: Vec<String>,
    pub reason: String,
    #[serde(default)]
    pub required: bool,
}

/// Network access requirements.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NetworkPermissions {
    #[serde(default)]
    pub http_enabled: bool,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub websocket_enabled: bool,
    #[serde(default)]
    pub tcp_enabled: bool,
    #[serde(default)]
    pub udp_enabled: bool,
    #[serde(default)]
    pub dns_enabled: bool,
}

/// File-system access requirements.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FileSystemPermissions {
    #[serde(default)]
    pub node_storage: bool,
    #[serde(default)]
    pub user_storage: bool,
    #[serde(default)]
    pub upload_dir: bool,
    #[serde(default)]
    pub cache_dir: bool,
}

/// Package permissions declaration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackagePermissions {
    #[serde(default)]
    pub memory: MemoryTier,
    #[serde(default)]
    pub timeout: TimeoutTier,
    #[serde(default)]
    pub network: NetworkPermissions,
    #[serde(default)]
    pub filesystem: FileSystemPermissions,
    #[serde(default)]
    pub oauth_scopes: Vec<OAuthScopeRequirement>,
    #[serde(default)]
    pub variables: bool,
    #[serde(default)]
    pub cache: bool,
    #[serde(default)]
    pub streaming: bool,
    #[serde(default)]
    pub a2ui: bool,
    #[serde(default)]
    pub models: bool,
}

/// Host-side adapter for turning manifest permissions into a runtime config.
///
/// The schema crate owns the data contract but deliberately knows nothing
/// about a Wasmtime host. `flow-like-wasm` implements this for its
/// `WasmSecurityConfig`, preserving the familiar `permissions.to_security_config()`
/// call without pulling the runtime into registry/API builds.
pub trait PackageSecurityConfig: Sized {
    fn from_package_permissions(permissions: &PackagePermissions) -> Self;
}

impl PackagePermissions {
    pub fn to_capabilities(&self) -> WasmCapabilities {
        let mut capabilities = WasmCapabilities::NONE;

        if self.network.http_enabled {
            capabilities |= WasmCapabilities::HTTP_ALL;
        }
        if self.network.websocket_enabled {
            capabilities |= WasmCapabilities::WEBSOCKET;
        }
        if self.network.tcp_enabled {
            capabilities |= WasmCapabilities::TCP;
        }
        if self.network.udp_enabled {
            capabilities |= WasmCapabilities::UDP;
        }
        if self.network.dns_enabled {
            capabilities |= WasmCapabilities::DNS;
        }
        if self.filesystem.node_storage || self.filesystem.user_storage {
            capabilities |= WasmCapabilities::STORAGE_ALL;
        }
        if self.variables {
            capabilities |= WasmCapabilities::VARIABLES_ALL;
        }
        if self.cache {
            capabilities |= WasmCapabilities::CACHE_ALL;
        }
        if !self.oauth_scopes.is_empty() {
            capabilities |= WasmCapabilities::OAUTH;
        }
        if self.streaming {
            capabilities |= WasmCapabilities::STREAMING;
        }
        if self.a2ui {
            capabilities |= WasmCapabilities::A2UI;
        }
        if self.models {
            capabilities |= WasmCapabilities::MODELS;
        }

        capabilities
    }

    pub fn to_limits(&self) -> WasmLimits {
        WasmLimits {
            memory_limit: self.memory.bytes(),
            timeout: self.timeout.duration(),
            ..Default::default()
        }
    }

    pub fn to_security_config<T: PackageSecurityConfig>(&self) -> T {
        T::from_package_permissions(self)
    }

    pub fn summary(&self) -> Vec<String> {
        let mut permissions = vec![
            format!("Memory: {}", self.memory.display_name()),
            format!("Timeout: {}", self.timeout.display_name()),
        ];

        if self.network.http_enabled {
            if self.network.allowed_hosts.is_empty() {
                permissions.push("Network: All hosts".to_string());
            } else {
                permissions.push(format!(
                    "Network: {}",
                    self.network.allowed_hosts.join(", ")
                ));
            }
        }
        if self.filesystem.node_storage {
            permissions.push("Storage: Node-scoped".to_string());
        }
        if self.filesystem.user_storage {
            permissions.push("Storage: User-scoped".to_string());
        }
        for oauth in &self.oauth_scopes {
            permissions.push(format!(
                "OAuth {}: {} ({})",
                oauth.provider,
                oauth.scopes.join(", "),
                oauth.reason
            ));
        }
        if self.streaming {
            permissions.push("Streaming: Enabled".to_string());
        }
        if self.a2ui {
            permissions.push("A2UI: Enabled".to_string());
        }
        if self.models {
            permissions.push("Models/LLM: Enabled".to_string());
        }

        permissions
    }

    /// Stable machine-readable capability tags for listing surfaces.
    pub fn capability_tags(&self) -> Vec<String> {
        let mut tags = Vec::new();
        let mut push = |enabled: bool, tag: &str| {
            if enabled {
                tags.push(tag.to_string());
            }
        };

        push(self.network.http_enabled, "net.http");
        push(self.network.websocket_enabled, "net.ws");
        push(self.network.tcp_enabled, "net.tcp");
        push(self.network.udp_enabled, "net.udp");
        push(self.network.dns_enabled, "net.dns");
        push(!self.oauth_scopes.is_empty(), "oauth");
        push(self.models, "models");
        push(self.filesystem.user_storage, "storage.user");
        push(self.filesystem.node_storage, "storage.node");
        push(self.filesystem.upload_dir, "storage.uploads");
        push(self.filesystem.cache_dir, "storage.cache");
        push(self.variables, "variables");
        push(self.cache, "cache");
        push(self.streaming, "streaming");
        push(self.a2ui, "a2ui");
        tags
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_tiers_keep_the_wire_contract_values() {
        assert_eq!(MemoryTier::Standard.bytes(), 64 * 1024 * 1024);
        assert_eq!(MemoryTier::Maximum.bytes(), 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn capability_tags_list_sensitive_access_first() {
        let permissions = PackagePermissions {
            network: NetworkPermissions {
                http_enabled: true,
                ..Default::default()
            },
            filesystem: FileSystemPermissions {
                node_storage: true,
                user_storage: true,
                ..Default::default()
            },
            cache: true,
            models: true,
            ..Default::default()
        };

        assert_eq!(
            permissions.capability_tags(),
            [
                "net.http",
                "models",
                "storage.user",
                "storage.node",
                "cache"
            ]
        );
    }
}
