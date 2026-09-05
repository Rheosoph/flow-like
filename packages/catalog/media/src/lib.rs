//! Media catalog assembled from independent audio, document, image, and video crates.

extern crate flow_like_runtime as flow_like;

pub use flow_like_catalog_core::{NodeConstructor, NodeLogic, register_node};
use std::{path::Path, sync::Arc};
pub mod bit;
pub use flow_like_catalog_media_audio::audio;
pub use flow_like_catalog_media_document::document;
pub use flow_like_catalog_media_image::image;
pub use flow_like_catalog_media_video::video;

#[allow(dead_code)]
mod local_registry {
    include!(concat!(env!("OUT_DIR"), "/node_registry.rs"));
}

#[doc(hidden)]
pub fn collect_node_entries() -> Vec<(&'static str, Arc<dyn NodeLogic>)> {
    let mut entries = local_registry::collect_node_entries();
    entries.extend(flow_like_catalog_media_audio::collect_node_entries());
    entries.extend(flow_like_catalog_media_document::collect_node_entries());
    entries.extend(flow_like_catalog_media_image::collect_node_entries());
    entries.extend(flow_like_catalog_media_video::collect_node_entries());
    entries.sort_by(|left, right| Path::new(left.0).cmp(Path::new(right.0)));
    entries
}

pub fn collect_nodes() -> Vec<Arc<dyn NodeLogic>> {
    collect_node_entries()
        .into_iter()
        .map(|(_, node)| node)
        .collect()
}

pub fn get_catalog() -> Vec<Arc<dyn NodeLogic>> {
    collect_nodes()
}
