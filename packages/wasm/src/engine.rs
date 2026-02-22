//! WASM Engine configuration and management
//!
//! The engine is the core compilation and configuration unit for wasmtime.

use crate::aot_cache::AotCache;
use crate::error::{WasmError, WasmResult};
use crate::limits::WasmSecurityConfig;
use crate::module::WasmModule;
use crate::unified::LoadedWasm;
use dashmap::DashMap;
use flow_like::utils::cache::get_cache_dir;
use parking_lot::RwLock;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use wasmtime::{Config, Engine, OptLevel};

#[cfg(feature = "component-model")]
use crate::component::WasmComponent;

/// Configuration for the WASM engine
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Enable parallel compilation
    pub parallel_compilation: bool,
    /// Enable cranelift optimizations
    pub optimizations: bool,
    /// Optimization level
    pub opt_level: OptLevel,
    /// Enable fuel metering for execution limits
    pub fuel_metering: bool,
    /// Enable epoch interruption for timeouts
    pub epoch_interruption: bool,
    /// Enable memory growth
    pub memory_growth: bool,
    /// Cache directory for compiled modules (None = no caching)
    pub cache_dir: Option<PathBuf>,
    /// Default security configuration
    pub default_security: WasmSecurityConfig,
    /// Cross-compilation target triple (e.g. "x86_64-unknown-linux-gnu").
    /// When set the engine produces artifacts for this target instead of
    /// the host platform.  Use `Engine::precompile_module` / `precompile_component`
    /// because `Module::new` is not available for cross targets.
    pub target: Option<String>,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            parallel_compilation: true,
            optimizations: true,
            opt_level: OptLevel::Speed,
            fuel_metering: true,
            epoch_interruption: true,
            memory_growth: true,
            cache_dir: Some(get_cache_dir().join("wasm")),
            default_security: WasmSecurityConfig::default(),
            target: None,
        }
    }
}

impl WasmConfig {
    pub fn new() -> Self {
        Self::default()
    }

    /// Development configuration (faster compilation, less optimization)
    pub fn development() -> Self {
        Self {
            parallel_compilation: true,
            optimizations: false,
            opt_level: OptLevel::None,
            fuel_metering: true,
            epoch_interruption: true,
            memory_growth: true,
            cache_dir: None,
            default_security: WasmSecurityConfig::default(),
            target: None,
        }
    }

    /// Production configuration (maximum optimization, auto-detected cache)
    pub fn production() -> Self {
        Self {
            parallel_compilation: true,
            optimizations: true,
            opt_level: OptLevel::Speed,
            fuel_metering: true,
            epoch_interruption: true,
            memory_growth: true,
            cache_dir: Some(get_cache_dir().join("wasm")),
            default_security: WasmSecurityConfig::default(),
            target: None,
        }
    }

    /// Lambda-optimized configuration with permissive security limits.
    pub fn lambda() -> Self {
        Self {
            parallel_compilation: true,
            optimizations: true,
            opt_level: OptLevel::Speed,
            fuel_metering: true,
            epoch_interruption: true,
            memory_growth: true,
            cache_dir: Some(get_cache_dir().join("wasm")),
            default_security: WasmSecurityConfig::permissive(),
            target: None,
        }
    }

    pub fn with_cache_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.cache_dir = Some(path.into());
        self
    }

    pub fn without_cache(mut self) -> Self {
        self.cache_dir = None;
        self
    }

    pub fn with_security(mut self, security: WasmSecurityConfig) -> Self {
        self.default_security = security;
        self
    }

    /// Set a cross-compilation target triple (e.g. `"x86_64-unknown-linux-gnu"`).
    /// The resulting engine can only precompile; `Module::new` will fail.
    pub fn with_target(mut self, triple: impl Into<String>) -> Self {
        self.target = Some(triple.into());
        self
    }

    /// Build wasmtime Config from our config
    fn to_wasmtime_config(&self) -> WasmResult<Config> {
        let mut config = Config::new();

        // Compilation settings
        config.parallel_compilation(self.parallel_compilation);
        config.cranelift_opt_level(self.opt_level);

        // Runtime settings
        config.consume_fuel(self.fuel_metering);
        config.epoch_interruption(self.epoch_interruption);
        config.async_support(true);

        // Enable WASM GC and exception handling proposals (needed for Kotlin/Wasm, etc.)
        config.wasm_gc(true);
        config.wasm_exceptions(true);
        config.wasm_function_references(true);

        // Memory settings
        config.memory_init_cow(true);

        // Apply resource limits
        let limits = &self.default_security.limits;
        config.max_wasm_stack(limits.max_stack_depth as usize * 1024);

        // Cross-compilation target
        if let Some(ref triple) = self.target {
            config.target(triple).map_err(|e| {
                WasmError::compilation(format!(
                    "Unsupported cross-compilation target '{}': {}",
                    triple, e
                ))
            })?;
        }

        Ok(config)
    }
}

