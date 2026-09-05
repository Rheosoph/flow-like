//! Web catalog nodes and connector APIs.

use std::{path::Path, sync::Arc};

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};
pub use flow_like_catalog_web_discord::discord;
pub use flow_like_catalog_web_mail::mail;
pub use flow_like_catalog_web_telegram::telegram;

pub mod http;
pub mod web;

#[allow(dead_code)]
mod local_registry {
    include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));
}

/// Collect every web node in the historical source traversal order.
pub fn collect_nodes() -> Vec<Arc<dyn NodeLogic>> {
    let mut entries = local_registry::collect_node_entries();
    entries.extend(flow_like_catalog_web_telegram::collect_node_entries());
    entries.extend(flow_like_catalog_web_discord::collect_node_entries());
    entries.extend(flow_like_catalog_web_mail::collect_node_entries());
    entries.sort_by(|left, right| Path::new(left.0).cmp(Path::new(right.0)));
    entries.into_iter().map(|(_, node)| node).collect()
}

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
