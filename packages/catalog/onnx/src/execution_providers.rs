//! Execution Provider Configuration for ONNX Runtime
//!
//! This module provides global initialization of ONNX Runtime's execution providers (EPs)
//! with automatic hardware detection and graceful fallback behavior.
//!
//! # Supported Execution Providers
//!
//! - **TensorRT**: NVIDIA GPUs with TensorRT (fastest for NVIDIA inference)
//! - **CUDA**: NVIDIA GPUs (requires CUDA toolkit)
//! - **CoreML**: Apple Neural Engine and GPU (macOS, iOS, tvOS)
//! - **DirectML**: Windows GPUs via DirectX 12 (AMD, Intel, NVIDIA)
//! - **XNNPACK**: Optimized CPU inference for ARM and x86 (great for mobile/edge)
//! - **CPU**: Always available fallback
//!
//! # Global Initialization
//!
//! Call `initialize_ort()` once at application startup, before creating any ONNX sessions.
//! This configures the global execution provider defaults that all sessions will use.
//!
//! ```ignore
//! // At app startup
//! flow_like_catalog_onnx::onnx::execution_providers::initialize_ort();
//!
//! // Later, all sessions automatically use the configured EPs
//! let session = Session::builder()?.commit_from_file("model.onnx")?;
//! ```
//!
//! # Graceful Fallback Behavior
//!
//! The initialization automatically:
//! 1. Detects available hardware acceleration
//! 2. Registers all available EPs in order of preference
//! 3. Falls back to CPU if no accelerators are available
//!
//! # Cross-Compilation Notes
//!
//! - All EP features compile on all platforms (they become no-ops where unsupported)
//! - CoreML only activates on Apple platforms at runtime
//! - DirectML only activates on Windows at runtime
//! - CUDA/TensorRT require the NVIDIA runtime on the target system
//! - XNNPACK works on all platforms with ARM or x86 CPUs

use std::sync::RwLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// Track whether ORT has been initialized
static ORT_INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Information about the active execution providers after initialization
#[derive(Debug, Clone, Default)]
pub struct ExecutionProviderInfo {
    /// List of active execution providers (in priority order)
    pub active_providers: Vec<String>,
    /// Whether any GPU/NPU acceleration is active
    pub accelerated: bool,
    /// Warnings during initialization
    pub warnings: Vec<String>,
}

/// Global EP info set during initialization (using RwLock for thread safety)
static EP_INFO: RwLock<Option<ExecutionProviderInfo>> = RwLock::new(None);

/// Initialize ONNX Runtime with the best available execution providers.
///
/// This function should be called once at application startup, before creating any ONNX sessions.
/// It's safe to call multiple times - subsequent calls are no-ops.
///
/// # Returns
///
/// Information about which execution providers were registered.
///
/// # Example
///
/// ```ignore
/// let info = initialize_ort();
/// println!("Active providers: {:?}", info.active_providers);
/// println!("GPU acceleration: {}", info.accelerated);
/// ```
pub fn initialize_ort() -> ExecutionProviderInfo {
    // Only initialize once
    if ORT_INITIALIZED.swap(true, Ordering::SeqCst) {
        // Already initialized, return cached info
        return EP_INFO
            .read()
            .ok()
            .and_then(|guard| guard.clone())
            .unwrap_or_default();
    }

    let info = do_initialize_ort();

    // Cache the info
    if let Ok(mut guard) = EP_INFO.write() {
        *guard = Some(info.clone());
    }

    info
}

/// Check if ORT has been initialized
pub fn is_initialized() -> bool {
    ORT_INITIALIZED.load(Ordering::SeqCst)
}

/// Get the current execution provider info (returns None if not initialized)
pub fn get_ep_info() -> Option<ExecutionProviderInfo> {
    if !is_initialized() {
        return None;
    }
    EP_INFO.read().ok().and_then(|guard| guard.clone())
}

