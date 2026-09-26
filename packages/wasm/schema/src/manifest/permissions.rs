use crate::limits::{WasmCapabilities, WasmLimits};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

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

/// Database permissions still require wired, typed handles for each resource.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DatabasePermissions {
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
}

/// Package permissions declaration.
///
/// Authors set what nodes cannot state: the resource tiers, the outbound host
/// allowlist, and the OAuth scopes with their reasons. The capability flags are
/// what the store lists, and the registry derives them from the compiled node
/// definitions (`with_node_capabilities`), because each node declares the
/// permissions the sandbox enforces.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackagePermissions {
    #[serde(default)]
    pub database: DatabasePermissions,
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
        if self.database.read {
            capabilities |= WasmCapabilities::DATABASE_READ;
        }
        if self.database.write {
            capabilities |= WasmCapabilities::DATABASE_WRITE;
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

    /// Capability flags an authored manifest still sets although no node backs
    /// them, phrased for package authors. Capabilities the nodes declare and
    /// the manifest omits are the normal case, not a difference.
    pub fn unbacked_capability_flags(authored: &Self, derived: &Self) -> Vec<String> {
        let authored_tags: BTreeSet<String> = authored.capability_tags().into_iter().collect();
        let derived_tags: BTreeSet<String> = derived.capability_tags().into_iter().collect();
        authored_tags
            .difference(&derived_tags)
            .map(|tag| {
                format!(
                    "flow-like.toml declares `{tag}`, but no node uses it; capabilities come from the nodes, so remove the flag"
                )
            })
            .collect()
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
        if self.database.read {
            permissions.push("Database: Read connected tables and sessions".to_string());
        }
        if self.database.write {
            permissions.push("Database: Write connected tables".to_string());
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
        push(self.database.read, "database.read");
        push(self.database.write, "database.write");
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

#[cfg(feature = "nodes")]
impl PackagePermissions {
    /// This manifest's resource tiers, host allowlist and OAuth scopes with the
    /// capability flags replaced by the permissions the given nodes declare.
    pub fn with_capabilities_from<'a>(
        &self,
        permissions: impl IntoIterator<Item = &'a flow_like::flow::node::NodePermission>,
    ) -> Self {
        use flow_like::flow::node::NodePermission;

        let mut derived = Self {
            memory: self.memory,
            timeout: self.timeout,
            network: NetworkPermissions {
                allowed_hosts: self.network.allowed_hosts.clone(),
                ..Default::default()
            },
            oauth_scopes: self.oauth_scopes.clone(),
            ..Default::default()
        };
        for permission in permissions {
            match permission {
                NodePermission::NetworkHttp => derived.network.http_enabled = true,
                NodePermission::NetworkWebsocket => derived.network.websocket_enabled = true,
                NodePermission::NetworkTcp => derived.network.tcp_enabled = true,
                NodePermission::NetworkUdp => derived.network.udp_enabled = true,
                NodePermission::NetworkDns => derived.network.dns_enabled = true,
                // The sandbox gates the node, user, upload and cache
                // directories on one storage capability, so list them all.
                NodePermission::StorageRead | NodePermission::StorageWrite => {
                    derived.filesystem = FileSystemPermissions {
                        node_storage: true,
                        user_storage: true,
                        upload_dir: true,
                        cache_dir: true,
                    }
                }
                NodePermission::DatabaseRead => derived.database.read = true,
                NodePermission::DatabaseWrite => derived.database.write = true,
                NodePermission::Variables => derived.variables = true,
                NodePermission::Cache => derived.cache = true,
                NodePermission::Streaming => derived.streaming = true,
                NodePermission::Models => derived.models = true,
                NodePermission::A2ui => derived.a2ui = true,
                // OAuth is listed through the authored scopes; nodes cannot
                // state providers, scopes or reasons.
                NodePermission::OAuth | NodePermission::Functions => {}
            }
        }
        derived
    }

    /// `with_capabilities_from` over stored node entries. Scopes a node entry
    /// requires are added to the authored ones for the same provider.
    pub fn with_node_capabilities(&self, nodes: &[super::node::PackageNodeEntry]) -> Self {
        let mut derived =
            self.with_capabilities_from(nodes.iter().flat_map(|node| node.permissions.iter()));

        for node in nodes {
            let label = node.friendly_name.as_deref().unwrap_or(&node.name);
            for (provider, scopes) in node.required_oauth_scopes.iter().flatten() {
                let position = derived
                    .oauth_scopes
                    .iter()
                    .position(|requirement| &requirement.provider == provider);
                let requirement = match position {
                    Some(position) => &mut derived.oauth_scopes[position],
                    None => {
                        derived.oauth_scopes.push(OAuthScopeRequirement {
                            provider: provider.clone(),
                            scopes: Vec::new(),
                            reason: format!("Required by {label}"),
                            required: true,
                        });
                        derived.oauth_scopes.last_mut().expect("just pushed")
                    }
                };
                for scope in scopes {
                    if !requirement.scopes.contains(scope) {
                        requirement.scopes.push(scope.clone());
                    }
                }
            }
        }
        derived
    }
}

#[cfg(all(test, feature = "nodes"))]
mod node_capability_tests {
    use super::*;
    use flow_like::flow::node::NodePermission;
    use std::collections::HashMap;

    fn node(name: &str, permissions: &[NodePermission]) -> super::super::node::PackageNodeEntry {
        super::super::node::PackageNodeEntry {
            id: name.to_string(),
            name: name.to_string(),
            permissions: permissions.to_vec(),
            ..Default::default()
        }
    }

    #[test]
    fn capabilities_come_from_nodes_and_tiers_from_the_manifest() {
        let authored: PackagePermissions = serde_json::from_str(
            r#"{"memory":"heavy","network":{"allowed_hosts":["packages.typst.org"]},"filesystem":{"user_storage":true}}"#,
        )
        .unwrap();

        let derived = authored.with_node_capabilities(&[
            node(
                "compile",
                &[NodePermission::NetworkHttp, NodePermission::StorageRead],
            ),
            node("render", &[NodePermission::Models]),
        ]);

        assert_eq!(derived.memory, MemoryTier::Heavy);
        assert_eq!(derived.network.allowed_hosts, ["packages.typst.org"]);
        assert_eq!(
            derived.capability_tags(),
            [
                "net.http",
                "models",
                "storage.user",
                "storage.node",
                "storage.uploads",
                "storage.cache"
            ]
        );
        // Capabilities only the nodes declare are the normal case.
        assert!(PackagePermissions::unbacked_capability_flags(&authored, &derived).is_empty());
    }

    #[test]
    fn authored_flags_without_a_node_behind_them_are_dropped() {
        let authored: PackagePermissions =
            serde_json::from_str(r#"{"streaming":true,"network":{"http_enabled":true}}"#).unwrap();

        let derived = authored.with_node_capabilities(&[node("plain", &[])]);

        assert!(derived.capability_tags().is_empty());
        let unbacked = PackagePermissions::unbacked_capability_flags(&authored, &derived);
        assert_eq!(unbacked.len(), 2);
        assert!(unbacked[0].starts_with("flow-like.toml declares `net.http`, but no node uses it"));
        assert!(unbacked[1].starts_with("flow-like.toml declares `streaming`, but no node uses it"));
    }

    #[test]
    fn oauth_scopes_stay_authored_and_absorb_node_requirements() {
        let authored: PackagePermissions = serde_json::from_str(
            r#"{"oauth_scopes":[{"provider":"google","scopes":["calendar.read"],"reason":"Reads your calendar","required":true}]}"#,
        )
        .unwrap();
        let mut mail = node("Mail", &[NodePermission::OAuth]);
        mail.required_oauth_scopes = Some(HashMap::from([
            (
                "google".to_string(),
                vec!["mail.send".to_string(), "calendar.read".to_string()],
            ),
            ("github".to_string(), vec!["repo".to_string()]),
        ]));

        // Compiled WASM nodes carry no OAuth data today, so the authored
        // scopes must survive on their own.
        let compiled = authored.with_node_capabilities(&[node("Plain", &[NodePermission::OAuth])]);
        assert_eq!(compiled.capability_tags(), ["oauth"]);
        assert_eq!(compiled.oauth_scopes[0].reason, "Reads your calendar");
        assert!(PackagePermissions::unbacked_capability_flags(&authored, &compiled).is_empty());

        let derived = authored.with_node_capabilities(&[mail]);
        assert_eq!(derived.oauth_scopes.len(), 2);
        assert_eq!(derived.oauth_scopes[0].provider, "google");
        assert_eq!(derived.oauth_scopes[0].scopes, ["calendar.read", "mail.send"]);
        assert_eq!(derived.oauth_scopes[0].reason, "Reads your calendar");
        assert_eq!(derived.oauth_scopes[1].provider, "github");
        assert_eq!(derived.oauth_scopes[1].reason, "Required by Mail");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn database_permissions_are_explicit_and_independent() {
        let read: super::PackagePermissions =
            serde_json::from_str(r#"{"database":{"read":true}}"#).unwrap();
        assert!(
            read.to_capabilities()
                .has(crate::limits::WasmCapabilities::DATABASE_READ)
        );
        assert_eq!(read.capability_tags(), ["database.read"]);
        assert!(
            read.summary()
                .iter()
                .any(|entry| entry == "Database: Read connected tables and sessions")
        );
        assert!(
            !read
                .to_capabilities()
                .has(crate::limits::WasmCapabilities::DATABASE_WRITE)
        );
        let models: super::PackagePermissions = serde_json::from_str(r#"{"models":true}"#).unwrap();
        assert!(!models.to_capabilities().intersects(
            crate::limits::WasmCapabilities::DATABASE_READ
                | crate::limits::WasmCapabilities::DATABASE_WRITE
        ));
    }

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
