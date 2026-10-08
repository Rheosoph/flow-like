use crate::{Error, Result};
use libloading::Library;
use std::{path::PathBuf, sync::OnceLock};

// Keep the checked runtime loaded. HIP's generated extern-C wrappers abort if a symbol is absent.
static LIBRARIES: OnceLock<std::result::Result<(Library, Library), String>> = OnceLock::new();

pub(crate) fn check() -> Result<()> {
    LIBRARIES
        .get_or_init(load)
        .as_ref()
        .map(|_| ())
        .map_err(|error| Error::Backend(error.clone()))
}

fn load() -> std::result::Result<(Library, Library), String> {
    let hip = load_library("amdhip64")?;
    let hiprtc = load_library("hiprtc")?;
    for &name in REQUIRED_SYMBOLS {
        let library = if name.starts_with("hiprtc") {
            &hiprtc
        } else {
            &hip
        };
        // Resolve addresses only; no foreign code is called by this check.
        unsafe { library.get::<unsafe extern "C" fn()>(name.as_bytes()) }
            .map_err(|error| format!("ROCm runtime is missing {name}: {error}"))?;
    }
    Ok((hip, hiprtc))
}

fn load_library(name: &str) -> std::result::Result<Library, String> {
    let names = if cfg!(target_os = "windows") {
        vec![format!("{name}.dll")]
    } else if cfg!(target_os = "macos") {
        vec![format!("lib{name}.dylib")]
    } else {
        vec![
            format!("lib{name}.so"),
            format!("lib{name}.so.1"),
            format!("lib{name}.so.0"),
        ]
    };
    // Follow the search order used by the pinned cubecl-hip-sys dynamic loader.
    let search_paths = ["ROCM_PATH", "HIP_PATH"]
        .into_iter()
        .filter_map(std::env::var_os)
        .flat_map(|path| {
            let path = PathBuf::from(path);
            [path.join("lib"), path]
        });
    let candidates = search_paths
        .flat_map(|path| names.iter().map(move |name| path.join(name)))
        .chain(names.iter().map(PathBuf::from));
    for candidate in candidates {
        if let Ok(library) = unsafe { Library::new(candidate) } {
            return Ok(library);
        }
    }
    Err(format!("ROCm runtime library {name} is unavailable"))
}

// Symbols invoked by the pinned CubeCL 0.11 HIP runtime, including graph and allocation paths.
const REQUIRED_SYMBOLS: &[&str] = &[
    "hipDeviceGetPCIBusId",
    "hipEventCreateWithFlags",
    "hipEventDestroy",
    "hipEventElapsedTime",
    "hipEventRecord",
    "hipEventSynchronize",
    "hipFree",
    "hipFreeHost",
    "hipGetDeviceCount",
    "hipGetDevicePropertiesR0600",
    "hipGraphDestroy",
    "hipGraphExecDestroy",
    "hipGraphGetNodes",
    "hipGraphInstantiate",
    "hipGraphLaunch",
    "hipGraphNodeGetType",
    "hipGraphUpload",
    "hipHostMalloc",
    "hipMalloc",
    "hipMemGetInfo",
    "hipMemcpy2DAsync",
    "hipMemcpyDtoDAsync",
    "hipMemcpyDtoHAsync",
    "hipMemcpyHtoDAsync",
    "hipModuleGetFunction",
    "hipModuleLaunchKernel",
    "hipModuleLoadData",
    "hipSetDevice",
    "hipSetDeviceFlags",
    "hipStreamBeginCapture",
    "hipStreamCreateWithFlags",
    "hipStreamDestroy",
    "hipStreamEndCapture",
    "hipStreamSynchronize",
    "hipStreamWaitEvent",
    "hiprtcCompileProgram",
    "hiprtcCreateProgram",
    "hiprtcDestroyProgram",
    "hiprtcGetCode",
    "hiprtcGetCodeSize",
    "hiprtcGetProgramLog",
    "hiprtcGetProgramLogSize",
];
