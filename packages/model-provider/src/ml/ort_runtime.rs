//! Process-wide ONNX Runtime configuration shared by every FlowLike model path.
//!
//! ORT fixes its environment when the first session builder is created. Keeping the
//! initializer in the model-provider crate lets raw ONNX sessions and FastEmbed callers
//! establish the same execution-provider policy before either one can create a builder.

use std::sync::OnceLock;

/// Information about FlowLike's process-wide ONNX Runtime environment.
#[derive(Debug, Clone, Default)]
pub struct ExecutionProviderInfo {
    /// Providers offered to each session, in priority order, including CPU fallback.
    pub active_providers: Vec<String>,
    /// Whether at least one non-CPU provider was configured.
    pub accelerated: bool,
    /// Whether FlowLike successfully committed the process-wide ORT environment.
    pub configured: bool,
    /// Non-fatal provider and environment configuration warnings.
    pub warnings: Vec<String>,
}

static EP_INFO: OnceLock<ExecutionProviderInfo> = OnceLock::new();

/// Initialize ORT once and return the resulting provider information.
///
/// This is retained as a non-failing startup/reporting API. Session-producing code should
/// use [`ensure_ort_initialized`] or [`configured_session_builder`] so a previously-created,
/// unknown ORT environment cannot silently force CPU-only inference.
pub fn initialize_ort() -> ExecutionProviderInfo {
    EP_INFO.get_or_init(do_initialize_ort).clone()
}

/// Ensure FlowLike owns the process-wide ORT configuration.
#[cfg(feature = "local-ml")]
pub fn ensure_ort_initialized() -> ort::Result<ExecutionProviderInfo> {
    let info = initialize_ort();
    if info.configured {
        Ok(info)
    } else {
        Err(ort::Error::new(
            "ONNX Runtime was initialized before FlowLike could configure execution providers",
        ))
    }
}

/// Create a session builder only after the shared provider policy is in place.
#[cfg(feature = "local-ml")]
pub fn configured_session_builder() -> ort::Result<ort::session::builder::SessionBuilder> {
    ensure_ort_initialized()?;
    #[allow(unused_mut)]
    let mut builder = ort::session::Session::builder()?;

    // DirectML requires these two options on every session. Windows providers therefore
    // live on the session rather than in ORT's environment (where session options cannot
    // be adjusted). Other platforms inherit their provider list from the environment.
    #[cfg(target_os = "windows")]
    {
        let providers = session_execution_providers(true)?;
        let has_directml = providers
            .iter()
            .any(|provider| provider.downcast_ref::<ort::ep::DirectML>().is_some());
        if !providers.is_empty() {
            builder = builder
                .with_execution_providers(providers)
                .map_err(|error| ort::Error::new(error.to_string()))?;
        }
        if has_directml {
            builder = builder
                .with_memory_pattern(false)
                .map_err(|error| ort::Error::new(error.to_string()))?
                .with_parallel_execution(false)
                .map_err(|error| ort::Error::new(error.to_string()))?;
        }
    }

    Ok(builder)
}

/// Return providers which must be attached directly to a session.
///
/// Apple, Android, Linux, and non-DirectML NVIDIA sessions inherit providers from the ORT
/// environment, so this returns an empty list there. Windows uses a per-session list because
/// DirectML has mandatory session options. Set `directml_compatible` to `false` for third-party
/// builders which cannot disable memory patterns and parallel execution.
#[cfg(feature = "local-ml")]
pub fn session_execution_providers(
    directml_compatible: bool,
) -> ort::Result<Vec<ort::ep::ExecutionProviderDispatch>> {
    let info = ensure_ort_initialized()?;
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (info, directml_compatible);
        return Ok(Vec::new());
    }

    #[cfg(target_os = "windows")]
    {
        use ort::ep::ExecutionProvider;

        let active = |name: &str| {
            info.active_providers
                .iter()
                .any(|provider| provider == name)
        };
        let mut providers = Vec::new();

        #[cfg(feature = "tensorrt")]
        if active("TensorRT") {
            providers.push(ort::ep::TensorRT::default().build());
        }
        #[cfg(feature = "cuda")]
        if active("CUDA") {
            providers.push(ort::ep::CUDA::default().build());
        }
        if directml_compatible && active("DirectML") {
            providers.push(
                ort::ep::DirectML::default()
                    .with_device_filter(ort::ep::directml::DeviceFilter::Gpu)
                    .with_performance_preference(
                        ort::ep::directml::PerformancePreference::HighPerformance,
                    )
                    .build(),
            );
        }
        if active("XNNPACK") {
            providers.push(ort::ep::XNNPACK::default().build());
        }

        Ok(providers)
    }
}

