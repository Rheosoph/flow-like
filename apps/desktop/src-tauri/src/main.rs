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
    run()
}
