//! Resource limits and capability system for WASM sandboxing

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default memory limit: 256MB (needed for heavy runtimes like Python, Kotlin, C#)
pub const DEFAULT_MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// Default execution timeout: 120 seconds (heavy runtimes need more startup time)
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Default fuel limit: 100 billion instructions (~100s of compute)
pub const DEFAULT_FUEL_LIMIT: u64 = 100_000_000_000;

/// Resource limits for WASM execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmLimits {
    /// Maximum memory in bytes (default: 64MB)
    pub memory_limit: usize,

    /// Maximum execution time
    pub timeout: Duration,

    /// Maximum fuel (instruction count)
    pub fuel_limit: u64,

    /// Maximum stack depth
    pub max_stack_depth: u32,

    /// Maximum number of tables
    pub max_tables: u32,

    /// Maximum number of memories
    pub max_memories: u32,

    /// Maximum table elements
    pub max_table_elements: u32,

    /// Maximum instances
    pub max_instances: u32,
}

impl Default for WasmLimits {
    fn default() -> Self {
        Self {
            memory_limit: DEFAULT_MEMORY_LIMIT,
            timeout: DEFAULT_TIMEOUT,
            fuel_limit: DEFAULT_FUEL_LIMIT,
            max_stack_depth: 1024,
            max_tables: 100,
            max_memories: 10,
            max_table_elements: 100000,
            max_instances: 100,
        }
    }
}

impl WasmLimits {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrictive limits for untrusted code
    pub fn restrictive() -> Self {
        Self {
            memory_limit: 16 * 1024 * 1024, // 16MB
            timeout: Duration::from_secs(10),
            fuel_limit: 1_000_000_000, // ~1s of compute
            max_stack_depth: 256,
            max_tables: 2,
            max_memories: 1,
            max_table_elements: 1000,
            max_instances: 2,
        }
    }

    /// Permissive limits for trusted code
    pub fn permissive() -> Self {
        Self {
            memory_limit: 256 * 1024 * 1024, // 256MB
            timeout: Duration::from_secs(300),
            fuel_limit: 100_000_000_000, // ~100s of compute
            max_stack_depth: 1024,
            max_tables: 100,
            max_memories: 10,
            max_table_elements: 100000,
            max_instances: 100,
        }
    }