/// WASM Engine for compiling and running modules
pub struct WasmEngine {
    /// Wasmtime engine
    engine: Engine,
    /// Configuration
    config: WasmConfig,
    /// Cached compiled modules (hash -> module)
    module_cache: DashMap<String, Arc<WasmModule>>,
    /// Cached compiled components (hash -> component)
    #[cfg(feature = "component-model")]
    component_cache: DashMap<String, Arc<WasmComponent>>,
    /// Epoch ticker for timeouts
    epoch_ticker: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
    /// AOT disk cache for precompiled artifacts
    aot_cache: Option<AotCache>,
}

impl WasmEngine {
    /// Create a new WASM engine with the given configuration
    pub fn new(config: WasmConfig) -> WasmResult<Self> {
        let wasmtime_config = config.to_wasmtime_config()?;
        let engine = Engine::new(&wasmtime_config).map_err(|e| {
            WasmError::compilation(format!("Failed to create wasmtime engine: {}", e))
        })?;

        let aot_cache = config.cache_dir.as_ref().map(AotCache::new);

        Ok(Self {
            engine,
            config,
            module_cache: DashMap::new(),
            #[cfg(feature = "component-model")]
            component_cache: DashMap::new(),
            epoch_ticker: Arc::new(RwLock::new(None)),
            aot_cache,
        })
    }

    /// Create with default configuration
    pub fn default_engine() -> WasmResult<Self> {
        Self::new(WasmConfig::default())
    }

    /// Get reference to wasmtime engine
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Get configuration
    pub fn config(&self) -> &WasmConfig {
        &self.config
    }

    /// AOT-compile a raw `.wasm` binary into serialized bytes.
    ///
    /// Works for both host and cross-compilation targets.
    /// Auto-detects whether the input is a core module or a Component Model binary.
    pub fn precompile(&self, wasm_bytes: &[u8]) -> WasmResult<Vec<u8>> {
        #[cfg(feature = "component-model")]
        if crate::component::is_component_model(wasm_bytes) {
            return self
                .engine
                .precompile_component(wasm_bytes)
                .map_err(|e| WasmError::compilation(format!("Failed to precompile component: {}", e)));
        }
        self.engine
            .precompile_module(wasm_bytes)
            .map_err(|e| WasmError::compilation(format!("Failed to precompile module: {}", e)))
    }