pub fn is_initialized() -> bool {
    EP_INFO.get().is_some()
}

pub fn get_ep_info() -> Option<ExecutionProviderInfo> {
    EP_INFO.get().cloned()
}

#[cfg(feature = "local-ml")]
fn collect_execution_providers() -> (
    Vec<ort::ep::ExecutionProviderDispatch>,
    Vec<String>,
    Vec<String>,
) {
    #[allow(unused_imports)]
    use ort::ep::ExecutionProvider;
    #[allow(unused_imports)]
    use tracing::{info, warn};

    #[allow(unused_mut)]
    let mut eps = Vec::new();
    #[allow(unused_mut)]
    let mut active_providers = Vec::new();
    #[allow(unused_mut)]
    let mut warnings = Vec::new();

    // Highest-capability device provider first, optimized CPU provider next, and ORT CPU last.
    // ORT may partition unsupported operators onto a lower-priority provider automatically.
    #[cfg(feature = "tensorrt")]
    {
        let provider = ort::ep::TensorRT::default();
        if provider.is_available().unwrap_or(false) {
            info!("TensorRT execution provider available");
            eps.push(provider.build());
            active_providers.push("TensorRT".to_string());
        } else {
            let message = "TensorRT was compiled in but is unavailable at runtime";
            warn!(message);
            warnings.push(message.to_string());
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
            let message = "CUDA was compiled in but is unavailable at runtime";
            warn!(message);
            warnings.push(message.to_string());
        }
    }

    #[cfg(any(feature = "coreml", target_vendor = "apple"))]
    {
        let provider = ort::ep::CoreML::default();
        if provider.is_available().unwrap_or(false) {
            info!("CoreML execution provider available");
            eps.push(provider.build());
            active_providers.push("CoreML".to_string());
        } else {
            let message = "CoreML was compiled in but is unavailable at runtime";
            warn!(message);
            warnings.push(message.to_string());
        }
    }

    #[cfg(any(feature = "directml", target_os = "windows"))]
    {
        let provider = ort::ep::DirectML::default();
        if provider.is_available().unwrap_or(false) {
            info!("DirectML execution provider available");
            eps.push(provider.build());
            active_providers.push("DirectML".to_string());
        } else {
            let message = "DirectML was compiled in but is unavailable at runtime";
            warn!(message);
            warnings.push(message.to_string());
        }
    }

    #[cfg(any(feature = "nnapi", target_os = "android"))]
    {
        let provider = ort::ep::NNAPI::default();
        #[cfg(target_os = "android")]
        let platform_supported = unsafe { ndk_sys::android_get_device_api_level() } >= 27;
        #[cfg(not(target_os = "android"))]
        let platform_supported = false;

        if platform_supported && provider.is_available().unwrap_or(false) {
            info!("NNAPI execution provider available");
            eps.push(provider.build());
            active_providers.push("NNAPI".to_string());
        } else {
            let message = "NNAPI requires Android API 27+ and an NNAPI-capable ORT runtime; using the next provider";
            warn!(message);
            warnings.push(message.to_string());
        }
    }

    #[cfg(any(
        feature = "xnnpack",
        target_arch = "aarch64",
        target_arch = "x86_64",
        all(target_arch = "arm", any(target_os = "linux", target_os = "android"))
    ))]
    {
        let provider = ort::ep::XNNPACK::default();
        if provider.is_available().unwrap_or(false) {
            info!("XNNPACK execution provider available");
            eps.push(provider.build());
            active_providers.push("XNNPACK".to_string());
        } else {
            let message = "XNNPACK was compiled in but is unavailable at runtime";
            warn!(message);
            warnings.push(message.to_string());
        }
    }

    (eps, active_providers, warnings)
}