/// Build the execution-provider dispatch list for the current platform and enabled features.
///
/// Returns the dispatch list, the names of the accelerators actually registered (CPU excluded),
/// and any warnings. Shared by global ORT init and per-session builders (e.g. the face_id
/// analyzer) so every session opts into the same acceleration instead of falling back to CPU.
#[cfg(feature = "execute")]
pub(crate) fn collect_execution_providers() -> (
    Vec<flow_like_model_provider::ml::ort::ep::ExecutionProviderDispatch>,
    Vec<String>,
    Vec<String>,
) {
    #[allow(unused_imports)]
    use flow_like_model_provider::ml::ort::{self, ep::ExecutionProvider};
    #[allow(unused_imports)]
    use tracing::{info, warn};

    #[allow(unused_mut)]
    let mut eps = Vec::new();
    #[allow(unused_mut)]
    let mut active_providers = Vec::new();
    #[allow(unused_mut)]
    let mut warnings = Vec::new();

    // Registered in order of preference: TensorRT > CUDA > CoreML > DirectML > XNNPACK > CPU.
    // `is_available()` reports whether the onnxruntime binary was compiled with the EP; an
    // unavailable/errored EP is skipped and the session falls back to the next one (ultimately CPU).

    #[cfg(feature = "tensorrt")]
    {
        let provider = ort::ep::TensorRT::default();
        if provider.is_available().unwrap_or(false) {
            info!("TensorRT execution provider available");
            eps.push(provider.build());
            active_providers.push("TensorRT".to_string());
        } else {
            let msg = "TensorRT feature enabled but runtime not available";
            warn!("{}", msg);
            warnings.push(msg.to_string());
        }
    }

    #[cfg(feature = "cuda")]
    {
        let provider = ort::ep::CUDA::default();
        if provider.is_available().unwrap_or(false) {
            info!("CUDA execution provider available");
            eps.push(provider.build());
            active_providers.push("CUDA".to_string());
        } else {
            let msg = "CUDA feature enabled but runtime not available";
            warn!("{}", msg);
            warnings.push(msg.to_string());
        }
    }

    #[cfg(feature = "coreml")]
    {
        let provider = ort::ep::CoreML::default();
        if provider.is_available().unwrap_or(false) {
            info!("CoreML execution provider available");
            eps.push(provider.build());
            active_providers.push("CoreML".to_string());
        } else {
            let msg = "CoreML feature enabled but not on Apple platform";
            warn!("{}", msg);
            warnings.push(msg.to_string());
        }
    }

    #[cfg(feature = "directml")]
    {
        let provider = ort::ep::DirectML::default();
        if provider.is_available().unwrap_or(false) {
            info!("DirectML execution provider available");
            eps.push(provider.build());
            active_providers.push("DirectML".to_string());
        } else {
            let msg = "DirectML feature enabled but not on Windows";
            warn!("{}", msg);
            warnings.push(msg.to_string());
        }
    }

    #[cfg(feature = "xnnpack")]
    {
        let provider = ort::ep::XNNPACK::default();
        if provider.is_available().unwrap_or(false) {
            info!("XNNPACK execution provider available");
            eps.push(provider.build());
            active_providers.push("XNNPACK".to_string());
        } else {
            let msg = "XNNPACK feature enabled but not available";
            warn!("{}", msg);
            warnings.push(msg.to_string());
        }
    }

    (eps, active_providers, warnings)
}

#[cfg(feature = "execute")]
fn do_initialize_ort() -> ExecutionProviderInfo {
    use flow_like_model_provider::ml::ort;
    use tracing::info;

    let (eps, mut active_providers, warnings) = collect_execution_providers();
    let accelerated = !eps.is_empty();

    // CPU is always available as the final fallback
    active_providers.push("CPU".to_string());

    // Initialize ORT with the collected execution providers
    if eps.is_empty() {
        info!("No GPU/NPU acceleration available, using CPU");
        ort::init().commit();
    } else {
        info!(
            "Initializing ORT with execution providers: {:?}",
            active_providers
        );
        ort::init().with_execution_providers(eps).commit();
    }

    ExecutionProviderInfo {
        active_providers,
        accelerated,
        warnings,
    }
}

