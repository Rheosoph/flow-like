//! Build-time compatibility contract shared by registry and runtime crates.

/// Wasmtime's serialization compatibility version.
///
/// This is generated from the workspace's `wasmtime` dependency, so registry
/// platform keys cannot drift from the runtime that consumes AOT artifacts.
pub const WASMTIME_MAJOR_VERSION: &str = env!("FLOW_LIKE_WASMTIME_MAJOR_VERSION");

/// Backwards-compatible name for the artifact compatibility version.
pub const WASMTIME_VERSION: &str = WASMTIME_MAJOR_VERSION;
