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

use std::sync::OnceLock;

/// Information about the execution providers configured during initialization.
/// Individual sessions or operators may still fall back to CPU if registration or model
/// compatibility prevents use of an accelerator.
#[derive(Debug, Clone, Default)]
pub struct ExecutionProviderInfo {
    /// Configured execution providers in priority order (including CPU fallback)
    pub active_providers: Vec<String>,
    /// Whether at least one non-CPU provider was configured
    pub accelerated: bool,
    /// Warnings during initialization
    pub warnings: Vec<String>,
}

/// Global EP info. `OnceLock` makes concurrent first callers wait until both the ORT
/// environment and this metadata are fully initialized.
static EP_INFO: OnceLock<ExecutionProviderInfo> = OnceLock::new();

/// Initialize ONNX Runtime with the best available execution providers.
///
/// This function should be called once at application startup, before creating any ONNX sessions.
/// It's safe to call multiple times - subsequent calls are no-ops.
///
/// # Returns
///
/// Information about which execution providers were configured.
///
/// # Example
///
/// ```ignore
/// let info = initialize_ort();
/// println!("Configured providers: {:?}", info.active_providers);
/// println!("Acceleration configured: {}", info.accelerated);
/// ```
pub fn initialize_ort() -> ExecutionProviderInfo {
    EP_INFO.get_or_init(do_initialize_ort).clone()
}

/// Check if ORT has been initialized
pub fn is_initialized() -> bool {
    EP_INFO.get().is_some()
}

/// Get the current execution provider info (returns None if not initialized)
pub fn get_ep_info() -> Option<ExecutionProviderInfo> {
    EP_INFO.get().cloned()
}

/// Build the execution-provider dispatch list for the current platform and enabled features.
///
/// Returns the dispatch list, the names of the configured accelerators (CPU excluded), and any
/// warnings. Sessions inherit this list from the process-wide ORT environment and may still
/// fall back to CPU for unsupported models or unavailable device dependencies.
#[cfg(feature = "execute")]
fn collect_execution_providers() -> (
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
    use tracing::{info, warn};

    let (eps, mut active_providers, mut warnings) = collect_execution_providers();
    let mut accelerated = !eps.is_empty();

    // CPU is always available as the final fallback
    active_providers.push("CPU".to_string());

    let available_threads = std::thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1);
    let thread_count = if cfg!(any(
        target_os = "android",
        target_os = "ios",
        target_os = "tvos"
    )) {
        available_threads.clamp(1, 2)
    } else {
        // This is one process-wide pool shared by every ORT session, so using the
        // scheduler-visible CPU budget does not multiply threads per model/session.
        available_threads
    };
    let thread_pool = ort::environment::GlobalThreadPoolOptions::default()
        .with_intra_threads(thread_count)
        .and_then(|options| options.with_inter_threads(1))
        .and_then(|options| options.with_spin_control(false));

    let builder = if eps.is_empty() {
        info!("No GPU/NPU acceleration available, using CPU");
        ort::init()
    } else {
        info!(
            "Initializing ORT with execution providers: {:?}",
            active_providers
        );
        ort::init().with_execution_providers(eps)
    };
    let committed = match thread_pool {
        Ok(thread_pool) => builder.with_global_thread_pool(thread_pool).commit(),
        Err(error) => {
            let warning = format!(
                "Failed to configure the shared ONNX Runtime thread pool; using per-session defaults: {error}"
            );
            warn!("{warning}");
            warnings.push(warning);
            builder.commit()
        }
    };

    if !committed {
        let warning = "ONNX Runtime was initialized before FlowLike configured its execution providers; existing process-wide provider settings will be used";
        warn!("{warning}");
        warnings.push(warning.to_string());
        active_providers = vec![
            "Existing ORT environment (configuration unknown)".to_string(),
            "CPU".to_string(),
        ];
        accelerated = false;
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