#[cfg(not(feature = "execute"))]
fn do_initialize_ort() -> ExecutionProviderInfo {
    ExecutionProviderInfo {
        active_providers: vec!["CPU (execute feature disabled)".to_string()],
        accelerated: false,
        warnings: vec!["Execute feature not enabled".to_string()],
    }
}

/// Check availability of specific execution providers
#[cfg(feature = "execute")]
pub mod availability {
    #[allow(unused_imports)]
    use flow_like_model_provider::ml::ort::{self, ep::ExecutionProvider};

    /// Check if CUDA is compiled in and available at runtime
    pub fn cuda_available() -> bool {
        #[cfg(feature = "cuda")]
        {
            ort::ep::CUDA::default().is_available().unwrap_or(false)
        }
        #[cfg(not(feature = "cuda"))]
        {
            false
        }
    }

    /// Check if TensorRT is compiled in and available at runtime
    pub fn tensorrt_available() -> bool {
        #[cfg(feature = "tensorrt")]
        {
            ort::ep::TensorRT::default().is_available().unwrap_or(false)
        }
        #[cfg(not(feature = "tensorrt"))]
        {
            false
        }
    }

    /// Check if CoreML is compiled in and available at runtime
    pub fn coreml_available() -> bool {
        #[cfg(feature = "coreml")]
        {
            ort::ep::CoreML::default().is_available().unwrap_or(false)
        }
        #[cfg(not(feature = "coreml"))]
        {
            false
        }
    }

    /// Check if DirectML is compiled in and available at runtime
    pub fn directml_available() -> bool {
        #[cfg(feature = "directml")]
        {
            ort::ep::DirectML::default().is_available().unwrap_or(false)
        }
        #[cfg(not(feature = "directml"))]
        {
            false
        }
    }

    /// Check if XNNPACK is compiled in and available at runtime
    pub fn xnnpack_available() -> bool {
        #[cfg(feature = "xnnpack")]
        {
            ort::ep::XNNPACK::default().is_available().unwrap_or(false)
        }
        #[cfg(not(feature = "xnnpack"))]
        {
            false
        }
    }

    /// Get a summary of all available execution providers
    pub fn list_available() -> Vec<&'static str> {
        let mut available = Vec::new();
        if tensorrt_available() {
            available.push("TensorRT");
        }
        if cuda_available() {
            available.push("CUDA");
        }
        if coreml_available() {
            available.push("CoreML");
        }
        if directml_available() {
            available.push("DirectML");
        }
        if xnnpack_available() {
            available.push("XNNPACK");
        }
        available.push("CPU");
        available
    }
}

#[cfg(not(feature = "execute"))]
pub mod availability {
    /// Check if CUDA is compiled in and available at runtime
    pub fn cuda_available() -> bool {
        false
    }
    /// Check if TensorRT is compiled in and available at runtime
    pub fn tensorrt_available() -> bool {
        false
    }
    /// Check if CoreML is compiled in and available at runtime
    pub fn coreml_available() -> bool {
        false
    }
    /// Check if DirectML is compiled in and available at runtime
    pub fn directml_available() -> bool {
        false
    }
    /// Check if XNNPACK is compiled in and available at runtime
    pub fn xnnpack_available() -> bool {
        false
    }
    /// Get a summary of all available execution providers
    pub fn list_available() -> Vec<&'static str> {
        vec!["CPU (execute feature disabled)"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_idempotent() {
        // First call initializes
        let info1 = initialize_ort();
        // Second call returns cached info
        let info2 = initialize_ort();
        assert_eq!(info1.active_providers, info2.active_providers);
    }

    #[test]
    fn test_cpu_always_available() {
        let info = initialize_ort();
        assert!(info.active_providers.iter().any(|p| p.contains("CPU")));
    }
}
