//! Runtime-independent wire schemas for Flow-Like WASM packages and widgets.

#[cfg(feature = "nodes")]
extern crate flow_like_runtime as flow_like;

pub mod limits;

#[cfg(feature = "manifest")]
pub mod manifest;
pub mod runtime;
pub mod widget;

#[cfg(feature = "bundle")]
pub mod widget_bundle;
