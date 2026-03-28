//! AOT (Ahead-of-Time) compilation cache for WASM modules.
//!
//! Each cached `.cwasm` artifact is keyed by:
//! - blake3 hash of the original `.wasm` bytes  (content identity)
//! - OS + architecture                          (platform identity)
//! - wasmtime major version                     (compiler identity)
//!
//! The system always compiles from `.wasm` source itself, so no external
//! integrity verification is needed — a cache miss simply recompiles.

use crate::error::WasmResult;
use std::path::{Path, PathBuf};
use wasmtime::Module;

#[cfg(feature = "component-model")]
use wasmtime::component::Component;

const WASMTIME_VERSION: &str = "42";

/// Build the platform key for the current host (e.g. `ios-aarch64-wt42`).
/// This always returns the native platform key so the client can request
/// native precompiled artifacts from the server.
pub fn host_platform_key() -> String {
    format!(
        "{}-{}-wt{}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        WASMTIME_VERSION,
    )
}

fn cache_key(wasm_hash: &str) -> String {
    format!(
        "{}-{}-{}-wt{}",
        wasm_hash,
        std::env::consts::OS,
        std::env::consts::ARCH,
        WASMTIME_VERSION,
    )
}

pub struct AotCache {
    modules_dir: PathBuf,
    #[cfg(feature = "component-model")]
    components_dir: PathBuf,
}

impl AotCache {
    pub fn new(cache_dir: impl Into<PathBuf>) -> Self {
        let base = cache_dir.into();
        Self {
            modules_dir: base.join("modules"),
            #[cfg(feature = "component-model")]
            components_dir: base.join("components"),
        }
    }

    fn artifact_path(dir: &Path, wasm_hash: &str) -> PathBuf {
        dir.join(format!("{}.cwasm", cache_key(wasm_hash)))
    }

    fn load_artifact(dir: &Path, wasm_hash: &str) -> Option<Vec<u8>> {
        std::fs::read(Self::artifact_path(dir, wasm_hash)).ok()
    }

    fn write_artifact(dir: &Path, wasm_hash: &str, serialized: &[u8]) -> WasmResult<()> {
        std::fs::create_dir_all(dir)?;
        let path = Self::artifact_path(dir, wasm_hash);
        std::fs::write(&path, serialized)?;
        tracing::info!("Saved AOT cache: {} ({} bytes)", path.display(), serialized.len());
        Ok(())
    }

    fn evict(dir: &Path, wasm_hash: &str) {
        let _ = std::fs::remove_file(Self::artifact_path(dir, wasm_hash));
    }

    /// Try to load a precompiled module. Returns `None` on cache miss.
    ///
    /// # Safety
    /// `Module::deserialize` loads native machine code. This is safe here because
    /// only self-compiled artifacts from verified `.wasm` bytecode enter this cache.
    pub fn load_module(&self, engine: &wasmtime::Engine, wasm_hash: &str) -> Option<Module> {
        let serialized = Self::load_artifact(&self.modules_dir, wasm_hash)?;

        // SAFETY: only self-compiled artifacts enter this cache
        match unsafe { Module::deserialize(engine, &serialized) } {
            Ok(module) => {
                tracing::debug!("AOT cache hit for module {}", wasm_hash);
                Some(module)
            }
            Err(e) => {
                tracing::warn!("AOT deserialize failed for module {}: {}", wasm_hash, e);
                Self::evict(&self.modules_dir, wasm_hash);
                None
            }
        }
    }

    pub fn save_module(&self, module: &Module, wasm_hash: &str) {
        match module.serialize() {
            Ok(s) => {
                if let Err(e) = Self::write_artifact(&self.modules_dir, wasm_hash, &s) {
                    tracing::warn!("Failed to save AOT module {}: {}", wasm_hash, e);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize module {}: {}", wasm_hash, e),
        }
    }

    #[cfg(feature = "component-model")]
    pub fn load_component(&self, engine: &wasmtime::Engine, wasm_hash: &str) -> Option<Component> {
        let serialized = Self::load_artifact(&self.components_dir, wasm_hash)?;

        // SAFETY: same guarantees as load_module
        match unsafe { Component::deserialize(engine, &serialized) } {
            Ok(component) => {
                tracing::debug!("AOT cache hit for component {}", wasm_hash);
                Some(component)
            }
            Err(e) => {
                tracing::warn!("AOT deserialize failed for component {}: {}", wasm_hash, e);
                Self::evict(&self.components_dir, wasm_hash);
                None
            }
        }
    }

    #[cfg(feature = "component-model")]
    pub fn save_component(&self, component: &Component, wasm_hash: &str) {
        match component.serialize() {
            Ok(s) => {
                if let Err(e) = Self::write_artifact(&self.components_dir, wasm_hash, &s) {
                    tracing::warn!("Failed to save AOT component {}: {}", wasm_hash, e);
                }
            }
            Err(e) => tracing::warn!("Failed to serialize component {}: {}", wasm_hash, e),
        }
    }

    pub fn clear(&self) {
        let _ = std::fs::remove_dir_all(&self.modules_dir);
        #[cfg(feature = "component-model")]
        let _ = std::fs::remove_dir_all(&self.components_dir);
    }

    /// Inject an externally-compiled `.cwasm` artifact into the module cache.
    ///
    /// Used to populate the cache with artifacts downloaded from the backend.
    pub fn inject_module(&self, wasm_hash: &str, cwasm_bytes: &[u8]) -> WasmResult<()> {
        Self::write_artifact(&self.modules_dir, wasm_hash, cwasm_bytes)
    }

    /// Inject an externally-compiled component `.cwasm` artifact into the cache.
    #[cfg(feature = "component-model")]
    pub fn inject_component(&self, wasm_hash: &str, cwasm_bytes: &[u8]) -> WasmResult<()> {
        Self::write_artifact(&self.components_dir, wasm_hash, cwasm_bytes)
    }
}
