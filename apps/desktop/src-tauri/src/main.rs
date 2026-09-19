// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Compile the shared application implementation into the desktop executable
// directly. The package library is reserved for Tauri's mobile entry point, so
// its required `staticlib`/`cdylib` outputs remain tiny on host builds.
include!("application.rs");

#[cfg(not(any(all(target_os = "macos", target_arch = "aarch64"), target_os = "ios")))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() {
    #[cfg(target_os = "linux")]
    remove_webkit_adaptive_streaming_overrides();
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.iter().any(|arg| {
        arg == "--flowpilot-workflow-benchmark"
            || arg
                .to_str()
                .is_some_and(|arg| arg.starts_with("--flowpilot-workflow-benchmark="))
    }) {
        #[cfg(all(debug_assertions, desktop))]
        std::process::exit(functions::ai::copilot::run_workflow_benchmark_cli(args));
        #[cfg(not(all(debug_assertions, desktop)))]
        {
            eprintln!("Workflow benchmark CLI requires a development build.");
            std::process::exit(2);
        }
    }
    run()
}

/// WebKitGTK's GStreamer HLS and DASH demuxers fetch playlists and segments
/// outside WebKit's loader, and so outside the widget document CSP; these
/// variables would promote them. Runs before any thread starts, so mutating
/// the environment cannot race a reader.
#[cfg(target_os = "linux")]
fn remove_webkit_adaptive_streaming_overrides() {
    for name in [
        "WEBKIT_GST_ENABLE_HLS_SUPPORT",
        "WEBKIT_GST_ENABLE_DASH_SUPPORT",
    ] {
        unsafe { std::env::remove_var(name) };
    }
}