#[cfg(feature = "local-ml")]
fn do_initialize_ort() -> ExecutionProviderInfo {
    use tracing::{info, warn};

    let (eps, mut active_providers, mut warnings) = collect_execution_providers();
    let requested_acceleration = !eps.is_empty();
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
        available_threads
    };
    let thread_pool = ort::environment::GlobalThreadPoolOptions::default()
        .with_intra_threads(thread_count)
        .and_then(|options| options.with_inter_threads(1))
        .and_then(|options| options.with_spin_control(false));

    // DirectML has mandatory per-session options. Keep the Windows environment provider-free
    // and attach the ordered providers in `configured_session_builder` and supported third-party
    // constructors instead. This also avoids registering DirectML twice.
    #[cfg(target_os = "windows")]
    let environment_eps = Vec::new();
    #[cfg(not(target_os = "windows"))]
    let environment_eps = eps;

    let builder = if environment_eps.is_empty() {
        #[cfg(not(target_os = "windows"))]
        info!("No GPU/NPU execution provider available; using ORT CPU");
        #[cfg(target_os = "windows")]
        info!(providers = ?active_providers, "configuring Windows execution providers per session");
        ort::init()
    } else {
        info!(providers = ?active_providers, "configuring ONNX Runtime execution providers");
        ort::init().with_execution_providers(environment_eps)
    };
    let configured = match thread_pool {
        Ok(thread_pool) => builder.with_global_thread_pool(thread_pool).commit(),
        Err(error) => {
            let message = format!(
                "Failed to configure ORT's shared thread pool; using per-session defaults: {error}"
            );
            warn!("{message}");
            warnings.push(message);
            builder.commit()
        }
    };

    if !configured {
        let message = "ONNX Runtime was initialized before FlowLike configured its execution providers; refusing new FlowLike sessions with unknown provider settings";
        warn!(message);
        warnings.push(message.to_string());
        active_providers = vec!["Existing ORT environment (configuration unknown)".to_string()];
    }

    ExecutionProviderInfo {
        active_providers,
        accelerated: requested_acceleration && configured,
        configured,
        warnings,
    }
}

#[cfg(not(feature = "local-ml"))]
fn do_initialize_ort() -> ExecutionProviderInfo {
    ExecutionProviderInfo {
        active_providers: vec!["CPU (local-ml feature disabled)".to_string()],
        accelerated: false,
        configured: false,
        warnings: vec!["Local ML execution is not enabled".to_string()],
    }
}

#[cfg(feature = "local-ml")]
pub mod availability {
    #[allow(unused_imports)]
    use ort::ep::ExecutionProvider;

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

    pub fn coreml_available() -> bool {
        #[cfg(any(feature = "coreml", target_vendor = "apple"))]
        {
            ort::ep::CoreML::default().is_available().unwrap_or(false)
        }
        #[cfg(not(any(feature = "coreml", target_vendor = "apple")))]
        {
            false
        }
    }

    pub fn directml_available() -> bool {
        #[cfg(any(feature = "directml", target_os = "windows"))]
        {
            ort::ep::DirectML::default().is_available().unwrap_or(false)
        }
        #[cfg(not(any(feature = "directml", target_os = "windows")))]
        {
            false
        }
    }

    pub fn nnapi_available() -> bool {
        #[cfg(target_os = "android")]
        {
            (unsafe { ndk_sys::android_get_device_api_level() }) >= 27
                && ort::ep::NNAPI::default().is_available().unwrap_or(false)
        }
        #[cfg(not(target_os = "android"))]
        {
            false
        }
    }

    pub fn xnnpack_available() -> bool {
        #[cfg(any(
            feature = "xnnpack",
            target_arch = "aarch64",
            target_arch = "x86_64",
            all(target_arch = "arm", any(target_os = "linux", target_os = "android"))
        ))]
        {
            ort::ep::XNNPACK::default().is_available().unwrap_or(false)
        }
        #[cfg(not(any(
            feature = "xnnpack",
            target_arch = "aarch64",
            target_arch = "x86_64",
            all(target_arch = "arm", any(target_os = "linux", target_os = "android"))
        )))]
        {
            false
        }
    }

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
        if nnapi_available() {
            available.push("NNAPI");
        }
        if xnnpack_available() {
            available.push("XNNPACK");
        }
        available.push("CPU");
        available
    }
}

