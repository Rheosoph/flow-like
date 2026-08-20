//! Media processing catalog for Flow-Like
//!
//! This crate contains media processing nodes:
//! - Image processing and transformation
//! - Bit manipulation

use std::sync::Arc;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};

pub mod audio;
pub mod bit;
pub mod document;
pub mod image;
pub mod video;

include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}

/// The `custom:vertex` media providers fall back to Google application-default
/// credentials when the provider carries neither an access token nor a
/// service-account key — i.e. to the host process's own identity. Refuse that
/// server-side before any provider dispatch; the resolved bearer token would
/// otherwise be sent to a flow-supplied `endpoint`.
pub(crate) fn ensure_vertex_credentials_explicit(
    context: &flow_like::flow::execution::context::ExecutionContext,
    provider: &flow_like_model_provider::provider::ModelProvider,
) -> flow_like_types::Result<()> {
    if provider.provider_name != "custom:vertex" {
        return Ok(());
    }
    let has_explicit = provider.params.as_ref().is_some_and(|params| {
        [
            "access_token",
            "service_account_json",
            "service_account_key",
        ]
        .iter()
        .any(|key| {
            params
                .get(*key)
                .and_then(|value| value.as_str())
                .is_some_and(|value| !value.trim().is_empty())
        })
    });
    if has_explicit {
        return Ok(());
    }
    context
        .execution_environment()
        .ensure_no_ambient_credentials("custom:vertex", "application_default")
}