    pub fn with_memory_limit(mut self, bytes: usize) -> Self {
        self.memory_limit = bytes;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_fuel_limit(mut self, fuel: u64) -> Self {
        self.fuel_limit = fuel;
        self
    }
}

bitflags::bitflags! {
    /// Capabilities that can be granted to WASM modules
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct WasmCapabilities: u32 {
        /// No capabilities
        const NONE              = 0b00000000_00000000_00000000_00000000;

        // === Storage ===
        /// Read from storage
        const STORAGE_READ      = 0b00000000_00000000_00000000_00000001;
        /// Write to storage
        const STORAGE_WRITE     = 0b00000000_00000000_00000000_00000010;
        /// Delete from storage
        const STORAGE_DELETE    = 0b00000000_00000000_00000000_00000100;

        // === Network ===
        /// Make HTTP GET requests
        const HTTP_GET          = 0b00000000_00000000_00000000_00001000;
        /// Make HTTP POST/PUT/DELETE requests
        const HTTP_WRITE        = 0b00000000_00000000_00000000_00010000;
        /// Open WebSocket connections
        const WEBSOCKET         = 0b00000000_00000000_10000000_00000000;
        /// Open TCP socket connections
        const TCP               = 0b00000000_00000001_00000000_00000000;
        /// Open UDP socket connections
        const UDP               = 0b00000000_00000010_00000000_00000000;
        /// Perform DNS lookups
        const DNS               = 0b00000000_00000100_00000000_00000000;

        // === Flow Context ===
        /// Read variables
        const VARIABLES_READ    = 0b00000000_00000000_00000000_00100000;
        /// Write variables
        const VARIABLES_WRITE   = 0b00000000_00000000_00000000_01000000;
        /// Read cache
        const CACHE_READ        = 0b00000000_00000000_00000000_10000000;
        /// Write cache
        const CACHE_WRITE       = 0b00000000_00000000_00000001_00000000;

        // === Authentication ===
        /// Access OAuth tokens
        const OAUTH             = 0b00000000_00000000_00000010_00000000;
        /// Alias for OAuth access
        const OAUTH_ACCESS      = Self::OAUTH.bits();
        /// Access execution token
        const TOKEN             = 0b00000000_00000000_00000100_00000000;

        // === Streaming ===
        /// Stream responses to client
        const STREAMING         = 0b00000000_00000000_00001000_00000000;
        /// A2UI operations
        const A2UI              = 0b00000000_00000000_00010000_00000000;

        // === Advanced ===
        /// Access LLM/Model providers
        const MODELS            = 0b00000000_00000000_00100000_00000000;
        /// Execute referenced functions
        const FUNCTIONS         = 0b00000000_00000000_01000000_00000000;

        // === Compound capabilities ===
        /// All storage operations
        const STORAGE_ALL   = Self::STORAGE_READ.bits() | Self::STORAGE_WRITE.bits() | Self::STORAGE_DELETE.bits();
        /// All HTTP operations
        const HTTP_ALL      = Self::HTTP_GET.bits() | Self::HTTP_WRITE.bits();
        /// Alias for HTTP request capability
        const HTTP_REQUEST  = Self::HTTP_ALL.bits();
        /// All network operations (HTTP + WebSocket + TCP + UDP + DNS)
        const NETWORK_ALL   = Self::HTTP_ALL.bits() | Self::WEBSOCKET.bits() | Self::TCP.bits() | Self::UDP.bits() | Self::DNS.bits();
        /// All variable operations
        const VARIABLES_ALL = Self::VARIABLES_READ.bits() | Self::VARIABLES_WRITE.bits();
        /// All cache operations
        const CACHE_ALL     = Self::CACHE_READ.bits() | Self::CACHE_WRITE.bits();
        /// All authentication
        const AUTH_ALL      = Self::OAUTH.bits() | Self::TOKEN.bits();

        /// Standard capabilities for most nodes
        const STANDARD = Self::STORAGE_READ.bits()
            | Self::HTTP_GET.bits()
            | Self::VARIABLES_READ.bits()
            | Self::CACHE_ALL.bits();

        /// Full capabilities
        const ALL = Self::STORAGE_ALL.bits()
            | Self::NETWORK_ALL.bits()
            | Self::VARIABLES_ALL.bits()
            | Self::CACHE_ALL.bits()
            | Self::AUTH_ALL.bits()
            | Self::STREAMING.bits()
            | Self::A2UI.bits()
            | Self::MODELS.bits()
            | Self::FUNCTIONS.bits();
    }
}

impl Serialize for WasmCapabilities {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.bits().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for WasmCapabilities {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let bits = u32::deserialize(deserializer)?;
        Ok(WasmCapabilities::from_bits_truncate(bits))
    }
}

impl Default for WasmCapabilities {
    fn default() -> Self {
        Self::STANDARD
    }
}

impl WasmCapabilities {
    /// Check if a specific capability is granted
    pub fn has(&self, cap: WasmCapabilities) -> bool {
        self.contains(cap)
    }

    /// Create from a list of capability names
    pub fn from_names(names: &[&str]) -> Self {
        let mut caps = Self::NONE;
        for name in names {
            match *name {
                "storage_read" => caps |= Self::STORAGE_READ,
                "storage_write" => caps |= Self::STORAGE_WRITE,
                "storage_delete" => caps |= Self::STORAGE_DELETE,
                "storage_all" | "storage" => caps |= Self::STORAGE_ALL,
                "http_get" => caps |= Self::HTTP_GET,
                "http_write" => caps |= Self::HTTP_WRITE,
                "http_all" | "http" => caps |= Self::HTTP_ALL,
                "websocket" | "network:websocket" | "ws" => caps |= Self::WEBSOCKET,
                "tcp" | "network:tcp" => caps |= Self::TCP,
                "udp" | "network:udp" => caps |= Self::UDP,
                "dns" | "network:dns" => caps |= Self::DNS,
                "network_all" | "network" => caps |= Self::NETWORK_ALL,
                "variables_read" => caps |= Self::VARIABLES_READ,
                "variables_write" => caps |= Self::VARIABLES_WRITE,
                "variables_all" | "variables" => caps |= Self::VARIABLES_ALL,
                "cache_read" => caps |= Self::CACHE_READ,
                "cache_write" => caps |= Self::CACHE_WRITE,
                "cache_all" | "cache" => caps |= Self::CACHE_ALL,
                "oauth" => caps |= Self::OAUTH,
                "token" => caps |= Self::TOKEN,
                "auth_all" | "auth" => caps |= Self::AUTH_ALL,
                "streaming" => caps |= Self::STREAMING,
                "a2ui" => caps |= Self::A2UI,
                "models" | "llm" => caps |= Self::MODELS,
                "functions" => caps |= Self::FUNCTIONS,
                "standard" => caps |= Self::STANDARD,
                "all" => caps |= Self::ALL,
                _ => {}
            }
        }
        caps
    }
}

/// Combined security configuration for a WASM module
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmSecurityConfig {
    /// Resource limits
    pub limits: WasmLimits,
    /// Granted capabilities
    pub capabilities: WasmCapabilities,
    /// Allow WASI (file system, env vars, etc.)
    pub allow_wasi: bool,
    /// Allow networking through WASI
    pub allow_wasi_network: bool,
    /// Specific allowed hosts for HTTP
    pub allowed_hosts: Option<Vec<String>>,
}

impl Default for WasmSecurityConfig {
    fn default() -> Self {
        Self {
            limits: WasmLimits::default(),
            capabilities: WasmCapabilities::STANDARD,
            allow_wasi: false,
            allow_wasi_network: false,
            allowed_hosts: None,
        }
    }
}

impl WasmSecurityConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrictive config for untrusted modules
    pub fn restrictive() -> Self {
        Self {
            limits: WasmLimits::restrictive(),
            capabilities: WasmCapabilities::NONE,
            allow_wasi: false,
            allow_wasi_network: false,
            allowed_hosts: Some(vec![]),
        }
    }