#[cfg(not(feature = "local-ml"))]
pub mod availability {
    pub fn cuda_available() -> bool {
        false
    }
    pub fn tensorrt_available() -> bool {
        false
    }
    pub fn coreml_available() -> bool {
        false
    }
    pub fn directml_available() -> bool {
        false
    }
    pub fn nnapi_available() -> bool {
        false
    }
    pub fn xnnpack_available() -> bool {
        false
    }
    pub fn list_available() -> Vec<&'static str> {
        vec!["CPU (local-ml feature disabled)"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Small static Identity graph encoded as ONNX IR 8/opset 13. Keeping it inline makes
    // the runtime smoke test hermetic on desktop and mobile CI targets.
    const SMOKE_MODEL_BASE64: &str = "CAgSDmZsb3ctbGlrZS10ZXN0OlgKGQoFaW5wdXQSBm91dHB1dCIISWRlbnRpdHkSCGlkZW50aXR5WhcKBWlucHV0Eg4KDAgBEggKAggBCgIIAWIYCgZvdXRwdXQSDgoMCAESCAoCCAEKAggBQgIQDQ==";

    #[test]
    fn initialize_is_idempotent() {
        let first = initialize_ort();
        let second = initialize_ort();
        assert_eq!(first.active_providers, second.active_providers);
        assert_eq!(first.configured, second.configured);
    }

    #[test]
    fn cpu_is_always_a_fallback_when_configured() {
        let info = initialize_ort();
        if info.configured {
            assert!(
                info.active_providers
                    .iter()
                    .any(|provider| provider == "CPU")
            );
        }
    }

    #[cfg(feature = "local-ml")]
    #[test]
    fn configured_builder_commits_a_local_model() {
        use base64::Engine;

        let model = base64::engine::general_purpose::STANDARD
            .decode(SMOKE_MODEL_BASE64)
            .expect("embedded smoke model must be valid base64");
        let mut builder = configured_session_builder().expect("shared ORT policy must initialize");
        let session = builder
            .commit_from_memory(&model)
            .expect("configured ORT session must load a local model");
        assert!(!session.inputs().is_empty());
        assert!(!session.outputs().is_empty());
    }

    #[cfg(all(feature = "local-ml", target_os = "windows"))]
    #[test]
    fn windows_session_providers_are_ordered_and_directml_can_be_excluded() {
        fn provider_name(provider: &ort::ep::ExecutionProviderDispatch) -> &'static str {
            if provider.downcast_ref::<ort::ep::TensorRT>().is_some() {
                "TensorRT"
            } else if provider.downcast_ref::<ort::ep::CUDA>().is_some() {
                "CUDA"
            } else if provider.downcast_ref::<ort::ep::DirectML>().is_some() {
                "DirectML"
            } else if provider.downcast_ref::<ort::ep::XNNPACK>().is_some() {
                "XNNPACK"
            } else {
                "Unknown"
            }
        }

        let info = ensure_ort_initialized().expect("shared ORT policy must initialize");
        let with_directml = session_execution_providers(true)
            .expect("Windows provider selection with DirectML must succeed");
        let without_directml = session_execution_providers(false)
            .expect("Windows provider selection without DirectML must succeed");
        let with_names: Vec<_> = with_directml.iter().map(provider_name).collect();
        let without_names: Vec<_> = without_directml.iter().map(provider_name).collect();
        let expected: Vec<_> = info
            .active_providers
            .iter()
            .filter(|name| name.as_str() != "CPU")
            .map(String::as_str)
            .collect();
        let expected_without_directml: Vec<_> = expected
            .iter()
            .copied()
            .filter(|name| *name != "DirectML")
            .collect();

        assert_eq!(with_names, expected);
        assert_eq!(without_names, expected_without_directml);
        assert!(!without_names.contains(&"DirectML"));
    }
}
