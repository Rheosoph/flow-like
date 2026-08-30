//! Lightweight entry point for bare workspace Cargo commands.
//!
//! Cargo cannot attach a feature set to `workspace.default-members`. Depending
//! on core through the workspace baseline lets `cargo check` compile the shared
//! AST/types/core editing surface without changing `flow-like`'s public default
//! features or pulling in the database runtime.

pub use flow_like as core;
