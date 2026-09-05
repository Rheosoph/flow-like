#![recursion_limit = "256"]

//! A2UI schemas, state helpers, and protobuf conversions shared by runtime and editor consumers.
//! This crate has no dependency on the Flow runtime, model providers, or storage clients.

pub mod component;
pub mod components;
pub mod data;
pub mod element_cache;
pub mod element_key;
pub mod element_ref;
pub mod id_refs;
pub mod page_remap;
pub mod page_targets;
pub mod protobuf;
pub mod serde_helpers;
pub mod style;
pub mod surface;
pub mod widget;

pub use component::*;
pub use components::*;
pub use data::*;
pub use element_cache::*;
pub use element_key::*;
pub use element_ref::*;
pub use style::*;
pub use surface::*;
pub use widget::*;