    /// Permissive config for trusted modules
    pub fn permissive() -> Self {
        Self {
            limits: WasmLimits::permissive(),
            capabilities: WasmCapabilities::ALL,
            allow_wasi: true,
            allow_wasi_network: true,
            allowed_hosts: None,
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

    /// Build a security config from a set of node-level permissions.
    /// This is the primary way to derive sandbox capabilities from what
    /// individual nodes declare they need.
    pub fn from_node_permissions(permissions: &[flow_like::flow::node::NodePermission]) -> Self {
        use flow_like::flow::node::NodePermission;

        let mut caps = WasmCapabilities::NONE;
        for perm in permissions {
            caps |= match perm {
                NodePermission::NetworkHttp => WasmCapabilities::HTTP_ALL,
                NodePermission::NetworkWebsocket => WasmCapabilities::WEBSOCKET,
                NodePermission::NetworkTcp => WasmCapabilities::TCP,
                NodePermission::NetworkUdp => WasmCapabilities::UDP,
                NodePermission::NetworkDns => WasmCapabilities::DNS,
                NodePermission::StorageRead => WasmCapabilities::STORAGE_READ,
                NodePermission::StorageWrite => {
                    WasmCapabilities::STORAGE_WRITE | WasmCapabilities::STORAGE_DELETE
                }
                NodePermission::Variables => WasmCapabilities::VARIABLES_ALL,
                NodePermission::Cache => WasmCapabilities::CACHE_ALL,
                NodePermission::Streaming => WasmCapabilities::STREAMING,
                NodePermission::Models => WasmCapabilities::MODELS,
                NodePermission::A2ui => WasmCapabilities::A2UI,
                NodePermission::OAuth => WasmCapabilities::OAUTH,
                NodePermission::Functions => WasmCapabilities::FUNCTIONS,
            };
        }

        let has_network = caps.intersects(WasmCapabilities::NETWORK_ALL);
        Self {
            limits: WasmLimits::default(),
            capabilities: caps,
            allow_wasi: false,
            allow_wasi_network: has_network,
            allowed_hosts: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capabilities_from_names() {
        let caps = WasmCapabilities::from_names(&["storage_read", "http_get", "cache"]);
        assert!(caps.has(WasmCapabilities::STORAGE_READ));
        assert!(caps.has(WasmCapabilities::HTTP_GET));
        assert!(caps.has(WasmCapabilities::CACHE_READ));
        assert!(caps.has(WasmCapabilities::CACHE_WRITE));
        assert!(!caps.has(WasmCapabilities::STORAGE_WRITE));
    }

    #[test]
    fn test_websocket_capability() {
        let caps = WasmCapabilities::from_names(&["websocket"]);
        assert!(caps.has(WasmCapabilities::WEBSOCKET));
        assert!(!caps.has(WasmCapabilities::HTTP_GET));

        let caps = WasmCapabilities::from_names(&["network"]);
        assert!(caps.has(WasmCapabilities::HTTP_GET));
        assert!(caps.has(WasmCapabilities::HTTP_WRITE));
        assert!(caps.has(WasmCapabilities::WEBSOCKET));
    }

    #[test]
    fn test_standard_capabilities() {
        let caps = WasmCapabilities::STANDARD;
        assert!(caps.has(WasmCapabilities::STORAGE_READ));
        assert!(caps.has(WasmCapabilities::HTTP_GET));
        assert!(caps.has(WasmCapabilities::CACHE_READ));
        assert!(!caps.has(WasmCapabilities::STORAGE_WRITE));
        assert!(!caps.has(WasmCapabilities::HTTP_WRITE));
    }
}