    /// Start the epoch ticker for timeout enforcement
    pub fn start_epoch_ticker(&self) {
        let mut ticker = self.epoch_ticker.write();
        if ticker.is_some() {
            return; // Already running
        }

        let engine = self.engine.clone();
        let handle = tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_millis(10));
            loop {
                interval.tick().await;
                engine.increment_epoch();
            }
        });

        *ticker = Some(handle);
    }

    /// Stop the epoch ticker
    pub fn stop_epoch_ticker(&self) {
        let mut ticker = self.epoch_ticker.write();
        if let Some(handle) = ticker.take() {
            handle.abort();
        }
    }

    /// Load a module from bytes
    pub async fn load_module(&self, bytes: &[u8]) -> WasmResult<Arc<WasmModule>> {
        let hash = blake3::hash(bytes).to_hex().to_string();

        if let Some(cached) = self.module_cache.get(&hash) {
            tracing::debug!("In-memory cache hit for module: {}", hash);
            return Ok(cached.clone());
        }

        // Try AOT disk cache before expensive Cranelift compilation
        let module = if let Some(aot) = &self.aot_cache {
            if let Some(precompiled) = aot.load_module(&self.engine, &hash) {
                WasmModule::from_precompiled(precompiled, hash.clone())?
            } else {
                let m = WasmModule::from_bytes(self, bytes, hash.clone()).await?;
                aot.save_module(m.module(), &hash);
                m
            }
        } else {
            WasmModule::from_bytes(self, bytes, hash.clone()).await?
        };

        let module = Arc::new(module);
        self.module_cache.insert(hash, module.clone());
        Ok(module)
    }

    /// Load a module from file
    pub async fn load_module_from_file(
        &self,
        path: impl AsRef<Path>,
    ) -> WasmResult<Arc<WasmModule>> {
        let path = path.as_ref();
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_e| WasmError::ModuleNotFound {
                path: path.display().to_string(),
            })?;

        self.load_module(&bytes).await
    }

    /// Load a Component Model binary from bytes
    #[cfg(feature = "component-model")]
    pub async fn load_component(&self, bytes: &[u8]) -> WasmResult<Arc<WasmComponent>> {
        let hash = blake3::hash(bytes).to_hex().to_string();

        if let Some(cached) = self.component_cache.get(&hash) {
            tracing::debug!("In-memory cache hit for component: {}", hash);
            return Ok(cached.clone());
        }

        let component = if let Some(aot) = &self.aot_cache {
            if let Some(precompiled) = aot.load_component(&self.engine, &hash) {
                WasmComponent::from_precompiled(precompiled, bytes, hash.clone())?
            } else {
                let c = WasmComponent::from_bytes(self, bytes, hash.clone()).await?;
                aot.save_component(c.component(), &hash);
                c
            }
        } else {
            WasmComponent::from_bytes(self, bytes, hash.clone()).await?
        };

        let component = Arc::new(component);
        self.component_cache.insert(hash, component.clone());
        Ok(component)
    }

    /// Auto-detect format and load either a core module or Component Model binary
    pub async fn load_auto(&self, bytes: &[u8]) -> WasmResult<LoadedWasm> {
        #[cfg(feature = "component-model")]
        if crate::component::is_component_model(bytes) {
            return self.load_component(bytes).await.map(LoadedWasm::Component);
        }
        self.load_module(bytes).await.map(LoadedWasm::Module)
    }

    /// Auto-detect format and load from file
    pub async fn load_auto_from_file(&self, path: impl AsRef<Path>) -> WasmResult<LoadedWasm> {
        let path = path.as_ref();
        let bytes = tokio::fs::read(path)
            .await
            .map_err(|_e| WasmError::ModuleNotFound {
                path: path.display().to_string(),
            })?;
        self.load_auto(&bytes).await
    }

    /// Load a module from URL
    #[cfg(feature = "http")]
    pub async fn load_module_from_url(&self, url: &str) -> WasmResult<Arc<WasmModule>> {
        let response: reqwest::Response =
            reqwest::get(url)
                .await
                .map_err(|_e| WasmError::ModuleNotFound {
                    path: url.to_string(),
                })?;

        if !response.status().is_success() {
            return Err(WasmError::ModuleNotFound {
                path: format!("{} (status: {})", url, response.status()),
            });
        }

        let bytes = response
            .bytes()
            .await
            .map_err(|e| WasmError::Internal(format!("Failed to read module bytes: {}", e)))?;

        self.load_module(&bytes).await
    }

    /// Preload modules from a directory
    pub async fn preload_directory(
        &self,
        dir: impl AsRef<Path>,
    ) -> WasmResult<Vec<Arc<WasmModule>>> {
        let dir = dir.as_ref();
        let mut modules = Vec::new();

        let mut entries = tokio::fs::read_dir(dir).await.map_err(WasmError::Io)?;

        while let Some(entry) = entries.next_entry().await.map_err(WasmError::Io)? {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "wasm") {
                match self.load_module_from_file(&path).await {
                    Ok(module) => {
                        tracing::info!("Loaded WASM module: {}", path.display());
                        modules.push(module);
                    }
                    Err(e) => {
                        tracing::warn!("Failed to load WASM module {}: {}", path.display(), e);
                    }
                }
            }
        }

        Ok(modules)
    }

    /// Clear the module cache
    pub fn clear_cache(&self) {
        self.module_cache.clear();
        #[cfg(feature = "component-model")]
        self.component_cache.clear();
    }

    /// Get number of cached modules
    pub fn cached_module_count(&self) -> usize {
        let count = self.module_cache.len();
        #[cfg(feature = "component-model")]
        let count = count + self.component_cache.len();
        count
    }

    /// Remove a specific module from cache
    pub fn evict_module(&self, hash: &str) -> bool {
        self.module_cache.remove(hash).is_some()
    }
}

impl Drop for WasmEngine {
    fn drop(&mut self) {
        self.stop_epoch_ticker();
    }
}

// Make WasmEngine Send + Sync safe
unsafe impl Send for WasmEngine {}
unsafe impl Sync for WasmEngine {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_engine_creation() {
        let engine = WasmEngine::new(WasmConfig::development()).unwrap();
        assert_eq!(engine.cached_module_count(), 0);
    }

    #[test]
    fn test_config_development() {
        let config = WasmConfig::development();
        assert!(!config.optimizations);
        assert_eq!(config.opt_level, OptLevel::None);
    }

    #[test]
    fn test_config_production() {
        let config = WasmConfig::production();
        assert!(config.optimizations);
        assert_eq!(config.opt_level, OptLevel::Speed);
    }

    #[test]
    fn test_config_lambda() {
        let config = WasmConfig::lambda();
        assert!(config.optimizations);
        assert_eq!(config.opt_level, OptLevel::Speed);
        assert!(config.cache_dir.is_some());
    }

    #[test]
    fn test_without_cache() {
        let config = WasmConfig::production().without_cache();
        assert!(config.cache_dir.is_none());
    }
}
