//! Runtime-independent WASM resource limits and capability flags.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Default memory limit: 256 MB (needed for heavy guest runtimes).
pub const DEFAULT_MEMORY_LIMIT: usize = 256 * 1024 * 1024;

/// Default execution timeout: 120 seconds.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Default fuel limit: 100 billion instructions.
pub const DEFAULT_FUEL_LIMIT: u64 = 100_000_000_000;

/// Resource limits for WASM execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmLimits {
    pub memory_limit: usize,
    pub timeout: Duration,
    pub fuel_limit: u64,
    pub max_stack_depth: u32,
    pub max_tables: u32,
    pub max_memories: u32,
    pub max_table_elements: u32,
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
            max_table_elements: 100_000,
            max_instances: 100,
        }
    }
}

impl WasmLimits {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn restrictive() -> Self {
        Self {
            memory_limit: 16 * 1024 * 1024,
            timeout: Duration::from_secs(10),
            fuel_limit: 1_000_000_000,
            max_stack_depth: 256,
            max_tables: 2,
            max_memories: 1,
            max_table_elements: 1_000,
            max_instances: 2,
        }
    }

    pub fn permissive() -> Self {
        Self {
            memory_limit: 256 * 1024 * 1024,
            timeout: Duration::from_secs(300),
            fuel_limit: 100_000_000_000,
            max_stack_depth: 1024,
            max_tables: 100,
            max_memories: 10,
            max_table_elements: 100_000,
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
    /// Capabilities that can be granted to WASM modules.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub struct WasmCapabilities: u32 {
        const NONE              = 0b00000000_00000000_00000000_00000000;
        const STORAGE_READ      = 0b00000000_00000000_00000000_00000001;
        const STORAGE_WRITE     = 0b00000000_00000000_00000000_00000010;
        const STORAGE_DELETE    = 0b00000000_00000000_00000000_00000100;
        const HTTP_GET          = 0b00000000_00000000_00000000_00001000;
        const HTTP_WRITE        = 0b00000000_00000000_00000000_00010000;
        const WEBSOCKET         = 0b00000000_00000000_10000000_00000000;
        const TCP               = 0b00000000_00000001_00000000_00000000;
        const UDP               = 0b00000000_00000010_00000000_00000000;
        const DNS               = 0b00000000_00000100_00000000_00000000;
        const VARIABLES_READ    = 0b00000000_00000000_00000000_00100000;
        const VARIABLES_WRITE   = 0b00000000_00000000_00000000_01000000;
        const CACHE_READ        = 0b00000000_00000000_00000000_10000000;
        const CACHE_WRITE       = 0b00000000_00000000_00000001_00000000;
        const OAUTH             = 0b00000000_00000000_00000010_00000000;
        const OAUTH_ACCESS      = Self::OAUTH.bits();
        const TOKEN             = 0b00000000_00000000_00000100_00000000;
        const STREAMING         = 0b00000000_00000000_00001000_00000000;
        const A2UI              = 0b00000000_00000000_00010000_00000000;
        const MODELS            = 0b00000000_00000000_00100000_00000000;
        const FUNCTIONS         = 0b00000000_00000000_01000000_00000000;

        const STORAGE_ALL   = Self::STORAGE_READ.bits() | Self::STORAGE_WRITE.bits() | Self::STORAGE_DELETE.bits();
        const HTTP_ALL      = Self::HTTP_GET.bits() | Self::HTTP_WRITE.bits();
        const HTTP_REQUEST  = Self::HTTP_ALL.bits();
        const NETWORK_ALL   = Self::HTTP_ALL.bits() | Self::WEBSOCKET.bits() | Self::TCP.bits() | Self::UDP.bits() | Self::DNS.bits();
        const VARIABLES_ALL = Self::VARIABLES_READ.bits() | Self::VARIABLES_WRITE.bits();
        const CACHE_ALL     = Self::CACHE_READ.bits() | Self::CACHE_WRITE.bits();
        const AUTH_ALL      = Self::OAUTH.bits() | Self::TOKEN.bits();
        const STANDARD      = Self::STORAGE_READ.bits() | Self::HTTP_GET.bits() | Self::VARIABLES_READ.bits() | Self::CACHE_ALL.bits();
        const ALL           = Self::STORAGE_ALL.bits()
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
        Ok(Self::from_bits_truncate(u32::deserialize(deserializer)?))
    }
}

impl Default for WasmCapabilities {
    fn default() -> Self {
        Self::STANDARD
    }
}

impl WasmCapabilities {
    pub fn has(&self, capability: Self) -> bool {
        self.contains(capability)
    }

    pub fn from_names(names: &[&str]) -> Self {
        let mut capabilities = Self::NONE;
        for name in names {
            capabilities |= match *name {
                "storage_read" => Self::STORAGE_READ,
                "storage_write" => Self::STORAGE_WRITE,
                "storage_delete" => Self::STORAGE_DELETE,
                "storage_all" | "storage" => Self::STORAGE_ALL,
                "http_get" => Self::HTTP_GET,
                "http_write" => Self::HTTP_WRITE,
                "http_all" | "http" => Self::HTTP_ALL,
                "websocket" | "network:websocket" | "ws" => Self::WEBSOCKET,
                "tcp" | "network:tcp" => Self::TCP,
                "udp" | "network:udp" => Self::UDP,
                "dns" | "network:dns" => Self::DNS,
                "network_all" | "network" => Self::NETWORK_ALL,
                "variables_read" => Self::VARIABLES_READ,
                "variables_write" => Self::VARIABLES_WRITE,
                "variables_all" | "variables" => Self::VARIABLES_ALL,
                "cache_read" => Self::CACHE_READ,
                "cache_write" => Self::CACHE_WRITE,
                "cache_all" | "cache" => Self::CACHE_ALL,
                "oauth" => Self::OAUTH,
                "token" => Self::TOKEN,
                "auth_all" | "auth" => Self::AUTH_ALL,
                "streaming" => Self::STREAMING,
                "a2ui" => Self::A2UI,
                "models" | "llm" => Self::MODELS,
                "functions" => Self::FUNCTIONS,
                "standard" => Self::STANDARD,
                "all" => Self::ALL,
                _ => Self::NONE,
            };
        }
        capabilities
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_capabilities_preserve_compound_flags() {
        let capabilities = WasmCapabilities::from_names(&["storage_read", "http_get", "cache"]);
        assert!(capabilities.has(WasmCapabilities::STORAGE_READ));
        assert!(capabilities.has(WasmCapabilities::HTTP_GET));
        assert!(capabilities.has(WasmCapabilities::CACHE_READ));
        assert!(capabilities.has(WasmCapabilities::CACHE_WRITE));
        assert!(!capabilities.has(WasmCapabilities::STORAGE_WRITE));
    }
}
