//! Host security configuration for WASM sandboxing.

pub use flow_like_wasm_schema::limits::{
    WasmCapabilities, WasmLimits, DEFAULT_FUEL_LIMIT, DEFAULT_MEMORY_LIMIT, DEFAULT_TIMEOUT,
};
use serde::{Deserialize, Serialize};

/// Combined security configuration for a WASM module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSecurityConfig {
    pub limits: WasmLimits,
    pub capabilities: WasmCapabilities,
    /// Allow general WASI integration. This never implies host environment inheritance.
    pub allow_wasi: bool,
    /// Explicit grant-all override for WASI networking.
    pub allow_wasi_network: bool,
    /// Specific allowed hosts for HTTP.
    pub allowed_hosts: Option<Vec<String>>,
    /// Where the enclosing flow runs. Server-side guest network paths apply
    /// the execution egress policy for this environment.
    #[serde(default)]
    pub execution_environment: flow_like::flow::execution::ExecutionEnvironment,
    /// Metadata extraction closes every guest observation channel.
    #[serde(default)]
    pub deterministic: bool,
}

impl Default for WasmSecurityConfig {
    fn default() -> Self {
        Self {
            limits: WasmLimits::default(),
            capabilities: WasmCapabilities::STANDARD,
            allow_wasi: false,
            allow_wasi_network: false,
            allowed_hosts: None,
            execution_environment: Default::default(),
            deterministic: false,
        }
    }
}

impl WasmSecurityConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn restrictive() -> Self {
        Self {
            limits: WasmLimits::restrictive(),
            capabilities: WasmCapabilities::NONE,
            allow_wasi: false,
            allow_wasi_network: false,
            allowed_hosts: Some(vec![]),
            execution_environment: Default::default(),
            deterministic: false,
        }
    }

    pub fn permissive() -> Self {
        Self {
            limits: WasmLimits::permissive(),
            capabilities: WasmCapabilities::ALL,
            allow_wasi: true,
            allow_wasi_network: true,
            allowed_hosts: None,
            execution_environment: Default::default(),
            deterministic: false,
        }
    }

    pub fn with_limits(mut self, limits: WasmLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_capabilities(mut self, capabilities: WasmCapabilities) -> Self {
        self.capabilities = capabilities;
        self
    }

    pub fn with_allowed_hosts(mut self, hosts: Vec<String>) -> Self {
        self.allowed_hosts = Some(hosts);
        self
    }

    /// Derive the deterministic, closed configuration used to read a module's
    /// node definitions. Resource limits carry over unchanged.
    pub fn for_metadata(&self) -> Self {
        Self {
            limits: self.limits.clone(),
            capabilities: WasmCapabilities::NONE,
            allow_wasi: false,
            allow_wasi_network: false,
            allowed_hosts: Some(vec![]),
            execution_environment: Default::default(),
            deterministic: true,
        }
    }

    /// Take the package-wide settings from the manifest: resource limits and
    /// the host allowlist. Capabilities stay exactly what the node declares;
    /// the registry derives the manifest's capability listing from the nodes,
    /// so it can never be the wider of the two.
    pub fn with_package_settings(mut self, manifest: &Self) -> Self {
        self.limits = manifest.limits.clone();
        self.allowed_hosts = manifest.allowed_hosts.clone();
        self
    }

    /// Build a host security configuration from node-level permissions.
    pub fn from_node_permissions(permissions: &[flow_like::flow::node::NodePermission]) -> Self {
        use flow_like::flow::node::NodePermission;

        let mut capabilities = WasmCapabilities::NONE;
        for permission in permissions {
            capabilities |= match permission {
                NodePermission::NetworkHttp => WasmCapabilities::HTTP_ALL,
                NodePermission::NetworkWebsocket => WasmCapabilities::WEBSOCKET,
                NodePermission::NetworkTcp => WasmCapabilities::TCP,
                NodePermission::NetworkUdp => WasmCapabilities::UDP,
                NodePermission::NetworkDns => WasmCapabilities::DNS,
                NodePermission::StorageRead => WasmCapabilities::STORAGE_READ,
                NodePermission::StorageWrite => {
                    WasmCapabilities::STORAGE_WRITE | WasmCapabilities::STORAGE_DELETE
                }
                NodePermission::DatabaseRead => WasmCapabilities::DATABASE_READ,
                NodePermission::DatabaseWrite => WasmCapabilities::DATABASE_WRITE,
                NodePermission::Variables => WasmCapabilities::VARIABLES_ALL,
                NodePermission::Cache => WasmCapabilities::CACHE_ALL,
                NodePermission::Streaming => WasmCapabilities::STREAMING,
                NodePermission::Models => WasmCapabilities::MODELS,
                NodePermission::A2ui => WasmCapabilities::A2UI,
                NodePermission::OAuth => WasmCapabilities::OAUTH,
                NodePermission::Functions => WasmCapabilities::FUNCTIONS,
            };
        }

        Self {
            limits: WasmLimits::default(),
            capabilities,
            allow_wasi: false,
            allow_wasi_network: false,
            allowed_hosts: None,
            execution_environment: Default::default(),
            deterministic: false,
        }
    }
}

pub(crate) fn store_limits(limits: &WasmLimits) -> wasmtime::StoreLimits {
    wasmtime::StoreLimitsBuilder::new()
        .memory_size(limits.memory_limit)
        .table_elements(limits.max_table_elements as usize)
        .instances(limits.max_instances as usize)
        .tables(limits.max_tables as usize)
        .memories(limits.max_memories as usize)
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_mode_closes_every_observation_channel() {
        let metadata = WasmSecurityConfig::permissive().for_metadata();
        assert_eq!(metadata.capabilities, WasmCapabilities::NONE);
        assert!(!metadata.allow_wasi);
        assert!(!metadata.allow_wasi_network);
        assert_eq!(metadata.allowed_hosts.as_deref(), Some(&[][..]));
        assert!(metadata.deterministic);
        assert_eq!(
            metadata.execution_environment,
            flow_like::flow::execution::ExecutionEnvironment::default()
        );
    }

    #[test]
    fn metadata_mode_preserves_resource_limits() {
        let permissive = WasmSecurityConfig::permissive();
        let metadata = permissive.for_metadata();
        assert_eq!(metadata.limits.memory_limit, permissive.limits.memory_limit);
        assert_eq!(metadata.limits.fuel_limit, permissive.limits.fuel_limit);
        assert_eq!(metadata.limits.timeout, permissive.limits.timeout);
    }

    #[test]
    fn package_settings_leave_node_capabilities_alone() {
        use flow_like::flow::node::NodePermission;

        // A manifest without capability flags, as the templates ship it.
        let manifest = WasmSecurityConfig::from_node_permissions(&[])
            .with_allowed_hosts(vec!["api.example.com".to_string()]);
        let declared = WasmSecurityConfig::from_node_permissions(&[
            NodePermission::DatabaseRead,
            NodePermission::StorageWrite,
        ]);
        let node = declared.clone().with_package_settings(&manifest);

        assert_eq!(node.capabilities, declared.capabilities);
        assert!(!node.capabilities.intersects(WasmCapabilities::HTTP_ALL));
        assert_eq!(
            node.allowed_hosts.as_deref(),
            Some(&["api.example.com".to_string()][..])
        );
        assert_eq!(node.limits.memory_limit, manifest.limits.memory_limit);
    }
}
