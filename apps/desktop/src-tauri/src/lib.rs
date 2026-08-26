//! Mobile entry library for the Flow-Like Tauri application.
//!
//! Tauri's mobile toolchain builds this package with `--lib`: iOS links the
//! resulting `libapp.a`, while Android loads the generated shared library. On
//! desktop, Tauri builds the binary target, which includes the shared
//! application implementation directly. The host library intentionally stays
//! empty so Cargo does not archive the complete application into a multi-GB
//! `staticlib` and `cdylib` during normal desktop development.

#![cfg(mobile)]

include!("application.rs");
